//! POS schema and the few reads the shell needs.
//!
//! Forward-only: append migrations with a higher version. Never edit one that
//! has shipped, because some till somewhere already applied it.
//!
//! One file per table group. Everything each file declares is re-exported here,
//! so callers keep writing `db::products(...)` and never learn the file layout.

mod backup;
mod catalog;
mod customers;
mod purchasing;
mod sales;
mod schema;
mod staff;
mod stock;
mod suppliers;
mod sync;
mod types;

#[cfg(test)]
mod tests;

pub use backup::*;
pub use catalog::*;
pub use customers::*;
pub use purchasing::*;
pub use sales::*;
pub use schema::*;
pub use staff::*;
pub use stock::*;
pub use suppliers::*;
pub use sync::*;
pub use types::*;

use anyhow::Context as _;
use rusqlite::OptionalExtension as _;
use std::path::{Path, PathBuf};

use crate::sync::protocol as sync_proto;
