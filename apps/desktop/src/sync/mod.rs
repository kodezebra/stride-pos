//! Offline-first replication between tills and the blackout phone.
//!
//! The desktop writes every sale to its own SQLite database first and never
//! waits for a network. This module carries those already-committed sales to a
//! peer and applies the peer's sales in return. There is no server and no
//! elected master: each sale has a UUID and a commutative effect, so any order
//! of exchange converges.
//!
//! The pieces are deliberately separate:
//!
//! - [`protocol`] is the wire shape and nothing else.
//! - [`ingest`] decides whether a sale is true and writes it locally.
//! - [`worker`] is the cursor loop and knows neither SQL nor HTTP.
//!
//! Phase 3 wires this into the LAN listener. Until then the headless core is
//! exercised by its inline tests, so some items are not yet reachable from
//! `main`; the allow is for that gap, not for permanent dead code.
#![allow(dead_code)]

pub mod ingest;
pub mod protocol;
pub mod worker;

#[cfg(test)]
mod tests;
