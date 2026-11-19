//! Sync state: this device's name, whether sync is on, and the peers it trusts.
//!
//! Sync is a setting plus a small peer list, and both only ever change from the
//! settings screen — so the draft that edits a peer lives here rather than with
//! the settings cards that draw it.

use super::*;
/// The "pair another till" form in Settings.
///
/// The port is text, not a number: a field that silently rewrites what was
/// typed is how a form loses an address. It is parsed once, on Add.
pub struct SyncPeerDraft {
    pub name: String,
    pub host: String,
    pub port: String,
    pub token: String,
}

impl Default for SyncPeerDraft {
    fn default() -> Self {
        Self {
            name: String::new(),
            host: String::new(),
            port: crate::scan::DEFAULT_PORT.to_string(),
            token: String::new(),
        }
    }
}

impl State {
    /// Commit the name other tills see. Blank is allowed; the protocol falls
    /// back to "Till" so a peer list never shows an empty row.
    pub fn save_sync_device_name(&mut self) {
        let name = self.sync_name_draft.trim().to_owned();
        self.sync_name_draft.clone_from(&name);
        if self.settings.sync.device_name != name {
            self.settings.sync.device_name = name;
            self.settings_dirty = true;
        }
    }

    /// Turn till-to-till sync on or off. The shell restarts the listener and
    /// wakes the worker from the flag, so this only records the decision.
    pub fn set_sync_enabled(&mut self, enabled: bool) {
        if self.settings.sync.enabled != enabled {
            self.settings.sync.enabled = enabled;
            self.settings_dirty = true;
        }
    }

    /// Declare (or stop declaring) this till as the catalog author.
    ///
    /// The flag is a promise to peers, not a lock: replicas read it from the
    /// pull response and ignore catalogs from tills that do not claim it.
    pub fn set_catalog_owner(&mut self, owner: bool) {
        if self.settings.sync.catalog_owner != owner {
            self.settings.sync.catalog_owner = owner;
            self.settings_dirty = true;
        }
    }

    /// Re-read the peer list, including the error and last-seen time the worker
    /// wrote. Called after a pass so the card does not show a stale failure.
    pub fn reload_sync_peers(&mut self) {
        match crate::db::sync_peers(&self.store) {
            Ok(peers) => {
                self.sync_peers = peers;
                self.sync_peer_error = None;
            }
            Err(error) => {
                self.sync_peer_error = Some(format!("Could not read the paired tills: {error}"));
            }
        }
    }

    /// Pair another till from the Sync card.
    pub fn add_sync_peer(&mut self) {
        let port = match self.sync_peer_draft.port.trim().parse::<u16>() {
            Ok(port) if port != 0 => port,
            _ => {
                self.sync_peer_error = Some("Enter a port between 1 and 65535.".to_owned());
                return;
            }
        };

        match crate::db::add_sync_peer(
            &self.store,
            &self.sync_peer_draft.name,
            &self.sync_peer_draft.host,
            port,
            &self.sync_peer_draft.token,
        ) {
            Ok(()) => {
                self.sync_peer_draft = SyncPeerDraft::default();
                self.sync_peer_error = None;
                self.reload_sync_peers();
                // Pairing a till is a request to use it now, not to wait half a
                // minute for the next poll.
                self.sync_now_requested = self.settings.sync.enabled;
            }
            Err(error) => {
                self.sync_peer_error = Some(error.to_string());
            }
        }
    }

    /// Stop pulling from one till. Its cursor goes with the row: re-adding the
    /// till starts a fresh feed at zero. Ingestion is idempotent, so that replay
    /// converges; it is only slower than keeping the cursor.
    pub fn remove_sync_peer(&mut self, id: &str) {
        match crate::db::remove_sync_peer(&self.store, id) {
            Ok(()) => {
                self.sync_peer_error = None;
                self.reload_sync_peers();
            }
            Err(error) => {
                self.sync_peer_error = Some(format!("Could not remove the till: {error}"));
            }
        }
    }
}
