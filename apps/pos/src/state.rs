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

use crate::db::Product;
use crate::settings::Settings;

/// One line on the till. Money is integer minor units, always.
///
/// The line keeps the product id and a copy of the name and price: a sale has to
/// be able to say what was sold at what price even after the catalog changes.
#[derive(Clone, Debug)]
pub struct CartLine {
    pub product_id: i64,
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

/// The product form, open as a modal over the app.
///
/// Price is held as typed text, not as a number: the operator is mid-edit, and
/// parsing on every keystroke would either eat a half-typed `28.` or need a
/// second "committed" copy of the field. It is parsed once, on save.
pub struct ProductDraft {
    /// `None` while adding, `Some` while editing.
    pub id: Option<i64>,
    pub name: String,
    pub price: String,
    pub category: String,
    pub barcode: String,
    pub error: Option<String>,
}

impl ProductDraft {
    fn new(category: String) -> Self {
        Self {
            id: None,
            name: String::new(),
            price: String::new(),
            category,
            barcode: String::new(),
            error: None,
        }
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
    /// The category both Sell and Products filter by. `None` is "All"; the value
    /// is a category name, because the categories are the shop's, not an index
    /// into a list the app invented.
    pub category: Option<String>,
    pub range: usize,
    pub settings_section: usize,

    /// The catalog, read once and re-read after every write. Filtering happens
    /// in memory: a few hundred products are cheaper to filter than to query
    /// once per frame, and the grid redraws every frame.
    pub products: Vec<Product>,
    /// The distinct categories currently in the catalog, for the sidebars.
    pub categories: Vec<String>,
    /// The product form, when it is open.
    pub product_editor: Option<ProductDraft>,
    /// A product the operator asked to delete, waiting for confirmation. Inline
    /// in the row rather than a modal: the row is where the mistake would be.
    pub product_delete: Option<i64>,

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
        let products = crate::db::products(&store);
        let categories = crate::db::categories(&store);
        Self {
            store,
            cart: Vec::new(),
            module,
            sidebar_open,
            prefs_dirty: false,
            search: String::new(),
            category: None,
            range: 0,
            settings_section: 0,
            products,
            categories,
            product_editor: None,
            product_delete: None,
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

    /// Add one unit of a product, merging with an existing line.
    ///
    /// Scanning the same barcode twice should read "2 × Espresso", not two
    /// lines: the cashier checks quantities, not line counts. The line keeps the
    /// name and price it was added at, so editing the catalog mid-sale cannot
    /// silently reprice something already in the basket.
    pub fn add_to_cart(&mut self, product: &Product) {
        self.notice = None;
        match self
            .cart
            .iter_mut()
            .find(|line| line.product_id == product.id)
        {
            Some(line) => line.qty += 1,
            None => self.cart.push(CartLine {
                product_id: product.id,
                name: product.name.clone(),
                qty: 1,
                unit_minor: product.price_minor,
            }),
        }
    }

    /// Re-read the catalog and the category list after a write.
    ///
    /// If the selected category no longer exists (its last product was deleted
    /// or recategorised), the filter falls back to All rather than showing an
    /// empty grid with a category chip that is not in the sidebar.
    pub fn reload_products(&mut self) {
        self.products = crate::db::products(&self.store);
        self.categories = crate::db::categories(&self.store);

        if let Some(selected) = &self.category
            && !self.categories.iter().any(|category| category == selected)
        {
            self.category = None;
        }
    }

    /// Open an empty product form, pre-filled with the category being viewed.
    pub fn start_product(&mut self) {
        let category = self
            .category
            .clone()
            .unwrap_or_else(|| "General".to_owned());
        self.product_editor = Some(ProductDraft::new(category));
        self.product_delete = None;
        self.notice = None;
    }

    /// Open the form on an existing product.
    pub fn edit_product(&mut self, product: &Product) {
        self.product_editor = Some(ProductDraft {
            id: Some(product.id),
            name: product.name.clone(),
            price: plain_amount(product.price_minor, self.currency),
            category: product.category.clone(),
            barcode: product.barcode.clone().unwrap_or_default(),
            error: None,
        });
        self.product_delete = None;
        self.notice = None;
    }

    /// Parse and store the open form.
    ///
    /// Everything is validated before the first write, so a rejected form leaves
    /// the database untouched and the text the operator typed still on screen.
    pub fn save_product(&mut self) {
        let Some(draft) = self.product_editor.as_ref() else {
            return;
        };

        let id = draft.id;
        let name = draft.name.trim().to_owned();
        let category = draft.category.trim().to_owned();
        let barcode = draft.barcode.trim().to_owned();
        let price_text = draft.price.trim().to_owned();

        if name.is_empty() {
            return self.editor_error("A product needs a name.");
        }
        if category.is_empty() {
            return self.editor_error("A product needs a category.");
        }

        let price_minor = match Money::parse(&price_text, self.currency) {
            Ok(price) if price.minor() >= 0 => price.minor(),
            Ok(_) => return self.editor_error("A price cannot be negative."),
            Err(error) => return self.editor_error(error.to_string()),
        };

        let barcode = (!barcode.is_empty()).then_some(barcode);
        if let Some(code) = barcode.as_deref()
            && crate::db::barcode_taken(&self.store, code, id)
        {
            return self.editor_error("Another product already uses that barcode.");
        }

        let result = match id {
            Some(id) => crate::db::update_product(
                &self.store,
                id,
                &name,
                price_minor,
                &category,
                barcode.as_deref(),
            ),
            None => crate::db::insert_product(
                &self.store,
                &name,
                price_minor,
                &category,
                barcode.as_deref(),
            )
            .map(|_| ()),
        };

        match result {
            Ok(()) => {
                self.product_editor = None;
                self.reload_products();
                self.notice = Some(format!("Saved {name}."));
            }
            Err(error) => {
                tracing::error!(error = %error, "saving a product failed");
                self.editor_error(format!("Could not save: {error}"));
            }
        }
    }

    /// Remove a product the operator confirmed.
    pub fn delete_product(&mut self, id: i64) {
        match crate::db::delete_product(&self.store, id) {
            Ok(()) => {
                self.product_delete = None;
                self.reload_products();
                self.notice = Some("Product removed.".to_owned());
            }
            Err(error) => {
                tracing::error!(error = %error, "deleting a product failed");
                self.product_delete = None;
                self.notice = Some(format!("Could not remove the product: {error}"));
            }
        }
    }

    fn editor_error(&mut self, message: impl Into<String>) {
        if let Some(draft) = self.product_editor.as_mut() {
            draft.error = Some(message.into());
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

/// The amount without its symbol, for putting in an edit field: `2800` in
/// Uganda, `28.00` in Kenya.
///
/// [`Money::format`] is the one canonical rendering; this is the inverse the
/// product form needs, and the test below pins the two together so a currency
/// where they disagree fails here rather than in an operator's hands.
fn plain_amount(minor: i64, currency: Currency) -> String {
    let formatted = Money::new(minor, currency).format();
    match formatted.rsplit_once(' ') {
        Some((_, amount)) => amount.to_owned(),
        // A glyph symbol such as `$` is glued to the digits; drop everything
        // before the number instead.
        None => formatted
            .trim_start_matches(|ch: char| !ch.is_ascii_digit() && ch != '-')
            .to_owned(),
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    /// The edit field shows an amount without its symbol and the save path
    /// parses it back. If the two ever disagree, an operator opens a product,
    /// presses Save without touching anything, and the price changes.
    #[test]
    fn plain_amount_round_trips_through_parse() {
        for currency in [Currency::UGX, Currency::KES] {
            for minor in [0, 1, 100, 2_800, 12_345] {
                let text = plain_amount(minor, currency);
                let parsed = Money::parse(&text, currency)
                    .unwrap_or_else(|error| panic!("{currency:?} {minor} -> {text:?}: {error}"));
                assert_eq!(
                    parsed.minor(),
                    minor,
                    "{currency:?} {minor} formatted as {text:?}"
                );
            }
        }
    }
}
