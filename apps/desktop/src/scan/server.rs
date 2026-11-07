//! The local HTTP listener behind phone scanning.
//!
//! One thread, one route that matters, and a token check before the body is
//! read. `tiny_http` is synchronous on purpose: a till sends a handful of
//! scans a minute, and an async runtime would be more machinery than the
//! problem needs.

use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};

use tiny_http::{Header, Request, Response, Server};

use super::ScanEvent;
use super::protocol::{self, MAX_BODY_BYTES};

const TOKEN_HEADER: &str = "X-Till-Token";

pub struct Listener {
    stop: Arc<AtomicBool>,
    port: u16,
    /// Kept so [`Listener::stop`] can unblock a worker stuck in the blocking
    /// [`Server::recv`] below, letting it observe `stop` and exit without
    /// waiting for the next request.
    server: Arc<Server>,
    worker: Option<JoinHandle<()>>,
}

impl Listener {
    pub fn start(port: u16, token: String, tx: Sender<ScanEvent>) -> Result<Self, String> {
        let server = Arc::new(
            Server::http(("0.0.0.0", port))
                .map_err(|error| format!("Could not listen on port {port}: {error}"))?,
        );
        let bound = server
            .server_addr()
            .to_ip()
            .map(|address| address.port())
            .unwrap_or(port);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker_server = Arc::clone(&server);

        let worker = thread::Builder::new()
            .name("scan-listener".to_owned())
            .spawn(move || run(worker_server, token, tx, worker_stop))
            .map_err(|error| format!("Could not start the scanner listener: {error}"))?;

        tracing::info!(port = bound, "barcode listener started");
        Ok(Self {
            stop,
            port: bound,
            server,
            worker: Some(worker),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Wake a worker blocked in recv() so it observes `stop` and exits
        // instead of waiting for the next request (which may never come).
        self.server.unblock();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        tracing::info!("barcode listener stopped");
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.server.unblock();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Blocks until an HTTP request arrives, so a scan is handled the instant it
/// reaches the till instead of waiting for a poll interval.
fn run(server: Arc<Server>, token: String, tx: Sender<ScanEvent>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Relaxed) {
        match server.recv() {
            Ok(request) => handle(request, &token, &tx),
            Err(error) => {
                // `recv()` returns an error when the server is unblocked
                // (shutdown) or the socket closes. Only log it when we were
                // not asked to stop.
                if !stop.load(Ordering::Relaxed) {
                    tracing::warn!(error = %error, "barcode listener could not receive a request");
                }
                break;
            }
        }
    }
}

fn handle(mut request: Request, token: &str, tx: &Sender<ScanEvent>) {
    if request.method().as_str() != "POST" {
        return reply(request, 405, r#"{"ok":false,"error":"method not allowed"}"#);
    }

    let path = request
        .url()
        .split('?')
        .next()
        .unwrap_or_default()
        .to_owned();

    if path != "/v1/scan" && path != "/v1/ping" {
        return reply(request, 404, r#"{"ok":false,"error":"not found"}"#);
    }

    if !authorized(&request, token) {
        return reply(request, 401, r#"{"ok":false,"error":"unauthorized"}"#);
    }

    if path == "/v1/ping" {
        return reply(request, 200, r#"{"ok":true,"service":"stride-pos"}"#);
    }

    if request
        .body_length()
        .is_some_and(|length| length > MAX_BODY_BYTES)
    {
        return reply(request, 413, r#"{"ok":false,"error":"scan too large"}"#);
    }

    // Read one byte past the limit so a missing or lying Content-Length cannot
    // turn into an unbounded allocation.
    let mut body = Vec::new();
    let read = {
        let mut limited = request.as_reader().take((MAX_BODY_BYTES + 1) as u64);
        limited.read_to_end(&mut body)
    };
    if read.is_err() {
        return reply(
            request,
            400,
            r#"{"ok":false,"error":"could not read scan"}"#,
        );
    }
    if body.len() > MAX_BODY_BYTES {
        return reply(request, 413, r#"{"ok":false,"error":"scan too large"}"#);
    }

    match protocol::parse(&body) {
        Ok(event) => {
            if tx.send(event).is_err() {
                reply(
                    request,
                    503,
                    r#"{"ok":false,"error":"the till is shutting down"}"#,
                );
            } else {
                reply(request, 200, r#"{"ok":true}"#);
            }
        }
        Err(error) => {
            let body = serde_json::json!({
                "ok": false,
                "error": error.message(),
            })
            .to_string();
            reply(request, 400, &body);
        }
    }
}

fn authorized(request: &Request, expected: &str) -> bool {
    request
        .headers()
        .iter()
        .find(|header| header.field.equiv(TOKEN_HEADER))
        .is_some_and(|header| token_matches(expected, header.value.as_str()))
}

/// Compare without returning early. The token never leaves the LAN, but a
/// length-independent comparison costs nothing and removes the obvious oracle.
///
/// The header may carry the code the way a person would type it; the comparison
/// is always between two canonical tokens.
fn token_matches(expected: &str, supplied: &str) -> bool {
    let (Some(expected), Some(supplied)) = (
        super::normalize_token(expected),
        super::normalize_token(supplied),
    ) else {
        return false;
    };
    let expected = expected.as_bytes();
    let supplied = supplied.as_bytes();
    if expected.len() != supplied.len() {
        return false;
    }
    let mut difference = 0u8;
    for (left, right) in expected.iter().zip(supplied) {
        difference |= left ^ right;
    }
    difference == 0
}

fn reply(request: Request, code: u16, body: &str) {
    let mut response = Response::from_string(body).with_status_code(code);
    if let Ok(header) = Header::from_bytes("Content-Type", "application/json; charset=utf-8") {
        response.add_header(header);
    }
    if let Ok(header) = Header::from_bytes("Cache-Control", "no-store") {
        response.add_header(header);
    }
    if let Err(error) = request.respond(response) {
        tracing::debug!(error = %error, "could not write a scanner response");
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::net::TcpStream;
    use std::sync::mpsc::{self, Receiver};
    use std::time::Duration;

    use super::*;
    use crate::scan::ScanAction;

    fn start() -> (Listener, Receiver<ScanEvent>, u16, String) {
        let token = "7QK4M2X9AB".to_owned();
        let (tx, rx) = mpsc::channel();
        let listener = Listener::start(0, token.clone(), tx).unwrap();
        let port = listener.port();
        (listener, rx, port, token)
    }

    fn request(
        port: u16,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: &str,
    ) -> (u16, String) {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let auth = token
            .map(|token| format!("{TOKEN_HEADER}: {token}\r\n"))
            .unwrap_or_default();
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(request.as_bytes()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let status = response
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse().ok())
            .unwrap();
        (status, response)
    }

    #[test]
    fn a_token_bearing_scan_reaches_the_till() {
        let (_listener, rx, port, token) = start();
        let (status, _) = request(
            port,
            "POST",
            "/v1/scan",
            Some(&token),
            r#"{"barcode":"4006381333931","action":"add"}"#,
        );
        assert_eq!(status, 200);
        let event = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(event.barcode, "4006381333931");
        assert_eq!(event.action, ScanAction::Add);
    }

    #[test]
    fn a_scan_without_the_token_is_refused_and_never_reaches_the_till() {
        let (_listener, rx, port, _token) = start();
        let (status, _) = request(
            port,
            "POST",
            "/v1/scan",
            Some("7QK4M2X9ZZ"),
            r#"{"barcode":"4006381333931"}"#,
        );
        assert_eq!(status, 401);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn the_token_may_arrive_lower_case_or_with_the_display_dash() {
        let (_listener, rx, port, _token) = start();
        let (status, _) = request(port, "POST", "/v1/ping", Some("7qk4-m2x9ab"), "");
        assert_eq!(status, 200);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn the_default_action_is_sell() {
        let (_listener, rx, port, token) = start();
        let (status, _) = request(
            port,
            "POST",
            "/v1/scan",
            Some(&token),
            r#"{"barcode":"4006381333931"}"#,
        );
        assert_eq!(status, 200);
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(1)).unwrap().action,
            ScanAction::Sell
        );
    }

    #[test]
    fn a_malformed_scan_is_a_bad_request() {
        let (_listener, rx, port, token) = start();
        let (status, body) = request(port, "POST", "/v1/scan", Some(&token), r#"{"barcode":"#);
        assert_eq!(status, 400);
        assert!(body.contains("expected a JSON object"));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn ping_answers_without_creating_a_scan() {
        let (_listener, rx, port, token) = start();
        let (status, body) = request(port, "POST", "/v1/ping", Some(&token), "");
        assert_eq!(status, 200);
        assert!(body.contains("stride-pos"));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn only_post_is_accepted() {
        let (_listener, rx, port, token) = start();
        let (status, _) = request(port, "GET", "/v1/scan", Some(&token), "");
        assert_eq!(status, 405);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn token_comparison_needs_the_exact_value() {
        assert!(token_matches("7QK4M2X9AB", "7QK4M2X9AB"));
        assert!(token_matches("7QK4M2X9AB", "7qk4-m2x9ab"));
        assert!(!token_matches("7QK4M2X9AB", "7QK4M2X9AC"));
        assert!(!token_matches("7QK4M2X9AB", "7QK4M2X9A"));
        assert!(!token_matches("7QK4M2X9AB", ""));
    }

    #[test]
    fn the_listener_reports_the_port_it_actually_bound() {
        let (listener, _rx, port, _token) = start();
        assert_ne!(port, 0);
        assert_eq!(listener.port(), port);
    }
}
