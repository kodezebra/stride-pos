//! POS schema and the few reads the shell needs.
//!
//! Forward-only: append migrations with a higher version. Never edit one that
//! has shipped, because some till somewhere already applied it.

mod backup;

pub use backup::*;

mod sales;

pub use sales::*;

mod purchasing;

pub use purchasing::*;

mod stock;

pub use stock::*;

mod suppliers;

pub use suppliers::*;

mod catalog;

pub use catalog::*;

mod sync;

pub use sync::*;

mod staff;

pub use staff::*;

mod schema;

pub use schema::*;

mod types;

pub use types::*;

use anyhow::Context as _;
use rusqlite::OptionalExtension as _;
use std::path::{Path, PathBuf};

use crate::sync::protocol as sync_proto;

#[cfg(test)]
mod tests;
