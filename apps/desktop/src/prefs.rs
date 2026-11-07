//! UI preferences that survive a restart.
//!
//! Deliberately separate from `settings.json`: settings are the terminal's
//! configuration (theme, printer, tax), preferences are one person's habits.
//! Mixing them means a shared till rewrites the shop's configuration every time
//! someone collapses a sidebar.

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// The module that was open, as an index into [`crate::modules::all`].
    pub module: usize,

    /// Per-module sidebar visibility.
    ///
    /// A `Vec` rather than a fixed array: adding a module must not invalidate an
    /// older file, and an unknown index simply defaults to open.
    pub sidebar: Vec<bool>,
}
