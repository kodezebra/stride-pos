//! The one listener on the shop's local network.
//!
//! Phone scanning and till-to-till sync are two clients of the same socket.
//! They share a port, a token header and a body cap, and keeping one listener
//! means a shop never has to open two firewall ports or explain why two
//! addresses exist.
//!
//! The listener runs for the life of the app while either feature is on, not
//! per request: a till that opened its port only when Settings was visible
//! would be unreachable exactly when a phone needs it.
//!
//! [`ScanStatus`] is reused as the listener's state. From the operator's side
//! "Listening on port 47831" is one fact, whether a scan or a peer arrives.

pub mod server;

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

use crate::scan::{ScanConfig, ScanEvent, ScanStatus};
use crate::sync::SyncConfig;

/// Everything the running listener needs to know.
///
/// A change to any field restarts the listener: tokens and the device name are
/// copied into the request handler, while catalog data and cursors are read
/// from the database on every request.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Config {
    port: u16,
    scanner_token: String,
    sync_token: String,
    device_name: String,
    catalog_owner: bool,
    tax_rate_percent: i64,
}

/// The UI's handle on the listener.
pub struct Service {
    tx: Sender<ScanEvent>,
    rx: Receiver<ScanEvent>,
    listener: Option<server::Listener>,
    /// The configuration the listener was started with. `None` before the first
    /// frame; equal configs do not restart a healthy listener.
    config: Option<Config>,
    status: ScanStatus,
    db_path: PathBuf,
}

impl Service {
    pub fn new(db_path: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx,
            listener: None,
            config: None,
            status: ScanStatus::Off,
            db_path,
        }
    }

    /// Bring the listener in line with Settings. Cheap when nothing changed.
    pub fn sync(&mut self, scanner: &ScanConfig, sync: &SyncConfig, tax_rate_percent: i64) {
        let Some((scanner_token, sync_token)) = canon_tokens(scanner, sync) else {
            self.stop_listener();
            self.config = None;
            self.status = ScanStatus::Error(
                "A pairing code is missing. Restart the till to make a new one.".to_owned(),
            );
            return;
        };

        let config = Config {
            port: scanner.port,
            scanner_token,
            sync_token,
            device_name: sync.name().to_owned(),
            catalog_owner: sync.catalog_owner,
            tax_rate_percent,
        };
        if self.config.as_ref() == Some(&config) {
            return;
        }

        self.stop_listener();
        self.config = Some(config.clone());

        if !scanner.enabled && !sync.enabled {
            self.status = ScanStatus::Off;
            return;
        }
        if config.port == 0 {
            self.status = ScanStatus::Error("Choose a port between 1 and 65535.".to_owned());
            return;
        }

        let routes = server::Routes {
            scanner_token: config.scanner_token,
            sync_token: config.sync_token,
            device_name: config.device_name,
            catalog_owner: config.catalog_owner,
            tax_rate_percent: config.tax_rate_percent,
        };
        match server::Listener::start(config.port, routes, &self.db_path, self.tx.clone()) {
            Ok(listener) => {
                let port = listener.port();
                self.listener = Some(listener);
                self.status = ScanStatus::Listening { port };
            }
            Err(error) => self.status = ScanStatus::Error(error),
        }
    }

    /// Take every scan that arrived since the last frame.
    pub fn drain(&mut self) -> Vec<ScanEvent> {
        self.rx.try_iter().collect()
    }

    pub fn status(&self) -> &ScanStatus {
        &self.status
    }

    fn stop_listener(&mut self) {
        // Dropping the server wakes its worker and joins it.
        self.listener = None;
    }
}

fn canon_tokens(scanner: &ScanConfig, sync: &SyncConfig) -> Option<(String, String)> {
    Some((
        lan_http::pairing::normalize(&scanner.token)?,
        lan_http::pairing::normalize(&sync.sync_token)?,
    ))
}

impl Drop for Service {
    fn drop(&mut self) {
        self.stop_listener();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scanner(token: &str) -> ScanConfig {
        ScanConfig {
            enabled: false,
            port: 47_831,
            token: token.to_owned(),
        }
    }

    fn sync(token: &str) -> SyncConfig {
        SyncConfig {
            sync_token: token.to_owned(),
            ..SyncConfig::default()
        }
    }

    #[test]
    fn a_broken_token_stops_the_listener_instead_of_guessing() {
        assert!(canon_tokens(&scanner("7QK4M2X9AB"), &sync("7QK4M2X9AB")).is_some());
        assert!(canon_tokens(&scanner("broken"), &sync("7QK4M2X9AB")).is_none());
        assert!(canon_tokens(&scanner("7QK4M2X9AB"), &sync("")).is_none());
    }

    #[test]
    fn a_disabled_pair_never_binds() {
        let mut service = Service::new(PathBuf::from("unused.db"));
        service.sync(&scanner("7QK4M2X9AB"), &sync("7QK4M2X9AB"), 0);
        assert_eq!(service.status(), &ScanStatus::Off);
        assert!(service.listener.is_none());
    }

    #[test]
    fn a_zero_port_is_reported_not_bound() {
        let mut service = Service::new(PathBuf::from("unused.db"));
        let mut scanner = scanner("7QK4M2X9AB");
        scanner.enabled = true;
        scanner.port = 0;
        service.sync(&scanner, &sync("7QK4M2X9AB"), 0);
        assert!(matches!(service.status(), ScanStatus::Error(_)));
    }
}
