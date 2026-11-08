//! The cursor loop that makes two tills agree.
//!
//! There is no coordinator and no master in the protocol. A till asks a peer
//! for everything after the cursor it last applied, applies it locally, and
//! remembers the new cursor. Running the same loop in the other direction
//! closes the circle; a till that was offline catches up the first time it can
//! reach any peer.
//!
//! The loop knows about `Replica` and `PeerLink`, not about SQLite or HTTP. That
//! is the extraction seam: a second product can reuse this file by providing
//! its own two implementations, while `ingest.rs` stays in the product because
//! it is the meaning of a sale.

use anyhow::{Context as _, Result};

use crate::sync::protocol::{CatalogSnapshot, PROTOCOL, PullResponse, PushRequest, PushResponse};
use storage::Store;

/// How many sales to ask for per round trip.
///
/// The seller may return fewer because it also caps the response size; the
/// caller must not treat a short page as the end unless `has_more` says so.
pub const PAGE_SIZE: usize = 100;

/// The local side of a sync: a store that can accept the two kinds of change.
///
/// `&mut self` is deliberate. A till that is a replica also has to persist the
/// owner's tax rate and cash step, which live in `settings.json` rather than
/// the database; implementation types may need to write them.
pub trait Replica {
    fn apply_sales(&mut self, request: &PushRequest) -> Result<PushResponse>;
    fn apply_catalog(&mut self, snapshot: &CatalogSnapshot) -> Result<()>;
}

/// The remote side: a peer that can serve a page and its catalog.
pub trait PeerLink {
    fn pull(&self, since_seq: i64, limit: usize) -> Result<PullResponse>;
    fn catalog(&self) -> Result<CatalogSnapshot>;
}

/// What this till has applied from one peer.
///
/// Persisted per peer in `sync_peers` in Phase 3. Kept as a value here so the
/// loop has no opinion about where the row lives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PeerCursor {
    pub since_seq: i64,
    pub catalog_version: i64,
}

/// Pull until the peer has nothing more.
///
/// The catalog is refreshed before the sales in the same page, because a sale
/// can name a product the local catalog has not seen yet. The cursor advances
/// only after every apply in the page has succeeded; a failure leaves it where
/// it was and the next attempt replays the page, which per-sale transactions
/// make harmless.
pub fn sync_once(
    replica: &mut dyn Replica,
    peer: &dyn PeerLink,
    cursor: &mut PeerCursor,
) -> Result<()> {
    loop {
        let page = peer.pull(cursor.since_seq, PAGE_SIZE)?;
        ensure_protocol(page.protocol)?;

        if page.catalog_version != cursor.catalog_version {
            let snapshot = peer.catalog()?;
            ensure_protocol(snapshot.protocol)?;
            replica
                .apply_catalog(&snapshot)
                .context("applying the peer's catalog")?;
            cursor.catalog_version = snapshot.version;
        }

        if !page.sales.is_empty() {
            let request = PushRequest {
                protocol: PROTOCOL,
                sales: page.sales,
            };
            let response = replica
                .apply_sales(&request)
                .context("applying the peer's sales")?;
            if !response.rejected.is_empty() {
                tracing::warn!(
                    rejected = response.rejected.len(),
                    first = %response.rejected[0].reason,
                    "a peer offered sales this till refused"
                );
            }
        }

        let previous = cursor.since_seq;
        cursor.since_seq = page.next_seq;

        if !page.has_more {
            return Ok(());
        }
        anyhow::ensure!(
            cursor.since_seq > previous,
            "the peer said there was more but did not advance the cursor"
        );
    }
}

fn ensure_protocol(protocol: i64) -> Result<()> {
    anyhow::ensure!(
        protocol == 0 || protocol == PROTOCOL,
        "peer speaks sync protocol {protocol}, this till speaks {PROTOCOL}"
    );
    Ok(())
}

/// The local store is already both sides of the contract.
///
/// Keeping this impl here rather than in the HTTP layer means the worker can be
/// tested end to end with two stores and no sockets.
impl Replica for Store {
    fn apply_sales(&mut self, request: &PushRequest) -> Result<PushResponse> {
        crate::sync::ingest::ingest_sales(self, request)
    }

    fn apply_catalog(&mut self, snapshot: &CatalogSnapshot) -> Result<()> {
        crate::db::apply_catalog_snapshot(self, snapshot)
    }
}
