//! Everything the POS screen tree can change, in one place.
//!
//! One struct, passed as `&mut` to whichever module is drawing. Modules keep no
//! state of their own: switching module must not lose the cart, the search box,
//! or the scroll position, and a module that owns data makes that rule hard to
//! keep. It also means a command in the palette can change anything the user can
//! change, without reaching into widgets.

use std::path::PathBuf;

use money::{Currency, Money};
use ui_kit::theme::Theme;

use crate::settings::Settings;

/// One line on the till. Money is integer minor units, always.
#[derive(Clone, Debug)]
pub struct CartLine {
    pub name: String,
    pub qty: i64,
    /// Price per unit, in minor units.
    pub unit_minor: i64,
}

impl CartLine {
    pub fn total_minor(&self) -> i64 {
        self.qty * self.unit_minor
    }
}

pub struct State {
    pub store: storage::Store,

    pub cart: Vec<CartLine>,

    /// Index into [`crate::modules::all`]. Also the activity-bar selection.
    pub module: usize,
    /// Per-module sidebar visibility, so collapsing Sell does not collapse
    /// Settings. Indexed in parallel with the module list.
    pub sidebar_open: Vec<bool>,
    /// Set when something persisted changed; the shell writes `prefs.json` and
    /// clears it.
    pub prefs_dirty: bool,

    // Per-module view state. Kept here rather than in the module structs for the
    // reason in the module doc: switching away and back should not reset them.
    pub search: String,
    pub category: usize,
    pub range: usize,
    pub settings_section: usize,

    /// One-line feedback for an action that has no flow behind it yet. Cleared
    /// by the next action, so it never turns into a permanent lie in the UI.
    pub notice: Option<String>,

    /// The shop's configuration, loaded once at startup and written back when
    /// something in Settings changes.
    pub settings: Settings,
    /// Where `settings.json` lives, so the shell can persist a change without
    /// the screens knowing about paths.
    pub settings_path: PathBuf,
    /// The theme currently installed on the context.
    pub theme: Theme,
    /// Set when the theme changed and the context still needs reinstalling.
    /// The shell owns `install`, so the screen only records the intent.
    pub theme_dirty: bool,
    /// Set when `settings` changed and needs writing.
    pub settings_dirty: bool,

    /// The one currency this till prices in. Stored in the database, not in
    /// `settings.json`: it belongs with the amounts it names.
    pub currency: Currency,
    /// No currency has been stored yet: the picker is modal and the till is not
    /// usable until the operator chooses. Defaults to UGX for the first frame.
    pub needs_currency: bool,
    /// Settings asked to change the currency while the till is still empty.
    pub currency_picker: bool,
    /// Why the last currency write failed, shown inside the picker.
    pub currency_error: Option<String>,
}

impl State {
    pub fn new(
        store: storage::Store,
        module: usize,
        sidebar_open: Vec<bool>,
        settings: Settings,
        settings_path: PathBuf,
        currency: Currency,
        needs_currency: bool,
    ) -> Self {
        let theme = settings.theme();
        Self {
            store,
            cart: Vec::new(),
            module,
            sidebar_open,
            prefs_dirty: false,
            search: String::new(),
            category: 0,
            range: 0,
            settings_section: 0,
            notice: None,
            settings,
            settings_path,
            theme,
            theme_dirty: false,
            settings_dirty: false,
            currency,
            needs_currency,
            currency_picker: false,
            currency_error: None,
        }
    }

    /// Format an amount in this till's currency: `12345` → `"Ksh 123.45"`.
    ///
    /// The currency is the one fact every screen needs and none of them own, so
    /// it lives here and the screens ask the state rather than passing it down.
    pub fn money(&self, minor: i64) -> String {
        Money::new(minor, self.currency).format()
    }

    /// Record a currency choice. On a failed write the picker stays open with
    /// the reason, because selling in an unrecorded currency is worse than
    /// blocking the till for one more attempt.
    pub fn set_currency(&mut self, currency: Currency) {
        match crate::db::set_currency(&self.store, currency) {
            Ok(()) => {
                self.currency = currency;
                self.needs_currency = false;
                self.currency_picker = false;
                self.currency_error = None;
            }
            Err(error) => {
                tracing::error!(error = %error, "storing the currency failed");
                self.currency_error = Some(format!("Could not save the currency: {error}"));
            }
        }
    }

    /// Switch appearance. The shell installs it on the next frame; screens never
    /// touch global visuals.
    pub fn set_theme(&mut self, theme: Theme) {
        if self.theme != theme {
            self.theme = theme;
            self.settings.set_theme(theme);
            self.theme_dirty = true;
            self.settings_dirty = true;
        }
    }

    /// Flip light/dark. One place decides, so the icon button, the palette
    /// command and Ctrl+T cannot drift apart.
    pub fn toggle_theme(&mut self) {
        self.set_theme(self.theme.flipped());
    }

    pub fn select(&mut self, module: usize) {
        if self.module != module {
            self.module = module;
            self.prefs_dirty = true;
            self.notice = None;
        }
    }

    pub fn sidebar_open(&self, module: usize) -> bool {
        self.sidebar_open.get(module).copied().unwrap_or(true)
    }

    pub fn set_sidebar_open(&mut self, module: usize, open: bool) {
        if module >= self.sidebar_open.len() {
            self.sidebar_open.resize(module + 1, true);
        }
        if self.sidebar_open[module] != open {
            self.sidebar_open[module] = open;
            self.prefs_dirty = true;
        }
    }

    pub fn toggle_sidebar(&mut self) {
        let open = self.sidebar_open(self.module);
        self.set_sidebar_open(self.module, !open);
    }

    /// Add one unit of an item, merging with an identical line.
    ///
    /// Scanning the same barcode twice should read "2 × Espresso", not two
    /// lines: the cashier checks quantities, not line counts.
    pub fn add_to_cart(&mut self, name: &str, unit_minor: i64) {
        self.notice = None;
        match self
            .cart
            .iter_mut()
            .find(|line| line.name == name && line.unit_minor == unit_minor)
        {
            Some(line) => line.qty += 1,
            None => self.cart.push(CartLine {
                name: name.to_owned(),
                qty: 1,
                unit_minor,
            }),
        }
    }

    pub fn clear_cart(&mut self) {
        self.cart.clear();
        self.notice = None;
    }

    pub fn cart_count(&self) -> i64 {
        self.cart.iter().map(|line| line.qty).sum()
    }

    pub fn cart_total_minor(&self) -> i64 {
        self.cart.iter().map(CartLine::total_minor).sum()
    }
}

/// Sidebar categories. `0` is the unfiltered view.
pub const CATEGORIES: &[&str] = &["All", "Coffee", "Bakery", "Cold", "Snacks"];

/// Date ranges offered by Dashboard and Sales. The index is what `db` turns
/// into a SQL window, so the label and the numbers can never drift apart.
pub const RANGES: &[&str] = &["Today", "7 days", "30 days", "All time"];

/// How a range reads as a section heading above the cards.
pub fn range_caption(range: usize) -> &'static str {
    match range {
        0 => "Today",
        1 => "Last 7 days",
        2 => "Last 30 days",
        _ => "All time",
    }
}

/// Settings sections. Each one is a page that does not exist yet; the list is
/// real so the sidebar has the right shape from the start.
pub const SETTINGS_SECTIONS: &[&str] = &["Terminal", "Receipt", "Tax", "Sync"];

/// The quick keys shown in Sell until there is a catalog to read from.
///
/// Prices are in the till's minor units, so the same `280` is `USh 280` on a
/// Ugandan till and `Ksh 2.80` on a Kenyan one — placeholder data, but real
/// arithmetic.
///
/// This is the one place in the app with invented data, and it is marked as
/// such: the grid, the cart, and the money are real code paths, the product list
/// is a placeholder. When `products` lands, this constant is deleted and the grid
/// iterates the store instead — the widget code does not change.
pub const QUICK_KEYS: &[(&str, i64)] = &[
    ("Espresso", 280),
    ("Flat white", 340),
    ("Filter", 260),
    ("Croissant", 320),
    ("Cinnamon roll", 380),
    ("Sourdough", 650),
    ("Iced latte", 380),
    ("Lemonade", 300),
];
