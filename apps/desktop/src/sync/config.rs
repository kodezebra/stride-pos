//! The shop's sync preferences, stored inside `settings.json`.
//!
//! The token is the same kind of code the phone scanner uses, but it is a
//! different secret: it lets another till pull this till's sales and catalog.
//! A phone never receives it, so a lost phone cannot read the shop's history.

use serde::{Deserialize, Serialize};

/// What the shop decided about till-to-till sync.
///
/// `catalog_owner` is the one asymmetric choice in an otherwise symmetric
/// protocol. Every till sells, and every sale travels both ways, but only one
/// till may author products and categories; replicas replace their catalog with
/// the owner's snapshot. Defaulting to `false` means turning sync on cannot
/// rewrite a working catalog by accident.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncConfig {
    /// Sync is off until the shop turns it on. The listener still serves the
    /// phone scanner when that is enabled separately.
    pub enabled: bool,
    /// What other tills call this one. The device id, not this name, is the
    /// identity; a renamed till keeps its cursor and its history.
    pub device_name: String,
    /// The code another till types to pull from this one. Generated on first
    /// launch so enabling sync never waits for a "generate" button.
    pub sync_token: String,
    /// Whether this till is the catalog author. A replica accepts the owner's
    /// products and categories and never publishes its own.
    pub catalog_owner: bool,
}

impl SyncConfig {
    /// Give a new or hand-broken settings file a usable token. Returns whether
    /// the token changed, so the caller can persist it.
    pub fn ensure_token(&mut self) -> bool {
        if lan_http::pairing::is_valid(&self.sync_token) {
            return false;
        }
        let previous = std::mem::take(&mut self.sync_token);
        self.sync_token = lan_http::pairing::generate();
        self.sync_token != previous
    }

    /// The name a peer sees. A blank name is still a till, not an empty column.
    pub fn name(&self) -> &str {
        let trimmed = self.device_name.trim();
        if trimmed.is_empty() { "Till" } else { trimmed }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_token_is_filled_once_and_kept() {
        let mut config = SyncConfig::default();
        assert!(config.ensure_token());
        let token = config.sync_token.clone();
        assert!(lan_http::pairing::is_valid(&token));

        assert!(!config.ensure_token());
        assert_eq!(config.sync_token, token);
    }

    #[test]
    fn a_hand_written_token_is_normalised() {
        let mut config = SyncConfig {
            sync_token: "abcd-efgh-jk".to_owned(),
            ..SyncConfig::default()
        };
        assert!(config.ensure_token());
        assert!(lan_http::pairing::is_valid(&config.sync_token));
        assert_ne!(config.sync_token, "abcd-efgh-jk");
    }

    #[test]
    fn a_blank_device_name_still_reads_as_a_till() {
        let config = SyncConfig::default();
        assert_eq!(config.name(), "Till");

        let named = SyncConfig {
            device_name: "  Front counter ".to_owned(),
            ..SyncConfig::default()
        };
        assert_eq!(named.name(), "Front counter");
    }
}
