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
//! There are three keys, not eight: F1, F2 and F3 open the three views, and the
//! tabs inside a view are clicked. A key per page does not survive contact with
//! a fourth page, and a till has no room for a function-key crib sheet.

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
use crate::db::{Product, Purchase, PurchaseStatus, StockItem, StockKind, Supplier};
use crate::picker::ExportKind;
use crate::state::{self, SellView, State};

// Indices into `all()`. Order is the persisted value, so these numbers are as
// permanent as the module list. It is not the order anything is drawn in: see
// [`views`] for the rail and [`tabs`] for what sits inside each view.
pub const SELL: usize = 0;
pub const DASHBOARD: usize = 1;
pub const PRODUCTS: usize = 2;
pub const SALES: usize = 3;
pub const SETTINGS: usize = 4;
pub const SUPPLIERS: usize = 5;
pub const INVENTORY: usize = 6;
pub const PURCHASES: usize = 7;

/// Number of modules. Used to clamp a persisted index from an older or newer
/// build before it is used as an array index.
pub const COUNT: usize = 8;

// ---------------------------------------------------------------------------
// Views and tabs
// ---------------------------------------------------------------------------

/// The three places a till has.
///
/// A view is a whole way of working, not a page: Sell is the counter, Day is
/// what the shop did, Shop is what the shop is. A tab is one page of that job,
/// so it is flat — nothing hides behind a tab, and nothing behind a tab hides
/// behind another one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Sell,
    Day,
    Shop,
}

impl View {
    pub fn label(self) -> &'static str {
        match self {
            Self::Sell => "Sell",
            Self::Day => "Day",
            Self::Shop => "Shop",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Self::Sell => Icon::CART,
            Self::Day => Icon::DASHBOARD,
            Self::Shop => Icon::BOXES,
        }
    }

    /// The key that jumps here. Three keys, no modifiers: the whole rail fits
    /// under the left hand.
    pub fn hotkey(self) -> egui::Key {
        match self {
            Self::Sell => egui::Key::F1,
            Self::Day => egui::Key::F2,
            Self::Shop => egui::Key::F3,
        }
    }
}

/// Day's tabs: the sales that happened, and what they add up to.
const DAY_TABS: [usize; 2] = [SALES, DASHBOARD];

/// Shop's tabs: what is for sale, what is on the shelf, what is coming in, who
/// brings it, and how the till itself is set up.
const SHOP_TABS: [usize; 5] = [PRODUCTS, INVENTORY, PURCHASES, SUPPLIERS, SETTINGS];

/// Width of the cart column inside the Sell workspace.
///
/// Not a shell panel on purpose: the cart belongs to selling, and a cashier who
/// switches to Settings should not drag a cart's worth of dead pixels along.
/// Promoting it to `regions::sidebar`-style right panel later is a local change
/// in [`Sell::workspace`].
///
/// It is a share of the workspace with a floor and a ceiling: a fixed width is
/// too wide on a 960 px till and too narrow on a 4K one.
const CART_MIN: f32 = 300.0;
const CART_MAX: f32 = 380.0;
const CART_SHARE: f32 = 0.34;

/// Height of one product tile. Two lines of name plus a price line fit.
const TILE_HEIGHT: f32 = 96.0;
/// Narrowest a tile may get before the grid drops a column.
const TILE_MIN: f32 = 140.0;
/// Height of one table row, including its breathing room.
const ROW_HEIGHT: f32 = 40.0;
/// Height of a module's pinned action strip.
///
/// It matches the main button height, so a compact action does not look like a
/// different class of control just because it left the page body.
const TOOLBAR_HEIGHT: f32 = 44.0;
/// Height of the daily-revenue chart, axis labels and date strip included.
const CHART_HEIGHT: f32 = 232.0;
/// Steps in the chart's value axis.
const AXIS_STEPS: usize = 4;
/// `%w` names, Sunday first, because that is how SQLite numbers the week.
const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
/// Months for the span over the plot — `strftime` hands back numbers only.
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

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

/// Lay out one module's pinned action strip.
///
/// The title is the left cell, so the page keeps the same heading position
/// whether or not it has pinned actions. The actions run right to left, like a
/// dialog footer: the first action added is the rightmost one. The strip draws
/// its own hairline and the gap before the page.
fn action_bar(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    let palette = theme::current(ui.ctx());
    let full = ui.available_rect_before_wrap();
    let rect = egui::Rect::from_min_size(full.min, egui::vec2(full.width(), TOOLBAR_HEIGHT));

    let mut bar = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    bar.spacing_mut().item_spacing.x = SPACING.sm;
    bar.label(
        egui::RichText::new(title)
            .family(theme::semibold())
            .size(TYPE.heading)
            .color(egui::Color32::from(palette.text)),
    );
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
        Box::new(Inventory),
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
    const MANAGER: [View; 3] = [View::Sell, View::Day, View::Shop];

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
        View::Sell => &[],
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
    }
}

/// Which view a module belongs to.
pub fn view_of(module: usize) -> View {
    match module {
        SELL => View::Sell,
        SALES | DASHBOARD => View::Day,
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
        INVENTORY => "Stock",
        PURCHASES => "Restock",
        SUPPLIERS => "Suppliers",
        _ => "Setup",
    }
}

// ---------------------------------------------------------------------------
// Sell
// ---------------------------------------------------------------------------

/// The till: the catalog on the left, the cart on the right.
struct Sell;

impl Module for Sell {
    /// Fixed: a drag that starts as a missed category button and ends as a
    /// 40 px sidebar is a mistake the cashier pays for.
    fn fixed_sidebar(&self) -> Option<f32> {
        Some(regions::size::SIDEBAR_MIN)
    }

    /// The till owns its own frame: grid on the left, cart pinned right. A page
    /// scroll would make the cart's height unbounded and un-pin the Pay button.
    fn scrolls(&self) -> bool {
        false
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, state: &mut State) {
        categories_sidebar(ui, state);
    }

    fn workspace(&mut self, ui: &mut egui::Ui, state: &mut State) {
        // Two columns, placed by rectangle rather than by accumulating widths.
        // The cart's right edge *is* the panel's right edge, so it cannot end up
        // one pixel too wide and clip its own totals — and it keeps a floor on a
        // 960 px window instead of squeezing the grid to nothing.
        let gap = SPACING.md;
        let full = ui.available_rect_before_wrap();
        let cart_width = (full.width() * CART_SHARE).clamp(CART_MIN, CART_MAX);
        let cart_width = cart_width.min((full.width() - TILE_MIN - gap).max(CART_MIN));

        let cart_rect =
            egui::Rect::from_min_max(egui::pos2(full.max.x - cart_width, full.min.y), full.max);
        let grid_rect = egui::Rect::from_min_max(
            full.min,
            egui::pos2((cart_rect.min.x - gap).max(full.min.x), full.max.y),
        );

        let mut grid_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(grid_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        // Paying owns the keyboard and the pointer. The grid stays exactly where
        // it was — the cashier's map of the screen survives the sale — but it
        // dims and stops accepting input, so a mis-click while counting notes
        // cannot add a product to a basket that is already being paid for.
        if state.is_tendering() {
            grid_ui.set_opacity(0.35);
            grid_ui.disable();
        }
        self.products(&mut grid_ui, state);

        let mut cart_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(cart_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        // The cart is a card, not a bare column: a till's most important pane
        // should not read as leftover space next to the grid.
        components::card(&mut cart_ui, |ui| {
            ui.set_min_width(ui.available_width());
            cart(ui, state);
        });

        // Claim the whole row so the workspace grows by its real height, not by
        // whichever column happened to be taller.
        ui.advance_cursor_after_rect(full);
    }
}

impl Sell {
    fn products(&mut self, ui: &mut egui::Ui, state: &mut State) {
        components::field_label(ui, "Scan or search");
        ui.add_space(SPACING.xs);
        // The view switch shares the search row: the cashier's eye is already
        // here, and a full-width field above a lone pair of buttons would push
        // the first product down for a control that is only a preference.
        let response = ui
            .horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    sell_view_toggle(ui, &mut state.sell_view);
                    ui.add_space(SPACING.sm);
                    components::search_field(ui, &mut state.search, "Name or barcode")
                })
                .inner
            })
            .inner;
        // The cursor belongs here whenever the selling screen opens: nothing else
        // on Sell takes text, and a scanner types into whatever has focus.
        if std::mem::take(&mut state.scan_focus) {
            response.request_focus();
        }
        // A barcode scanner is a keyboard that types and then presses Enter. The
        // field already filtered the products, but a cashier with a scanner in
        // their hand expects the item in the cart — asking them to click a tile
        // they just scanned is the whole bug. `lost_focus` alone would also fire
        // on a click elsewhere, so the Enter press is what confirms the scan. A
        // phone scan has no keyboard to press Enter, so it sets `scan_submit`
        // and the product panel consumes it here, through the same path.
        // A scan that arrives mid-payment is consumed and dropped: the basket is
        // frozen until the sale is written. Leaving the flag set would fire it
        // at the next customer, which is a different bug.
        let submitted = (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
            || std::mem::take(&mut state.scan_submit);
        let submitted = submitted && !state.is_tendering();
        ui.add_space(SPACING.md);

        // A till with an empty catalog is not a bug in the product panel: it is a
        // shop that has not entered its products yet. Point at the screen that
        // fixes it instead of showing an empty view with no explanation.
        if state.products.is_empty() {
            components::empty_state_card(
                ui,
                "No products yet",
                "Add the shop's first product and it appears here as a key.",
                200.0,
            );
            ui.add_space(SPACING.sm);
            ui.vertical_centered(|ui| {
                if actions::primary(ui, "Add a product").clicked() {
                    // Straight into the form: an empty catalog is a dead end
                    // otherwise, and the operator has already said what they want.
                    state.select(PRODUCTS);
                    state.start_product();
                }
            });
            return;
        }

        let needle = state.search.trim().to_lowercase();
        let mut visible = visible_products(&state.products, state.category.as_deref(), &needle);

        if submitted {
            let scanned = state.search.trim().to_owned();
            // Enter on an empty field is the till's fourth keystroke: scan,
            // scan, Enter, cash. There is no barcode to look up, so it means
            // "that is the basket" and the cart's footer becomes the tender.
            if scanned.is_empty() {
                if !state.cart.is_empty() {
                    state.start_payment();
                }
                response.request_focus();
            } else {
                match scan_target(&state.products, &scanned, &visible) {
                    Scan::One(index) => {
                        // Cloned because the product panel holds `state.products`
                        // borrowed and `add_to_cart` needs the whole state.
                        let product = state.products[index].clone();
                        state.add_to_cart(&product);
                        // Clear the field so the next scan starts clean, and keep the
                        // focus so it lands here without a mouse.
                        state.search.clear();
                        response.request_focus();
                        // The panel is drawn this frame from `visible`: recompute it,
                        // otherwise a successful scan leaves the panel showing only
                        // the product it just moved to the cart until the next repaint.
                        visible = visible_products(&state.products, state.category.as_deref(), "");
                    }
                    // Say what happened in words the cashier can act on. `add_to_cart`
                    // clears `notice`, so these are set after it, never before.
                    Scan::Ambiguous(count) => {
                        state.notice = Some(format!(
                            "{count} products match {scanned} — scan a barcode or type more."
                        ));
                    }
                    Scan::None => {
                        state.notice = Some(format!("No product matches {scanned}."));
                    }
                }
            }
        }

        if visible.is_empty() {
            components::empty_state_card(
                ui,
                "Nothing matches",
                "Clear the search or pick another category.",
                160.0,
            );
            return;
        }

        let clicked = match state.sell_view {
            SellView::Grid => product_tiles(ui, state, &visible),
            SellView::List => product_rows(ui, state, &visible),
        };

        if let Some(index) = clicked {
            let product = state.products[index].clone();
            state.add_to_cart(&product);
        }
    }
}

/// Two buttons that choose how the Sell screen arranges its products.
///
/// One bordered container, not two loose buttons: the pair is a single choice,
/// and a gap between them would read as two independent actions.
fn sell_view_toggle(ui: &mut egui::Ui, view: &mut SellView) {
    let palette = theme::current(ui.ctx());
    egui::Frame::NONE
        .stroke(egui::Stroke::new(1.0_f32, palette.border))
        .corner_radius(egui::CornerRadius::same(RADII.sm as u8))
        .inner_margin(egui::Margin::same(3))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(|ui| {
                for (mode, label) in [(SellView::List, "List"), (SellView::Grid, "Grid")] {
                    let selected = *view == mode;
                    let text = egui::RichText::new(label).size(TYPE.small);
                    let text = if selected {
                        text.family(theme::semibold())
                            .color(egui::Color32::from(palette.accent_ink()))
                    } else {
                        text.color(egui::Color32::from(palette.text_muted))
                    };
                    let button = egui::Button::new(text)
                        .min_size(egui::vec2(52.0, 36.0))
                        .fill(if selected {
                            egui::Color32::from(palette.accent_soft)
                        } else {
                            egui::Color32::TRANSPARENT
                        })
                        .stroke(egui::Stroke::NONE)
                        .corner_radius(egui::CornerRadius::same(RADII.sm as u8));
                    if ui.add(button).clicked() {
                        *view = mode;
                    }
                }
            });
        });
}

/// The touch layout: touch-sized tiles in rows of as many as the panel holds.
///
/// Tile by rectangle, not by wrapping. The column count is arithmetic on the
/// panel width, so a product keeps its place as the window resizes and a
/// two-line label can never push its neighbour onto the next row. Cell width
/// follows the count, so a narrow panel fills instead of leaving a stub of dead
/// space next to every key.
fn product_tiles(ui: &mut egui::Ui, state: &State, visible: &[usize]) -> Option<usize> {
    let gap = SPACING.sm;
    let available = ui.available_width();
    let columns = (((available + gap) / (TILE_MIN + gap)).floor() as usize).clamp(1, 6);
    let cell = egui::vec2(
        (available - (columns as f32 - 1.0) * gap) / columns as f32,
        TILE_HEIGHT,
    );

    // The click is recorded and applied after the loop: `state.products` is
    // borrowed for the whole grid, so the cart cannot be touched inside it.
    let mut clicked = None;
    egui::ScrollArea::vertical()
        .id_salt("sell_grid")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let area = ui.available_rect_before_wrap();
            for (index, &product_index) in visible.iter().enumerate() {
                let product = &state.products[product_index];
                let column = (index % columns) as f32;
                let row = (index / columns) as f32;
                let min = egui::pos2(
                    area.left() + column * (cell.x + gap),
                    area.top() + row * (cell.y + gap),
                );
                let rect = egui::Rect::from_min_size(min, cell);
                let price = state.money(product.price_minor);
                let tile = components::Tile {
                    name: &product.name,
                    price: &price,
                };
                if ui.put(rect, tile).clicked() {
                    clicked = Some(product_index);
                }
            }

            let rows = visible.len().div_ceil(columns);
            let height = rows as f32 * cell.y + rows.saturating_sub(1) as f32 * gap;
            ui.advance_cursor_after_rect(egui::Rect::from_min_size(
                area.min,
                egui::vec2(area.width(), height),
            ));
        });

    clicked
}

/// The list layout: one full-width row per product, name left, price right.
///
/// A list is for products whose names are long enough that a tile would cut
/// them, so the row is one line high and the name wears the ellipsis, never the
/// price.
fn product_rows(ui: &mut egui::Ui, state: &State, visible: &[usize]) -> Option<usize> {
    let mut clicked = None;
    egui::ScrollArea::vertical()
        .id_salt("sell_list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = SPACING.xs;
            for &product_index in visible {
                let product = &state.products[product_index];
                let price = state.money(product.price_minor);
                if sell_row(ui, &product.name, &price).clicked() {
                    clicked = Some(product_index);
                }
            }
        });

    clicked
}

/// One product in the Sell list: name and price on a row that is one big
/// click target.
fn sell_row(ui: &mut egui::Ui, name: &str, price: &str) -> egui::Response {
    let palette = theme::current(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_HEIGHT),
        egui::Sense::click(),
    );

    if ui.is_rect_visible(rect) {
        let hovered = ui.rect_contains_pointer(rect);
        let fill = if hovered {
            palette.surface_alt
        } else {
            palette.surface
        };
        let border = if hovered {
            palette.border_strong
        } else {
            palette.border
        };
        ui.painter().rect(
            rect,
            egui::CornerRadius::same(RADII.sm as u8),
            fill,
            egui::Stroke::new(1.0_f32, border),
            egui::StrokeKind::Inside,
        );

        let inner = rect.shrink2(egui::vec2(SPACING.md, SPACING.xs));
        let mut row = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        row.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(components::numeric(price).size(TYPE.body));
            // The name gets the width the amount did not take; a long one wears
            // an ellipsis instead of pushing the price off the row.
            let width = (ui.available_width() - ui.spacing().item_spacing.x).max(0.0);
            let label =
                ui.add(egui::Label::new(components::semibold(name).size(TYPE.body)).truncate());
            if is_cut(ui, name, width) {
                label.on_hover_text(name);
            }
        });
    }

    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn cart(ui: &mut egui::Ui, state: &mut State) {
    let palette = theme::current(ui.ctx());

    ui.horizontal(|ui| {
        ui.label(components::semibold("Cart").size(TYPE.heading));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            components::caption(ui, &format!("{} item(s)", state.cart_count()));
        });
    });
    ui.add_space(SPACING.sm);
    components::divider(ui);

    // Reserve the footer first so the line list scrolls instead of pushing the
    // total off the pane. A till whose total can scroll away is broken.
    //
    // The footer grows by one line when cash rounding applies, because that row
    // only exists then and a fixed reserve would push the Pay button off the
    // bottom at the window's narrow end.
    //
    // Paying takes the whole footer, and the tender panel is much taller than
    // the totals it replaces. Its height is the sum of its rows — two 44.8
    // amount rows, a 44 field, 34 chips, a 52 button, and the hint row that
    // shares its line with "Back to cart" — plus 6px gaps and a little slack.
    // An error replaces the hint rather than stacking under it, so that case
    // does not need its own reserve.
    let rounding_minor = state.cart_rounding_minor();
    let footer = match state.payment.as_ref() {
        Some(_) => {
            410.0
                + if rounding_minor != 0 {
                    TYPE.small * 1.8
                } else {
                    0.0
                }
        }
        None if rounding_minor == 0 => 196.0,
        None => 196.0 + TYPE.small * 1.8,
    };
    let list_height = (ui.available_height() - footer).max(80.0);

    if state.cart.is_empty() {
        // The cart *is* the pane, so its hint fills the space the lines would
        // use rather than sitting at the top of a tall void. Reserving the same
        // height as the scroll area keeps the footer exactly where it lands
        // with a full cart.
        let rect = egui::Rect::from_min_size(
            ui.cursor().min,
            egui::vec2(ui.available_width(), list_height),
        );
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        components::empty_state_fill(
            &mut child,
            "Cart is empty",
            "Scan an item or tap a product.",
        );
        ui.advance_cursor_after_rect(rect);
    } else {
        egui::ScrollArea::vertical()
            .id_salt("sell_cart_lines")
            .max_height(list_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut remove = None;
                let mut edit = None;
                let mut steps: Vec<(usize, components::Step)> = Vec::new();

                for (index, line) in state.cart.iter().enumerate() {
                    // The name and the line total share the first row, with the
                    // amount placed first from the right so the name truncates
                    // into what is left. A long product name then costs an
                    // ellipsis, never the price or the stepper.
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                components::numeric(state.money(line.total_minor()))
                                    .size(TYPE.body),
                            );

                            // The child is sized to the whole remainder, so a
                            // short name stays on the left edge instead of
                            // hugging the amount.
                            let width =
                                (ui.available_width() - ui.spacing().item_spacing.x).max(0.0);
                            ui.allocate_ui_with_layout(
                                egui::vec2(width, 0.0),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    let name = ui.add(
                                        egui::Label::new(
                                            components::semibold(&line.name).size(TYPE.body),
                                        )
                                        .truncate(),
                                    );
                                    // Only when something is hidden: a tooltip
                                    // repeating a name already on screen is
                                    // noise on every hover.
                                    if is_cut(ui, &line.name, width) {
                                        name.on_hover_text(&line.name);
                                    }
                                },
                            );
                        });
                    });

                    // Quantity and unit price under the name, the stepper under
                    // the amount: the control belongs to the number it changes.
                    //
                    // The count in the middle of the stepper is a button. A till
                    // is a touch screen, so there is no hover to reveal that the
                    // quantity is editable; tapping the number opens the keypad,
                    // which is how a cashier sells ten of something without
                    // tapping `+` ten times.
                    ui.horizontal(|ui| {
                        ui.label(
                            components::numeric(format!("× {}", state.money(line.unit_minor)))
                                .size(TYPE.small)
                                .color(palette.text_muted),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            // The stepper is minus, count, plus everywhere else it
                            // appears. `horizontal` inherits the parent's
                            // direction, so inside this right-to-left row it
                            // would read plus, count, minus — "more" before
                            // "less", which is the opposite of every other
                            // stepper in the app.
                            let (step, count) = ui
                                .with_layout(
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| components::stepper(ui, line.qty),
                                )
                                .inner;
                            if count.on_hover_text("Set quantity").clicked() {
                                edit = Some(index);
                            }
                            if step != components::Step::None {
                                steps.push((index, step));
                            }
                        });
                    });
                    ui.add_space(SPACING.sm);
                }

                for (index, step) in steps {
                    let line = &mut state.cart[index];
                    match step {
                        components::Step::Up => line.qty += 1,
                        components::Step::Down => {
                            line.qty -= 1;
                            if line.qty <= 0 {
                                remove = Some(index);
                            }
                        }
                        components::Step::None => {}
                    }
                }

                if let Some(index) = remove {
                    state.cart.remove(index);
                } else if let Some(index) = edit {
                    state.edit_cart_quantity(index);
                }
            });
    }

    ui.add_space(SPACING.sm);
    // Paying is not a screen: the footer becomes the tender panel in place.
    // The lines above stay visible, frozen, because the customer may still
    // change their mind about one of them.
    if state.payment.is_some() {
        crate::checkout::tender(ui, state);
        return;
    }
    let subtotal = state.cart_total_minor();
    amount_row(ui, "Subtotal", &state.money(subtotal), false);
    amount_row(ui, "Tax", "not configured", false);
    ui.add_space(SPACING.xs);
    // "Total" is the exact total, the number the receipt prints. The rounded
    // cash amount is the payment screen's, and it says so there: a cart that
    // showed 300 for a 340 basket would hide the 40 the shop is giving away.
    amount_row(ui, "Total", &state.money(subtotal), true);
    if rounding_minor != 0 {
        let due = state.money(state.due_minor());
        components::caption(ui, &format!("Rounds to {due} in cash"));
    }
    ui.add_space(SPACING.sm);

    if let Some(notice) = state.notice.as_deref() {
        components::caption(ui, notice);
        ui.add_space(SPACING.xs);
    }

    let pay = ui
        .add_sized(
            [ui.available_width(), 56.0],
            egui::Button::new(
                egui::RichText::new("Pay")
                    .size(TYPE.button)
                    .family(theme::semibold())
                    .color(palette.accent_text),
            )
            .fill(palette.accent)
            .stroke(egui::Stroke::NONE)
            .corner_radius(egui::CornerRadius::same(ui_kit::tokens::RADII.sm as u8)),
        )
        .on_hover_text("F12 or Ctrl+Enter");
    if pay.clicked() {
        state.start_payment();
    }
}

/// Whether `text` is wider than the space its label was given.
///
/// The cart asks before offering a tooltip: a name that already fits needs no
/// second copy on hover, and a truncated one is exactly the case where the full
/// name is worth the hover.
fn is_cut(ui: &egui::Ui, text: &str, width: f32) -> bool {
    let font = egui::FontId::new(TYPE.body, theme::semibold());
    let measured = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, egui::Color32::PLACEHOLDER)
        .size()
        .x;
    measured > width
}

/// One label/amount row: the cart's totals and the payment screen's due.
///
/// Shared with `checkout` on purpose — the amount the cashier reads on the cart
/// and the amount on the payment screen must sit on the same pixel.
pub(crate) fn amount_row(ui: &mut egui::Ui, label: &str, amount: &str, strong: bool) {
    let palette = theme::current(ui.ctx());
    // One right-to-left row of a *fixed* height, amount first: the total must
    // end on the same pixel as the Pay button above it, and the row must claim
    // only its own line. A bare `with_layout` child starts out as tall as the
    // space left in the pane, which would push every later row to the bottom.
    let height = if strong {
        TYPE.total * 1.4
    } else {
        TYPE.small * 1.8
    };
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), height),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            let text = components::numeric(amount);
            let text = if strong {
                text.size(TYPE.total).color(palette.text)
            } else {
                text.size(TYPE.small).color(palette.text_muted)
            };
            ui.label(text);

            if strong {
                ui.label(components::semibold(label).size(TYPE.body));
            } else {
                components::caption(ui, label);
            }
        },
    );
}

// ---------------------------------------------------------------------------
// Dashboard
// ---------------------------------------------------------------------------

struct Dashboard;

impl Module for Dashboard {
    fn sidebar(&mut self, ui: &mut egui::Ui, state: &mut State) {
        components::section_title(ui, "Range");
        for (index, label) in state::TREND_RANGES.iter().enumerate() {
            if regions::nav_item(ui, state.trend_range == index, label).clicked() {
                state.trend_range = index;
            }
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, _state: &mut State) {
        // No actions: every figure on the page is already in the range the
        // sidebar chose, and a trends page has nothing to add or export. The bar
        // stays because the page title has to sit where every other page's does.
        action_bar(ui, "Trends", |_ui| {});
    }

    fn workspace(&mut self, ui: &mut egui::Ui, state: &mut State) {
        let range = state::trend_window(state.trend_range);
        let scope = state.report_scope();
        let shop = scope.is_shop();
        let user = scope.user().map(str::to_owned);
        let user = user.as_deref();

        let (count, takings) = crate::db::totals(&state.store, range, user);
        let previous = crate::db::previous_totals(&state.store, range, user);
        let margin = crate::db::margin(&state.store, range, user);

        components::caption(
            ui,
            &format!(
                "{} · {}",
                state::range_caption(range),
                comparison_caption(range, shop)
            ),
        );
        ui.add_space(SPACING.md);

        if count == 0 {
            components::empty_state_card(
                ui,
                "No sales yet in this range",
                "Take a sale, and the shop's numbers start here.",
                200.0,
            );
            return;
        }

        // Six figures and no more. A shopkeeper asks four questions of a month:
        // what came in, what was kept, how many people paid, and what each of
        // them spent. The other two — how they paid, and what they bought — are
        // tables below, because a total cannot answer them.
        let (takings_sub, takings_trend) =
            change(takings, previous.map(|(_, revenue)| revenue), true);
        let (sales_sub, sales_trend) = change(count, previous.map(|(count, _)| count), false);
        let mut cards = vec![
            Kpi {
                label: "Takings",
                value: state.money(takings),
                sub: takings_sub,
                trend: takings_trend,
            },
            Kpi {
                label: "Sales",
                value: count.to_string(),
                sub: sales_sub,
                trend: sales_trend,
            },
            Kpi {
                label: "Average basket",
                value: state.money(takings / count),
                sub: "per sale".to_owned(),
                trend: Trend::Flat,
            },
        ];
        // Margin only where the shop records costs, and only over the items that
        // have one: a percentage across uncosted sales would be a guess wearing
        // a decimal point.
        if margin.has_cost() && margin.revenue_minor > 0 {
            let profit = margin.profit_minor();
            cards.push(Kpi {
                label: "Gross margin",
                value: format!(
                    "{:.0}%",
                    profit as f64 * 100.0 / margin.revenue_minor as f64
                ),
                sub: format!("{} kept", state.money(profit)),
                trend: Trend::Flat,
            });
        }
        kpi_row(ui, &cards);

        if margin.has_cost() && !margin.is_complete() {
            components::caption(
                ui,
                &format!(
                    "Margin covers {} of {} sale lines; the rest were sold before \
                     their cost was known.",
                    margin.costed_lines, margin.total_lines
                ),
            );
        }

        // Cash or not, because that is what decides whether the drawer has to be
        // counted tonight.
        ui.add_space(SPACING.lg);
        components::section_title(ui, "How it was paid");
        let methods = crate::db::by_method(&state.store, range, user);
        components::card(ui, |ui| {
            ui.set_min_width(ui.available_width());
            tally_row(ui, "Method", "Taken", "Share", true, false);
            for (index, method) in methods.iter().enumerate() {
                let share = if takings > 0 {
                    format!(
                        "{:.0}%",
                        method.amount_minor as f64 * 100.0 / takings as f64
                    )
                } else {
                    "—".to_owned()
                };
                tally_row(
                    ui,
                    &method_label(&method.method, method.sales),
                    &state.money(method.amount_minor),
                    &share,
                    false,
                    index % 2 == 1,
                );
            }
        });

        // One chart, because a shopkeeper reads a shape, not a spreadsheet. All
        // time has no calendar to draw, so its chart is capped and says so.
        ui.add_space(SPACING.lg);
        components::section_title(ui, "By day");
        if let Some(caption) = chart_caption(range) {
            components::caption(ui, caption);
        }
        let days = crate::db::by_day(&state.store, range, user);
        components::card(ui, |ui| {
            ui.set_min_width(ui.available_width());
            day_chart(ui, state, &days);
        });

        // Two sentences, because the numbers above do not say themselves out
        // loud: when the money came in, and what took the most of it.
        let busiest = days
            .iter()
            .max_by_key(|day| day.revenue)
            .filter(|day| day.revenue > 0);
        let peak = crate::db::peak_hour(&state.store, range, user);
        if busiest.is_some() || peak.is_some() {
            ui.add_space(SPACING.lg);
            components::card(ui, |ui| {
                ui.set_min_width(ui.available_width());
                if let Some(busiest) = busiest {
                    ui.label(
                        egui::RichText::new(format!(
                            "{} took the most: {}, {:.0}% of takings.",
                            weekday_sentence(busiest.weekday),
                            state.money(busiest.revenue),
                            busiest.revenue as f64 * 100.0 / takings as f64
                        ))
                        .size(TYPE.body),
                    );
                }
                if let Some(peak) = peak {
                    if busiest.is_some() {
                        ui.add_space(SPACING.xs);
                    }
                    ui.label(
                        egui::RichText::new(format!(
                            "Most sales land around {:02}:00 — {} of them.",
                            peak.hour, peak.sales
                        ))
                        .size(TYPE.body),
                    );
                }
            });
        }

        // What actually sells. Ranked by the money each name took, with the
        // units beside it — neither number alone is the story: many cheap units
        // can out-earn a few dear ones, and either way the owner wants both.
        ui.add_space(SPACING.lg);
        components::section_title(ui, if shop { "Top items" } else { "Your top items" });
        let items = crate::db::top_items(&state.store, range, 5, user);
        if items.is_empty() {
            components::empty_state_card(
                ui,
                "No items sold in this range",
                "Once the till rings things up, the best movers rank here.",
                160.0,
            );
        } else {
            components::card(ui, |ui| {
                ui.set_min_width(ui.available_width());
                tally_row(ui, "Item", "Sold", "Revenue", true, false);
                for (index, item) in items.iter().enumerate() {
                    tally_row(
                        ui,
                        &item.name,
                        &item.qty.to_string(),
                        &state.money(item.revenue),
                        false,
                        index % 2 == 1,
                    );
                }
            });
        }
    }
}

/// How a figure moved against the window before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Trend {
    Up,
    Down,
    Flat,
}

/// One card at the top of the dashboard.
struct Kpi {
    label: &'static str,
    value: String,
    /// The change against the previous window, or a plain reading of the figure
    /// ("per sale").
    sub: String,
    trend: Trend,
}

/// Cards laid out by width arithmetic, not by wrapping.
///
/// `horizontal_wrapped` decides where to break from the *parent's* available
/// width, which on a panel that already overflowed stays wrong for the rest of
/// the frame — the row spills past the window edge and drags every centred
/// widget below it out of place. Columns here are a function of the panel width,
/// so the row always sums to what is on screen.
fn kpi_row(ui: &mut egui::Ui, cards: &[Kpi]) {
    let gap = SPACING.sm;
    let available = ui.available_width();
    // Three figures in a four-column grid would leave a hole where the fourth
    // belongs, so the row never has more columns than it has cards.
    let columns = (((available + gap) / (140.0 + gap)).floor() as usize)
        .clamp(1, 4)
        .min(cards.len().max(1));
    let card = ((available - (columns as f32 - 1.0) * gap) / columns as f32).floor();

    for chunk in cards.chunks(columns) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for kpi in chunk {
                kpi_card(ui, card, kpi);
            }
        });
        ui.add_space(gap);
    }
}

fn kpi_card(ui: &mut egui::Ui, width: f32, kpi: &Kpi) {
    components::card(ui, |ui| {
        // The frame adds its margin around this width, so the card lands on the
        // column width instead of one margin too wide.
        ui.set_width((width - 2.0 * SPACING.lg).max(60.0));
        components::field_label(ui, kpi.label);
        ui.add_space(SPACING.xs);
        ui.label(components::numeric(&kpi.value).size(TYPE.kpi));
        ui.add_space(SPACING.xs);
        let palette = theme::current(ui.ctx());
        let color = match kpi.trend {
            Trend::Up => egui::Color32::from(palette.success),
            Trend::Down => egui::Color32::from(palette.danger),
            Trend::Flat => egui::Color32::from(palette.text_muted),
        };
        ui.label(egui::RichText::new(&kpi.sub).size(TYPE.small).color(color));
    });
}

/// A figure's change against the window before it, and the direction to paint.
///
/// Money moves as a percentage and counts as units: "+12%" is how a shopkeeper
/// reads a week's takings, and "+3" is how they read three more sales. A window
/// that took nothing cannot be a percentage, so the first sale in it is `new`;
/// two empty windows have nothing to compare and show a dash. All time passes no
/// previous window at all, and reads the same.
fn change(now: i64, before: Option<i64>, money: bool) -> (String, Trend) {
    let Some(before) = before else {
        return ("—".to_owned(), Trend::Flat);
    };
    if before == 0 {
        return if now == 0 {
            ("—".to_owned(), Trend::Flat)
        } else {
            ("new".to_owned(), Trend::Up)
        };
    }

    let delta = now - before;
    if delta == 0 {
        return ("—".to_owned(), Trend::Flat);
    }
    let trend = if delta > 0 { Trend::Up } else { Trend::Down };

    if money {
        let percent = (delta as f64 * 100.0 / before as f64).round() as i64;
        if percent == 0 {
            ("—".to_owned(), Trend::Flat)
        } else {
            (format!("{percent:+}%"), trend)
        }
    } else {
        (format!("{delta:+}"), trend)
    }
}

/// What the cards are being compared with, said once under the page title.
///
/// A continuation, not a sentence: it is joined to the range name on the same
/// line, so the page reads "Last 7 days · compared with the 7 days before"
/// rather than stacking two headings.
///
/// All time has no previous window, so its line says what the figures are
/// instead of promising a comparison.
fn comparison_caption(range: usize, shop: bool) -> &'static str {
    if shop {
        match range {
            0 => "compared with yesterday",
            1 => "compared with the 7 days before",
            2 => "compared with the 30 days before",
            _ => "everything the shop has sold",
        }
    } else {
        match range {
            0 => "only your sales · compared with yesterday",
            1 => "only your sales · compared with the 7 days before",
            2 => "only your sales · compared with the 30 days before",
            _ => "only your sales",
        }
    }
}

/// All time has no calendar to draw, so its chart is capped and says so.
fn chart_caption(range: usize) -> Option<&'static str> {
    match range {
        0..=2 => None,
        _ => Some("The latest 30 selling days"),
    }
}

/// `cash` as `Cash · 12 sales`: the column stores what the payment code writes,
/// and the page prints it the way a person would, with the count that says
/// whether the money came in one big sale or a dozen small ones.
fn method_label(method: &str, sales: i64) -> String {
    let mut chars = method.chars();
    let name: String = match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    };
    if sales == 1 {
        format!("{name} · 1 sale")
    } else {
        format!("{name} · {sales} sales")
    }
}

/// One row of a money table: identifier, when, amount.
///
/// Drawn by rectangle rather than with `Grid` because the amount must sit on the
/// card's right edge: a left-to-right grid sizes every column to its content, so
/// a column of totals ends up ragged in the middle of the pane.
/// One row of a sales table: sale number, when, by whom, and the amount paid.
///
/// Data rows answer clicks, which is what makes a sale worth talking about:
/// a list can only abbreviate, and the reverberating truth — lines, tender
/// and change — lives one tap away in the detail modal.
fn table_row(
    ui: &mut egui::Ui,
    id: &str,
    when: &str,
    by: &str,
    amount: &str,
    header: bool,
    stripe: bool,
) -> egui::Response {
    let palette = theme::current(ui.ctx());
    let width = ui.available_width();
    let height = if header { 30.0 } else { ROW_HEIGHT };
    let sense = if header {
        egui::Sense::hover()
    } else {
        egui::Sense::click()
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), sense);

    let painter = ui.painter();
    if stripe {
        painter.rect_filled(
            rect,
            egui::CornerRadius::ZERO,
            egui::Color32::from(palette.surface_alt),
        );
    }
    // The hover highlight replaces the stripe rather than layering on it, so
    // a row being reached for reads the same on even and odd rows.
    if response.hovered() {
        painter.rect_filled(
            rect,
            egui::CornerRadius::ZERO,
            egui::Color32::from(palette.surface),
        );
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    let color = if header {
        egui::Color32::from(palette.text_muted)
    } else {
        egui::Color32::from(palette.text)
    };
    let size = if header { TYPE.small } else { TYPE.body };
    let proportional = if header {
        theme::semibold()
    } else {
        egui::FontFamily::Proportional
    };
    let mono = theme::numeric();
    let y = rect.center().y;

    let left = painter.layout_no_wrap(id.to_owned(), egui::FontId::new(size, mono.clone()), color);
    painter.galley(
        egui::pos2(rect.left(), y - left.size().y * 0.5),
        left,
        color,
    );

    let middle = painter.layout_no_wrap(
        when.to_owned(),
        egui::FontId::new(size, proportional.clone()),
        color,
    );
    painter.galley(
        egui::pos2(rect.left() + width * 0.30, y - middle.size().y * 0.5),
        middle,
        color,
    );

    // Who took it. Empty on a till that has not been set up with staff yet, and
    // on sales from before the column existed.
    let who = painter.layout_no_wrap(
        by.to_owned(),
        egui::FontId::new(size, proportional.clone()),
        color,
    );
    painter.galley(
        egui::pos2(rect.left() + width * 0.55, y - who.size().y * 0.5),
        who,
        color,
    );

    let right = painter.layout_no_wrap(amount.to_owned(), egui::FontId::new(size, mono), color);
    painter.galley(
        egui::pos2(rect.right() - right.size().x, y - right.size().y * 0.5),
        right,
        color,
    );

    if header {
        painter.hline(
            rect.x_range(),
            rect.max.y,
            egui::Stroke::new(1.0_f32, palette.border),
        );
    }

    response
}

/// One row of a three-column tally: a name, a count, and the money.
///
/// Shared by "By person" (who, sales, take) and "Top items" (what, units,
/// revenue) on purpose: both are ranked lists whose middle column is a count
/// the shopkeeper glances at, and neither lines up under `table_row`'s four
/// columns — there is no time cell to reuse.
fn tally_row(ui: &mut egui::Ui, name: &str, count: &str, amount: &str, header: bool, stripe: bool) {
    let palette = theme::current(ui.ctx());
    let width = ui.available_width();
    let height = if header { 30.0 } else { ROW_HEIGHT };
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());

    let painter = ui.painter();
    if stripe {
        painter.rect_filled(
            rect,
            egui::CornerRadius::ZERO,
            egui::Color32::from(palette.surface_alt),
        );
    }

    let color = if header {
        egui::Color32::from(palette.text_muted)
    } else {
        egui::Color32::from(palette.text)
    };
    let size = if header { TYPE.small } else { TYPE.body };
    let proportional = if header {
        theme::semibold()
    } else {
        egui::FontFamily::Proportional
    };
    let mono = theme::numeric();
    let y = rect.center().y;

    let left = painter.layout_no_wrap(
        name.to_owned(),
        egui::FontId::new(size, proportional),
        color,
    );
    painter.galley(
        egui::pos2(rect.left(), y - left.size().y * 0.5),
        left,
        color,
    );

    let middle = painter.layout_no_wrap(
        count.to_owned(),
        egui::FontId::new(size, mono.clone()),
        color,
    );
    painter.galley(
        egui::pos2(rect.left() + width * 0.45, y - middle.size().y * 0.5),
        middle,
        color,
    );

    let right = painter.layout_no_wrap(amount.to_owned(), egui::FontId::new(size, mono), color);
    painter.galley(
        egui::pos2(rect.right() - right.size().x, y - right.size().y * 0.5),
        right,
        color,
    );

    if header {
        painter.hline(
            rect.x_range(),
            rect.max.y,
            egui::Stroke::new(1.0_f32, palette.border),
        );
    }
}

/// The range's daily revenue as a bar chart.
///
/// Painted rather than laid out widget by widget: a chart is one shape whose
/// parts must share a baseline and one scale, and independent rows cannot
/// promise either. `days` arrives newest first; the chart draws oldest on the
/// left, so the range reads left to right like a calendar.
fn day_chart(ui: &mut egui::Ui, state: &State, days: &[crate::db::DayTotal]) {
    let palette = theme::current(ui.ctx());
    let mono = theme::numeric();
    let width = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, CHART_HEIGHT), egui::Sense::hover());
    let painter = ui.painter();

    let hue = chart_hue(ui);
    // The slot wash is the series colour at a whisper, not the accent wash:
    // the highlight has to belong to the bar under it.
    let wash = egui::Color32::from_rgba_unmultiplied(hue.r(), hue.g(), hue.b(), 26);

    let peak = days.iter().map(|day| day.revenue).max().unwrap_or_default();
    let (axis_max, step) = nice_axis(peak);

    let muted = egui::Color32::from(palette.text_muted);

    // The month is said once, over the plot, so the ticks below can be bare
    // day numbers. `26 27 28` under a "26 Oct – 1 Nov" is a calendar; `26/10`
    // ten times over is a spreadsheet.
    let caption = painter.layout_no_wrap(
        period_label(days),
        egui::FontId::new(TYPE.small, mono.clone()),
        muted,
    );
    let caption_height = caption.size().y;
    painter.galley(
        egui::pos2(rect.right() - caption.size().x, rect.top()),
        caption,
        muted,
    );

    // A gutter for the value labels, the caption's row, and a strip under the
    // bars for the dates.
    let plot = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 46.0, rect.top() + caption_height + 10.0),
        egui::pos2(rect.right(), rect.bottom() - 24.0),
    );

    // Four steps of a round size. The baseline is the axis and gets the
    // stronger hairline; the rest are hints behind the bars.
    let ticks = (axis_max / step) as usize;
    for tick in 0..=ticks {
        let fraction = tick as f32 / ticks as f32;
        let y = plot.bottom() - plot.height() * fraction;
        painter.hline(
            plot.x_range(),
            y,
            egui::Stroke::new(
                1.0,
                if tick == 0 {
                    egui::Color32::from(palette.border_strong)
                } else {
                    egui::Color32::from(palette.border)
                },
            ),
        );
        let galley = painter.layout_no_wrap(
            compact_amount(step * tick as i64, state),
            egui::FontId::new(TYPE.small, mono.clone()),
            muted,
        );
        painter.galley(
            egui::pos2(
                plot.left() - 10.0 - galley.size().x,
                y - galley.size().y * 0.5,
            ),
            galley,
            muted,
        );
    }

    // One slot per day, with air on both sides of the bar so neighbouring days
    // never read as one block. The pointer selects a slot, so the highlight
    // and the tooltip always agree with the bar under it.
    let count = days.len();
    let slot = plot.width() / count as f32;
    let bar_width = (slot * 0.56).clamp(3.0, 36.0);
    let hovered = response
        .hover_pos()
        .filter(|pos| plot.contains(*pos))
        .map(|pos| (((pos.x - plot.left()) / slot) as usize).min(count - 1));

    for (index, day) in days.iter().rev().enumerate() {
        let centre = plot.left() + slot * (index as f32 + 0.5);

        if hovered == Some(index) {
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(centre - slot * 0.5 + 1.0, plot.top()),
                    egui::pos2(centre + slot * 0.5 - 1.0, plot.bottom()),
                ),
                egui::CornerRadius::same(6),
                wash,
            );
        }

        let fraction = (day.revenue as f32 / axis_max as f32).clamp(0.0, 1.0);
        let top = plot.bottom() - plot.height() * fraction;
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(centre - bar_width * 0.5, top),
                egui::pos2(centre + bar_width * 0.5, plot.bottom()),
            ),
            egui::CornerRadius {
                nw: 4,
                ne: 4,
                sw: 0,
                se: 0,
            },
            hue,
        );
    }

    // As many dates as fit without colliding, counted from the newest day so
    // the right edge is always labelled. A week is read as a rhythm, so its
    // days are named; a month is read as a calendar, so it is numbered. The
    // step comes from the widest label the axis will draw, so a long range
    // thins out instead of running the numbers together.
    let widest = days
        .iter()
        .map(|day| axis_label(day, count))
        .max_by_key(|label| label.len())
        .unwrap_or_default();
    let label_width = painter
        .layout_no_wrap(widest, egui::FontId::new(TYPE.small, mono.clone()), muted)
        .size()
        .x
        + 12.0;
    let every = (count as f32 * label_width / plot.width()).ceil().max(1.0) as usize;
    for (index, day) in days.iter().rev().enumerate() {
        if !(count - 1 - index).is_multiple_of(every) {
            continue;
        }
        let centre = plot.left() + slot * (index as f32 + 0.5);
        let galley = painter.layout_no_wrap(
            axis_label(day, count),
            egui::FontId::new(TYPE.small, mono.clone()),
            muted,
        );
        painter.galley(
            egui::pos2(centre - galley.size().x * 0.5, plot.bottom() + 6.0),
            galley,
            muted,
        );
    }

    if let Some(index) = hovered {
        let day = &days[count - 1 - index];
        let centre = plot.left() + slot * (index as f32 + 0.5);
        let fraction = (day.revenue as f32 / axis_max as f32).clamp(0.0, 1.0);
        let top = plot.bottom() - plot.height() * fraction;
        day_callout(painter, plot, centre, top, day, state, &palette);
    }
}

/// The chart's own colour.
///
/// Deliberately not `accent`: in this app blue is the colour of things you can
/// press, and a bar is a recorded fact, not a button. Violet also stays clear
/// of the green that means "paid" and the red that means "wrong", so a bar can
/// never be misread as a state.
fn chart_hue(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        egui::Color32::from_rgb(0xa5, 0x96, 0xff)
    } else {
        egui::Color32::from_rgb(0x6a, 0x5c, 0xe8)
    }
}

/// The hover card over a bar: two lines, pinned to the bar rather than to the
/// pointer.
///
/// egui's default tooltip follows the cursor, so on a chart it drifts off the
/// bar it describes and covers its neighbours. This one sits directly above
/// the bar, stays inside the plot, and is only as tall as a date and a figure —
/// the three stacked labels of the old tooltip read as a paragraph.
fn day_callout(
    painter: &egui::Painter,
    plot: egui::Rect,
    centre: f32,
    bar_top: f32,
    day: &crate::db::DayTotal,
    state: &State,
    palette: &ui_kit::tokens::Palette,
) {
    let text = egui::Color32::from(palette.text);
    let muted = egui::Color32::from(palette.text_muted);

    let heading = painter.layout_no_wrap(
        format!("{} · {} sales", long_date(day), day.sales),
        egui::FontId::new(TYPE.small, theme::semibold()),
        muted,
    );
    let amount = painter.layout_no_wrap(
        state.money(day.revenue),
        egui::FontId::new(TYPE.body, theme::numeric()),
        text,
    );
    let heading_size = heading.size();
    let amount_size = amount.size();

    let pad_x = SPACING.sm;
    let pad_y = 7.0;
    let gap = 2.0;
    let width = heading_size.x.max(amount_size.x) + pad_x * 2.0;
    let height = heading_size.y + gap + amount_size.y + pad_y * 2.0;

    // Above the bar when the plot has room, otherwise inside the bar's top; and
    // never past either edge, so the card cannot hang over the axis labels.
    let left = (centre - width * 0.5).clamp(plot.left(), (plot.right() - width).max(plot.left()));
    let above = bar_top - SPACING.sm - height;
    let top = if above >= plot.top() {
        above
    } else {
        (bar_top + SPACING.sm).min(plot.bottom() - height)
    };
    let rect = egui::Rect::from_min_size(egui::pos2(left, top), egui::vec2(width, height));
    let corner = egui::CornerRadius::same(RADII.sm as u8);

    painter.rect_filled(rect, corner, egui::Color32::from(palette.surface));
    painter.rect_stroke(
        rect,
        corner,
        egui::Stroke::new(1.0_f32, egui::Color32::from(palette.border_strong)),
        egui::StrokeKind::Inside,
    );
    painter.galley(
        egui::pos2(rect.left() + pad_x, rect.top() + pad_y),
        heading,
        muted,
    );
    painter.galley(
        egui::pos2(
            rect.left() + pad_x,
            rect.top() + pad_y + heading_size.y + gap,
        ),
        amount,
        text,
    );
}

/// A round axis that fits `peak`: a step of a 1/2/2.5/5 × 10ⁿ size, chosen so
/// about [`AXIS_STEPS`] of them span the data, then rounded up to a whole step
/// above the tallest bar.
///
/// The top of the axis is the next step, not a fixed multiple: a peak of
/// 258,700 gets 300,000 with 100,000 gridlines, so the bars use most of the
/// height instead of floating under an empty band.
fn nice_axis(peak: i64) -> (i64, i64) {
    if peak <= 0 {
        return (1, 1);
    }
    let rough = peak as f64 / AXIS_STEPS as f64;
    let magnitude = 10f64.powf(rough.log10().floor());
    let normal = rough / magnitude;
    let nice = if normal <= 1.0 {
        1.0
    } else if normal <= 2.0 {
        2.0
    } else if normal <= 2.5 {
        2.5
    } else if normal <= 5.0 {
        5.0
    } else {
        10.0
    };
    let step = (nice * magnitude).round().max(1.0) as i64;
    ((peak + step - 1) / step * step, step)
}

/// A gridline value in the till's currency, short enough for a 40 px gutter:
/// `250K`, `1.2M`, `900`.
fn compact_amount(minor: i64, state: &State) -> String {
    let scale = money::Money::new(0, state.currency).scale();
    let value = minor as f64 / 10f64.powi(scale as i32);
    if value >= 1_000_000.0 {
        format!("{:.1}M", value / 1_000_000.0)
    } else if value >= 10_000.0 {
        format!("{:.0}K", value / 1_000.0)
    } else if value >= 1_000.0 {
        format!("{:.1}K", value / 1_000.0)
    } else {
        format!("{value:.0}")
    }
}

/// One date-axis label. A week is read as a rhythm, so its days are named; a
/// month is read as a calendar, so it is numbered. The month itself is said
/// once over the plot, so no tick repeats it.
fn axis_label(day: &crate::db::DayTotal, count: usize) -> String {
    if count <= 8 {
        weekday_name(day.weekday).to_owned()
    } else {
        day_of_month(day).to_owned()
    }
}

/// The span the chart covers: `26 Oct – 1 Nov`, or one month said once.
fn period_label(days: &[crate::db::DayTotal]) -> String {
    match days {
        [] => String::new(),
        [only] => format!("{} {}", day_of_month(only), month_name(only)),
        [newest, .., oldest] => {
            if month_name(newest) == month_name(oldest) {
                format!(
                    "{}–{} {}",
                    day_of_month(oldest),
                    day_of_month(newest),
                    month_name(newest)
                )
            } else {
                format!(
                    "{} {} – {} {}",
                    day_of_month(oldest),
                    month_name(oldest),
                    day_of_month(newest),
                    month_name(newest)
                )
            }
        }
    }
}

/// `Sun 1 Nov` — the way a day is said out loud, for the hover card.
fn long_date(day: &crate::db::DayTotal) -> String {
    format!(
        "{} {} {}",
        weekday_name(day.weekday),
        day_of_month(day),
        month_name(day)
    )
}

/// `dd/mm` without the zero the calendar pads it with: an axis tick says `9`.
fn day_of_month(day: &crate::db::DayTotal) -> &str {
    match day.day.get(..2).map(|day| day.trim_start_matches('0')) {
        Some(day) if !day.is_empty() => day,
        _ => "0",
    }
}

fn month_name(day: &crate::db::DayTotal) -> &'static str {
    let month = day
        .day
        .get(3..5)
        .and_then(|month| month.parse::<usize>().ok())
        .unwrap_or(1);
    MONTHS[month.clamp(1, MONTHS.len()) - 1]
}

fn weekday_name(weekday: u8) -> &'static str {
    WEEKDAYS[(weekday as usize).min(WEEKDAYS.len() - 1)]
}

/// The same day, spelled out, for the sentences under the chart. A tick can say
/// `Fri`; a sentence cannot.
fn weekday_sentence(weekday: u8) -> &'static str {
    const NAMES: [&str; 7] = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    NAMES[(weekday as usize).min(NAMES.len() - 1)]
}

/// The import glyph, resolved by name because the shared vocabulary does not
/// carry a file-transfer icon yet. The fallback costs a different picture, never
/// a missing action.
fn import_icon() -> Icon {
    Icon::named("upload").unwrap_or(Icon::DOCUMENT)
}

fn export_icon() -> Icon {
    Icon::named("download").unwrap_or(Icon::DOCUMENT)
}

// ---------------------------------------------------------------------------
// Products
// ---------------------------------------------------------------------------

struct Products;

impl Module for Products {
    fn sidebar(&mut self, ui: &mut egui::Ui, state: &mut State) {
        categories_sidebar(ui, state);

        ui.add_space(SPACING.lg);
        components::field_label(ui, "Search");
        ui.add_space(SPACING.xs);
        components::search_field(ui, &mut state.search, "Name or barcode");
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, state: &mut State) {
        let can_manage = state.permits(crate::auth::Permission::ChangePrice);
        let mut new = false;
        let mut import = false;
        let mut export_visible = false;
        let mut export_all = false;
        action_bar(ui, "Catalog", |ui| {
            if !can_manage {
                return;
            }
            if actions::primary_compact(ui, "New product").clicked() {
                new = true;
            }
            // Import and both exports are setup actions, not everyday ones.
            // The flyout keeps them one click away without letting them compete
            // with the catalog's primary action for the strip's width.
            actions::toolbar_menu(ui, export_icon(), "Export", |ui| {
                ui.set_min_width(190.0);
                if ui
                    .button("This view")
                    .on_hover_text("Save the products on screen with the columns the import reads")
                    .clicked()
                {
                    export_visible = true;
                    ui.close();
                }
                if ui
                    .button("All products")
                    .on_hover_text("Save every product with the columns the import reads")
                    .clicked()
                {
                    export_all = true;
                    ui.close();
                }
            });
            if components::icon_button(ui, import_icon(), 32.0)
                .on_hover_text(
                    "Import CSV — read name, category, price and barcode. A barcode \
                     that starts with 0 may need a text column in Excel.",
                )
                .clicked()
            {
                import = true;
            }
        });

        if new {
            state.start_product();
        }
        if import {
            state.start_import_picker();
        }
        if export_visible {
            state.start_export(ExportKind::Products);
        }
        if export_all {
            state.start_export(ExportKind::AllProducts);
        }
    }

    fn workspace(&mut self, ui: &mut egui::Ui, state: &mut State) {
        if !state.permits(crate::auth::Permission::ChangePrice) {
            return locked_card(ui, crate::auth::Permission::ChangePrice);
        }

        let needle = state.search.trim().to_lowercase();
        let visible = visible_products(&state.products, state.category.as_deref(), &needle);

        file_feedback(ui, state);

        if state.products.is_empty() {
            components::empty_state_card(
                ui,
                "No products yet",
                "Add the first product. Products become keys on the Sell screen.",
                200.0,
            );
            return;
        }

        if visible.is_empty() {
            components::empty_state_card(
                ui,
                "Nothing matches",
                "Clear the search or pick another category.",
                160.0,
            );
            return;
        }

        let mut action = None;
        // The page scrolls (the shell owns it), so the catalog card is laid out
        // whole: a list inside a list would fight the page for the wheel.
        components::card(ui, |ui| {
            ui.set_min_width(ui.available_width());
            product_header(ui);
            for &product_index in &visible {
                let product = &state.products[product_index];
                let deleting = state.product_delete == Some(product.id);
                if let Some(row_action) = product_row(ui, product, state, deleting) {
                    action = Some(row_action);
                }
            }
        });

        match action {
            Some(ProductAction::Edit(id)) => {
                if let Some(product) = state.products.iter().find(|product| product.id == id) {
                    let product = product.clone();
                    state.edit_product(&product);
                }
            }
            Some(ProductAction::AskDelete(id)) => state.product_delete = Some(id),
            Some(ProductAction::Delete(id)) => state.delete_product(id),
            Some(ProductAction::CancelDelete) => state.product_delete = None,
            None => {}
        }
    }
}

/// Which category both Sell and Products are filtered to.
///
/// One function, so the two sidebars cannot offer different filters over the
/// same grid.
fn categories_sidebar(ui: &mut egui::Ui, state: &mut State) {
    components::section_title(ui, "Categories");
    if regions::nav_item(ui, state.category.is_none(), "All").clicked() {
        state.category = None;
    }

    // Cloned because the click mutates `state.category` while the list is being
    // drawn from `state.categories`; both are fields of the same struct.
    let categories = state.categories.clone();
    for category in &categories {
        let selected = state.category.as_deref() == Some(category.as_str());
        if regions::nav_item(ui, selected, category).clicked() {
            state.category = Some(category.clone());
        }
    }
}

/// Write one of the exports into the folder the picker chose.
///
/// The rows are rebuilt from the screen's current filters at the moment the
/// folder is confirmed. The modal blocks the screen while it is open, so the
/// file still matches what the operator was looking at. The template is the
/// exception: it is not a snapshot of anything, only a blank catalog to fill
/// in.
pub(crate) fn run_export(state: &mut State, kind: ExportKind, dir: &Path) {
    // "Export CSV" is what is on screen; "Export all" is the whole catalog.
    // The names of the files carry the difference, because a folder with both
    // in it must not make the operator open them to find out which one they
    // got.
    match kind {
        ExportKind::Products => {
            let needle = state.search.trim().to_lowercase();
            let visible = visible_products(&state.products, state.category.as_deref(), &needle);
            let rows: Vec<Product> = visible
                .iter()
                .map(|&index| state.products[index].clone())
                .collect();
            export_products(state, &rows, "products", dir);
        }
        ExportKind::AllProducts => {
            let rows = state.products.clone();
            export_products(state, &rows, "products-all", dir);
        }
        ExportKind::Sales => export_sales(state, dir),
        ExportKind::Template => save_template(state, dir),
    }
}

/// The line under a screen that just wrote a file, with its path one press
/// away.
///
/// A path printed in a sentence is hard to retype, and a till's exports often
/// end up in an email to the accountant, so the copy is the point.
fn file_feedback(ui: &mut egui::Ui, state: &State) {
    let Some(notice) = state.file_notice.as_deref() else {
        return;
    };
    ui.horizontal_wrapped(|ui| {
        components::caption(ui, notice);
        if let Some(path) = state.file_path.as_deref()
            && actions::quiet_compact(ui, "Copy path").clicked()
        {
            ui.ctx().copy_text(path.display().to_string());
        }
    });
}

/// Write a product export and say where it went.
///
/// The path is the point: an operator who does not know where the file is has
/// not been helped. A failure lands in the same line a success would have, so a
/// missing export cannot look like a button that did nothing.
fn export_products(state: &mut State, rows: &[Product], name: &str, dir: &Path) {
    let body = crate::export::products_csv(rows, state.currency);
    save_export(state, name, &body, rows.len(), ("product", "products"), dir);
}

/// Write the sales export for the range the screen is showing.
///
/// The file reads the same scope as the list: a cashier who cannot see a
/// colleague's takings on screen cannot export them either.
fn export_sales(state: &mut State, dir: &Path) {
    let scope = state.report_scope();
    let rows = match crate::db::export_sales(&state.store, state.range, scope.user()) {
        Ok(rows) => rows,
        Err(err) => {
            state.file_path = None;
            state.file_notice = Some(format!("Export failed: {err:#}"));
            return;
        }
    };
    let body = crate::export::sales_csv(&rows, state.currency);
    save_export(state, "sales", &body, rows.len(), ("sale", "sales"), dir);
}

/// Write the blank catalog with two example rows, and point the next import at
/// the folder it went into.
///
/// The fixed name is deliberate: saving twice replaces the template instead of
/// filling the folder with copies, which is also why it carries no stamp.
fn save_template(state: &mut State, dir: &Path) {
    let body = crate::export::template_csv(state.currency);
    match crate::export::write(dir, "products-template.csv", &body) {
        Ok(path) => {
            // The next step after saving a template is editing it somewhere
            // else and picking it back up, so start the import where it landed.
            state.import_dir = path.parent().map(Path::to_path_buf);
            state.file_path = Some(path.clone());
            state.file_notice = Some(format!(
                "Saved a template with two example rows to {}",
                path.display()
            ));
        }
        Err(err) => {
            state.file_path = None;
            state.file_notice = Some(format!("Export failed: {err:#}"));
        }
    }
}

/// Name a file after the second it was written and report it in `file_notice`.
///
/// The stamp comes from SQLite, the same clock that dates the sales inside the
/// file, so two exports cannot disagree about when the books were taken.
fn save_export(
    state: &mut State,
    name: &str,
    body: &str,
    count: usize,
    nouns: (&str, &str),
    dir: &Path,
) {
    let stamp = match crate::db::stamp(&state.store) {
        Ok(stamp) => stamp,
        Err(err) => {
            state.file_path = None;
            state.file_notice = Some(format!("Export failed: {err:#}"));
            return;
        }
    };

    let noun = if count == 1 { nouns.0 } else { nouns.1 };
    let file = format!("{name}-{stamp}.csv");
    match crate::export::write(dir, &file, body) {
        Ok(path) => {
            state.file_path = Some(path.clone());
            state.file_notice = Some(format!("Exported {count} {noun} to {}", path.display()));
        }
        Err(err) => {
            state.file_path = None;
            state.file_notice = Some(format!("Export failed: {err:#}"));
        }
    }
}

/// Whether a product passes the category and search filters.
fn matches(product: &Product, category: Option<&str>, needle: &str) -> bool {
    let in_category = category.is_none_or(|selected| product.category == selected);
    let in_search = needle.is_empty()
        || product.name.to_lowercase().contains(needle)
        || product
            .barcode
            .as_deref()
            .is_some_and(|barcode| barcode.to_lowercase().contains(needle));
    in_category && in_search
}

/// Indices of the products the grid shows, in catalog order.
///
/// Its own function because the scan path needs the same answer the grid drew:
/// if the two ever disagreed, a scan could add a product that is not on screen.
fn visible_products(products: &[Product], category: Option<&str>, needle: &str) -> Vec<usize> {
    products
        .iter()
        .enumerate()
        .filter(|(_, product)| matches(product, category, needle))
        .map(|(index, _)| index)
        .collect()
}

/// What a scanned code or a typed name resolved to.
#[derive(Debug, PartialEq, Eq)]
enum Scan {
    One(usize),
    Ambiguous(usize),
    None,
}

/// Resolve a scan (or a typed name) to exactly one product, or say why not.
///
/// The order is the contract:
/// 1. an exact barcode wins outright, **ignoring the category filter** — a scan
///    is an explicit instruction, so a sidebar the cashier forgot about must not
///    turn it into "no product matches";
/// 2. an exact name within what the cashier can see;
/// 3. the only product the current filter shows;
/// 4. nothing, or a count so the message can say how many.
fn scan_target(products: &[Product], needle: &str, visible: &[usize]) -> Scan {
    let needle = needle.trim().to_lowercase();
    if needle.is_empty() {
        return Scan::None;
    }

    // Barcodes are unique when present, so `position` is enough here.
    if let Some(index) = products.iter().position(|product| {
        product
            .barcode
            .as_deref()
            .is_some_and(|barcode| barcode.trim().to_lowercase() == needle)
    }) {
        return Scan::One(index);
    }

    let named: Vec<usize> = visible
        .iter()
        .copied()
        .filter(|&index| products[index].name.trim().to_lowercase() == needle)
        .collect();

    if named.len() > 1 {
        return Scan::Ambiguous(named.len());
    }
    if let Some(&index) = named.first() {
        return Scan::One(index);
    }

    match visible {
        [] => Scan::None,
        [index] => Scan::One(*index),
        _ => Scan::Ambiguous(visible.len()),
    }
}

/// The edit glyph for a catalogue row.
///
/// A pencil is the edit affordance the rest of the desktop uses. It is in the
/// Lucide set but not yet in `ui-kit`'s vocabulary, so it is resolved by name —
/// the same lookup a settings file uses. The fallback means a renamed glyph
/// costs a different icon, never a missing button.
fn edit_icon() -> Icon {
    Icon::named("pencil").unwrap_or(Icon::SETTINGS)
}

enum ProductAction {
    Edit(i64),
    AskDelete(i64),
    Delete(i64),
    CancelDelete,
}

fn product_header(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        components::table_header(ui, "Product");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            components::table_header(ui, "Price");
        });
    });
    components::divider(ui);
}

/// One product: name and category on the left, price and actions on the right.
///
/// The row returns what was clicked instead of mutating the state, because the
/// caller is iterating `state.products` and cannot borrow it mutably yet.
fn product_row(
    ui: &mut egui::Ui,
    product: &Product,
    state: &State,
    deleting: bool,
) -> Option<ProductAction> {
    let mut action = None;

    ui.horizontal(|ui| {
        ui.set_min_height(52.0);

        // Keep the name clear of the price and row actions. A fixed reserve is
        // enough for the money column and two icon buttons, and it lets a long
        // name wrap instead of running underneath them.
        const ACTIONS_WIDTH: f32 = 190.0;
        let text_width = (ui.available_width() - ACTIONS_WIDTH).max(120.0);
        ui.allocate_ui_with_layout(
            egui::vec2(text_width, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.label(components::semibold(&product.name).size(TYPE.body));
                let mut detail = product.category.clone();
                if let Some(barcode) = &product.barcode {
                    detail.push_str(" · ");
                    detail.push_str(barcode);
                }
                components::caption(ui, &detail);
            },
        );

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(components::numeric(state.money(product.price_minor)).size(TYPE.body));

            // Icon-only actions: the row is about the product, and two words of
            // chrome per line crowd the price. The tooltip carries the word.
            if deleting {
                if components::icon_button(ui, Icon::CHECK, 32.0)
                    .on_hover_text("Delete")
                    .clicked()
                {
                    action = Some(ProductAction::Delete(product.id));
                }
                if components::icon_button(ui, Icon::CLOSE, 32.0)
                    .on_hover_text("Cancel")
                    .clicked()
                {
                    action = Some(ProductAction::CancelDelete);
                }
                components::caption(ui, "Remove this product?");
            } else {
                if components::icon_button(ui, Icon::TRASH, 32.0)
                    .on_hover_text("Delete")
                    .clicked()
                {
                    action = Some(ProductAction::AskDelete(product.id));
                }
                if components::icon_button(ui, edit_icon(), 32.0)
                    .on_hover_text("Edit")
                    .clicked()
                {
                    action = Some(ProductAction::Edit(product.id));
                }
            }
        });
    });
    components::divider(ui);

    action
}

/// A dialog frame, with the same breathing room everywhere.
///
/// egui's popup frame is sized for a menu: fine for a list of choices, cramped
/// for a form that is read and typed into. Every modal in the till goes through
/// here, so the product form, the receipt and the payment screen sit the same
/// distance from their edge — and at the dialog radius rather than the menu's.
pub(crate) fn modal(ctx: &egui::Context, id: egui::Id) -> egui::Modal {
    egui::Modal::new(id).frame(
        egui::Frame::popup(&ctx.global_style())
            .inner_margin(egui::Margin::same(SPACING.xl as i8))
            .corner_radius(egui::CornerRadius::same(RADII.lg as u8)),
    )
}

/// The add/edit form, as a modal drawn by the shell.
///
/// A modal rather than a card above the list: the catalog is a list you click
/// through, and a form pinned above it gets pushed off screen at the narrow end
/// of the window — and, worse, is silently replaced when the operator clicks
/// New product or another row's Edit, losing whatever was typed. A modal makes
/// "one product at a time" explicit and Cancel always means the same thing.
///
/// Called from `shell` after the frame, like the currency picker, so the
/// backdrop covers the screen and the list behind it cannot be clicked.
pub fn product_editor_modal(ctx: &egui::Context, state: &mut State) {
    let currency = state.currency;
    // Suggested category names: categories the shop has already created, then
    // the seed list for this till's business type. Case-insensitive
    // de-duplication keeps "Vitamins" and "vitamins" as one shelf.
    let suggestions: Vec<String> = match state.product_editor.as_ref() {
        Some(draft) => {
            let mut names: Vec<String> = crate::db::categories_list(&state.store)
                .iter()
                .map(|category| category.name.clone())
                .collect();
            if let Some(business_type) = &state.settings.business_type {
                for seed in category_seeds::seed_names(business_type) {
                    if !names.iter().any(|name| name.eq_ignore_ascii_case(&seed)) {
                        names.push(seed);
                    }
                }
            }
            // Remove the category already in the field: suggesting it is noise.
            let current = draft.category.trim();
            names.retain(|name| !name.eq_ignore_ascii_case(current));
            names
        }
        None => Vec::new(),
    };
    let Some(draft) = state.product_editor.as_mut() else {
        return;
    };

    let mut save = false;
    let mut cancel = false;

    modal(ctx, egui::Id::new("pos_product_editor")).show(ctx, |ui| {
        ui.set_width(460.0);

        let heading = if draft.id.is_some() {
            "Edit product"
        } else {
            "New product"
        };
        ui.label(components::semibold(heading).size(TYPE.heading));
        components::caption(
            ui,
            "The price is stored exactly as entered. The till rounds cash at payment.",
        );

        ui.add_space(SPACING.md);
        components::field_label(ui, "Name");
        ui.add_space(SPACING.xs);
        components::search_field(ui, &mut draft.name, "Espresso");

        // Price and category share a row because the modal has the width for
        // it; the barcode gets its own, since it is scanned rather than typed.
        ui.add_space(SPACING.md);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.md;

            ui.vertical(|ui| {
                components::field_label(ui, &format!("Price ({})", currency.code()));
                ui.add_space(SPACING.xs);
                text_field(ui, &mut draft.price, "0", 150.0);
            });

            ui.vertical(|ui| {
                components::field_label(ui, "Category");
                ui.add_space(SPACING.xs);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = SPACING.xs;
                    text_field(ui, &mut draft.category, "General", 148.0);
                    // Existing names in a dropdown instead of a wall of chips:
                    // the suggestion list stays one click away without pushing
                    // the barcode field off the modal. The field stays editable,
                    // because a shop must still be able to name a new shelf.
                    egui::ComboBox::from_id_salt("pos_product_category")
                        .width(220.0)
                        .selected_text("Pick…")
                        .show_ui(ui, |ui| {
                            if suggestions.is_empty() {
                                components::caption(ui, "No other categories yet.");
                            }
                            for name in &suggestions {
                                if ui.selectable_label(false, name).clicked() {
                                    draft.category = name.clone();
                                    ui.close();
                                }
                            }
                        });
                });
            });
        });

        ui.add_space(SPACING.md);
        components::field_label(ui, "Barcode (optional)");
        ui.add_space(SPACING.xs);
        text_field(ui, &mut draft.barcode, "Scanned or typed", 350.0);

        if let Some(error) = &draft.error {
            ui.add_space(SPACING.sm);
            ui.label(
                egui::RichText::new(error)
                    .size(TYPE.small)
                    .color(ui.visuals().error_fg_color),
            );
        }

        actions::footer(ui, |ui| {
            if actions::primary(ui, "Save").clicked() {
                save = true;
            }
            if actions::secondary(ui, "Cancel").clicked() {
                cancel = true;
            }
        });
    });

    // Keyboard, because this form is typed into: Enter saves from any field and
    // Escape leaves without saving, which is what the buttons do.
    if ctx.input(|input| input.key_pressed(egui::Key::Enter)) {
        save = true;
    }
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        cancel = true;
    }

    // Applied outside the borrow of `state.product_editor`: saving reads the
    // catalog and writes the store, which needs the whole state.
    if save {
        state.save_product();
    }
    if cancel {
        state.product_editor = None;
    }
}

/// The supplier form, drawn by the shell beside the product form.
///
/// A modal for the same reason the product form is one: the directory is a list
/// you click through, and a form pinned above it would be pushed off screen at
/// the narrow end of the window and silently replaced by the next row's Edit.
///
/// Called from `shell` after the frame, so the backdrop covers the screen and
/// the directory behind it cannot be clicked.
pub fn supplier_editor_modal(ctx: &egui::Context, state: &mut State) {
    let Some(draft) = state.supplier_editor.as_mut() else {
        return;
    };

    let mut save = false;
    let mut cancel = false;

    modal(ctx, egui::Id::new("pos_supplier_editor")).show(ctx, |ui| {
        ui.set_width(460.0);

        let heading = if draft.id.is_some() {
            "Edit supplier"
        } else {
            "New supplier"
        };
        ui.label(components::semibold(heading).size(TYPE.heading));
        components::caption(
            ui,
            "Only the name is required. Fill in the rest as you learn it.",
        );

        ui.add_space(SPACING.md);
        components::field_label(ui, "Name");
        ui.add_space(SPACING.xs);
        components::search_field(ui, &mut draft.name, "Acme Wholesale");

        // Contact and phone share a row because the modal has the width for it;
        // they are the two things a shop knows first about a rep.
        ui.add_space(SPACING.md);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.md;

            ui.vertical(|ui| {
                components::field_label(ui, "Contact (optional)");
                ui.add_space(SPACING.xs);
                text_field(ui, &mut draft.contact_name, "Amina", 210.0);
            });

            ui.vertical(|ui| {
                components::field_label(ui, "Phone (optional)");
                ui.add_space(SPACING.xs);
                text_field(ui, &mut draft.phone, "+256 700 000", 210.0);
            });
        });

        ui.add_space(SPACING.md);
        components::field_label(ui, "Email (optional)");
        ui.add_space(SPACING.xs);
        text_field(ui, &mut draft.email, "orders@example.com", 360.0);

        ui.add_space(SPACING.md);
        components::field_label(ui, "Address (optional)");
        ui.add_space(SPACING.xs);
        text_field(ui, &mut draft.address, "Street, town", 360.0);

        ui.add_space(SPACING.md);
        components::field_label(ui, "Notes (optional)");
        ui.add_space(SPACING.xs);
        text_field(
            ui,
            &mut draft.notes,
            "Delivery days, terms, rep's name",
            360.0,
        );

        if let Some(error) = &draft.error {
            ui.add_space(SPACING.sm);
            ui.label(
                egui::RichText::new(error)
                    .size(TYPE.small)
                    .color(ui.visuals().error_fg_color),
            );
        }

        actions::footer(ui, |ui| {
            if actions::primary(ui, "Save").clicked() {
                save = true;
            }
            if actions::secondary(ui, "Cancel").clicked() {
                cancel = true;
            }
        });
    });

    // Keyboard, because this form is typed into: Enter saves from any field and
    // Escape leaves without saving, which is what the buttons do.
    if ctx.input(|input| input.key_pressed(egui::Key::Enter)) {
        save = true;
    }
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        cancel = true;
    }

    // Applied outside the borrow of `state.supplier_editor`: saving reads the
    // directory and writes the store, which needs the whole state.
    if save {
        state.save_supplier();
    }
    if cancel {
        state.supplier_editor = None;
    }
}

/// The CSV import, drawn with the other foreground layers in `shell.rs`.
///
/// The normal route is the picker in [`crate::picker`]; the path field here is
/// the fallback for one pasted from an email, and the place an owner with no
/// file to start from can save a template. The modal closes on a successful
/// import and stays open on anything the till could not read, with the reason
/// it could not.
pub fn import_modal(ctx: &egui::Context, state: &mut State) {
    let Some(draft) = state.import.as_mut() else {
        return;
    };

    let mut import = false;
    let mut cancel = false;
    let mut template = false;

    modal(ctx, egui::Id::new("pos_import")).show(ctx, |ui| {
        ui.set_width(460.0);

        ui.label(components::semibold("Import products from CSV").size(TYPE.heading));
        components::caption(
            ui,
            "Rows are read as name, category, price and an optional barcode. Headings \
             are honoured when present, so a file from the shop's spreadsheet works as is; \
             prices are read in this till's currency.",
        );
        components::caption(
            ui,
            "Rows that cannot be read are skipped and counted, never half-imported.",
        );

        ui.add_space(SPACING.md);
        components::field_label(ui, "File path");
        ui.add_space(SPACING.xs);
        text_field(ui, &mut draft.path, "/home/shop/products.csv", 350.0);

        if let Some(error) = &draft.error {
            ui.add_space(SPACING.sm);
            ui.label(
                egui::RichText::new(error)
                    .size(TYPE.small)
                    .color(ui.visuals().error_fg_color),
            );
        }

        ui.add_space(SPACING.sm);
        ui.horizontal(|ui| {
            if actions::quiet_compact(ui, "Save a template").clicked() {
                template = true;
            }
            components::caption(
                ui,
                "A CSV with these columns and two example rows to replace.",
            );
        });

        actions::footer(ui, |ui| {
            if actions::primary(ui, "Import").clicked() {
                import = true;
            }
            if actions::secondary(ui, "Cancel").clicked() {
                cancel = true;
            }
        });
    });

    // Saving a template is not an import: close this modal and open the folder
    // chooser instead. Only one modal is drawn per frame, so this one has to
    // come down before the picker goes up.
    if template {
        state.import = None;
        state.start_export(ExportKind::Template);
        return;
    }

    // Keyboard, matching the product form: Enter runs the import and Escape
    // leaves without touching the catalog.
    if ctx.input(|input| input.key_pressed(egui::Key::Enter)) {
        import = true;
    }
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        cancel = true;
    }

    if import {
        state.import_products();
    }
    if cancel {
        state.import = None;
    }
}

/// The sale detail, drawn with the other foreground layers in `shell.rs`.
/// Set the quantity of a cart line by typing a number.
///
/// A small focused modal — just the field and two buttons — so the cashier
/// cannot mis-tap a product tile while the count is half-entered. The editor
/// stays open until the cashier commits or cancels; Escape and Enter do the
/// same thing as the buttons so a keyboard-only workflow needs no mouse.
pub fn cart_quantity_modal(ctx: &egui::Context, state: &mut State) {
    if state.cart_quantity.is_none() {
        return;
    }

    let mut confirm = false;
    let mut cancel = false;

    modal(ctx, egui::Id::new("pos_cart_quantity")).show(ctx, |ui| {
        ui.set_width(280.0);
        ui.label(components::semibold("Set quantity").size(TYPE.heading));
        ui.add_space(SPACING.sm);

        if let Some(editor) = state.cart_quantity.as_mut() {
            components::field_label(ui, "Quantity");
            ui.add_space(SPACING.xs);

            let mut field_text = editor.digits.clone();
            let response = ui.add(
                egui::TextEdit::singleline(&mut field_text)
                    .hint_text("1")
                    .desired_width(ui.available_width()),
            );
            if editor.focus {
                response.request_focus();
                editor.focus = false;
            }
            if field_text != editor.digits {
                // Keep only digits, up to four characters.
                let filtered: String = field_text
                    .chars()
                    .filter(|c| c.is_ascii_digit())
                    .take(4)
                    .collect();
                editor.digits = filtered;
                editor.replace = false;
                editor.error = None;
            }

            if let Some(error) = &editor.error {
                ui.add_space(SPACING.xs);
                ui.label(
                    egui::RichText::new(error)
                        .size(TYPE.small)
                        .color(ui.visuals().error_fg_color),
                );
            }

            // A counter is not a desk: the keypad is the primary way to set a
            // count, and the text field stays for a hardware keyboard. Both
            // feed the same four-digit limit in `CartQuantity`.
            ui.add_space(SPACING.sm);
            egui::Grid::new("pos_cart_quantity_keypad")
                .num_columns(3)
                .spacing([SPACING.sm, SPACING.sm])
                .show(ui, |ui| {
                    for row in [['1', '2', '3'], ['4', '5', '6'], ['7', '8', '9']] {
                        for digit in row {
                            if quantity_key(ui, &digit.to_string()).clicked() {
                                editor.push_digit(digit);
                            }
                        }
                        ui.end_row();
                    }
                    if quantity_key(ui, "Clear").clicked() {
                        editor.clear();
                    }
                    if quantity_key(ui, "0").clicked() {
                        editor.push_digit('0');
                    }
                    if quantity_key(ui, "Del").clicked() {
                        editor.backspace();
                    }
                    ui.end_row();
                });
        }

        ui.add_space(SPACING.sm);
        actions::footer(ui, |ui| {
            if actions::primary(ui, "Set").clicked() {
                confirm = true;
            }
            if actions::secondary(ui, "Cancel").clicked() {
                cancel = true;
            }
        });
    });

    if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
        confirm = true;
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        cancel = true;
    }

    if confirm && let Some(editor) = &state.cart_quantity {
        match editor.value() {
            Some(qty) => state.set_cart_quantity(editor.product_id, qty),
            None => {
                if let Some(editor) = state.cart_quantity.as_mut() {
                    editor.error = Some("Enter a quantity of 1 or more.".to_owned());
                }
            }
        }
    }
    if cancel {
        state.close_cart_quantity();
    }
}

/// One key in the quantity keypad, sized for a finger rather than a word.
fn quantity_key(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let text = egui::RichText::new(label)
        .size(TYPE.button)
        .family(theme::semibold());
    ui.add(
        egui::Button::new(text)
            .min_size(egui::vec2(72.0, 40.0))
            .corner_radius(egui::CornerRadius::same(RADII.sm as u8)),
    )
}

///
/// The receipt behind a list row. Every figure on it is what was recorded at
/// the time — `unit × qty` as charged, the rounding the customer actually
/// paid — because a customer conversation starts from "this is what the till
/// says", not from a re-derivation that can disagree with the ledger.
pub fn sale_detail_modal(ctx: &egui::Context, state: &mut State) {
    let Some(detail) = state.sale_open.as_ref() else {
        return;
    };
    let currency = state.currency;
    let shop = state.settings.shop_name().to_owned();
    let paper = crate::receipt::Paper::from_receipt(&shop, &state.settings.receipt);
    let copy_text = crate::receipt::plain(detail, &paper, currency);

    let mut close = false;
    let mut copied = false;

    modal(ctx, egui::Id::new("pos_sale_detail")).show(ctx, |ui| {
        ui.set_width(460.0);

        // The same block the customer saw at the till, scrolled when the sale
        // is longer than the window: an owner checking a discrepancy is
        // reading paper, not a summary of paper.
        egui::ScrollArea::vertical()
            .max_height(420.0)
            .show(ui, |ui| {
                crate::receipt::body(ui, detail, &paper, currency, true)
            });

        actions::footer(ui, |ui| {
            if actions::secondary(ui, "Close").clicked() {
                close = true;
            }
            // Same slip, as text: this is the screen an owner opens when a
            // customer is asking about a figure, and the answer has to travel.
            if actions::secondary(ui, "Copy receipt")
                .on_hover_text("Copy this slip as text")
                .clicked()
            {
                ui.ctx().copy_text(copy_text.clone());
                copied = true;
            }
        });
        if copied {
            components::caption(ui, "Copied to the clipboard.");
        }
    });

    // Escape, because an owner checking sale after sale keeps one hand on the
    // keyboard and the list one keypress away.
    if close || ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        state.close_sale();
    }
}

/// A single-line field at a fixed width, matching the height of `search_field`.
pub(crate) fn text_field(
    ui: &mut egui::Ui,
    text: &mut String,
    hint: &str,
    width: f32,
) -> egui::Response {
    field(ui, text, hint, width, false)
}

/// The same field, with the characters replaced by dots.
///
/// A PIN typed at a counter has a queue in front of it. The field still reports
/// what it holds to the caller; only the screen forgets.
pub(crate) fn secret_field(
    ui: &mut egui::Ui,
    text: &mut String,
    hint: &str,
    width: f32,
) -> egui::Response {
    field(ui, text, hint, width, true)
}

fn field(
    ui: &mut egui::Ui,
    text: &mut String,
    hint: &str,
    width: f32,
    secret: bool,
) -> egui::Response {
    let palette = theme::current(ui.ctx());
    ui.scope(|ui| {
        ui.visuals_mut().widgets.inactive.bg_stroke =
            egui::Stroke::new(1.0_f32, egui::Color32::from(palette.border_strong));
        let edit = egui::TextEdit::singleline(text)
            .hint_text(hint)
            .margin(egui::Margin::symmetric(12, 12));
        ui.add_sized(
            [width, 44.0],
            if secret { edit.password(true) } else { edit },
        )
    })
    .inner
}

// ---------------------------------------------------------------------------
// Purchases
// ---------------------------------------------------------------------------

/// Restocking trips: what the shop means to buy, and what came back.
///
/// A list is not a stock movement. Nothing on the shelf changes until the trip
/// is received, so a list can be made at home, adjusted at the market, or
/// abandoned without leaving a trace in the ledger. Planning and counting in are
/// the same page: the quantity column is the count sheet.
struct Purchases;

impl Module for Purchases {
    fn sidebar(&mut self, ui: &mut egui::Ui, state: &mut State) {
        components::section_title(ui, "Buying");
        components::field_label(ui, "Search");
        ui.add_space(SPACING.xs);
        components::search_field(ui, &mut state.purchase_search, "Supplier or note");

        ui.add_space(SPACING.md);
        components::field_label(ui, "Show");
        ui.add_space(SPACING.xs);
        for (index, label) in ["All", "To buy", "Received"].into_iter().enumerate() {
            if regions::nav_item(ui, state.purchase_filter == index, label).clicked() {
                state.purchase_filter = index;
            }
        }

        ui.add_space(SPACING.md);
        components::caption(
            ui,
            "Plan a trip before you go, then change the quantities to what came \
             home. Stock changes only when you mark it received.",
        );
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, state: &mut State) {
        let can_manage = state.permits(crate::auth::Permission::ManageInventory);

        // A trip takes the whole tab: the heading names what is open, and the
        // one way back is on the same row, so the page never hides its exits.
        if state.purchase_editor.is_some() || state.purchase_view.is_some() {
            let editing = state.purchase_editor.is_some();
            let title = purchase_title(state);
            let mut back = false;
            let mut save = false;
            let mut receive = false;
            action_bar(ui, &title, |ui| {
                if editing && can_manage {
                    if actions::primary_compact(ui, "Received").clicked() {
                        receive = true;
                    }
                    if actions::secondary_compact(ui, "Save").clicked() {
                        save = true;
                    }
                }
                if actions::quiet_compact(ui, "← Restock").clicked() {
                    back = true;
                }
            });
            if receive {
                state.receive_purchase();
            }
            if save {
                state.save_purchase();
            }
            if back {
                state.purchase_editor = None;
                state.purchase_view = None;
            }
            return;
        }

        let mut new = false;
        let mut low = false;
        let any_low = state.stock.iter().any(|item| item.needs_restock());
        action_bar(ui, "Restock", |ui| {
            if !can_manage {
                return;
            }
            if actions::primary_compact(ui, "New list").clicked() {
                new = true;
            }
            if any_low && actions::secondary_compact(ui, "Add low stock").clicked() {
                low = true;
            }
        });
        if new {
            state.start_purchase();
        }
        if low {
            state.start_purchase();
            state.purchase_add_low_stock();
        }
    }

    fn workspace(&mut self, ui: &mut egui::Ui, state: &mut State) {
        if !state.permits(crate::auth::Permission::ManageInventory) {
            return locked_card(ui, crate::auth::Permission::ManageInventory);
        }

        if state.purchase_editor.is_some() {
            return purchase_page(ui, state);
        }
        if state.purchase_view.is_some() {
            return purchase_received_page(ui, state);
        }

        components::caption(
            ui,
            "A shopping list for the shop. Add what is low, adjust the quantities \
             at the market, and receive what you actually bought.",
        );
        if let Some(notice) = state.notice.as_deref() {
            components::caption(ui, notice);
        }

        if state.purchases.is_empty() {
            components::empty_state_card(
                ui,
                "No lists yet",
                "Start a list before the next trip. Everything that is low can be \
                 added in one click.",
                180.0,
            );
            return;
        }

        let to_buy = state
            .purchases
            .iter()
            .filter(|purchase| purchase.status == PurchaseStatus::Planned)
            .count();
        let received = state.purchases.len() - to_buy;
        ui.add_space(SPACING.xs);
        components::caption(ui, &format!("{to_buy} to buy · {received} received"));

        let needle = state.purchase_search.trim().to_lowercase();
        let visible = visible_purchases(&state.purchases, &needle, state.purchase_filter);
        if visible.is_empty() {
            components::empty_state_card(
                ui,
                "Nothing matches",
                "Clear the search or choose another filter.",
                160.0,
            );
            return;
        }

        let currency = state.currency;
        let mut open = None;
        components::card(ui, |ui| {
            ui.set_min_width(ui.available_width());
            purchase_header(ui);
            for &index in &visible {
                let purchase = &state.purchases[index];
                if let Some(id) = purchase_row(ui, purchase, currency) {
                    open = Some(id);
                }
            }
        });

        if let Some(id) = open {
            state.edit_purchase(id);
        }
    }
}

/// The name of the trip on screen: its supplier, or what kind of trip it is.
fn purchase_title(state: &State) -> String {
    if let Some(draft) = &state.purchase_editor {
        if let Some(supplier) = draft
            .supplier_id
            .as_deref()
            .and_then(|id| state.suppliers.iter().find(|supplier| supplier.id == id))
        {
            return supplier.name.clone();
        }
        return if draft.id.is_none() {
            "New list".to_owned()
        } else {
            "Market trip".to_owned()
        };
    }
    if let Some(detail) = &state.purchase_view {
        return detail
            .purchase
            .supplier
            .clone()
            .unwrap_or_else(|| "Market trip".to_owned());
    }
    "Restock".to_owned()
}

fn purchase_header(ui: &mut egui::Ui) {
    components::table_header(ui, "Purchase");
    components::divider(ui);
}

/// One trip: what it is and where it stands, with the one action at the edge.
fn purchase_row(ui: &mut egui::Ui, purchase: &Purchase, currency: money::Currency) -> Option<i64> {
    let palette = theme::current(ui.ctx());
    let mut open = false;
    let title = purchase
        .supplier
        .clone()
        .unwrap_or_else(|| "Market trip".to_owned());

    ui.horizontal(|ui| {
        ui.set_min_height(52.0);
        ui.vertical(|ui| {
            ui.label(components::semibold(&title).size(TYPE.body));
            let detail = match purchase.status {
                PurchaseStatus::Planned => format!(
                    "To buy · {} · {}",
                    purchase.created_at,
                    purchase.count_label()
                ),
                PurchaseStatus::Received => {
                    let when = purchase.received_at.as_deref().unwrap_or("Received");
                    format!("Received · {when} · {}", purchase.count_label())
                }
            };
            components::caption(ui, &detail);
        });

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let response = match purchase.status {
                PurchaseStatus::Planned => actions::primary_compact(ui, "Open"),
                PurchaseStatus::Received => actions::secondary_compact(ui, "View"),
            };
            if response.clicked() {
                open = true;
            }

            if let Some(cost) = purchase.cost_minor {
                // A list's total is a guess, a received trip's is what was
                // actually paid, so only the guess wears the tilde.
                let money = Money::new(cost, currency).format();
                let shown = match purchase.status {
                    PurchaseStatus::Planned => format!("~{money}"),
                    PurchaseStatus::Received => money,
                };
                components::caption(ui, &shown);
            }
            let color = if purchase.status == PurchaseStatus::Received {
                palette.success
            } else {
                palette.text_muted
            };
            components::badge(ui, purchase.status.label(), color);
        });
    });
    components::divider(ui);

    if open { Some(purchase.id) } else { None }
}

/// Whether a purchase passes the search box. Supplier and note are what a
/// person remembers about a trip; the product list is on the page itself.
fn purchase_matches(purchase: &Purchase, needle: &str) -> bool {
    needle.is_empty()
        || purchase
            .supplier
            .as_deref()
            .is_some_and(|supplier| supplier.to_lowercase().contains(needle))
        || purchase
            .note
            .as_deref()
            .is_some_and(|note| note.to_lowercase().contains(needle))
}

fn visible_purchases(purchases: &[Purchase], needle: &str, filter: usize) -> Vec<usize> {
    purchases
        .iter()
        .enumerate()
        .filter(|(_, purchase)| {
            let passes = match filter {
                1 => purchase.status == PurchaseStatus::Planned,
                2 => purchase.status == PurchaseStatus::Received,
                _ => true,
            };
            passes && purchase_matches(purchase, needle)
        })
        .map(|(index, _)| index)
        .collect()
}

/// Plan a trip: choose a supplier if there is one, list what to buy, and put
/// an expected price beside each line so the total is known before leaving.
///
/// The same page counts the goods in. A quantity is a number of units, and the
/// trip is over when it is marked received, so there is no second screen to
/// learn at the market with one hand full.
fn purchase_page(ui: &mut egui::Ui, state: &mut State) {
    let currency = state.currency;
    let suppliers: Vec<(String, String)> = state
        .suppliers
        .iter()
        .filter(|supplier| !supplier.archived)
        .map(|supplier| (supplier.id.clone(), supplier.name.clone()))
        .collect();
    // An old list can point at a supplier that has since been archived. Keep
    // showing that name rather than silently rewriting the trip to "none".
    let saved_supplier = state
        .purchase_editor
        .as_ref()
        .and_then(|draft| draft.supplier_id.clone())
        .and_then(|id| {
            state
                .suppliers
                .iter()
                .find(|supplier| supplier.id == id)
                .map(|supplier| supplier.name.clone())
        });
    let candidates: Vec<(i64, String, String)> = state
        .stock
        .iter()
        .map(|item| (item.product_id, item.name.clone(), item.category.clone()))
        .collect();

    let Some(draft) = state.purchase_editor.as_mut() else {
        return;
    };
    let mut delete = false;
    let mut add_low = false;
    let mut add_product = None;
    let mut remove_line = None;

    egui::ScrollArea::vertical()
        .id_salt("pos_purchase_page")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            components::caption(
                ui,
                "A list, not a stock change. Change a quantity to what you \
                 actually bought, then mark the trip received.",
            );

            ui.add_space(SPACING.md);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = SPACING.md;
                ui.vertical(|ui| {
                    components::field_label(ui, "Supplier (optional)");
                    ui.add_space(SPACING.xs);
                    let selected = saved_supplier
                        .clone()
                        .unwrap_or_else(|| "No supplier".to_owned());
                    egui::ComboBox::from_id_salt("pos_purchase_supplier")
                        .width(240.0)
                        .selected_text(selected)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(draft.supplier_id.is_none(), "No supplier")
                                .clicked()
                            {
                                draft.supplier_id = None;
                                ui.close();
                            }
                            for (id, name) in &suppliers {
                                let selected = draft.supplier_id.as_deref() == Some(id.as_str());
                                if ui.selectable_label(selected, name).clicked() {
                                    draft.supplier_id = Some(id.clone());
                                    ui.close();
                                }
                            }
                        });
                });
                ui.vertical(|ui| {
                    components::field_label(ui, "Note (optional)");
                    ui.add_space(SPACING.xs);
                    text_field(ui, &mut draft.note, "Market, stall, reference", 300.0);
                });
            });

            ui.add_space(SPACING.md);
            ui.horizontal(|ui| {
                components::field_label(ui, "To buy");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if actions::secondary_compact(ui, "Add low stock").clicked() {
                        add_low = true;
                    }
                });
            });
            ui.add_space(SPACING.xs);

            if draft.lines.is_empty() {
                components::caption(
                    ui,
                    "Nothing on the list yet. Add a product below, or fill it from low stock.",
                );
            } else {
                components::divider(ui);
                for line in &mut draft.lines {
                    ui.horizontal(|ui| {
                        ui.set_min_height(44.0);
                        let name_width = (ui.available_width() - 240.0).max(120.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(name_width, 44.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.label(components::semibold(&line.name).size(TYPE.body));
                            },
                        );
                        text_field(ui, &mut line.qty, "Qty", 70.0);
                        text_field(ui, &mut line.cost, "Cost", 110.0);
                        if components::icon_button(ui, Icon::TRASH, 32.0)
                            .on_hover_text("Remove")
                            .clicked()
                        {
                            remove_line = Some(line.product_id);
                        }
                    });
                    components::divider(ui);
                }
            }

            let (estimated, missing_cost) = purchase_estimate(&draft.lines, currency);
            ui.add_space(SPACING.sm);
            if estimated > 0 {
                components::caption(
                    ui,
                    &if missing_cost {
                        format!(
                            "Estimated total {} · lines without a cost are not counted.",
                            Money::new(estimated, currency).format()
                        )
                    } else {
                        format!(
                            "Estimated total {}.",
                            Money::new(estimated, currency).format()
                        )
                    },
                );
            } else {
                components::caption(ui, "Add a quantity and cost to see the trip total.");
            }

            ui.add_space(SPACING.lg);
            components::field_label(ui, "Add a product");
            ui.add_space(SPACING.xs);
            components::search_field(ui, &mut draft.search, "Name or category");

            let needle = draft.search.trim().to_lowercase();
            let already: Vec<i64> = draft.lines.iter().map(|line| line.product_id).collect();
            let matches: Vec<(i64, String, String)> = candidates
                .iter()
                .filter(|(id, name, category)| {
                    !already.contains(id)
                        && (needle.is_empty()
                            || name.to_lowercase().contains(&needle)
                            || category.to_lowercase().contains(&needle))
                })
                .cloned()
                .collect();

            ui.add_space(SPACING.xs);
            if candidates.is_empty() {
                components::caption(
                    ui,
                    "No tracked products yet. Track stock first, then plan a purchase.",
                );
            } else if matches.is_empty() {
                components::caption(ui, "No other tracked product matches.");
            } else {
                // Eight is enough to pick from without scrolling the page; a
                // longer catalog is reached by typing, which is faster anyway.
                for (id, name, category) in matches.iter().take(8) {
                    let label = if category.is_empty() {
                        name.clone()
                    } else {
                        format!("{name} — {category}")
                    };
                    if ui.selectable_label(false, label).clicked() {
                        add_product = Some(*id);
                    }
                }
                if matches.len() > 8 {
                    components::caption(
                        ui,
                        &format!("{} more — keep typing to narrow it.", matches.len() - 8),
                    );
                }
            }

            if let Some(error) = &draft.error {
                ui.add_space(SPACING.sm);
                ui.label(
                    egui::RichText::new(error)
                        .size(TYPE.small)
                        .color(ui.visuals().error_fg_color),
                );
            }

            if draft.id.is_some() {
                ui.add_space(SPACING.lg);
                components::divider(ui);
                ui.add_space(SPACING.sm);
                let label = if draft.confirm_delete {
                    "Tap again to delete this list"
                } else {
                    "Delete this list"
                };
                if actions::quiet(ui, label).clicked() {
                    delete = true;
                }
            }
        });

    if add_low {
        state.purchase_add_low_stock();
    }
    if let Some(product_id) = add_product {
        state.purchase_add_product(product_id);
    }
    if let Some(product_id) = remove_line {
        state.purchase_remove_line(product_id);
    }
    if delete {
        state.delete_purchase();
    }
    if ui.ctx().input(|input| input.key_pressed(egui::Key::Escape)) {
        state.purchase_editor = None;
    }
}

/// The sum of every line that has both a quantity and a cost, and whether any
/// line is missing a cost. A missing cost is not zero: the total is a lower
/// bound, and the caption says so.
fn purchase_estimate(lines: &[state::PurchaseLineDraft], currency: money::Currency) -> (i64, bool) {
    let mut total = 0_i64;
    let mut missing = false;
    for line in lines {
        let qty = line.qty.trim().parse::<i64>().ok().filter(|qty| *qty > 0);
        let cost = Money::parse(&line.cost, currency)
            .ok()
            .map(|money| money.minor())
            .filter(|cost| *cost >= 0);
        match (qty, cost) {
            (Some(qty), Some(cost)) => total += qty * cost,
            _ => missing = true,
        }
    }
    (total, missing)
}

/// A trip that came home, read as a record.
///
/// There is nothing to change here: the goods are on the shelf and the ledger
/// has moved. This page answers "what did we buy and what did it cost", which
/// is the question a week later.
fn purchase_received_page(ui: &mut egui::Ui, state: &mut State) {
    let Some(detail) = state.purchase_view.as_ref() else {
        return;
    };
    let currency = state.currency;

    egui::ScrollArea::vertical()
        .id_salt("pos_purchase_received")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let status = match detail.purchase.received_at.as_deref() {
                Some(at) => format!("Received {at} · {}", detail.purchase.count_label()),
                None => format!("Received · {}", detail.purchase.count_label()),
            };
            components::caption(ui, &status);
            if let Some(note) = detail.purchase.note.as_deref() {
                components::caption(ui, note);
            }

            ui.add_space(SPACING.md);
            if detail.lines.is_empty() {
                components::caption(ui, "No lines on this trip.");
            } else {
                components::divider(ui);
                for line in &detail.lines {
                    ui.horizontal(|ui| {
                        ui.set_min_height(44.0);
                        ui.vertical(|ui| {
                            ui.label(components::semibold(&line.name).size(TYPE.body));
                            components::caption(ui, &bought_label(line));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if let Some(cost) = line.unit_cost_minor {
                                components::caption(
                                    ui,
                                    &format!("{} each", Money::new(cost, currency).format()),
                                );
                            }
                        });
                    });
                    components::divider(ui);
                }
            }

            ui.add_space(SPACING.sm);
            match detail.purchase.cost_minor {
                Some(cost) => components::caption(
                    ui,
                    &format!("Trip total {}.", Money::new(cost, currency).format()),
                ),
                None => components::caption(ui, "No cost was recorded for this trip."),
            }
        });

    if ui.ctx().input(|input| input.key_pressed(egui::Key::Escape)) {
        state.purchase_view = None;
    }
}

/// What came home on one line, in the fewest words that are still true.
///
/// Buying more than planned is not a shortfall, so only a genuine short count
/// spells out both numbers.
fn bought_label(line: &crate::db::PurchaseLine) -> String {
    if line.qty_received == 0 {
        "not bought".to_owned()
    } else if line.qty_received < line.qty_ordered {
        format!("{} of {} bought", line.qty_received, line.qty_ordered)
    } else {
        format!("{} bought", line.qty_received)
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
        PRODUCTS => Permission::ChangePrice,
        SUPPLIERS => Permission::ManageSuppliers,
        // Seeing stock is part of selling: a cashier answering "do you have
        // any?" must not need a manager. Changing it is gated inside the
        // workspace on `ManageInventory`.
        INVENTORY => Permission::Sell,
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

// ---------------------------------------------------------------------------
// Suppliers
// ---------------------------------------------------------------------------

struct Suppliers;

impl Module for Suppliers {
    fn sidebar(&mut self, ui: &mut egui::Ui, state: &mut State) {
        components::section_title(ui, "Directory");
        components::field_label(ui, "Search");
        ui.add_space(SPACING.xs);
        components::search_field(ui, &mut state.supplier_search, "Name, contact or phone");

        ui.add_space(SPACING.md);
        ui.checkbox(&mut state.show_archived_suppliers, "Show archived");
        components::caption(
            ui,
            "Archived suppliers leave the working list but stay for the record.",
        );
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, state: &mut State) {
        let can_manage = state.permits(crate::auth::Permission::ManageSuppliers);
        let mut new = false;
        action_bar(ui, "Suppliers", |ui| {
            if !can_manage {
                return;
            }
            if actions::primary_compact(ui, "New supplier").clicked() {
                new = true;
            }
        });
        if new {
            state.start_supplier();
        }
    }

    fn workspace(&mut self, ui: &mut egui::Ui, state: &mut State) {
        if !state.permits(crate::auth::Permission::ManageSuppliers) {
            return locked_card(ui, crate::auth::Permission::ManageSuppliers);
        }

        components::caption(
            ui,
            "Who the shop buys from. Only a name is required; add a contact and \
             phone number as you learn them.",
        );
        if let Some(notice) = state.notice.as_deref() {
            components::caption(ui, notice);
        }

        if state.suppliers.is_empty() {
            components::empty_state_card(
                ui,
                "No suppliers yet",
                "Add the first supplier. The directory is separate from the catalog.",
                180.0,
            );
            return;
        }

        let needle = state.supplier_search.trim().to_lowercase();
        let visible = visible_suppliers(&state.suppliers, &needle, state.show_archived_suppliers);
        if visible.is_empty() {
            components::empty_state_card(
                ui,
                "Nothing matches",
                "Clear the search or show archived suppliers.",
                160.0,
            );
            return;
        }

        let mut action = None;
        // The page scrolls (the shell owns it), so the directory card is laid
        // out whole: a list inside a list would fight the page for the wheel.
        components::card(ui, |ui| {
            ui.set_min_width(ui.available_width());
            supplier_header(ui);
            for &index in &visible {
                let supplier = &state.suppliers[index];
                let confirming = state.supplier_archive.as_deref() == Some(supplier.id.as_str());
                if let Some(row_action) = supplier_row(ui, supplier, confirming) {
                    action = Some(row_action);
                }
            }
        });

        match action {
            Some(SupplierAction::Edit(id)) => {
                if let Some(supplier) = state.suppliers.iter().find(|supplier| supplier.id == id) {
                    let supplier = supplier.clone();
                    state.edit_supplier(&supplier);
                }
            }
            Some(SupplierAction::AskArchive(id)) => state.supplier_archive = Some(id),
            Some(SupplierAction::Archive(id)) => state.set_supplier_archived(&id, true),
            Some(SupplierAction::Restore(id)) => state.set_supplier_archived(&id, false),
            Some(SupplierAction::CancelArchive) => state.supplier_archive = None,
            None => {}
        }
    }
}

enum SupplierAction {
    Edit(String),
    AskArchive(String),
    Archive(String),
    Restore(String),
    CancelArchive,
}

fn supplier_header(ui: &mut egui::Ui) {
    components::table_header(ui, "Supplier");
    components::divider(ui);
}

/// One supplier: name and whatever contact details exist, actions on the right.
///
/// The row returns what was clicked instead of mutating the state, because the
/// caller is iterating `state.suppliers` and cannot borrow it mutably yet.
fn supplier_row(
    ui: &mut egui::Ui,
    supplier: &Supplier,
    confirming: bool,
) -> Option<SupplierAction> {
    let mut action = None;

    ui.horizontal(|ui| {
        ui.set_min_height(52.0);
        ui.vertical(|ui| {
            ui.label(components::semibold(&supplier.name).size(TYPE.body));

            let mut detail = supplier_detail(supplier);
            if supplier.archived {
                detail.insert_str(0, "Archived · ");
            }
            components::caption(ui, &detail);
        });

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if confirming {
                if components::icon_button(ui, Icon::CHECK, 32.0)
                    .on_hover_text("Archive")
                    .clicked()
                {
                    action = Some(SupplierAction::Archive(supplier.id.clone()));
                }
                if components::icon_button(ui, Icon::CLOSE, 32.0)
                    .on_hover_text("Cancel")
                    .clicked()
                {
                    action = Some(SupplierAction::CancelArchive);
                }
                components::caption(ui, "Archive this supplier?");
            } else {
                if supplier.archived {
                    if components::icon_button(ui, restore_icon(), 32.0)
                        .on_hover_text("Restore")
                        .clicked()
                    {
                        action = Some(SupplierAction::Restore(supplier.id.clone()));
                    }
                } else if components::icon_button(ui, archive_icon(), 32.0)
                    .on_hover_text("Archive")
                    .clicked()
                {
                    action = Some(SupplierAction::AskArchive(supplier.id.clone()));
                }
                if components::icon_button(ui, edit_icon(), 32.0)
                    .on_hover_text("Edit")
                    .clicked()
                {
                    action = Some(SupplierAction::Edit(supplier.id.clone()));
                }
            }
        });
    });
    components::divider(ui);

    action
}

/// Contact details as one line, or a hint that there are none.
fn supplier_detail(supplier: &Supplier) -> String {
    let parts: Vec<&str> = [
        supplier.contact_name.as_deref(),
        supplier.phone.as_deref(),
        supplier.email.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect();

    if parts.is_empty() {
        "No contact details".to_owned()
    } else {
        parts.join(" · ")
    }
}

/// Whether a supplier passes the search box.
///
/// Searched fields are the ones a person at the counter would try: who they
/// speak to, and the number they call.
fn supplier_matches(supplier: &Supplier, needle: &str) -> bool {
    needle.is_empty()
        || supplier.name.to_lowercase().contains(needle)
        || supplier
            .contact_name
            .as_deref()
            .is_some_and(|contact| contact.to_lowercase().contains(needle))
        || supplier
            .phone
            .as_deref()
            .is_some_and(|phone| phone.to_lowercase().contains(needle))
        || supplier
            .email
            .as_deref()
            .is_some_and(|email| email.to_lowercase().contains(needle))
}

/// Indices of the suppliers the directory shows, in name order.
fn visible_suppliers(suppliers: &[Supplier], needle: &str, show_archived: bool) -> Vec<usize> {
    suppliers
        .iter()
        .enumerate()
        .filter(|(_, supplier)| {
            (show_archived || !supplier.archived) && supplier_matches(supplier, needle)
        })
        .map(|(index, _)| index)
        .collect()
}

/// The archive glyph for a directory row.
///
/// `archive` is in the Lucide set but not in `ui-kit`'s vocabulary, so it is
/// resolved by name. The fallback means a renamed glyph costs a different icon,
/// never a missing button.
fn archive_icon() -> Icon {
    Icon::named("archive").unwrap_or(Icon::BOXES)
}

fn restore_icon() -> Icon {
    Icon::named("archive-restore").unwrap_or(Icon::REFUND)
}

// ---------------------------------------------------------------------------
// Inventory
// ---------------------------------------------------------------------------

/// The stock ledger: what is tracked, what is low, and the movements behind it.
///
/// This is not a product list. The catalog answers "what does the shop sell";
/// this screen answers "how many are on the shelf", which is a different
/// question with a different truth. Products become visible here only when
/// somebody counts them, because an imported catalog has no stock and a screen
/// full of guessed zeros would teach the shop to distrust it.
struct Inventory;

impl Module for Inventory {
    fn sidebar(&mut self, ui: &mut egui::Ui, state: &mut State) {
        components::section_title(ui, "Stock");
        components::field_label(ui, "Search");
        ui.add_space(SPACING.xs);
        components::search_field(ui, &mut state.stock_search, "Name or category");

        ui.add_space(SPACING.md);
        components::field_label(ui, "Show");
        ui.add_space(SPACING.xs);
        for (index, label) in ["All", "Low", "Out"].into_iter().enumerate() {
            if regions::nav_item(ui, state.stock_filter == index, label).clicked() {
                state.stock_filter = index;
            }
        }

        ui.add_space(SPACING.md);
        components::caption(
            ui,
            "Stock is the sum of every movement. A sale subtracts what it sold.",
        );
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, state: &mut State) {
        let can_manage = state.permits(crate::auth::Permission::ManageInventory);
        let mut track = false;
        action_bar(ui, "Stock", |ui| {
            if !can_manage {
                return;
            }
            if actions::primary_compact(ui, "Track stock").clicked() {
                track = true;
            }
        });
        if track {
            state.start_tracking();
        }
    }

    fn workspace(&mut self, ui: &mut egui::Ui, state: &mut State) {
        // Seeing stock is part of selling; changing it is not. A cashier who is
        // asked "do you have any?" must be able to answer, and the screen says
        // who can record a movement instead of hiding the list outright.
        let can_manage = state.permits(crate::auth::Permission::ManageInventory);

        components::caption(
            ui,
            "What the ledger says is on the shelf. Tracked products subtract sales \
             automatically; receiving, returns, waste and adjustments are recorded here.",
        );
        if !can_manage {
            components::caption(
                ui,
                "You can see stock. A manager or the owner records movements.",
            );
        }
        if let Some(notice) = state.notice.as_deref() {
            components::caption(ui, notice);
        }

        if state.stock.is_empty() {
            components::empty_state_card(
                ui,
                "No stock tracked yet",
                "Track a product and count what is on the shelf. Sales of tracked \
                 products will subtract automatically.",
                180.0,
            );
            return;
        }

        let tracked = state.stock.len();
        let low = state.stock.iter().filter(|item| item.is_low()).count();
        let out = state.stock.iter().filter(|item| item.is_out()).count();
        ui.add_space(SPACING.xs);
        components::caption(
            ui,
            &format!("{tracked} tracked · {low} low · {out} out of stock"),
        );

        let needle = state.stock_search.trim().to_lowercase();
        let visible = visible_stock(&state.stock, &needle, state.stock_filter);
        if visible.is_empty() {
            components::empty_state_card(
                ui,
                "Nothing matches",
                "Clear the search or choose another filter.",
                160.0,
            );
            return;
        }

        let mut action = None;
        // The page scrolls (the shell owns it), so the ledger is laid out whole:
        // a list inside a list would fight the page for the wheel.
        components::card(ui, |ui| {
            ui.set_min_width(ui.available_width());
            stock_header(ui);
            for &index in &visible {
                let item = &state.stock[index];
                if let Some(row_action) = stock_row(ui, item, can_manage) {
                    action = Some(row_action);
                }
            }
        });

        match action {
            Some(StockAction::History { id, name }) => state.open_stock_history(id, &name),
            Some(StockAction::Move { id, name, on_hand }) => {
                state.start_stock_move(id, &name, on_hand, StockKind::Receive)
            }
            None => {}
        }
    }
}

enum StockAction {
    Move { id: i64, name: String, on_hand: i64 },
    History { id: i64, name: String },
}

fn stock_header(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        components::table_header(ui, "Product");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            components::table_header(ui, "On hand");
        });
    });
    components::divider(ui);
}

/// One tracked product: name and category on the left, count and actions right.
///
/// The name wraps rather than truncates, and the stock count stays on the
/// right so the column lines up down the list.
///
/// The row returns what was clicked instead of mutating the state, because the
/// caller is iterating `state.stock` and cannot borrow it mutably yet.
fn stock_row(ui: &mut egui::Ui, item: &StockItem, can_manage: bool) -> Option<StockAction> {
    let (_, action) = egui::containers::Sides::new()
        .height(52.0)
        .shrink_left()
        .wrap()
        .show(
            ui,
            |ui| {
                ui.vertical(|ui| stock_identity(ui, item));
            },
            |ui| stock_controls(ui, item, can_manage),
        );
    components::divider(ui);

    action
}

/// The product's identity: name over category, with the low-stock threshold.
fn stock_identity(ui: &mut egui::Ui, item: &StockItem) {
    ui.label(components::semibold(&item.name).size(TYPE.body));
    let mut detail = if item.category.is_empty() {
        "Uncategorised".to_owned()
    } else {
        item.category.clone()
    };
    detail.push_str(&format!(" · warns at {}", item.low_stock));
    components::caption(ui, &detail);
}

/// The row's right-hand cluster, added right-to-left: the count first, then the
/// button, then the history. The button therefore sits immediately before the
/// count, and the name keeps the rest of the line.
///
/// The count is a badge, green while the shelf is healthy and red once it is
/// low or empty, so the exception reads from the colour before the number.
///
/// Every part is a direct child. A nested vertical inside this right-to-left
/// layout claims the whole row and leaves the name no width.
fn stock_controls(ui: &mut egui::Ui, item: &StockItem, can_manage: bool) -> Option<StockAction> {
    let palette = theme::current(ui.ctx());
    let mut action = None;

    let (color, hint) = if item.is_out() {
        (palette.danger, "Out of stock")
    } else if item.is_low() {
        (palette.danger, "Low stock")
    } else {
        (palette.success, "In stock")
    };

    // Pin the scope to one control row: a badge frame in a right-to-left
    // layout otherwise inherits the row's full height.
    let badge = ui
        .scope(|ui| {
            ui.set_height(ui.spacing().interact_size.y);
            components::badge(ui, &item.on_hand.to_string(), color)
        })
        .response;
    badge.on_hover_text(hint);
    if can_manage
        && components::icon_button(ui, move_icon(), 32.0)
            .on_hover_text("Move stock: receive, return, waste or adjust")
            .clicked()
    {
        action = Some(StockAction::Move {
            id: item.product_id,
            name: item.name.clone(),
            on_hand: item.on_hand,
        });
    }
    if components::icon_button(ui, history_icon(), 32.0)
        .on_hover_text("History")
        .clicked()
    {
        action = Some(StockAction::History {
            id: item.product_id,
            name: item.name.clone(),
        });
    }

    action
}

/// The movement glyph. Two arrows say "in or out"; the tooltip names the four
/// reasons a movement can have.
fn move_icon() -> Icon {
    Icon::named("arrow-right-left").unwrap_or(Icon::BOXES)
}

/// The history glyph, resolved by name like the directory's archive icon: the
/// vocabulary can move without taking a button with it.
fn history_icon() -> Icon {
    Icon::named("history").unwrap_or(Icon::RECEIPT)
}

/// Whether a tracked product passes the search box.
fn stock_matches(item: &StockItem, needle: &str) -> bool {
    needle.is_empty()
        || item.name.to_lowercase().contains(needle)
        || item.category.to_lowercase().contains(needle)
}

/// Indices of the products the ledger shows, in catalog name order.
fn visible_stock(stock: &[StockItem], needle: &str, filter: usize) -> Vec<usize> {
    stock
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            let passes = match filter {
                1 => item.is_low(),
                2 => item.is_out(),
                _ => true,
            };
            passes && stock_matches(item, needle)
        })
        .map(|(index, _)| index)
        .collect()
}

/// Start tracking a product: pick one, count the shelf, choose a warning line.
pub fn track_modal(ctx: &egui::Context, state: &mut State) {
    if state.stock_editor.is_none() {
        return;
    }

    // Products with no ledger yet. Built before the draft is borrowed, because
    // the modal needs the catalog and the ledger, and both live on `state`.
    let candidates: Vec<(i64, String, String)> = state
        .products
        .iter()
        .filter(|product| !state.stock.iter().any(|item| item.product_id == product.id))
        .map(|product| (product.id, product.name.clone(), product.category.clone()))
        .collect();

    let Some(draft) = state.stock_editor.as_mut() else {
        return;
    };
    let mut save = false;
    let mut cancel = false;

    modal(ctx, egui::Id::new("pos_track_stock")).show(ctx, |ui| {
        ui.set_width(480.0);
        ui.label(components::semibold("Track stock").size(TYPE.heading));
        components::caption(
            ui,
            "Choose a product and count what is on the shelf. From then on, sales \
             subtract from that count.",
        );

        if candidates.is_empty() {
            ui.add_space(SPACING.md);
            components::empty_state_card(
                ui,
                "Every product is tracked",
                "Add a product in the catalog first, or adjust one already here.",
                130.0,
            );
        } else {
            ui.add_space(SPACING.md);
            components::field_label(ui, "Product");
            ui.add_space(SPACING.xs);
            components::search_field(ui, &mut draft.search, "Name or category");

            let needle = draft.search.trim().to_lowercase();
            let matches: Vec<(i64, String, String)> = candidates
                .iter()
                .filter(|(_, name, category)| {
                    needle.is_empty()
                        || name.to_lowercase().contains(&needle)
                        || category.to_lowercase().contains(&needle)
                })
                .cloned()
                .collect();

            ui.add_space(SPACING.xs);
            egui::ScrollArea::vertical()
                .max_height(180.0)
                .show(ui, |ui| {
                    for (id, name, category) in &matches {
                        let label = if category.is_empty() {
                            name.clone()
                        } else {
                            format!("{name} — {category}")
                        };
                        if ui
                            .selectable_label(draft.product_id == Some(*id), label)
                            .clicked()
                        {
                            draft.product_id = Some(*id);
                        }
                    }
                });
            if matches.is_empty() {
                components::caption(ui, "No untracked product matches.");
            }
        }

        ui.add_space(SPACING.md);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.md;
            ui.vertical(|ui| {
                components::field_label(ui, "Opening count");
                ui.add_space(SPACING.xs);
                text_field(ui, &mut draft.opening, "0", 150.0);
            });
            ui.vertical(|ui| {
                components::field_label(ui, "Warn at or below");
                ui.add_space(SPACING.xs);
                text_field(ui, &mut draft.low_stock, "5", 150.0);
            });
            ui.vertical(|ui| {
                components::field_label(ui, "Keep in stock");
                ui.add_space(SPACING.xs);
                text_field(ui, &mut draft.target_stock, "Auto", 150.0);
            });
        });
        components::caption(
            ui,
            "The opening count is what you counted today, not what the till has sold. \
             Keep in stock is how much you like to have; leave it empty to use twice \
             the warning line.",
        );

        if let Some(error) = &draft.error {
            ui.add_space(SPACING.sm);
            ui.label(
                egui::RichText::new(error)
                    .size(TYPE.small)
                    .color(ui.visuals().error_fg_color),
            );
        }

        actions::footer(ui, |ui| {
            if actions::primary(ui, "Start tracking").clicked() {
                save = true;
            }
            if actions::secondary(ui, "Cancel").clicked() {
                cancel = true;
            }
        });
    });

    // Keyboard, matching the other forms: Enter saves and Escape leaves.
    if ctx.input(|input| input.key_pressed(egui::Key::Enter)) {
        save = true;
    }
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        cancel = true;
    }
    if save {
        state.save_tracking();
    }
    if cancel {
        state.stock_editor = None;
    }
}

/// One manual movement: receive, a customer return, waste, or an adjustment.
pub fn stock_move_modal(ctx: &egui::Context, state: &mut State) {
    let Some(draft) = state.stock_move.as_mut() else {
        return;
    };
    let mut save = false;
    let mut cancel = false;

    modal(ctx, egui::Id::new("pos_stock_move")).show(ctx, |ui| {
        ui.set_width(420.0);
        ui.label(components::semibold("Move stock").size(TYPE.heading));
        components::caption(
            ui,
            &format!("{} · {} on hand", draft.product_name, draft.on_hand),
        );

        ui.add_space(SPACING.md);
        components::field_label(ui, "What happened");
        ui.add_space(SPACING.xs);
        ui.horizontal_wrapped(|ui| {
            for kind in [
                StockKind::Receive,
                StockKind::Return,
                StockKind::Waste,
                StockKind::Adjust,
            ] {
                ui.selectable_value(&mut draft.kind, kind, movement_label(kind));
            }
        });
        components::caption(ui, movement_hint(draft.kind));

        ui.add_space(SPACING.md);
        components::field_label(ui, movement_quantity_label(draft.kind));
        ui.add_space(SPACING.xs);
        text_field(
            ui,
            &mut draft.quantity,
            if draft.kind == StockKind::Adjust {
                "Counted quantity"
            } else {
                "Quantity"
            },
            160.0,
        );

        ui.add_space(SPACING.md);
        components::field_label(ui, "Note (optional)");
        ui.add_space(SPACING.xs);
        text_field(ui, &mut draft.note, "Delivery, reason, reference", 320.0);

        if let Some(error) = &draft.error {
            ui.add_space(SPACING.sm);
            ui.label(
                egui::RichText::new(error)
                    .size(TYPE.small)
                    .color(ui.visuals().error_fg_color),
            );
        }

        actions::footer(ui, |ui| {
            if actions::primary(ui, "Record").clicked() {
                save = true;
            }
            if actions::secondary(ui, "Cancel").clicked() {
                cancel = true;
            }
        });
    });

    if ctx.input(|input| input.key_pressed(egui::Key::Enter)) {
        save = true;
    }
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        cancel = true;
    }
    if save {
        state.save_stock_move();
    }
    if cancel {
        state.stock_move = None;
    }
}

/// A movement kind in the till's words, not the stored ones.
fn movement_label(kind: StockKind) -> &'static str {
    match kind {
        StockKind::Receive => "Received",
        StockKind::Return => "Customer return",
        StockKind::Waste => "Waste",
        StockKind::Adjust => "Adjustment",
        StockKind::Opening => "Opening count",
        StockKind::Sale => "Sale",
    }
}

fn movement_hint(kind: StockKind) -> &'static str {
    match kind {
        StockKind::Receive => "Goods arriving from a supplier add to the shelf.",
        StockKind::Return => "Stock a customer brought back adds to the shelf.",
        StockKind::Waste => "Spoiled, broken or unsellable stock leaves the shelf.",
        StockKind::Adjust => "Enter what you counted. The till records the difference.",
        StockKind::Opening | StockKind::Sale => "",
    }
}

fn movement_quantity_label(kind: StockKind) -> &'static str {
    if kind == StockKind::Adjust {
        "Counted quantity"
    } else {
        "Quantity"
    }
}

/// One product's recent movements, in the order they happened.
pub fn stock_history_modal(ctx: &egui::Context, state: &mut State) {
    let Some(history) = state.stock_history.as_ref() else {
        return;
    };
    let mut close = false;

    modal(ctx, egui::Id::new("pos_stock_history")).show(ctx, |ui| {
        ui.set_width(520.0);
        ui.label(
            components::semibold(format!("History · {}", history.product_name)).size(TYPE.heading),
        );
        components::caption(
            ui,
            "Every change to this count, newest first. A sale appears as a negative movement.",
        );

        ui.add_space(SPACING.md);
        if history.movements.is_empty() {
            components::empty_state_card(
                ui,
                "No movements yet",
                "The count has not changed since tracking started.",
                130.0,
            );
        } else {
            egui::ScrollArea::vertical()
                .max_height(340.0)
                .show(ui, |ui| {
                    for movement in &history.movements {
                        movement_row(ui, movement);
                    }
                });
            if history.movements.len() as i64 >= crate::db::MOVEMENT_HISTORY {
                components::caption(
                    ui,
                    &format!(
                        "Showing the newest {} movements.",
                        crate::db::MOVEMENT_HISTORY
                    ),
                );
            }
        }

        actions::footer(ui, |ui| {
            if actions::secondary(ui, "Close").clicked() {
                close = true;
            }
        });
    });

    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    if close {
        state.stock_history = None;
    }
}

fn movement_row(ui: &mut egui::Ui, movement: &crate::db::StockMovement) {
    let palette = theme::current(ui.ctx());
    let color = if movement.delta < 0 {
        palette.danger
    } else {
        palette.success
    };
    let signed = if movement.delta > 0 {
        format!("+{}", movement.delta)
    } else {
        movement.delta.to_string()
    };

    ui.horizontal(|ui| {
        ui.set_min_height(44.0);
        ui.vertical(|ui| {
            ui.label(components::semibold(movement_label(movement.kind)).size(TYPE.body));
            let mut detail = movement.created_at.clone();
            if let Some(user) = &movement.user {
                detail.push_str(" · ");
                detail.push_str(user);
            }
            if let Some(note) = &movement.note {
                detail.push_str(" · ");
                detail.push_str(note);
            }
            components::caption(ui, &detail);
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                components::numeric(signed)
                    .size(TYPE.body)
                    .color(egui::Color32::from(color)),
            );
        });
    });
    components::divider(ui);
}

// ---------------------------------------------------------------------------
// Sales
// ---------------------------------------------------------------------------

struct Sales;

/// The cash drawer, at the top of Today.
///
/// One card with two states. Open: what the till expects to be in the drawer,
/// and the one number it cannot know — what is actually there. Closed: what was
/// counted and how it compared. Nothing is drawn before the first sale of the
/// day, because there is nothing to count and a card saying so is noise.
///
/// The drawer is the shop's, not the cashier's: the count covers every sale in
/// the shift even when the list below is scoped to one person.
fn drawer_card(ui: &mut egui::Ui, state: &mut State) {
    let Some(drawer) = state.drawer.clone() else {
        return;
    };
    let palette = theme::current(ui.ctx());

    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());

        let opened_by = drawer
            .shift
            .opened_by
            .as_deref()
            .map(|name| format!(" · {name}"))
            .unwrap_or_default();
        let count = drawer.totals.sales_count;
        let sales = match count {
            0 => String::new(),
            1 => "1 sale · ".to_owned(),
            _ => format!("{count} sales · "),
        };
        let title = if drawer.shift.is_open() {
            "Cash drawer"
        } else {
            "Day closed"
        };
        ui.horizontal(|ui| {
            ui.label(components::semibold(title).size(TYPE.body));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                components::caption(
                    ui,
                    &format!("{sales}opened {}{opened_by}", drawer.shift.opened_at),
                );
            });
        });
        ui.add_space(SPACING.md);

        // The figures are peers in one row, not a form: three numbers a shopkeeper
        // reads at a glance, the same three every day.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 40.0;
            drawer_figure(
                ui,
                "Cash taken",
                &state.money(drawer.totals.cash_minor),
                None,
            );
            if drawer.shift.float_minor > 0 {
                drawer_figure(
                    ui,
                    "Change float",
                    &state.money(drawer.shift.float_minor),
                    None,
                );
            }
            drawer_figure(
                ui,
                "Expected in drawer",
                &state.money(drawer.expected_minor()),
                None,
            );

            if drawer.shift.is_open() {
                return;
            }
            if let Some(counted) = drawer.shift.counted_minor {
                drawer_figure(ui, "Counted", &state.money(counted), None);
            }
            if let Some(difference) = drawer.difference_minor() {
                let (text, color) = match difference.cmp(&0) {
                    std::cmp::Ordering::Equal => ("Balanced".to_owned(), palette.success),
                    std::cmp::Ordering::Less => (
                        format!("Short {}", state.money(-difference)),
                        palette.danger,
                    ),
                    std::cmp::Ordering::Greater => {
                        (format!("Over {}", state.money(difference)), palette.warning)
                    }
                };
                drawer_figure(ui, "Difference", &text, Some(egui::Color32::from(color)));
            }
        });

        if !drawer.shift.is_open() {
            return;
        }

        // The list below can be one person's; the drawer never is. Say so where
        // the two disagree, rather than leaving a cashier to wonder whose cash
        // they are counting.
        if !state.report_scope().is_shop() {
            ui.add_space(SPACING.xs);
            components::caption(
                ui,
                "The drawer is the shop's: this count covers every sale since it opened.",
            );
        }

        // A rule, because what follows is the one thing the till is asking
        // rather than telling.
        ui.add_space(SPACING.md);
        let rule = ui.available_rect_before_wrap();
        ui.painter().hline(
            rule.x_range(),
            rule.top(),
            egui::Stroke::new(1.0_f32, palette.border),
        );
        ui.add_space(SPACING.md);

        let mut close = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.sm;
            components::field_label(ui, "Cash counted");
            let response = text_field(ui, &mut state.drawer_count, "0", 140.0);
            // Enter is the keyboard-first path: count, Enter, done. The button is
            // the same act with a finger.
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                close = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if actions::primary(ui, "Close day").clicked() {
                    close = true;
                }
            });
        });
        if close {
            state.close_day();
        }

        if let Some(error) = state.drawer_error.clone() {
            ui.add_space(SPACING.xs);
            ui.label(
                egui::RichText::new(error)
                    .size(TYPE.small)
                    .color(palette.danger),
            );
        }

        ui.add_space(SPACING.xs);
        components::caption(
            ui,
            if drawer.shift.float_minor > 0 {
                "Type what is in the drawer, then close. Take the takings out and leave the float."
            } else {
                "Type what is in the drawer, then close. Take the takings out."
            },
        );
    });
    ui.add_space(SPACING.lg);
}

/// One figure with its caption above it, in a row of peers.
fn drawer_figure(ui: &mut egui::Ui, label: &str, value: &str, color: Option<egui::Color32>) {
    ui.vertical(|ui| {
        components::field_label(ui, label);
        ui.add_space(2.0);
        let text = components::numeric(value).size(TYPE.heading);
        ui.label(match color {
            Some(color) => text.color(color),
            None => text,
        });
    });
}

impl Module for Sales {
    fn sidebar(&mut self, ui: &mut egui::Ui, state: &mut State) {
        components::section_title(ui, "Range");
        for (index, range) in state::RANGES.iter().enumerate() {
            if regions::nav_item(ui, state.range == index, range).clicked() {
                state.range = index;
            }
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, state: &mut State) {
        let title = if state.report_scope().is_shop() {
            "Transactions"
        } else {
            "Your transactions"
        };
        let mut export = false;
        action_bar(ui, title, |ui| {
            if components::icon_button(ui, export_icon(), 32.0)
                .on_hover_text(
                    "Export sales as sale_id, receipt, when, cashier, total, rounding, paid, method",
                )
                .clicked()
            {
                export = true;
            }
        });
        if export {
            state.start_export(ExportKind::Sales);
        }
    }

    fn workspace(&mut self, ui: &mut egui::Ui, state: &mut State) {
        // The drawer belongs to Today. "7 days" is a report, and a report is not
        // something you count.
        if state.range == 0 {
            drawer_card(ui, state);
        }

        let scope = state.report_scope();
        let shop = scope.is_shop();
        let user = scope.user().map(str::to_owned);
        let user = user.as_deref();

        if !shop {
            // The export reads the same scope as the list, so the caption
            // promises the same thing about both.
            components::caption(ui, "Only your sales · the export matches this list");
        }
        file_feedback(ui, state);

        let sales = crate::db::sales(&state.store, state.range, 100, user);
        if sales.is_empty() {
            components::empty_state_card(
                ui,
                "No transactions",
                "Every completed sale in this range is listed here, newest first.",
                240.0,
            );
            return;
        }

        // The page scrolls (the shell owns it), so the table is laid out whole.
        components::card(ui, |ui| {
            ui.set_min_width(ui.available_width());
            table_row(ui, "Sale", "When", "By", "Total", true, false);
            for (index, sale) in sales.iter().enumerate() {
                // The number the customer's slip carries. A sale from before
                // numbers had a device keeps the row id it was printed with.
                let receipt = sale
                    .receipt_no
                    .clone()
                    .unwrap_or_else(|| format!("#{}", sale.id));
                if table_row(
                    ui,
                    &receipt,
                    &sale.created_at,
                    sale.cashier.as_deref().unwrap_or("-"),
                    &state.money(sale.paid_minor),
                    false,
                    index % 2 == 1,
                )
                .clicked()
                {
                    state.open_sale(sale.id);
                }
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

struct Settings;

impl Module for Settings {
    fn sidebar(&mut self, ui: &mut egui::Ui, state: &mut State) {
        components::section_title(ui, "Settings");
        for (index, section) in state::SETTINGS_SECTIONS.iter().enumerate() {
            if regions::nav_item(ui, state.settings_section == index, section).clicked() {
                state.settings_section = index;
            }
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, state: &mut State) {
        // The section name is the page title. It stays even when the section
        // below is locked, so the chrome never depends on a permission.
        let section = state::SETTINGS_SECTIONS
            .get(state.settings_section)
            .copied()
            .unwrap_or("Terminal");
        action_bar(ui, section, |_ui| {});
    }

    fn workspace(&mut self, ui: &mut egui::Ui, state: &mut State) {
        if !state.permits(crate::auth::Permission::EditSettings) {
            return locked_card(ui, crate::auth::Permission::EditSettings);
        }

        let section = state::SETTINGS_SECTIONS
            .get(state.settings_section)
            .copied()
            .unwrap_or("Terminal");

        match section {
            "Appearance" => appearance_card(ui, state),
            "Receipt" => receipt_preview(ui, state),
            "Terminal" => {
                shop_card(ui, state);
                ui.add_space(SPACING.lg);
                terminal_card(ui, state);
                ui.add_space(SPACING.lg);
                business_type_card(ui, state);
                ui.add_space(SPACING.lg);
                backup_card(ui, state);
            }
            "Scanner" => scanner_card(ui, state),
            "Staff" => {
                if state.permits(crate::auth::Permission::ManageUsers) {
                    crate::auth::staff_card(ui, state);
                } else {
                    locked_card(ui, crate::auth::Permission::ManageUsers);
                }
            }
            "Money" => money_card(ui, state),
            "Sync" => sync_card(ui, state),
            _ => {}
        }
    }
}

/// The appearance controls: one card, two rows.
///
/// The toolbar already names the section, so the card starts with the first
/// setting instead of repeating the title. Both choices apply immediately and
/// are written to settings by `State`.
fn appearance_card(ui: &mut egui::Ui, state: &mut State) {
    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());

        setting_row(
            ui,
            "Theme",
            "Light or dark, across the whole till.",
            THEME_CONTROL_WIDTH,
            |ui| {
                if let Some(chosen) = theme_segmented(ui, state.theme) {
                    state.set_theme(chosen);
                }
            },
        );

        components::divider(ui);

        setting_row(
            ui,
            "Accent colour",
            "Tints buttons, links and highlights.",
            ACCENT_CONTROL_WIDTH,
            |ui| {
                if let Some(accent) = accent_picker(ui, state.brand_accent) {
                    state.set_brand_accent(accent);
                }
            },
        );
    });
}

/// The width of one accent circle, and of the quiet "Default" chip beside them.
const SWATCH_SIZE: f32 = 40.0;
const DEFAULT_CHIP_WIDTH: f32 = 78.0;

/// Exact widths let [`setting_row`] reserve the right column before the control
/// is drawn, so the title and hint never reflow when a control changes state.
const THEME_CONTROL_WIDTH: f32 = 208.0;
const ACCENT_CONTROL_WIDTH: f32 =
    BRANDS.len() as f32 * SWATCH_SIZE + BRANDS.len() as f32 * SPACING.sm + DEFAULT_CHIP_WIDTH;

/// How much room a hint needs before the row is worth splitting in two. Below
/// this the title, hint and control stack, because a hint squeezed narrower
/// than this wraps and stops being a hint.
const ROW_TEXT_MIN_WIDTH: f32 = 240.0;

/// One settings row: a title and a short hint on the left, its control on the
/// right.
///
/// The control gets a fixed-width column so the row cannot depend on how wide
/// the widget happens to measure. Below a small window width the pair stacks
/// instead of squeezing the hint into a column too narrow to read.
fn setting_row<R>(
    ui: &mut egui::Ui,
    title: &str,
    hint: &str,
    control_width: f32,
    control: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let height = 46.0;
    if ui.available_width() < control_width + ROW_TEXT_MIN_WIDTH {
        ui.label(components::semibold(title).size(TYPE.body));
        ui.add_space(SPACING.xs);
        components::caption(ui, hint);
        ui.add_space(SPACING.sm);
        return control(ui);
    }

    let full = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(full, height), egui::Sense::hover());
    let text_width = (full - control_width - SPACING.lg).max(0.0);

    let mut label_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(egui::Rect::from_min_size(
                rect.min,
                egui::vec2(text_width, height),
            ))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    label_ui.label(components::semibold(title).size(TYPE.body));
    label_ui.add_space(SPACING.xs);
    components::caption(&mut label_ui, hint);

    let mut control_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(egui::Rect::from_min_size(
                egui::pos2(rect.max.x - control_width, rect.min.y),
                egui::vec2(control_width, height),
            ))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    control(&mut control_ui)
}

/// A two-way switch with an icon and a word per side.
///
/// Selected is drawn the same way a selected navigation row is: a pale accent
/// wash with accent ink. The track is a well, so the control reads as one
/// object rather than two buttons that happen to be adjacent.
fn theme_segmented(ui: &mut egui::Ui, selected: theme::Theme) -> Option<theme::Theme> {
    let palette = theme::current(ui.ctx());
    let options = [
        (theme::Theme::Light, Icon::SUN, theme::Theme::Light.label()),
        (theme::Theme::Dark, Icon::MOON, theme::Theme::Dark.label()),
    ];
    let height = 40.0;
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(THEME_CONTROL_WIDTH, height),
        egui::Sense::hover(),
    );
    let inset = 3.0;
    let gap = 2.0;
    let segment_width = (rect.width() - inset * 2.0 - gap) / 2.0;
    let mut choice = None;

    if ui.is_rect_visible(rect) {
        ui.painter().rect(
            rect,
            egui::CornerRadius::same(RADII.pill as u8),
            palette.surface_alt,
            egui::Stroke::new(1.0_f32, palette.border),
            egui::StrokeKind::Inside,
        );
    }

    for (index, (side, icon, label)) in options.into_iter().enumerate() {
        let segment = egui::Rect::from_min_size(
            egui::pos2(
                rect.min.x + inset + index as f32 * (segment_width + gap),
                rect.min.y + inset,
            ),
            egui::vec2(segment_width, height - inset * 2.0),
        );
        let response = ui.interact(
            segment,
            ui.id().with(("appearance_theme", index)),
            egui::Sense::click(),
        );
        if response.clicked() {
            choice = Some(side);
        }

        if ui.is_rect_visible(segment) {
            let active = side == selected;
            if active || response.hovered() {
                ui.painter().rect(
                    segment,
                    egui::CornerRadius::same(RADII.pill as u8),
                    if active {
                        palette.accent_soft
                    } else {
                        palette.surface
                    },
                    egui::Stroke::new(
                        1.0_f32,
                        if active {
                            palette.accent_ink()
                        } else {
                            palette.border
                        },
                    ),
                    egui::StrokeKind::Inside,
                );
            }

            let color = if active {
                palette.accent_ink()
            } else {
                palette.text
            };
            let galley = ui.painter().layout_no_wrap(
                label.to_owned(),
                egui::FontId::new(TYPE.small, theme::semibold()),
                color.into(),
            );
            let glyph = 16.0;
            let glyph_gap = SPACING.xs + 2.0;
            let content_width = glyph + glyph_gap + galley.size().x;
            let start = segment.center().x - content_width * 0.5;

            icon.image(glyph, color.into()).paint_at(
                ui,
                egui::Rect::from_center_size(
                    egui::pos2(start + glyph * 0.5, segment.center().y),
                    egui::Vec2::splat(glyph),
                ),
            );
            ui.painter().galley(
                egui::pos2(
                    start + glyph + glyph_gap,
                    segment.center().y - galley.size().y * 0.5,
                ),
                galley,
                color.into(),
            );
        }

        let _ = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    }

    choice
}

/// Four brand accents, plus a way back to the theme's own accent.
///
/// `Some(None)` means the default was chosen, `Some(Some(accent))` a brand, and
/// `None` that nothing was clicked. The caller owns the state write so the
/// comparison after the call still sees the old value.
fn accent_picker(ui: &mut egui::Ui, selected: Option<Rgb>) -> Option<Option<Rgb>> {
    let mut choice = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = SPACING.sm;
        for brand in BRANDS {
            let active = selected == Some(brand.accent);
            if accent_swatch(ui, brand.accent, brand.name, active).clicked() {
                choice = Some(Some(brand.accent));
            }
        }
        if default_chip(ui, selected.is_none()).clicked() {
            choice = Some(None);
        }
    });
    choice
}

fn accent_swatch(ui: &mut egui::Ui, accent: Rgb, label: &str, selected: bool) -> egui::Response {
    let palette = theme::current(ui.ctx());
    let (rect, response) =
        ui.allocate_exact_size(egui::Vec2::splat(SWATCH_SIZE), egui::Sense::click());

    if ui.is_rect_visible(rect) {
        let center = rect.center();
        let outer = SWATCH_SIZE * 0.5;
        if selected {
            ui.painter().circle_stroke(
                center,
                outer - 3.0,
                egui::Stroke::new(2.0_f32, palette.accent_ink()),
            );
        } else if response.hovered() {
            ui.painter().circle_stroke(
                center,
                outer - 3.0,
                egui::Stroke::new(1.0_f32, palette.border_strong),
            );
        }
        ui.painter()
            .circle_filled(center, outer - 7.0, egui::Color32::from(accent));

        if selected {
            Icon::CHECK.image(14.0, accent.best_ink().into()).paint_at(
                ui,
                egui::Rect::from_center_size(center, egui::Vec2::splat(14.0)),
            );
        }
    }

    response
        .on_hover_text(label)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The quiet option at the end of the swatch row: drop the brand override.
fn default_chip(ui: &mut egui::Ui, selected: bool) -> egui::Response {
    let palette = theme::current(ui.ctx());
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(DEFAULT_CHIP_WIDTH, 32.0), egui::Sense::click());

    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let fill: egui::Color32 = if selected {
            palette.accent_soft.into()
        } else if hovered {
            palette.surface_alt.into()
        } else {
            egui::Color32::TRANSPARENT
        };
        let stroke = if selected {
            egui::Stroke::new(1.0_f32, palette.accent_ink())
        } else if hovered {
            egui::Stroke::new(1.0_f32, palette.border_strong)
        } else {
            egui::Stroke::NONE
        };
        ui.painter().rect(
            rect,
            egui::CornerRadius::same(RADII.pill as u8),
            fill,
            stroke,
            egui::StrokeKind::Inside,
        );

        let color = if selected {
            palette.accent_ink()
        } else {
            palette.text_muted
        };
        let galley = ui.painter().layout_no_wrap(
            "Default".to_owned(),
            egui::FontId::new(TYPE.small, theme::semibold()),
            color.into(),
        );
        ui.painter().galley(
            egui::pos2(
                rect.center().x - galley.size().x * 0.5,
                rect.center().y - galley.size().y * 0.5,
            ),
            galley,
            color.into(),
        );
    }

    response
        .on_hover_text("Use the theme's own accent")
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The phone scanner: one switch, the address to type, and a QR code that
/// carries all of it.
///
/// The listener is off by default. Turning it on is the whole opt-in: nothing
/// binds a port until this card says so, and turning it off stops the worker in
/// the same frame the shell next syncs.
fn scanner_card(ui: &mut egui::Ui, state: &mut State) {
    let config = state.settings.scanner.clone();
    let status = state.scanner_status.clone();
    let palette = theme::current(ui.ctx());

    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        components::caption(
            ui,
            "Scan barcodes with an Android phone on the same Wi-Fi. Scans land in the \
             scan field exactly as a USB scanner's would.",
        );
        ui.add_space(SPACING.md);

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.sm;
            if config.enabled {
                if actions::secondary_compact(ui, "Turn off").clicked() {
                    state.settings.scanner.enabled = false;
                    state.settings_dirty = true;
                }
            } else if actions::secondary_compact(ui, "Turn on").clicked() {
                state.settings.scanner.enabled = true;
                state.settings_dirty = true;
            }

            let (text, color) = match &status {
                crate::scan::ScanStatus::Off => {
                    ("Off. Nothing is listening.".to_owned(), palette.text_muted)
                }
                crate::scan::ScanStatus::Listening { port } => {
                    (format!("Listening on port {port}."), palette.success)
                }
                crate::scan::ScanStatus::Error(error) => (error.clone(), palette.danger),
            };
            ui.label(egui::RichText::new(text).size(TYPE.small).color(color));
        });

        ui.add_space(SPACING.xs);
        components::caption(
            ui,
            "Only devices with this till's pairing code can send a scan. Keep the till on \
             the shop network; this listener is not for the internet.",
        );
    });

    if !config.enabled {
        return;
    }

    ui.add_space(SPACING.lg);
    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        components::caption(
            ui,
            "Open Stride Scan on the phone and point it at this code. One till can take \
             scans from as many phones as the shop pairs.",
        );
        ui.add_space(SPACING.md);

        let Some(address) = crate::scan::local_ip() else {
            components::empty_state_card(
                ui,
                "No network address",
                "Connect this till to the shop Wi-Fi, then open Settings again.",
                140.0,
            );
            return;
        };
        let host = address.to_string();
        let endpoint = config.endpoint(&host);

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.lg;
            pairing_qr(ui, &config.pair_uri(&host));
            ui.vertical(|ui| {
                info_row(ui, "Address", &endpoint);
                info_row(ui, "Port", &config.port.to_string());
                let shown_token = crate::scan::spaced_token(&config.token);
                info_row(ui, "Code", &shown_token);
                ui.add_space(SPACING.sm);
                if actions::secondary_compact(ui, "Copy code").clicked() {
                    ui.ctx().copy_text(shown_token);
                }
                ui.add_space(SPACING.xs);
                components::caption(
                    ui,
                    "If the camera cannot read the code, type the address, port and this \
                     ten-character code into the app.",
                );
            });
        });

        if config.enabled {
            ui.add_space(SPACING.md);
            #[cfg(target_os = "linux")]
            firewall_note(ui, state, config.port);
        }
    });
}

/// Till-to-till sync: one switch, this till's identity, and the list of peers
/// it exchanges with.
///
/// Every till keeps its own database and sells offline. Pairing two tills is a
/// choice about who reads whose history, not a hierarchy: the only asymmetry is
/// the catalog owner, because two tills editing products is two catalogs, and a
/// shop wants one.
fn sync_card(ui: &mut egui::Ui, state: &mut State) {
    let config = state.settings.sync.clone();
    let status = state.sync_status.clone();
    let palette = theme::current(ui.ctx());

    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        components::caption(
            ui,
            "Sales travel between tills on the shop's Wi-Fi. Each till keeps \
             selling on its own database, and everything it records shows up \
             on the others the next time they talk.",
        );
        ui.add_space(SPACING.md);

        // The status can be a long error. Let it wrap to a second line instead
        // of pushing the card past the page's right edge.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.sm;
            if config.enabled {
                if actions::secondary_compact(ui, "Turn off").clicked() {
                    state.set_sync_enabled(false);
                }
            } else if actions::secondary_compact(ui, "Turn on").clicked() {
                state.set_sync_enabled(true);
            }
            let (text, color) = sync_status_text(&status, &palette);
            ui.label(egui::RichText::new(text).size(TYPE.small).color(color));
        });

        ui.add_space(SPACING.xs);
        components::caption(
            ui,
            "A till that cannot reach the others keeps working. Sales are \
             exchanged later, in any order, without duplicating.",
        );
    });

    if !config.enabled {
        return;
    }

    ui.add_space(SPACING.lg);
    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        components::caption(
            ui,
            "What other tills call this one. The name is only a label; the \
             database keeps its own identity, so renaming it does not lose \
             history or a peer's place in the feed.",
        );
        ui.add_space(SPACING.md);

        let dirty = state.sync_name_draft.trim() != config.device_name.trim();
        components::field_label(ui, "Till name");
        ui.add_space(SPACING.xs);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.sm;
            text_field(ui, &mut state.sync_name_draft, "Till", 320.0);
            if dirty && actions::primary(ui, "Save").clicked() {
                state.save_sync_device_name();
            }
        });
        ui.add_space(SPACING.xs);
        components::caption(ui, if dirty { "Not saved yet." } else { "Saved." });

        ui.add_space(SPACING.md);
        let mut owner = config.catalog_owner;
        if ui
            .checkbox(&mut owner, "This till authors the product catalog")
            .changed()
        {
            state.set_catalog_owner(owner);
        }
        ui.add_space(SPACING.xs);
        components::caption(
            ui,
            "Exactly one till should claim this. Replicas replace their products \
             and categories with the owner's the next time they sync; a till \
             that does not claim it never publishes a catalog.",
        );
    });

    ui.add_space(SPACING.lg);
    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        components::caption(
            ui,
            "Another till pulls this till's sales with the address, port and \
             code below. Give them only to a till you trust: the code also \
             reads every sale kept here.",
        );
        ui.add_space(SPACING.md);

        let Some(address) = crate::scan::local_ip() else {
            components::empty_state_card(
                ui,
                "No network address",
                "Connect this till to the shop Wi-Fi, then open Settings again.",
                140.0,
            );
            return;
        };
        let host = address.to_string();
        let shown_token = crate::scan::spaced_token(&config.sync_token);
        // `info_row` already lays its value against the card's right edge, so
        // stacking the rows keeps the address, port and code inside the page.
        // Three of them side by side made each row claim the whole width and
        // pushed the port out of the viewport.
        info_row(ui, "Address", &host);
        info_row(ui, "Port", &state.settings.scanner.port.to_string());
        info_row(ui, "Code", &shown_token);
        ui.add_space(SPACING.sm);
        if actions::secondary_compact(ui, "Copy code").clicked() {
            ui.ctx().copy_text(shown_token);
        }
    });

    ui.add_space(SPACING.lg);
    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.sm;
            components::section_title(ui, "Other tills");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if actions::secondary_compact(ui, "Sync now").clicked()
                    && !state.sync_peers.is_empty()
                {
                    state.sync_now_requested = true;
                }
            });
        });
        ui.add_space(SPACING.sm);

        if state.sync_peers.is_empty() {
            components::caption(
                ui,
                "No tills paired yet. Enter another till's address and code below.",
            );
        } else {
            // Copy so the row can ask to remove a peer without holding a borrow
            // on the list the removal rewrites.
            let peers = state.sync_peers.clone();
            for peer in &peers {
                let mut remove = false;
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(components::semibold(peer.display_name()).size(TYPE.body));
                        let (detail, color) = match (&peer.last_error, &peer.last_seen_at) {
                            (Some(error), _) => (error.clone(), palette.danger),
                            (None, Some(seen)) => (format!("Last read {seen}"), palette.text_muted),
                            (None, None) => ("Not contacted yet.".to_owned(), palette.text_muted),
                        };
                        components::caption(ui, &peer.address());
                        ui.label(egui::RichText::new(detail).size(TYPE.small).color(color));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if actions::quiet_compact(ui, "Remove").clicked() {
                            remove = true;
                        }
                    });
                });
                if remove {
                    state.remove_sync_peer(&peer.id);
                }
                ui.add_space(SPACING.sm);
            }
        }

        ui.add_space(SPACING.sm);
        components::field_label(ui, "Pair another till");
        ui.add_space(SPACING.xs);
        // The fields used fixed widths. On a 945 px window the two 220 px
        // columns and their gap were wider than the card, so the address and
        // pairing-code fields ran past the page edge. Derive the widths from
        // the card instead; the two rows always add up to its inner width.
        let gap = SPACING.sm;
        let full = ui.available_width();
        let half = ((full - gap) / 2.0).max(0.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            text_field(ui, &mut state.sync_peer_draft.name, "Name (optional)", half);
            text_field(ui, &mut state.sync_peer_draft.host, "Address", half);
        });
        ui.add_space(SPACING.sm);
        let port_width = 120.0_f32.min((full - gap).max(0.0));
        let token_width = (full - port_width - gap).max(0.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            text_field(ui, &mut state.sync_peer_draft.port, "Port", port_width);
            text_field(
                ui,
                &mut state.sync_peer_draft.token,
                "Pairing code",
                token_width,
            );
        });
        ui.add_space(SPACING.sm);
        if actions::primary(ui, "Pair").clicked() {
            state.add_sync_peer();
        }
        ui.add_space(SPACING.xs);
        components::caption(
            ui,
            "Both tills must be on the same network and have sync turned on.",
        );

        if let Some(error) = state.sync_peer_error.clone() {
            ui.add_space(SPACING.xs);
            ui.label(
                egui::RichText::new(error)
                    .size(TYPE.small)
                    .color(palette.danger),
            );
        }
    });
}

/// One line about what sync is doing right now.
fn sync_status_text(
    status: &crate::sync::SyncStatus,
    palette: &ui_kit::tokens::Palette,
) -> (String, egui::Color32) {
    if !status.enabled {
        return (
            "Off. Nothing is being exchanged.".to_owned(),
            palette.text_muted.into(),
        );
    }
    if let Some(error) = &status.error {
        return (error.clone(), palette.danger.into());
    }
    if status.syncing {
        return (
            format!("Talking to {} till(s)…", status.peers),
            palette.text_muted.into(),
        );
    }
    match status.last_synced {
        Some(last) => {
            let seconds = last.elapsed().as_secs();
            let when = if seconds < 60 {
                format!("{seconds} s ago")
            } else if seconds < 3600 {
                format!("{} min ago", seconds / 60)
            } else {
                format!("{} h ago", seconds / 3600)
            };
            (format!("Last synced {when}."), palette.success.into())
        }
        None if status.peers == 0 => (
            "No other tills paired yet.".to_owned(),
            palette.text_muted.into(),
        ),
        None => (
            "Waiting for the next pass.".to_owned(),
            palette.text_muted.into(),
        ),
    }
}

/// Whether the computer's firewall will let a phone reach the listening port.
///
/// This is the one part of pairing the app cannot finish on its own: reading a
/// firewall's rules and changing them both need the administrator password, so
/// the till only says which firewall is running and offers the operating
/// system's own permission dialog through `pkexec` — the same dialog the user
/// would see installing a printer. When that dialog cannot appear, the exact
/// command is printed and can be copied.
#[cfg(target_os = "linux")]
fn firewall_note(ui: &mut egui::Ui, state: &mut State, port: u16) {
    use crate::firewall::Snapshot;

    let palette = theme::current(ui.ctx());

    let (text, color) = match &state.firewall {
        Snapshot::Unprobed => (
            "Checking whether this computer's firewall lets phones in…".to_owned(),
            palette.text_muted,
        ),
        Snapshot::Clear => (
            "No host firewall is blocking phones on this network.".to_owned(),
            palette.text_muted,
        ),
        Snapshot::Allowed(backend) => (
            format!("Port {port} was allowed in {}.", backend.label()),
            palette.success,
        ),
        Snapshot::Unknown(backend) => (
            format!(
                "{} is running, so phones reach the till only after port {port} is \
                 allowed.",
                backend.label()
            ),
            palette.text_muted,
        ),
    };
    ui.label(egui::RichText::new(text).size(TYPE.small).color(color));

    if state.firewall.wants_allow() {
        ui.add_space(SPACING.sm);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.sm;
            ui.add_enabled_ui(!state.firewall_busy, |ui| {
                if actions::secondary_compact(ui, "Allow port").clicked() {
                    state.firewall_allow_requested = true;
                }
            });
            if state.firewall_busy {
                ui.label(
                    egui::RichText::new("Waiting for permission…")
                        .size(TYPE.small)
                        .color(palette.text_muted),
                );
            } else {
                components::caption(ui, "Asks for the administrator password once.");
            }
        });
    }

    let Some(error) = state.firewall_error.clone() else {
        return;
    };

    ui.add_space(SPACING.sm);
    ui.label(
        egui::RichText::new(error)
            .size(TYPE.small)
            .color(palette.danger),
    );

    if let Some(command) = state.firewall.command(port) {
        ui.add_space(SPACING.xs);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.sm;
            components::caption(ui, &format!("Run this instead: {command}"));
            if actions::quiet_compact(ui, "Copy").clicked() {
                ui.ctx().copy_text(command);
            }
        });
    }
}

/// Draw the pairing URI as a QR code.
///
/// Painted by hand instead of behind an image feature: `qrcode`'s default
/// features pull in encoders this app never uses, and a QR code is a grid of
/// squares either way. Two modules of quiet zone so a phone camera has a
/// margin, and fixed black on white so it scans in either theme.
fn pairing_qr(ui: &mut egui::Ui, data: &str) {
    let Ok(code) = qrcode::QrCode::new(data.as_bytes()) else {
        components::caption(
            ui,
            "Could not draw the pairing code. Type the details instead.",
        );
        return;
    };

    const SIDE: f32 = 176.0;
    const QUIET: usize = 2;

    let (rect, _) = ui.allocate_exact_size(egui::vec2(SIDE, SIDE), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        egui::CornerRadius::same(RADII.sm as u8),
        egui::Color32::WHITE,
    );

    let width = code.width();
    let modules = (width + QUIET * 2) as f32;
    let cell = SIDE / modules;
    let colors = code.to_colors();
    for y in 0..width {
        for x in 0..width {
            if colors[y * width + x] != qrcode::Color::Dark {
                continue;
            }
            let min = rect.min + egui::vec2((x + QUIET) as f32 * cell, (y + QUIET) as f32 * cell);
            painter.rect_filled(
                egui::Rect::from_min_size(min, egui::vec2(cell, cell)),
                egui::CornerRadius::ZERO,
                egui::Color32::BLACK,
            );
        }
    }
}

/// What a module shows when the person cannot open it.
///
/// The module stays on the rail so the cashier can see it exists and ask for it,
/// rather than finding out from a colleague that the screen is missing.
fn locked_card(ui: &mut egui::Ui, permission: crate::auth::Permission) {
    components::section_title(ui, "Not allowed");
    components::empty_state_card(
        ui,
        "Ask a manager",
        &format!(
            "Your account cannot {}. A manager or the owner can approve it for one minute.",
            permission.action()
        ),
        220.0,
    );
}

/// The shop's name, as it prints at the top of every receipt.
///
/// The field edits a draft and an explicit Save commits it. Binding it straight
/// to `settings` would rewrite `settings.json` on every keystroke, because the
/// shell persists whenever the flag is set: a crash mid-word would leave half a
/// shop name as the letterhead of every receipt.
fn shop_card(ui: &mut egui::Ui, state: &mut State) {
    let dirty = state.shop_draft.trim() != state.settings.shop_name.trim();

    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        components::caption(
            ui,
            "Prints at the top of every receipt. The sample under Receipt updates as you type.",
        );
        ui.add_space(SPACING.md);

        components::field_label(ui, "Shop name");
        ui.add_space(SPACING.xs);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.sm;
            text_field(ui, &mut state.shop_draft, crate::APP_NAME, 320.0);
            // A Save that is always there invites a press that does nothing.
            // The action appears with the change, and the caption below says
            // whether the name on the receipts is the one in the field.
            if dirty && actions::primary(ui, "Save").clicked() {
                state.save_shop_name();
            }
        });

        ui.add_space(SPACING.xs);
        components::caption(ui, shop_caption(state));
    });
}

/// What the card says under the field.
///
/// Derived from the field rather than remembered in a flag: a "Saved."
/// confirmation held in state can outlive the text it described — the operator
/// types on, and the card keeps congratulating a name that is no longer there.
fn shop_caption(state: &State) -> &'static str {
    let saved = state.settings.shop_name.trim();
    if state.shop_draft.trim() != saved {
        "Not saved yet."
    } else if saved.is_empty() {
        "Leave it blank to print the app name."
    } else {
        "Saved."
    }
}

/// The two numbers that describe money the till handles but did not earn: the
/// tax it collects on the shop's behalf, and the change it opens with.
///
/// `0` tax means no tax — some small stores are not VAT-registered. Prices on
/// the till are tax-inclusive: the shelf price is what the customer pays, and
/// the receipt backs the tax out underneath the total. Changing the rate does
/// not alter past sales; each sale records the tax it collected.
///
/// The float is the cash in the drawer before the first sale. Zero is right for
/// a shop that opens an empty drawer; a shop that starts with change sets it
/// once, so the evening count is compared against takings plus the float instead
/// of reading its own float as a surplus every single day.
fn money_card(ui: &mut egui::Ui, state: &mut State) {
    let tax_dirty = state.tax_draft.trim() != state.settings.tax_rate_percent.to_string();
    let saved_float = state::plain_amount(state.settings.cash_float_minor, state.currency);
    let float_dirty = state.float_draft.trim() != saved_float;
    let palette = theme::current(ui.ctx());

    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        components::caption(
            ui,
            "The tax rate the till charges, as a whole percentage. Zero means no \
             tax. Prices are tax-inclusive: the receipt breaks the tax out \
             underneath the total, and past sales keep the rate they were sold \
             at.",
        );
        ui.add_space(SPACING.md);

        components::field_label(ui, "Tax rate (%)");
        ui.add_space(SPACING.xs);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.sm;
            text_field(ui, &mut state.tax_draft, "0", 120.0);
            if tax_dirty && actions::primary(ui, "Save").clicked() {
                state.save_tax_rate();
            }
        });

        ui.add_space(SPACING.xs);
        components::caption(ui, tax_caption(state));

        ui.add_space(SPACING.md);
        let rule = ui.available_rect_before_wrap();
        ui.painter().hline(
            rule.x_range(),
            rule.top(),
            egui::Stroke::new(1.0_f32, palette.border),
        );
        ui.add_space(SPACING.md);

        components::caption(
            ui,
            "The change in the drawer before the first sale. It is added to the \
             day's takings when the drawer is counted, so the float never reads \
             as a surplus.",
        );
        ui.add_space(SPACING.md);

        components::field_label(ui, "Change float");
        ui.add_space(SPACING.xs);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.sm;
            text_field(ui, &mut state.float_draft, "0", 120.0);
            if float_dirty && actions::primary(ui, "Save").clicked() {
                state.save_cash_float();
            }
        });

        ui.add_space(SPACING.xs);
        let caption = if float_dirty {
            "Not saved yet.".to_owned()
        } else {
            format!(
                "Counted in {} at close.",
                state.money(state.settings.cash_float_minor)
            )
        };
        components::caption(ui, &caption);
    });
}

/// What the card says under the tax field.
fn tax_caption(state: &State) -> &'static str {
    let rate = state.settings.tax_rate_percent;
    if state.tax_draft.trim() != rate.to_string() {
        "Not saved yet."
    } else if rate == 0 {
        "No tax charged."
    } else {
        "Saved."
    }
}

/// The terminal's identity, in the same card language as the rest of the app.
fn terminal_card(ui: &mut egui::Ui, state: &mut State) {
    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        info_row(ui, "Terminal", crate::APP_NAME);
        info_row(
            ui,
            "Database schema",
            &format!("v{}", state.store.user_version().unwrap_or(-1)),
        );
        info_row(
            ui,
            "Currency",
            &format!("{} ({})", state.currency.name(), state.currency.code()),
        );

        ui.add_space(SPACING.sm);
        // Currency is a property of the amounts already stored, so it can only
        // change while there are none. Re-pricing a recorded history is not a
        // setting; it is a correction, and corrections are reversing entries.
        if crate::db::has_sales(&state.store) {
            components::caption(
                ui,
                "Locked: this till has sales. Past totals are never re-priced.",
            );
        } else {
            components::caption(ui, "No sales yet, so the currency can still change.");
            ui.add_space(SPACING.sm);
            // The currency is the owner's: it decides what every stored number
            // means, and a manager borrowing that for a minute is not a thing.
            if state.permits(crate::auth::Permission::ChangeCurrency) {
                if actions::secondary_compact(ui, "Change currency").clicked() {
                    // A failed attempt from last time is not about this one.
                    state.currency_error = None;
                    state.currency_picker = true;
                }
            } else {
                components::caption(ui, "Only the owner can change it.");
            }
        }
    });
}

/// The seeded business types the app has category suggestions for, in the
/// order they appear in the picker. Any value not here is treated as "Other"
/// and yields no seeded categories — the shop uses its own categories instead.
const BUSINESS_TYPES: [&str; 3] = ["Supermarket", "Pharmacy", "Stationery"];

/// Which kind of shop this till is. The choice only affects which category
/// names the product form offers as suggestions: it never inserts categories
/// automatically, and the shop can rename or add to them at any time.
///
/// A dropdown instead of a free-text field, so the operator cannot type a
/// case variant the seed lookup misses (e.g. "Supermarket" vs "supermarket").
/// "Other…" opens a custom text field for types the app does not seed.
fn business_type_card(ui: &mut egui::Ui, state: &mut State) {
    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        components::caption(
            ui,
            "Picks which category names the product form offers as suggestions. \
             It never inserts anything: the shop's own categories are the ones \
             it has actually used, and any name can be added from the form.",
        );
        ui.add_space(SPACING.md);

        components::field_label(ui, "Business type");
        ui.add_space(SPACING.xs);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.sm;
            egui::ComboBox::from_id_salt("pos_business_type")
                .width(220.0)
                .selected_text(if state.business_draft.trim().is_empty() {
                    "Pick…"
                } else {
                    &state.business_draft
                })
                .show_ui(ui, |ui| {
                    ui.set_min_width(200.0);
                    for kind in BUSINESS_TYPES {
                        let active = state.business_draft.trim().eq_ignore_ascii_case(kind);
                        if ui.selectable_label(active, kind).clicked() {
                            state.business_draft = kind.to_ascii_lowercase();
                            state.business_showing_other = false;
                            ui.close();
                        }
                    }
                    if ui
                        .selectable_label(state.business_showing_other, "Other…")
                        .clicked()
                    {
                        state.business_showing_other = true;
                        ui.close();
                    }
                });
        });

        if state.business_showing_other {
            ui.add_space(SPACING.xs);
            ui.horizontal(|ui| {
                ui.label("Custom type:");
                ui.add_space(SPACING.xs);
                text_field(ui, &mut state.business_draft, "e.g. boutique", 220.0);
            });
            components::caption(
                ui,
                "No seeded category suggestions for a custom type; the shop's \
                 own categories are used instead.",
            );
        }

        let chosen = state.business_draft.trim();
        let dirty = !chosen.is_empty() && state.settings.business_type.as_deref() != Some(chosen);
        ui.add_space(SPACING.xs);
        if dirty && actions::primary(ui, "Save").clicked() {
            state.save_business_type();
        }

        ui.add_space(SPACING.xs);
        if dirty {
            components::caption(ui, "Not saved yet.");
        } else if let Some(ref current) = state.settings.business_type {
            components::caption(ui, &format!("Saved: {current}."));
        } else {
            components::caption(ui, "No business type set. Category suggestions are off.");
        }
    });
}

/// The owner's copy of the books.
///
/// A button, not a dialog: a backup is the thing an owner postpones, so the
/// distance from "I should do that" to "done" has to be a press and a line
/// saying where the file went. The one extra press is a confirmation inline in
/// the card — the same two-step the product rows use — because a backup is the
/// one action on this screen that writes a second copy of everything.
/// Failures are shown in the same line, because a backup that quietly did not
/// happen is worse than none.
fn backup_card(ui: &mut egui::Ui, state: &mut State) {
    let folder = state.paths.data.join("backups");
    let palette = theme::current(ui.ctx());

    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        components::caption(
            ui,
            "A copy of the database, written beside it. One is taken every day by itself, and the newest 10 are kept.",
        );
        ui.add_space(SPACING.md);

        if state.backup_confirm {
            components::caption(ui, "Write a copy of the database now?");
            ui.add_space(SPACING.xs);
            actions::end(ui, |ui| {
                if actions::primary(ui, "Back up").clicked() {
                    state.backup_confirm = false;
                    match crate::db::backup(&state.store, &folder) {
                        Ok(path) => {
                            state.file_path = Some(path.clone());
                            state.file_notice = Some(format!("Backed up to {}", path.display()));
                        }
                        Err(err) => {
                            state.file_path = None;
                            state.file_notice = Some(format!("Backup failed: {err:#}"));
                        }
                    }
                }
                if actions::secondary(ui, "Cancel").clicked() {
                    state.backup_confirm = false;
                }
            });
        } else if actions::secondary(ui, "Back up now").clicked() {
            state.backup_confirm = true;
        }

        ui.add_space(SPACING.xs);
        components::caption(ui, &format!("Folder: {}", folder.display()));
        if let Some(error) = state.auto_backup_error.clone() {
            ui.add_space(SPACING.xs);
            ui.label(
                egui::RichText::new(format!("The automatic backup did not happen: {error}"))
                    .size(TYPE.small)
                    .color(palette.danger),
            );
        }
        if state.file_notice.is_some() {
            ui.add_space(SPACING.xs);
            file_feedback(ui, state);
        }
    });
}

fn info_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        components::field_label(ui, label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(components::semibold(value).size(TYPE.body));
        });
    });
    ui.add_space(SPACING.xs);
}

/// The receipt's optional lines, edited as a draft and committed by Save.
///
/// Same rule as the shop name: binding the fields straight to `settings` would
/// rewrite `settings.json` on every keystroke, so a crash mid-word could leave
/// half an address on every receipt.
fn receipt_details_card(ui: &mut egui::Ui, state: &mut State) {
    let dirty = state.receipt_draft.normalised() != state.settings.receipt;

    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        components::caption(
            ui,
            "Optional lines printed around the sale. Blank fields are left off the paper; \
             an empty closing line still prints \"Thank you.\"",
        );
        ui.add_space(SPACING.md);

        egui::Grid::new("receipt_details_fields")
            .num_columns(2)
            .spacing([SPACING.md, SPACING.sm])
            .show(ui, |ui| {
                receipt_field(
                    ui,
                    "Address",
                    &mut state.receipt_draft.address,
                    "Street, town",
                    360.0,
                );
                receipt_field(
                    ui,
                    "Phone",
                    &mut state.receipt_draft.phone,
                    "+256 700 000",
                    220.0,
                );
                receipt_field(
                    ui,
                    "Tax / VAT",
                    &mut state.receipt_draft.tax_number,
                    "TIN 1000123456",
                    220.0,
                );
                receipt_field(
                    ui,
                    "Return policy",
                    &mut state.receipt_draft.return_policy,
                    "Exchanges within 7 days with this receipt.",
                    420.0,
                );
                receipt_field(
                    ui,
                    "Closing line",
                    &mut state.receipt_draft.footer,
                    "Thank you for shopping with us.",
                    320.0,
                );
            });

        ui.add_space(SPACING.sm);
        ui.checkbox(
            &mut state.receipt_draft.show_cashier,
            "Print the cashier's name on the receipt",
        );

        ui.add_space(SPACING.sm);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.sm;
            if dirty && actions::primary(ui, "Save").clicked() {
                state.save_receipt_details();
            }
            components::caption(ui, receipt_caption(state));
        });
    });
}

fn receipt_field(ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str, width: f32) {
    components::field_label(ui, label);
    text_field(ui, value, hint, width);
    ui.end_row();
}

/// What the card says about the fields, derived from the draft rather than a
/// remembered flag so a "Saved." cannot outlive the text it described.
fn receipt_caption(state: &State) -> &'static str {
    if state.receipt_draft.normalised() != state.settings.receipt {
        "Not saved yet."
    } else {
        "Saved. Changes show in the sample below."
    }
}

/// The receipt section: the fields that shape the paper, then the sample those
/// fields produce.
fn receipt_preview(ui: &mut egui::Ui, state: &mut State) {
    receipt_details_card(ui, state);
    ui.add_space(SPACING.lg);

    components::section_title(ui, "Sample");
    receipt_sample(ui, state);
}

/// A sample receipt: the one place the receipt face and its 13 px column width
/// can be judged before a printer exists.
///
/// Reads the drafts, not the saved values: the point of the preview is to answer
/// "what will this look like?" for the name and lines currently being typed,
/// before Save makes them real.
fn receipt_sample(ui: &mut egui::Ui, state: &State) {
    let typed = state.shop_draft.trim();
    let shop = if typed.is_empty() {
        crate::APP_NAME
    } else {
        typed
    };
    // A preview of a name that has not been saved is still a preview, but it
    // must not pass for the letterhead: the till prints what was saved.
    let unsaved = typed != state.settings.shop_name.trim();
    let paper = crate::receipt::Paper::from_receipt(shop, &state.receipt_draft);

    components::card(ui, |ui| {
        ui.set_width(300.0);
        if unsaved {
            components::caption(ui, "Shop name not saved yet: save it under Terminal.");
            ui.add_space(SPACING.xs);
        }
        crate::receipt::body(ui, &crate::receipt::sample(), &paper, state.currency, true);
        ui.add_space(SPACING.sm);
        components::caption(ui, "Printer and paper width arrive with the receipt flow.");
    });
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
    fn the_rail_is_three_views_with_the_keys_in_order() {
        let manager = views(crate::auth::Role::Manager);
        assert_eq!(manager, &[View::Sell, View::Day, View::Shop][..]);

        // Three keys, in rail order, with no gaps: F1, F2, F3.
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
            !cashier.contains(&View::Shop),
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
            &[PRODUCTS, INVENTORY, PURCHASES, SUPPLIERS, SETTINGS][..]
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
        assert_eq!(scan_target(&products, "6001234", &visible), Scan::One(1));
    }

    #[test]
    fn the_only_product_in_view_is_the_one_a_scan_takes() {
        let products = [product(1, "Cola", None), product(2, "Fanta", None)];
        let visible = visible_products(&products, None, "fan");

        assert_eq!(scan_target(&products, "fan", &visible), Scan::One(1));
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
        assert_eq!(scan_target(&products, "6001234", &visible), Scan::One(0));
    }

    /// An exact name is the cashier confirming which of the matches they meant.
    #[test]
    fn an_exact_name_beats_a_looser_match() {
        let products = [product(1, "Cola 500", None), product(2, "Cola", None)];
        let visible = visible_products(&products, None, "cola");

        assert_eq!(scan_target(&products, "Cola", &visible), Scan::One(1));
    }

    #[test]
    fn an_unknown_code_takes_nothing() {
        let products = [product(1, "Cola", Some("6001234"))];
        let visible = visible_products(&products, None, "999");

        assert_eq!(scan_target(&products, "999", &visible), Scan::None);
        assert_eq!(scan_target(&products, "   ", &visible), Scan::None);
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
}
