//! The background loop that keeps this till caught up with its peers.
//!
//! The UI never waits for a network. A worker thread owns its own SQLite
//! connection, wakes every half minute while sync is on, and reports what
//! happened through a channel. Turning sync off parks the thread instead of
//! stopping it, so the next toggle does not pay for a new connection.
//!
//! The worker reads the peer list from `sync_peers` on every pass. Adding a till
//! in Settings therefore needs no message to the worker; it is picked up on the
//! next tick or immediately by "Sync now".

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use storage::Store;

use crate::db;
use crate::sync::client::HttpPeer;
use crate::sync::protocol::{CatalogSnapshot, PushRequest, PushResponse};
use crate::sync::worker::{PeerCursor, Replica, sync_once};

/// How often a healthy till looks for new sales.
const POLL: Duration = Duration::from_secs(30);

/// The longest a run of failures may stretch the interval to. A till that has
/// lost the network should not knock every five seconds for an hour.
const MAX_BACKOFF: Duration = Duration::from_secs(300);

/// How long a parked worker waits before checking whether it was switched on.
const IDLE: Duration = Duration::from_secs(3600);

/// Settings that live in `settings.json` but arrive inside a catalog snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CatalogSettings {
    pub tax_rate_percent: i64,
}

/// What the Sync card and the status bar show.
#[derive(Clone, Debug, Default)]
pub struct Status {
    pub enabled: bool,
    pub peers: usize,
    pub syncing: bool,
    pub last_synced: Option<Instant>,
    pub error: Option<String>,
}

/// Something the UI thread has to hear about.
#[derive(Clone, Debug)]
pub enum Event {
    Status(Status),
    /// A replica accepted the owner's catalog; the tax rate came with it.
    CatalogSettings(CatalogSettings),
}

enum Command {
    Enabled(bool),
    Now,
    Stop,
}

/// The UI's handle on the worker.
pub struct Service {
    tx: Sender<Command>,
    rx: Receiver<Event>,
    enabled: bool,
    status: Status,
    worker: Option<JoinHandle<()>>,
}

impl Service {
    pub fn new(db_path: PathBuf) -> Self {
        let (tx, commands) = mpsc::channel();
        let (events, rx) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("sync-worker".to_owned())
            .spawn(move || run(db_path, commands, events))
            .ok();
        if worker.is_none() {
            tracing::error!("could not start the sync worker");
        }
        Self {
            tx,
            rx,
            enabled: false,
            status: Status::default(),
            worker,
        }
    }

    /// Turn the worker on or off. Cheap when nothing changed.
    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled == enabled {
            return;
        }
        self.enabled = enabled;
        if !enabled {
            self.status = Status::default();
        }
        let _ = self.tx.send(Command::Enabled(enabled));
    }

    /// Ask for a pass now, without waiting for the poll interval.
    pub fn now(&mut self) {
        let _ = self.tx.send(Command::Now);
    }

    /// Apply what the worker has said since the last frame.
    ///
    /// Catalog settings are returned to the caller rather than written here:
    /// only the UI thread owns `settings.json`, and a background thread writing
    /// it would race the operator's next keystroke.
    pub fn drain(&mut self) -> Vec<Event> {
        self.rx.try_iter().collect()
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    /// Record a status event. Catalog events are left to the caller.
    pub fn observe(&mut self, event: &Event) {
        if let Event::Status(status) = event {
            self.status = status.clone();
        }
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Stop);
        // Detached rather than joined: a peer that has gone silent can hold the
        // worker in a ten second read, and the process is leaving anyway.
        self.worker.take();
    }
}

fn run(db_path: PathBuf, commands: Receiver<Command>, events: Sender<Event>) {
    let mut store = match open_store(&db_path) {
        Ok(store) => store,
        Err(error) => {
            let _ = events.send(Event::Status(Status {
                enabled: true,
                error: Some(format!("Could not open the sync database: {error}")),
                ..Status::default()
            }));
            return;
        }
    };

    let mut enabled = false;
    let mut backoff = POLL;
    let mut last_synced: Option<Instant> = None;

    loop {
        let wait = if enabled { backoff } else { IDLE };
        match commands.recv_timeout(wait) {
            Ok(Command::Stop) => return,
            Ok(Command::Enabled(value)) => {
                enabled = value;
                backoff = POLL;
                if !enabled {
                    let _ = events.send(Event::Status(Status::default()));
                    continue;
                }
            }
            Ok(Command::Now) => backoff = POLL,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }

        if !enabled {
            continue;
        }

        let outcome = sync_all(&mut store, &events, &mut last_synced);
        // No peers is not a failure: a shop may turn sync on before it adds the
        // second till, and that should not start a five minute backoff.
        if outcome.ok > 0 || outcome.peers == 0 {
            backoff = POLL;
        } else {
            backoff = (backoff * 2).min(MAX_BACKOFF);
        }
    }
}

struct Outcome {
    peers: usize,
    ok: usize,
}

fn sync_all(
    store: &mut Store,
    events: &Sender<Event>,
    last_synced: &mut Option<Instant>,
) -> Outcome {
    let peers = match db::sync_peers(store) {
        Ok(peers) => peers,
        Err(error) => {
            let _ = events.send(Event::Status(Status {
                enabled: true,
                peers: 0,
                syncing: false,
                last_synced: *last_synced,
                error: Some(format!("Could not read the peer list: {error}")),
            }));
            return Outcome { peers: 0, ok: 0 };
        }
    };

    let peers: Vec<db::SyncPeer> = peers
        .into_iter()
        .filter(|peer| peer.kind == "till" && peer.host.is_some() && peer.port.is_some())
        .collect();

    let _ = events.send(Event::Status(Status {
        enabled: true,
        peers: peers.len(),
        syncing: true,
        last_synced: *last_synced,
        error: None,
    }));

    let mut ok = 0;
    let mut first_error = None;
    for peer in &peers {
        match sync_peer(store, peer, events) {
            Ok(()) => ok += 1,
            Err(error) => {
                let message = error.to_string();
                let _ = db::set_sync_error(store, &peer.id, &message);
                if first_error.is_none() {
                    first_error = Some(format!("{}: {message}", peer.display_name()));
                }
            }
        }
    }

    if ok > 0 {
        *last_synced = Some(Instant::now());
    }
    let _ = events.send(Event::Status(Status {
        enabled: true,
        peers: peers.len(),
        syncing: false,
        last_synced: *last_synced,
        error: first_error,
    }));

    Outcome {
        peers: peers.len(),
        ok,
    }
}

fn sync_peer(store: &mut Store, peer: &db::SyncPeer, events: &Sender<Event>) -> Result<()> {
    let (Some(host), Some(port)) = (peer.host.as_deref(), peer.port) else {
        anyhow::bail!("the peer has no address");
    };

    let link = HttpPeer::new(host, port, &peer.token);
    let mut cursor = PeerCursor {
        since_seq: peer.last_pulled_seq,
        catalog_version: peer.last_catalog_version,
    };

    let catalog = {
        let mut replica = ReplicaWithSettings {
            store,
            catalog: None,
        };
        sync_once(&mut replica, &link, &mut cursor)?;
        replica.catalog
    };

    // The cursor is saved only after every page applied. A crash between the
    // apply and this write replays the page, and per-sale transactions make
    // that harmless.
    db::save_sync_cursor(store, &peer.id, cursor.since_seq, cursor.catalog_version)?;
    if let Some(settings) = catalog {
        let _ = events.send(Event::CatalogSettings(settings));
    }
    Ok(())
}

fn open_store(path: &Path) -> Result<Store> {
    let store = Store::open(path)?;
    // Ingestion shares the till's database with checkout. `FULL` costs one
    // fsync per sale on this connection, and 500 ms is long enough for the
    // checkout writer to finish rather than failing a peer's batch.
    store
        .conn()
        .execute_batch("PRAGMA busy_timeout = 500; PRAGMA synchronous = FULL;")
        .context("configuring the sync database connection")?;
    Ok(store)
}

/// The local store plus the settings it learned from the owner's snapshot.
struct ReplicaWithSettings<'a> {
    store: &'a mut Store,
    catalog: Option<CatalogSettings>,
}

impl Replica for ReplicaWithSettings<'_> {
    fn apply_sales(&mut self, request: &PushRequest) -> Result<PushResponse> {
        crate::sync::ingest::ingest_sales(self.store, request)
    }

    fn apply_catalog(&mut self, snapshot: &CatalogSnapshot) -> Result<()> {
        crate::db::apply_catalog_snapshot(self.store, snapshot)?;
        self.catalog = Some(CatalogSettings {
            tax_rate_percent: snapshot.tax_rate_percent,
        });
        Ok(())
    }
}
