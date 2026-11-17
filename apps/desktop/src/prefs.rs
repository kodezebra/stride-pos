//! UI preferences that survive a restart.
//!
//! Deliberately separate from `settings.json`: settings are the terminal's
//! configuration (theme, printer, tax), preferences are one person's habits.
//! Mixing them means a shared till rewrites the shop's configuration every time
//! someone collapses a sidebar.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Layout version. Bumped when a module index changes meaning, so a file
    /// written by an older build can be moved across the change instead of
    /// landing on the wrong screen.
    pub version: u32,

    /// The module that was open, as an index into [`crate::modules::all`].
    pub module: usize,

    /// Per-module sidebar visibility.
    ///
    /// A `Vec` rather than a fixed array: adding a module must not invalidate an
    /// older file, and an unknown index simply defaults to open.
    pub sidebar: Vec<bool>,
}

impl Prefs {
    /// The current layout version.
    pub const CURRENT: u32 = 1;

    /// Move a file written by an older build onto the current module list.
    ///
    /// Stock used to be its own module at index 6; it now lives in Catalog, and
    /// Restock moved down from 7 to 6. A saved screen that no longer exists
    /// opens the nearest one instead of a blank page.
    pub fn normalize(&mut self) {
        if self.version >= Self::CURRENT {
            return;
        }

        self.module = match self.module {
            6 => crate::modules::PRODUCTS,
            7 => crate::modules::PURCHASES,
            other => other,
        };
        // Drop the Stock row from the saved sidebar state, so the modules after
        // it keep their own open/closed habit.
        if self.sidebar.len() > 6 {
            self.sidebar.remove(6);
        }
        self.version = Self::CURRENT;
    }
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            version: Self::CURRENT,
            module: 0,
            sidebar: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_stock_screen_lands_on_the_catalog() {
        let mut prefs = Prefs {
            version: 0,
            module: 6,
            sidebar: vec![false; 8],
        };

        prefs.normalize();

        assert_eq!(prefs.module, crate::modules::PRODUCTS);
        assert_eq!(prefs.sidebar.len(), 7);
        assert_eq!(prefs.version, Prefs::CURRENT);
    }

    #[test]
    fn a_saved_restock_screen_follows_it_down_one() {
        let mut prefs = Prefs {
            version: 0,
            module: 7,
            sidebar: Vec::new(),
        };

        prefs.normalize();

        assert_eq!(prefs.module, crate::modules::PURCHASES);
        assert_eq!(prefs.module, 6);
    }

    #[test]
    fn a_current_file_is_left_alone() {
        let mut prefs = Prefs {
            version: Prefs::CURRENT,
            module: 6,
            sidebar: vec![true; 7],
        };

        prefs.normalize();

        assert_eq!(prefs.module, 6);
        assert_eq!(prefs.sidebar.len(), 7);
    }

    #[test]
    fn a_fresh_file_is_current() {
        assert_eq!(Prefs::default().version, Prefs::CURRENT);
    }
}
