//! The POS modules: what each one puts in the sidebar and in the workspace.
//!
//! A module is a *pair of closures with a name*, not a page with its own state.
//! The frame (rail, sidebar, status bar) lives in `shell.rs`; the data lives in
//! [`crate::state::State`]. That split is what lets the palette, the hotkeys and
//! the rail all agree on what "module 2" means without a registry.
//!
//! Adding a module: add a struct here, add it to [`all`] in the same position as
//! its index constant, and list its index in [`tabs`] under the view it belongs
//! to. Everything else — rail entry, tab strip, palette entry, persistence index
//! — follows from those edits.
//!
//! There are four keys, not seven: F1–F4 open the four views, and the tabs
//! inside a view are clicked. A key per page does not survive contact with a
//! fifth page, and a till has no room for a function-key crib sheet.

use std::path::Path;

use money::Money;
use ui_kit::components;
use ui_kit::icons::Icon;
use ui_kit::theme;
use ui_kit::tokens::{BRANDS, RADII, Rgb, SPACING, TYPE};
use ui_shell::egui;
use ui_shell::regions;

use crate::actions;
use crate::category_seeds;
use crate::db::{ExpiryBucket, Product, Purchase, PurchaseStatus, StockItem, StockKind, Supplier};
use crate::picker::ExportKind;
use crate::state::{self, CustomerMode, MAX_PRICE_BREAKS, SellView, State, percent_text};

mod catalog;
mod customers;
mod day;
mod purchases;
mod sales;
mod sell;
mod settings;
mod suppliers;
mod widgets;

use catalog::*;
use customers::*;
use day::*;
use purchases::*;
use sales::*;
use sell::*;
use settings::*;
use suppliers::*;
use widgets::*;

// The modals the shell opens by name, kept at the module root so the shell does
// not have to know which file a screen lives in.
pub use catalog::{import_modal, product_editor_modal, warning_modal};
pub use sales::sale_detail_modal;
pub use sell::cart_quantity_modal;
pub use sell::credit_settle_modal;
pub use suppliers::{stock_history_modal, stock_move_modal, supplier_editor_modal, track_modal};

// The export runs from the picker, which knows a directory and not a screen.
pub(crate) use catalog::run_export;

// Re-exported at the module root because the shell and the payment screen reach
// for these by name; the rest stay inside the module tree.
pub(crate) use widgets::{amount_row, modal, prefixed_field, secret_field, text_field};

// Indices into `all()`. Order is the persisted value, so these numbers are as
// permanent as the module list. It is not the order anything is drawn in: see
// [`views`] for the rail and [`tabs`] for what sits inside each view.
pub const SELL: usize = 0;
pub const DASHBOARD: usize = 1;
pub const PRODUCTS: usize = 2;
pub const SALES: usize = 3;
pub const SETTINGS: usize = 4;
pub const SUPPLIERS: usize = 5;
pub const PURCHASES: usize = 6;
pub const CUSTOMERS: usize = 7;

/// Number of modules. Used to clamp a persisted index from an older or newer
/// build before it is used as an array index.
pub const COUNT: usize = 8;

// ---------------------------------------------------------------------------
// Views and tabs
// ---------------------------------------------------------------------------

/// The four places a till has.
///
/// A view is a whole way of working, not a page: Sell is the counter, Day is
/// what the shop did, Shop is what the shop is, and Settings is how the till
/// behaves. A tab is one page of that job, so it is flat — nothing hides behind
/// a tab, and nothing behind a tab hides behind another one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Sell,
    Day,
    Shop,
    Settings,
}

impl View {
    pub fn label(self) -> &'static str {
        match self {
            Self::Sell => "Sell",
            Self::Day => "Day",
            Self::Shop => "Shop",
            Self::Settings => "Settings",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Self::Sell => Icon::CART,
            Self::Day => Icon::DASHBOARD,
            Self::Shop => Icon::BOXES,
            Self::Settings => Icon::SETTINGS,
        }
    }

    /// The key that jumps here. Four keys, no modifiers: the whole rail fits
    /// under the left hand.
    pub fn hotkey(self) -> egui::Key {
        match self {
            Self::Sell => egui::Key::F1,
            Self::Day => egui::Key::F2,
            Self::Shop => egui::Key::F3,
            Self::Settings => egui::Key::F4,
        }
    }
}

/// Day's tabs: the sales that happened, and what they add up to.
const DAY_TABS: [usize; 2] = [SALES, DASHBOARD];

/// Shop's tabs: what is for sale, what is coming in, who brings it, and who
/// buys it. Stock is not a page of its own: it is a column and a filter on the
/// catalog.
const SHOP_TABS: [usize; 4] = [PRODUCTS, PURCHASES, SUPPLIERS, CUSTOMERS];

/// Height of a module's pinned action strip.
///
/// Taller than the 44 px actions inside it: the strip also holds the page's
/// search field, and a bar the same height as its tallest control leaves the
/// field touching the hairline above and below. One height on every page is
/// what makes the bar a place rather than a coincidence.
const TOOLBAR_HEIGHT: f32 = 52.0;
/// Height of a search field inside the bar. The same 44 px as a full-width
/// field, so scanning does not depend on which page the cashier is on.
const SEARCH_HEIGHT: f32 = 44.0;
/// A page's search field is a share of the bar, with a floor and a ceiling: a
/// fixed width is too wide on an 880 px till and too narrow on a 4K one.
const SEARCH_MIN: f32 = 200.0;
const SEARCH_MAX: f32 = 320.0;
const SEARCH_SHARE: f32 = 0.36;

pub trait Module {
    /// Whether this module has a sidebar. If false, no sidebar panel is drawn.
    fn has_sidebar(&self) -> bool {
        true
    }

    /// A fixed sidebar width, or `None` to let the user drag it.
    fn fixed_sidebar(&self) -> Option<f32> {
        None
    }

    /// Whether the shell wraps this page in a vertical scroll area.
    ///
    /// On by default: a 30-day chart or a full staff list must not end below the
    /// fold with no way to reach it. Sell opts out because its workspace is a
    /// fixed two-column frame — inside a scroll area `available_height` is
    /// unbounded, and the cart's Pay button would drift off the screen.
    fn scrolls(&self) -> bool {
        true
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, state: &mut State);

    /// The pinned action strip above a scrolling page.
    ///
    /// Only actions that must stay reachable while the list scrolls belong
    /// here. A page with no such action leaves the strip empty; the shell calls
    /// this before the scroll area, so the content still owns the vertical
    /// rhythm. Modules with nothing to pin simply do not override it.
    fn toolbar(&mut self, _ui: &mut egui::Ui, _state: &mut State) {}

    fn workspace(&mut self, ui: &mut egui::Ui, state: &mut State);
}

/// What a page bar puts in its left cell.
///
/// A page shows one or the other, never both. The search is the page's question
/// and the title is the page's name; a row carrying both makes the reader work
/// out which one the actions belong to. The title is `None` on a page named by
/// its tab, and a sub-view — a purchase in progress, a settings section — is
/// the only place a title earns its space.
enum PageLeft<'a> {
    Title(&'a str),
    Search {
        text: &'a mut String,
        hint: &'a str,
        /// A count of the rows on screen, e.g. `"12 of 48"`. Only a page whose
        /// filter can leave rows out has one worth reading, and only while the
        /// filter is on: `"48 of 48"` is a number the reader already knows.
        count: Option<String>,
    },
}

/// Lay out one module's pinned bar: the page's left cell, then its actions.
///
/// Every list page draws the same bar, in the same place, at the same height, so
/// the search field is where the eye already is and the primary action is always
/// the rightmost thing on the row. It is drawn before the scroll area: a long
/// list must not take "New product" past the fold.
///
/// The actions run right to left, like a dialog footer, so the first action
/// added is the rightmost one. The bar draws its own hairline and the gap before
/// the page. A page with nothing to say — no title, no search, no action — does
/// not call this at all: an empty strip is a row of dead pixels.
///
/// Returns the search field's response, because Sell needs it: the scan field is
/// where a barcode scanner's Enter arrives, and a page that forgets to ask is a
/// till that does not scan.
fn action_bar(
    ui: &mut egui::Ui,
    left: PageLeft<'_>,
    add: impl FnOnce(&mut egui::Ui),
) -> Option<egui::Response> {
    let palette = theme::current(ui.ctx());
    let full = ui.available_rect_before_wrap();
    let rect = egui::Rect::from_min_size(full.min, egui::vec2(full.width(), TOOLBAR_HEIGHT));

    let mut bar = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    bar.spacing_mut().item_spacing.x = SPACING.sm;

    let mut search = None;
    match left {
        PageLeft::Title(title) => {
            bar.label(
                egui::RichText::new(title)
                    .family(theme::semibold())
                    .size(TYPE.heading)
                    .color(egui::Color32::from(palette.text)),
            );
        }
        PageLeft::Search { text, hint, count } => {
            let width = (full.width() * SEARCH_SHARE).clamp(SEARCH_MIN, SEARCH_MAX);
            let response =
                components::search_field_sized(&mut bar, text, hint, width, SEARCH_HEIGHT);
            // `Ctrl+F` is the page's search key everywhere. It is handled here,
            // once, so a new page cannot ship with a field no key reaches.
            if bar.input(|input| input.modifiers.command && input.key_pressed(egui::Key::F)) {
                response.request_focus();
            }
            // `Escape` clears the field and keeps it. The text edit surrenders
            // focus on Escape before this runs, so `lost_focus` is what proves
            // the key was aimed at this field — checking the key alone would
            // also clear the query when a dialog above the page is dismissed.
            if response.lost_focus()
                && bar.input(|input| input.key_pressed(egui::Key::Escape))
                && !text.is_empty()
            {
                text.clear();
                response.request_focus();
            }
            if let Some(count) = count {
                components::caption(&mut bar, &count);
            }
            search = Some(response);
        }
    }

    bar.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = SPACING.sm;
        add(ui);
    });

    ui.advance_cursor_after_rect(rect);
    ui.painter().hline(
        rect.x_range(),
        rect.bottom(),
        egui::Stroke::new(1.0_f32, palette.border),
    );
    ui.add_space(SPACING.sm);
    search
}

/// Every module, in stable index order. Must stay in sync with the index
/// constants above.
///
/// This is the persisted order, not the order anything is drawn in: see
/// [`views`] for the rail and [`tabs`] for what each view holds.
pub fn all() -> Vec<Box<dyn Module>> {
    vec![
        Box::new(Sell),
        Box::new(Dashboard),
        Box::new(Products),
        Box::new(Sales),
        Box::new(Settings),
        Box::new(Suppliers),
        Box::new(Purchases),
        Box::new(Customers),
    ]
}

/// The activity bar for a role, top to bottom.
///
/// Three entries, because a till has three jobs: take money, know what the day
/// did, and keep the shop stocked. Everything that used to be its own rail row
/// is a tab inside one of these — a bar that grows a row per screen is a menu,
/// and a menu is something the cashier has to read instead of the cart.
///
/// A cashier sees only the screens their work needs. Hiding Shop is not the
/// permission boundary — [`open`] is — but a navigation bar that offers doors
/// the person cannot walk through is a dead end, and a dead end teaches a
/// cashier to stop using the bar.
pub fn views(role: crate::auth::Role) -> &'static [View] {
    const CASHIER: [View; 2] = [View::Sell, View::Day];
    const MANAGER: [View; 4] = [View::Sell, View::Day, View::Shop, View::Settings];

    if role >= crate::auth::Role::Manager {
        &MANAGER
    } else {
        &CASHIER
    }
}

/// The tabs of one view, left to right. The first one is where the view lands.
///
/// A cashier reads the day's own sales; the trends that read the shop's whole
/// history are the manager's screen.
pub fn tabs(view: View, role: crate::auth::Role) -> &'static [usize] {
    const CASHIER_DAY: [usize; 1] = [SALES];

    match view {
        View::Sell | View::Settings => &[],
        View::Day if role >= crate::auth::Role::Manager => &DAY_TABS,
        View::Day => &CASHIER_DAY,
        View::Shop if role >= crate::auth::Role::Manager => &SHOP_TABS,
        // The back office is the manager's, all of it.
        View::Shop => &[],
    }
}

/// The module a view opens on. Pressing the view's key lands here.
pub fn landing(view: View) -> usize {
    match view {
        View::Sell => SELL,
        View::Day => SALES,
        View::Shop => PRODUCTS,
        View::Settings => SETTINGS,
    }
}

/// Which view a module belongs to.
pub fn view_of(module: usize) -> View {
    match module {
        SELL => View::Sell,
        SALES | DASHBOARD => View::Day,
        SETTINGS => View::Settings,
        _ => View::Shop,
    }
}

/// Whether this role can reach a module at all.
///
/// Used to pull a persisted index back inside the rail after a sign-in, so a
/// cashier never inherits the manager's screen from the last session.
pub fn reachable(role: crate::auth::Role, module: usize) -> bool {
    let view = view_of(module);
    // A view with one page has no tab to point at, so its landing module is the
    // page. Every other module has to be listed on the strip.
    views(role).contains(&view) && (landing(view) == module || tabs(view, role).contains(&module))
}

/// What a tab is called on the strip and in the palette.
///
/// The label names the job, not the table: "Restock" is a thing a person does,
/// "Purchases" is a thing the database records.
pub fn tab_label(module: usize) -> &'static str {
    match module {
        SELL => "Sell",
        SALES => "Today",
        DASHBOARD => "Trends",
        PRODUCTS => "Catalog",
        PURCHASES => "Restock",
        SUPPLIERS => "Suppliers",
        CUSTOMERS => "Customers",
        _ => "Setup",
    }
}

/// What icon a tab uses on the strip.
pub fn tab_icon(module: usize) -> Icon {
    match module {
        SELL => Icon::CART,
        SALES => Icon::RECEIPT,
        DASHBOARD => Icon::DASHBOARD,
        PRODUCTS => Icon::PACKAGE,
        PURCHASES => Icon::TRUCK,
        SUPPLIERS => Icon::USER,
        CUSTOMERS => Icon::USER,
        _ => Icon::SETTINGS,
    }
}

// ---------------------------------------------------------------------------
// Who may open what
// ---------------------------------------------------------------------------

/// The permission a module asks for before it draws.
///
/// One table, read by [`open`], by the workspace guard and by the palette, so a
/// module cannot be reachable from one route and closed from another.
pub fn permission(index: usize) -> crate::auth::Permission {
    use crate::auth::Permission;

    match index {
        // Selling and reading the books are the job. A cashier who cannot see
        // yesterday's takings cannot answer the customer holding a receipt.
        // `ViewReports` is the scope, not the door: without it the screen still
        // opens, showing only the cashier's own sales.
        SELL | DASHBOARD | SALES => Permission::Sell,
        // The catalog now carries stock too. Changing prices is the page gate;
        // recording a movement is gated inside the workspace on
        // `ManageInventory`, so a reader without it still sees the counts.
        PRODUCTS => Permission::ChangePrice,
        SUPPLIERS => Permission::ManageSuppliers,
        // Planning and receiving a trip changes stock and cost, so it is the
        // same gate as recording a movement. A cashier never sees the screen.
        PURCHASES => Permission::ManageInventory,
        // Regulars used to live in Settings, so keep the same door. Moving the
        // page must not widen who can change a customer's rate.
        CUSTOMERS => Permission::EditSettings,
        _ => Permission::EditSettings,
    }
}

/// Switch module, asking for approval when the person cannot open it.
pub fn open(state: &mut State, index: usize) {
    let permission = permission(index);
    state.open(index, permission);
}

#[cfg(test)]
mod tests;
