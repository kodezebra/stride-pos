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
use crate::state::{self, SellView, State};

mod catalog;
mod day;
mod purchases;
mod sales;
mod sell;
mod settings;
mod suppliers;
mod widgets;

use catalog::*;
use day::*;
use purchases::*;
use sales::*;
use sell::*;
use settings::*;
use suppliers::*;
use widgets::*;

// The modals the shell opens by name, kept at the module root so the shell does
// not have to know which file a screen lives in.
pub use catalog::{import_modal, product_editor_modal};
pub use sales::sale_detail_modal;
pub use sell::cart_quantity_modal;
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

/// Number of modules. Used to clamp a persisted index from an older or newer
/// build before it is used as an array index.
pub const COUNT: usize = 7;

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

/// Shop's tabs: what is for sale, what is coming in, who brings it. Stock is
/// not a page of its own: it is a column and a filter on the catalog.
const SHOP_TABS: [usize; 3] = [PRODUCTS, PURCHASES, SUPPLIERS];

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
        _ => Permission::EditSettings,
    }
}

/// Switch module, asking for approval when the person cannot open it.
pub fn open(state: &mut State, index: usize) {
    let permission = permission(index);
    state.open(index, permission);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `COUNT` keeps a persisted index inside the list, and the view map is
    /// what keeps that index reachable. Both are easy to break when a module is
    /// added, and neither breakage is visible until a cashier lands on a screen
    /// the rail cannot show them — so pin them here.
    #[test]
    fn the_module_table_matches_its_index() {
        assert_eq!(
            all().len(),
            COUNT,
            "COUNT must equal the number of modules or a saved index can point past the end"
        );
    }

    #[test]
    fn the_rail_is_four_views_with_the_keys_in_order() {
        let manager = views(crate::auth::Role::Manager);
        assert_eq!(
            manager,
            &[View::Sell, View::Day, View::Shop, View::Settings][..]
        );

        // Four keys, in rail order, with no gaps: F1, F2, F3, F4.
        for (offset, view) in manager.iter().enumerate() {
            assert_eq!(
                view.hotkey() as u8,
                egui::Key::F1 as u8 + offset as u8,
                "{} should be F{}",
                view.label(),
                offset + 1
            );
        }

        let cashier = views(crate::auth::Role::Cashier);
        assert_eq!(cashier, &[View::Sell, View::Day][..]);
        assert!(
            !cashier.contains(&View::Shop) && !cashier.contains(&View::Settings),
            "a cashier rail must not offer the back office"
        );
    }

    #[test]
    fn every_module_is_a_tab_of_exactly_one_view() {
        for view in views(crate::auth::Role::Manager) {
            let tabs = tabs(*view, crate::auth::Role::Manager);
            // A view lands on its first tab, and a single-page view has none to
            // draw — but it still has a landing module.
            if let Some(first) = tabs.first() {
                assert_eq!(
                    *first,
                    landing(*view),
                    "{} lands on its first tab",
                    view.label()
                );
            }
            for &index in tabs {
                assert_eq!(
                    view_of(index),
                    *view,
                    "{} is on the wrong view",
                    tab_label(index)
                );
            }
        }

        // No module is orphaned: each one is either the landing page of its
        // view or a tab on it.
        for index in 0..COUNT {
            let view = view_of(index);
            assert!(
                landing(view) == index || tabs(view, crate::auth::Role::Manager).contains(&index),
                "{} has no tab to live on",
                tab_label(index)
            );
        }
    }

    #[test]
    fn the_tabs_of_a_view_are_named_and_ordered() {
        assert_eq!(
            tabs(View::Day, crate::auth::Role::Manager),
            &[SALES, DASHBOARD][..]
        );
        assert_eq!(
            tabs(View::Shop, crate::auth::Role::Manager),
            &[PRODUCTS, PURCHASES, SUPPLIERS][..]
        );

        // A cashier reads the day's own sales; the trends that read the shop's
        // whole history are the manager's screen.
        assert_eq!(tabs(View::Day, crate::auth::Role::Cashier), &[SALES][..]);
        assert!(!reachable(crate::auth::Role::Cashier, DASHBOARD));
        assert!(!reachable(crate::auth::Role::Cashier, SETTINGS));
        assert!(reachable(crate::auth::Role::Cashier, SELL));
        assert!(reachable(crate::auth::Role::Manager, SETTINGS));

        for index in 0..COUNT {
            assert!(!tab_label(index).is_empty());
        }
    }

    fn day(day: &str, weekday: u8, revenue: i64) -> crate::db::DayTotal {
        crate::db::DayTotal {
            day: day.to_owned(),
            weekday,
            sales: 1,
            revenue,
        }
    }

    /// The axis names the month once and then only numbers the days, so the
    /// reading of `dd/mm` is the whole contract: a leading zero reads as a
    /// typo, and a misread month captions the chart with the wrong span.
    #[test]
    fn the_axis_names_the_week_and_numbers_the_month() {
        let week = day("26/10", 1, 0);
        assert_eq!(axis_label(&week, 7), "Mon");
        assert_eq!(axis_label(&week, 8), "Mon");
        assert_eq!(axis_label(&week, 9), "26");
        assert_eq!(axis_label(&day("01/11", 0, 0), 30), "1");
    }

    #[test]
    fn the_caption_says_the_span_once() {
        // Newest first, the order `by_day` hands them over in.
        let crossing = [day("01/11", 0, 0), day("26/10", 1, 0)];
        assert_eq!(period_label(&crossing), "26 Oct – 1 Nov");
        let one_month = [day("31/10", 6, 0), day("12/10", 1, 0)];
        assert_eq!(period_label(&one_month), "12–31 Oct");
        assert_eq!(period_label(&[day("01/11", 0, 0)]), "1 Nov");
        assert_eq!(period_label(&[]), "");
    }

    #[test]
    fn a_day_is_said_with_its_weekday() {
        assert_eq!(long_date(&day("01/11", 0, 0)), "Sun 1 Nov");
    }

    #[test]
    fn money_moves_as_a_percentage_and_counts_as_units() {
        assert_eq!(
            change(1_200, Some(1_000), true),
            ("+20%".to_owned(), Trend::Up)
        );
        assert_eq!(
            change(800, Some(1_000), true),
            ("-20%".to_owned(), Trend::Down)
        );
        assert_eq!(change(3, Some(1), false), ("+2".to_owned(), Trend::Up));
        assert_eq!(change(1, Some(3), false), ("-2".to_owned(), Trend::Down));
        // A window that took nothing cannot be a percentage: the first sale in
        // it is new, and two empty windows have nothing to say.
        assert_eq!(change(5, Some(0), true), ("new".to_owned(), Trend::Up));
        assert_eq!(change(0, Some(0), true), ("—".to_owned(), Trend::Flat));
        // All time has no previous window at all.
        assert_eq!(change(5, None, false), ("—".to_owned(), Trend::Flat));
    }

    #[test]
    fn the_comparison_says_the_window_and_whether_it_is_the_shop() {
        assert_eq!(comparison_caption(0, true), "compared with yesterday");
        assert_eq!(
            comparison_caption(1, true),
            "compared with the 7 days before"
        );
        assert_eq!(comparison_caption(3, true), "everything the shop has sold");
        assert_eq!(
            comparison_caption(1, false),
            "only your sales · compared with the 7 days before"
        );
        assert_eq!(comparison_caption(3, false), "only your sales");
        // Only all time needs to admit that its chart is capped.
        assert_eq!(chart_caption(0), None);
        assert_eq!(chart_caption(3), Some("The latest 30 selling days"));
    }

    fn product(id: i64, name: &str, barcode: Option<&str>) -> Product {
        Product {
            id,
            name: name.to_owned(),
            price_minor: 1_000,
            category: "Drinks".to_owned(),
            category_id: None,
            barcode: barcode.map(str::to_owned),
            unit_label: "unit".to_owned(),
            pack_size: 1,
            pack_price_minor: None,
            pack_barcode: None,
            cost_milli: None,
        }
    }

    /// A barcode is what the scanner sends, and a product *name* can contain the
    /// same digits. If the name won, the wrong item would land in the cart and the
    /// till would be off by the difference — silently.
    #[test]
    fn a_scanned_barcode_beats_a_name_that_contains_it() {
        let products = [
            product(1, "Cola 6001234 crate", None),
            product(2, "Cola", Some("6001234")),
        ];
        let visible = visible_products(&products, None, "6001234");

        assert_eq!(visible.len(), 2, "both read as matches before resolution");
        assert_eq!(
            scan_target(&products, "6001234", &visible),
            Scan::One {
                index: 1,
                unit_factor: 1
            }
        );
    }

    #[test]
    fn the_only_product_in_view_is_the_one_a_scan_takes() {
        let products = [product(1, "Cola", None), product(2, "Fanta", None)];
        let visible = visible_products(&products, None, "fan");

        assert_eq!(
            scan_target(&products, "fan", &visible),
            Scan::One {
                index: 1,
                unit_factor: 1
            }
        );
    }

    /// Two tiles are a choice the cashier has to make. Picking the first would
    /// put the wrong product in the cart with no way to tell it had happened.
    #[test]
    fn two_matches_are_a_question_not_a_choice() {
        let products = [product(1, "Cola 500", None), product(2, "Cola 1L", None)];
        let visible = visible_products(&products, None, "cola");

        assert_eq!(scan_target(&products, "cola", &visible), Scan::Ambiguous(2));
    }

    /// The sidebar is a browse tool, not a restriction. A cashier who left a
    /// category selected must still be able to scan anything on the shelf.
    #[test]
    fn a_scan_ignores_the_category_sidebar() {
        let products = [product(1, "Cola", Some("6001234"))];
        let visible = visible_products(&products, Some("Food"), "");

        assert!(visible.is_empty(), "the sidebar hides the product");
        assert_eq!(
            scan_target(&products, "6001234", &visible),
            Scan::One {
                index: 0,
                unit_factor: 1
            }
        );
    }

    /// An exact name is the cashier confirming which of the matches they meant.
    #[test]
    fn an_exact_name_beats_a_looser_match() {
        let products = [product(1, "Cola 500", None), product(2, "Cola", None)];
        let visible = visible_products(&products, None, "cola");

        assert_eq!(
            scan_target(&products, "Cola", &visible),
            Scan::One {
                index: 1,
                unit_factor: 1
            }
        );
    }

    #[test]
    fn an_unknown_code_takes_nothing() {
        let products = [product(1, "Cola", Some("6001234"))];
        let visible = visible_products(&products, None, "999");

        assert_eq!(scan_target(&products, "999", &visible), Scan::None);
        assert_eq!(scan_target(&products, "   ", &visible), Scan::None);
    }

    /// The box code is one scan of one thing: the line must carry the pack's
    /// size so the shelf loses a hundred tablets, not one. The loose code still
    /// buys a single tablet from the same row.
    #[test]
    fn a_pack_barcode_adds_a_pack_but_the_loose_code_adds_one() {
        let mut medicine = product(1, "Amoxicillin", Some("100100"));
        medicine.unit_label = "tablet".to_owned();
        medicine.pack_size = 100;
        medicine.pack_price_minor = Some(2_800);
        medicine.pack_barcode = Some("100200".to_owned());
        let products = [medicine];

        let loose = visible_products(&products, None, "100100");
        assert_eq!(
            scan_target(&products, "100100", &loose),
            Scan::One {
                index: 0,
                unit_factor: 1
            }
        );

        let pack = visible_products(&products, None, "100200");
        assert_eq!(
            pack,
            vec![0],
            "typing the box code finds the row before the scan resolves"
        );
        assert_eq!(
            scan_target(&products, "100200", &pack),
            Scan::One {
                index: 0,
                unit_factor: 100
            }
        );
    }

    /// The two product buttons answer different questions: one is "what I am
    /// looking at", the other "everything this shop sells". With a filter on,
    /// the files must differ in rows, not just in name.
    #[test]
    fn a_filtered_export_is_shorter_than_the_whole_catalog() {
        let mut products = [
            product(1, "Cola", None),
            product(2, "Bread", None),
            product(3, "Milk", None),
        ];
        products[1].category = "Food".to_owned();
        products[2].category = "Food".to_owned();

        let visible = visible_products(&products, Some("Food"), "");
        let filtered: Vec<Product> = visible
            .iter()
            .map(|&index| products[index].clone())
            .collect();

        let currency = money::tender_from_code("UGX").expect("UGX is a tender");
        assert_eq!(
            crate::export::products_csv(&filtered, currency)
                .lines()
                .count(),
            3,
            "a header and the two Food products"
        );
        assert_eq!(
            crate::export::products_csv(&products, currency)
                .lines()
                .count(),
            4,
            "a header and all three products, whatever the sidebar says"
        );
    }

    fn supplier(id: &str, name: &str, contact: Option<&str>, phone: Option<&str>) -> Supplier {
        Supplier {
            id: id.to_owned(),
            name: name.to_owned(),
            contact_name: contact.map(str::to_owned),
            phone: phone.map(str::to_owned),
            email: None,
            address: None,
            notes: None,
            archived: false,
        }
    }

    /// Archiving is the supplier equivalent of switching a staff account off:
    /// the row stays, but the next order should not start from it.
    #[test]
    fn the_directory_hides_archived_suppliers_until_asked() {
        let mut suppliers = [
            supplier("a", "Acme", Some("Amina"), Some("+256 700")),
            supplier("b", "Beta", None, None),
        ];
        suppliers[1].archived = true;

        assert_eq!(
            visible_suppliers(&suppliers, "", false),
            [0],
            "the working list leaves archived suppliers out"
        );
        assert_eq!(
            visible_suppliers(&suppliers, "", true),
            [0, 1],
            "showing archived keeps name order"
        );
    }

    /// The search is the directory's whole job: a shop may remember the rep's
    /// name or the number long before it remembers the company's legal name.
    #[test]
    fn the_supplier_search_reaches_contact_and_phone() {
        let suppliers = [
            supplier("a", "Acme", Some("Amina"), Some("+256 700")),
            supplier("b", "Beta", None, None),
        ];

        assert_eq!(visible_suppliers(&suppliers, "acme", false), [0]);
        assert_eq!(
            visible_suppliers(&suppliers, "amina", false),
            [0],
            "the contact is searchable"
        );
        assert_eq!(
            visible_suppliers(&suppliers, "700", false),
            [0],
            "the phone number is searchable"
        );
        assert!(visible_suppliers(&suppliers, "nothing", false).is_empty());
    }

    /// One window width is a layout, two are a contract. The bar shares one row
    /// between the page's search field and its actions, so the two ways it can
    /// break are a field that eats the buttons at the minimum window and a
    /// cluster that runs under the field on a wide one. Both are cheap to catch
    /// here and awkward to notice on a screen with the right number of products.
    #[test]
    fn the_page_bar_fits_its_search_and_its_actions_on_one_row() {
        // 880×560 is the smallest window the app allows: the contents minus the
        // 280 px sidebar and the 24 px workspace margins each side.
        for (window, expected_field) in [(552.0_f32, SEARCH_MIN), (1112.0, SEARCH_MAX)] {
            let ctx = egui::Context::default();
            // The bar draws semibold text, and semibold is a font family the
            // app installs at startup; a bare context would panic on the first
            // word of a button.
            let (palette, dark) = crate::theme::palette_for(ui_kit::theme::Theme::Light);
            ui_kit::theme::install(&ctx, palette, dark);

            let mut query = String::from("milk");
            let mut field = egui::Rect::NOTHING;
            let mut actions = egui::Rect::NOTHING;
            let mut consumed = 0.0;

            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(window, 560.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let before = ui.cursor().top();
                    let response = action_bar(
                        ui,
                        PageLeft::Search {
                            text: &mut query,
                            hint: "Name or barcode",
                            count: Some("3 of 12".to_owned()),
                        },
                        |ui| {
                            let _ = actions::primary_compact(ui, "New product");
                            actions::toolbar_menu(ui, transfer_icon(), "Import / Export", |_| {});
                            actions = ui.min_rect();
                        },
                    );
                    field = response.expect("the bar has a search field").rect;
                    consumed = ui.cursor().top() - before;
                },
            );
            output.drop_without_applying_deltas();

            assert!(
                (field.width() - expected_field).abs() < 0.5,
                "a {window} px bar gave the search {:.1} px, not {expected_field}",
                field.width()
            );
            assert!(
                actions.left() >= field.right(),
                "the actions start at {:.1} but the field ends at {:.1} in a {window} px bar",
                actions.left(),
                field.right()
            );
            assert!(
                actions.right() <= window + 0.5,
                "the actions run past the bar's right edge in a {window} px bar"
            );
            // The field sits on the bar's centre line, not on its top edge: a
            // 52 px row with a 34 px control left-aligned looks like a mistake
            // in both themes.
            assert!(
                (field.center().y - TOOLBAR_HEIGHT / 2.0).abs() < 1.0,
                "the search sits at {:.1} px in a {TOOLBAR_HEIGHT} px bar",
                field.center().y
            );
            assert!(
                (field.height() - SEARCH_HEIGHT).abs() < 0.5,
                "the search is {:.1} px tall, not {SEARCH_HEIGHT}",
                field.height()
            );
            // The bar plus the gap before the page, and nothing beyond that: a
            // bar that claims more pushes the first row of the table down. The
            // gap is the bar's own 8 px on top of the row spacing every panel
            // pays between two widgets.
            assert!(
                (TOOLBAR_HEIGHT..=TOOLBAR_HEIGHT + SPACING.sm + 8.0).contains(&consumed),
                "the bar consumed {consumed} px of a 560 px window"
            );
        }
    }
}
