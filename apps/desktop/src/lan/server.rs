//! The routes the shop's LAN listener answers.
//!
//! One port carries both phone scans and till-to-till replication. A request is
//! routed by path, then checked against the token that path requires:
//!
//! | path                | who may call it              |
//! |---------------------|------------------------------|
//! | `/v1/scan`          | the phone scanner            |
//! | `/v1/ping`          | either                       |
//! | `/v1/sync/sales`    | either (a phone pushes cash) |
//! | `/v1/sync/catalog`  | either (a phone reads prices)|
//! | `/v1/sync/pull`     | another till only            |
//!
//! The phone's token is not the till's token. A lost phone can ring up a sale
//! but cannot read the sales history of the shop.

use std::path::Path;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use lan_http::server::{Incoming, Reply, Server};
use storage::Store;

use crate::db;
use crate::scan::{ScanEvent, protocol as scan_protocol};
use crate::sync::ingest;
use crate::sync::protocol::{self, PullRequest, PullResponse, PushRequest};

/// The token header every client sends.
const TOKEN_HEADER: &str = "X-Till-Token";

/// The largest body the listener will read. The scan route is far smaller; the
/// cap here is the one a sync page must fit inside.
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

/// The most sales one pull page may carry, whatever the peer asks for.
const MAX_PAGE: usize = 1000;

/// A wrong token is cheap to try, so the listener makes each further wrong one
/// wait. Three free attempts tolerate a mistyped phone; after that, guessing a
/// 40-bit code slows to about two a second. A correctly paired device resets
/// the count and never waits.
const FREE_FAILURES: u32 = 3;
const FAILURE_DELAY: Duration = Duration::from_millis(400);

/// What the listener was started with.
pub struct Routes {
    pub scanner_token: String,
    pub sync_token: String,
    pub device_name: String,
    pub catalog_owner: bool,
    pub tax_rate_percent: i64,
}

/// A bound listener plus the database connection its handler owns.
pub struct Listener {
    server: Server,
}

impl Listener {
    /// Bind and start answering.
    ///
    /// The handler owns its own [`Store`]: the UI thread is writing checkouts
    /// while a peer is ingesting sales, and sharing one connection would put
    /// the network in the path of the next sale.
    pub fn start(
        port: u16,
        routes: Routes,
        db_path: &Path,
        scans: Sender<ScanEvent>,
    ) -> Result<Self, String> {
        let store = open_store(db_path)?;
        let mut handler = Handler {
            store,
            routes,
            scans,
            device_id: None,
            failures: 0,
        };

        let server = Server::bind(port, MAX_BODY_BYTES, move |incoming| {
            handler.handle(incoming)
        })
        .map_err(|error| format!("Could not listen on port {port}: {error}"))?;

        tracing::info!(port = server.port(), "LAN listener started");
        Ok(Self { server })
    }

    pub fn port(&self) -> u16 {
        self.server.port()
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        tracing::info!("LAN listener stopped");
    }
}

struct Handler {
    store: Store,
    routes: Routes,
    scans: Sender<ScanEvent>,
    /// `meta` holds the device id; reading it once is enough for the life of
    /// this listener, which restarts when Settings change anyway.
    device_id: Option<String>,
    /// Consecutive refusals since the last accepted token.
    failures: u32,
}

impl Handler {
    fn handle(&mut self, incoming: Incoming) -> Reply {
        match incoming.path.as_str() {
            "/v1/scan" => {
                if incoming.method != "POST" {
                    return method_not_allowed();
                }
                self.scan(incoming)
            }
            "/v1/ping" => {
                if incoming.method != "POST" {
                    return method_not_allowed();
                }
                self.ping(&incoming)
            }
            "/v1/sync/pull" => {
                if incoming.method != "POST" {
                    return method_not_allowed();
                }
                self.pull(&incoming)
            }
            "/v1/sync/sales" => {
                if incoming.method != "POST" {
                    return method_not_allowed();
                }
                self.push(&incoming)
            }
            "/v1/sync/catalog" => {
                if incoming.method != "POST" {
                    return method_not_allowed();
                }
                self.catalog(&incoming)
            }
            _ => Reply::json(404, r#"{"ok":false,"error":"not found"}"#),
        }
    }

    fn scan(&mut self, incoming: Incoming) -> Reply {
        if !self.authorized(&incoming, Scope::Scanner) {
            return unauthorized();
        }
        if incoming.body.len() > scan_protocol::MAX_BODY_BYTES {
            return Reply::json(413, r#"{"ok":false,"error":"scan too large"}"#);
        }

        match scan_protocol::parse(&incoming.body) {
            Ok(event) => {
                if self.scans.send(event).is_err() {
                    Reply::json(503, r#"{"ok":false,"error":"the till is shutting down"}"#)
                } else {
                    Reply::json(200, r#"{"ok":true}"#)
                }
            }
            Err(error) => fail(400, error.message()),
        }
    }

    fn ping(&mut self, incoming: &Incoming) -> Reply {
        if !self.authorized(incoming, Scope::Either) {
            return unauthorized();
        }
        let body = serde_json::json!({
            "ok": true,
            "service": "stride-pos",
            "protocol": protocol::PROTOCOL,
            "device_id": self.device_id(),
            "device_name": self.routes.device_name,
            "server_time": now(&self.store),
            "catalog_owner": self.routes.catalog_owner,
        });
        json(200, &body)
    }

    fn pull(&mut self, incoming: &Incoming) -> Reply {
        if !self.authorized(incoming, Scope::Sync) {
            return unauthorized();
        }

        let request: PullRequest = match serde_json::from_slice(&incoming.body) {
            Ok(request) => request,
            Err(_) => return fail(400, "expected a pull request"),
        };
        if let Err(reason) = check_protocol(request.protocol) {
            return fail(400, reason);
        }

        let since = request.since_seq.max(0);
        let limit = request.limit.min(MAX_PAGE);
        let (sales, next_seq, has_more) = match db::pull_sales(&self.store, since, limit) {
            Ok(page) => page,
            Err(error) => {
                tracing::error!(error = %error, "could not read the change feed");
                return fail(500, "could not read the change feed");
            }
        };

        let body = PullResponse {
            protocol: protocol::PROTOCOL,
            device_id: self.device_id(),
            device_name: self.routes.device_name.clone(),
            catalog_owner: self.routes.catalog_owner,
            catalog_version: catalog_version(&self.store),
            sales,
            next_seq,
            has_more,
            server_time: now(&self.store),
        };
        json(200, &body)
    }

    fn push(&mut self, incoming: &Incoming) -> Reply {
        if !self.authorized(incoming, Scope::Either) {
            return unauthorized();
        }

        let request: PushRequest = match serde_json::from_slice(&incoming.body) {
            Ok(request) => request,
            Err(_) => return fail(400, "expected sales to ingest"),
        };
        if let Err(reason) = check_protocol(request.protocol) {
            return fail(400, reason);
        }

        match ingest::ingest_sales(&self.store, &request) {
            Ok(response) => json(200, &response),
            Err(error) => {
                // A database failure is not the sender's fault: answering 200
                // would tell a phone to delete an outbox that was never saved.
                tracing::error!(error = %error, "could not ingest sales");
                fail(500, "could not save the sales")
            }
        }
    }

    fn catalog(&mut self, incoming: &Incoming) -> Reply {
        if !self.authorized(incoming, Scope::Either) {
            return unauthorized();
        }

        match db::catalog_snapshot(&self.store, self.routes.tax_rate_percent) {
            Ok(snapshot) => json(200, &snapshot),
            Err(error) => {
                tracing::error!(error = %error, "could not read the catalog");
                fail(500, "could not read the catalog")
            }
        }
    }

    fn authorized(&mut self, incoming: &Incoming, scope: Scope) -> bool {
        let allowed = incoming
            .header(TOKEN_HEADER)
            .is_some_and(|supplied| match scope {
                Scope::Scanner => lan_http::token::matches(&self.routes.scanner_token, supplied),
                Scope::Sync => lan_http::token::matches(&self.routes.sync_token, supplied),
                Scope::Either => {
                    lan_http::token::matches(&self.routes.scanner_token, supplied)
                        || lan_http::token::matches(&self.routes.sync_token, supplied)
                }
            });

        if allowed {
            self.failures = 0;
            return true;
        }

        self.failures = self.failures.saturating_add(1);
        let delay = refusal_delay(self.failures);
        if !delay.is_zero() {
            thread::sleep(delay);
        }
        false
    }

    fn device_id(&mut self) -> String {
        if let Some(id) = &self.device_id {
            return id.clone();
        }
        match db::ensure_device_id(&self.store) {
            Ok(id) => {
                self.device_id = Some(id.clone());
                id
            }
            Err(error) => {
                tracing::warn!(error = %error, "could not read this till's device id");
                String::new()
            }
        }
    }
}

/// Which secret a route requires.
enum Scope {
    Scanner,
    Sync,
    Either,
}

fn open_store(path: &Path) -> Result<Store, String> {
    let store = Store::open(path)
        .map_err(|error| format!("Could not open the database for the listener: {error}"))?;
    // A peer's batch is allowed to wait for checkout, but not forever; and a
    // sale accepted from a phone is already on the phone, so a power cut must
    // not lose the copy the till just confirmed.
    store
        .conn()
        .execute_batch("PRAGMA busy_timeout = 500; PRAGMA synchronous = FULL;")
        .map_err(|error| format!("Could not configure the listener database: {error}"))?;
    Ok(store)
}

/// The pause a refused request earns. Pure so the policy can be tested without
/// making a timing assertion against a real socket.
fn refusal_delay(failures: u32) -> Duration {
    if failures > FREE_FAILURES {
        FAILURE_DELAY
    } else {
        Duration::ZERO
    }
}

fn check_protocol(protocol: i64) -> Result<(), &'static str> {
    if protocol == 0 || protocol == protocol::PROTOCOL {
        Ok(())
    } else {
        Err("this till does not speak that sync protocol")
    }
}

fn catalog_version(store: &Store) -> i64 {
    db::meta(store, "catalog_version")
        .and_then(|value| value.parse().ok())
        .unwrap_or(1)
}

fn now(store: &Store) -> String {
    store
        .conn()
        .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now')", [], |row| {
            row.get(0)
        })
        .unwrap_or_default()
}

fn json(status: u16, value: &impl serde::Serialize) -> Reply {
    match serde_json::to_vec(value) {
        Ok(body) => Reply::json(status, body),
        Err(error) => {
            tracing::error!(error = %error, "could not encode a listener response");
            fail(500, "could not encode the response")
        }
    }
}

fn fail(status: u16, message: &str) -> Reply {
    let body = serde_json::json!({ "ok": false, "error": message });
    Reply::json(status, body.to_string())
}

fn unauthorized() -> Reply {
    Reply::json(401, r#"{"ok":false,"error":"unauthorized"}"#)
}

fn method_not_allowed() -> Reply {
    Reply::json(405, r#"{"ok":false,"error":"method not allowed"}"#)
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::net::TcpStream;
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;
    use crate::scan::ScanAction;

    struct Fixture {
        path: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "stride-lan-test-{}-{}.db",
                std::process::id(),
                uuid::Uuid::now_v7()
            ));
            let mut store = Store::open(&path).expect("open");
            store.migrate(db::MIGRATIONS).expect("migrate");
            Self { path }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", self.path.display()));
            }
        }
    }

    const SCANNER: &str = "7QK4M2X9AB";
    const SYNC: &str = "3ZXC5VBN7K";

    fn start() -> (Fixture, Listener, mpsc::Receiver<ScanEvent>, u16) {
        let fixture = Fixture::new();
        let (tx, rx) = mpsc::channel();
        let listener = Listener::start(
            0,
            Routes {
                scanner_token: SCANNER.to_owned(),
                sync_token: SYNC.to_owned(),
                device_name: "Front counter".to_owned(),
                catalog_owner: true,
                tax_rate_percent: 16,
            },
            &fixture.path,
            tx,
        )
        .expect("listen");
        let port = listener.port();
        (fixture, listener, rx, port)
    }

    fn request(port: u16, path: &str, token: Option<&str>, body: &str) -> (u16, String) {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("timeout");
        let auth = token
            .map(|token| format!("{TOKEN_HEADER}: {token}\r\n"))
            .unwrap_or_default();
        let request = format!(
            "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(request.as_bytes()).expect("write");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read");
        let status = response
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse().ok())
            .expect("status");
        (status, response)
    }

    #[test]
    fn a_token_bearing_scan_reaches_the_till() {
        let (_fixture, _listener, rx, port) = start();
        let (status, _) = request(
            port,
            "/v1/scan",
            Some(SCANNER),
            r#"{"barcode":"4006381333931","action":"add"}"#,
        );
        assert_eq!(status, 200);
        let event = rx.recv_timeout(Duration::from_secs(1)).expect("scan");
        assert_eq!(event.barcode, "4006381333931");
        assert_eq!(event.action, ScanAction::Add);
    }

    #[test]
    fn a_scan_without_the_token_is_refused() {
        let (_fixture, _listener, rx, port) = start();
        let (status, _) = request(
            port,
            "/v1/scan",
            Some(SYNC),
            r#"{"barcode":"4006381333931"}"#,
        );
        assert_eq!(status, 401);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn ping_names_the_till_and_its_catalog_role() {
        let (_fixture, _listener, _rx, port) = start();
        let (status, body) = request(port, "/v1/ping", Some(SCANNER), "");
        assert_eq!(status, 200);
        assert!(body.contains("stride-pos"), "{body}");
        assert!(body.contains("Front counter"), "{body}");
        assert!(body.contains("\"catalog_owner\":true"), "{body}");
    }

    #[test]
    fn a_phone_may_read_the_catalog_but_not_the_sales_feed() {
        let (_fixture, _listener, _rx, port) = start();
        let (catalog, _) = request(port, "/v1/sync/catalog", Some(SCANNER), "");
        assert_eq!(catalog, 200);

        let (pull, _) = request(
            port,
            "/v1/sync/pull",
            Some(SCANNER),
            r#"{"since_seq":0,"limit":10}"#,
        );
        assert_eq!(pull, 401);
    }

    #[test]
    fn another_till_may_read_the_sales_feed() {
        let (_fixture, _listener, _rx, port) = start();
        let (status, body) = request(
            port,
            "/v1/sync/pull",
            Some(SYNC),
            r#"{"since_seq":0,"limit":10}"#,
        );
        assert_eq!(status, 200);
        assert!(body.contains("\"has_more\":false"), "{body}");
        assert!(body.contains("\"catalog_version\":"), "{body}");
    }

    #[test]
    fn a_phone_can_offer_its_offline_cash() {
        let (_fixture, _listener, _rx, port) = start();
        let (status, body) = request(port, "/v1/sync/sales", Some(SCANNER), r#"{"sales":[]}"#);
        assert_eq!(status, 200);
        assert!(body.contains("\"accepted\":[]"), "{body}");
    }

    #[test]
    fn an_unknown_protocol_is_refused_without_touching_the_database() {
        let (_fixture, _listener, _rx, port) = start();
        let (status, body) = request(
            port,
            "/v1/sync/pull",
            Some(SYNC),
            r#"{"protocol":99,"since_seq":0,"limit":10}"#,
        );
        assert_eq!(status, 400);
        assert!(body.contains("sync protocol"), "{body}");
    }

    #[test]
    fn an_unknown_path_is_not_a_route() {
        let (_fixture, _listener, _rx, port) = start();
        let (status, _) = request(port, "/v1/admin", Some(SYNC), "");
        assert_eq!(status, 404);
    }

    #[test]
    fn only_a_repeated_refusal_earns_a_pause() {
        assert_eq!(refusal_delay(1), Duration::ZERO);
        assert_eq!(refusal_delay(FREE_FAILURES), Duration::ZERO);
        assert_eq!(refusal_delay(FREE_FAILURES + 1), FAILURE_DELAY);
        assert_eq!(refusal_delay(u32::MAX), FAILURE_DELAY);
    }

    #[test]
    fn a_wrong_token_is_still_refused_after_a_pause() {
        let (_fixture, _listener, _rx, port) = start();
        for _ in 0..FREE_FAILURES {
            let (status, _) = request(port, "/v1/ping", Some("WRONGTOKEN"), "");
            assert_eq!(status, 401);
        }

        let paused = std::time::Instant::now();
        let (status, _) = request(port, "/v1/ping", Some("WRONGTOKEN"), "");
        assert_eq!(status, 401);
        assert!(
            paused.elapsed() >= Duration::from_millis(300),
            "the fourth wrong token should wait"
        );
    }

    #[test]
    fn a_correct_token_is_never_slowed_by_earlier_mistakes() {
        let (_fixture, _listener, _rx, port) = start();
        for _ in 0..FREE_FAILURES {
            let (status, _) = request(port, "/v1/ping", Some("WRONGTOKEN"), "");
            assert_eq!(status, 401);
        }

        // A valid token clears the count, so a phone that mistyped once is
        // not punished for the rest of the day.
        let (status, _) = request(port, "/v1/ping", Some(SCANNER), "");
        assert_eq!(status, 200);
        let (status, _) = request(port, "/v1/ping", Some(SCANNER), "");
        assert_eq!(status, 200);
    }
}
