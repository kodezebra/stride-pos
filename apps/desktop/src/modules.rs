//! The POS modules: what each one puts in the sidebar and in the workspace.
//!
//! A module is a *pair of closures with a name*, not a page with its own state.
//! The frame (rail, sidebar, status bar) lives in `shell.rs`; the data lives in
//! [`crate::state::State`]. That split is what lets the palette, the hotkeys and
//! the rail all agree on what "module 2" means without a registry.
//!
//! Adding a module: add a struct here, add it to [`all`] in the same position as
//! its index constant, pick a key in the F1–F8 band, and add its index to
//! [`rail_order`] for the roles that should see it. Everything else — rail
//! entry, tooltip, palette entry, persistence index — follows from those edits.
//!
//! F8 is reserved for Purchase orders. It is documented here and nowhere else
//! so the reservation cannot drift.

use std::path::Path;

use ui_kit::components;
use ui_kit::icons::Icon;
use ui_kit::theme;
use ui_kit::tokens::{RADII, SPACING, TYPE};
use ui_shell::egui;
use ui_shell::regions;

use crate::actions;
use crate::category_seeds;
use crate::db::{Product, StockItem, StockKind, Supplier};
use crate::picker::ExportKind;
use crate::state::{self, SellView, State};

// Indices into `all()`. Order is the activity-bar order and the persisted value,
// so these numbers are as permanent as the module list.
pub const SELL: usize = 0;
pub const DASHBOARD: usize = 1;
pub const PRODUCTS: usize = 2;
pub const SALES: usize = 3;
pub const SETTINGS: usize = 4;
pub const SUPPLIERS: usize = 5;
pub const INVENTORY: usize = 6;

/// Number of modules. Used to clamp a persisted index from an older or newer
/// build before it is used as an array index.
pub const COUNT: usize = 7;

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
    /// Activity-bar tooltip, palette label, and sidebar heading.
    fn title(&self) -> &'static str;

    fn icon(&self) -> Icon;

    /// The key that jumps here. F-keys are global and cheap: no modifier, no
    /// chord, and a cashier can hit them without looking.
    fn hotkey(&self) -> egui::Key;

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

    fn workspace(&mut self, ui: &mut egui::Ui, state: &mut State);
}

/// Every module, in stable index order. Must stay in sync with the index
/// constants above.
///
/// This is the persisted order, not the activity bar's order: see
/// [`rail_order`] for what each role sees, top to bottom.
pub fn all() -> Vec<Box<dyn Module>> {
    vec![
        Box::new(Sell),
        Box::new(Dashboard),
        Box::new(Products),
        Box::new(Sales),
        Box::new(Settings),
        Box::new(Suppliers),
        Box::new(Inventory),
    ]
}

/// The activity bar for a role, top to bottom.
///
/// Indices stay stable across releases; only this presentation order changes.
/// Keep the order aligned with the F-keys: the first item is F1, the next F2,
/// and so on. Settings is last because it is the back office, not a stop on the
/// selling loop.
///
/// A cashier sees only the screens their work needs. Hiding Products,
/// Suppliers and Settings is not the permission boundary — [`open`] is — but a
/// navigation bar that offers doors the person cannot walk through is a dead
/// end, and a dead end teaches a cashier to stop using the bar.
pub fn rail_order(role: crate::auth::Role) -> &'static [usize] {
    const CASHIER: [usize; 4] = [SELL, SALES, INVENTORY, DASHBOARD];
    const MANAGER: [usize; 7] = [
        SELL, SALES, INVENTORY, DASHBOARD, PRODUCTS, SUPPLIERS, SETTINGS,
    ];

    if role >= crate::auth::Role::Manager {
        &MANAGER
    } else {
        &CASHIER
    }
}

// ---------------------------------------------------------------------------
// Sell
// ---------------------------------------------------------------------------

/// The till: the catalog on the left, the cart on the right.
struct Sell;

impl Module for Sell {
    fn title(&self) -> &'static str {
        "Sell"
    }

    fn icon(&self) -> Icon {
        Icon::CART
    }

    fn hotkey(&self) -> egui::Key {
        egui::Key::F1
    }

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
        let submitted = (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
            || std::mem::take(&mut state.scan_submit);
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
                            .color(egui::Color32::from(palette.accent))
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
    let rounding_minor = state.cart_rounding_minor();
    let footer = if rounding_minor == 0 {
        196.0
    } else {
        196.0 + TYPE.small * 1.8
    };
    let list_height = (ui.available_height() - footer).max(120.0);

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
                    // The count is a boxed button, not muted text: a till is a
                    // touch screen, so there is no hover to reveal that the
                    // count is editable. Tapping it opens the keypad, which is
                    // how a cashier sells ten of something without tapping `+`
                    // ten times.
                    ui.horizontal(|ui| {
                        let qty = ui
                            .add(
                                egui::Button::new(
                                    components::numeric(format!("Qty {}", line.qty))
                                        .size(TYPE.small),
                                )
                                .min_size(egui::vec2(0.0, 26.0))
                                .corner_radius(egui::CornerRadius::same(RADII.sm as u8)),
                            )
                            .on_hover_text("Set quantity");
                        if qty.clicked() {
                            edit = Some(index);
                        }
                        ui.label(
                            components::numeric(format!("× {}", state.money(line.unit_minor)))
                                .size(TYPE.small)
                                .color(palette.text_muted),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let step = components::stepper(ui, line.qty);
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
    fn title(&self) -> &'static str {
        "Dashboard"
    }

    fn icon(&self) -> Icon {
        Icon::DASHBOARD
    }

    fn hotkey(&self) -> egui::Key {
        egui::Key::F4
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, state: &mut State) {
        components::section_title(ui, "Range");
        for (index, range) in state::RANGES.iter().enumerate() {
            if regions::nav_item(ui, state.range == index, range).clicked() {
                state.range = index;
            }
        }
    }

    fn workspace(&mut self, ui: &mut egui::Ui, state: &mut State) {
        // The scope borrows the session, but the sections below borrow the state
        // mutably to open a sale; take an owned copy of both halves up front.
        let scope = state.report_scope();
        let shop = scope.is_shop();
        let user = scope.user().map(str::to_owned);
        let user = user.as_deref();

        let (count, revenue) = crate::db::totals(&state.store, state.range, user);
        let previous = crate::db::previous_totals(&state.store, state.range, user);
        let units = crate::db::items_sold(&state.store, state.range, user);
        let previous_units = crate::db::previous_items_sold(&state.store, state.range, user);
        let peak = crate::db::peak_hour(&state.store, state.range, user);

        components::section_title(ui, state::range_caption(state.range));
        // The comparison is stated once, under the title: a caption on every
        // card would repeat it four times and squeeze the figures.
        components::caption(ui, comparison_caption(state.range, shop));
        ui.add_space(SPACING.sm);

        let (revenue_sub, revenue_trend) =
            change(revenue, previous.map(|(_, revenue)| revenue), true);
        let (sales_sub, sales_trend) = change(count, previous.map(|(count, _)| count), false);
        let (items_sub, items_trend) = change(units, previous_units, false);
        let (peak_value, peak_sub) = match peak {
            Some(peak) => (
                format!("{:02}:00", peak.hour),
                format!(
                    "{} {}",
                    peak.sales,
                    if peak.sales == 1 { "sale" } else { "sales" }
                ),
            ),
            None => ("—".to_owned(), "no sales".to_owned()),
        };

        kpi_row(
            ui,
            &[
                Kpi {
                    label: "Revenue",
                    value: state.money(revenue),
                    sub: revenue_sub,
                    trend: revenue_trend,
                },
                Kpi {
                    label: "Sales",
                    value: count.to_string(),
                    sub: sales_sub,
                    trend: sales_trend,
                },
                Kpi {
                    label: "Items sold",
                    value: units.to_string(),
                    sub: items_sub,
                    trend: items_trend,
                },
                Kpi {
                    label: "Busiest hour",
                    value: peak_value,
                    sub: peak_sub,
                    trend: Trend::Flat,
                },
            ],
        );

        // What was taken, by method. Deliberately not a cash-up: with no opening
        // float and no counted close, "expected in the drawer" would be a figure
        // the till cannot know. What the payments add up to, it does.
        ui.add_space(SPACING.lg);
        components::section_title(
            ui,
            if shop {
                "Payments taken"
            } else {
                "Your payments"
            },
        );
        let methods = crate::db::by_method(&state.store, state.range, user);
        if methods.is_empty() {
            components::empty_state_card(
                ui,
                "No money taken yet in this range",
                "Each method totals up here as soon as a sale is paid.",
                140.0,
            );
        } else {
            components::card(ui, |ui| {
                ui.set_min_width(ui.available_width());
                components::caption(ui, taken_caption(state.range));

                // One method is one figure, not a table with a single row and a
                // total that repeats it.
                if let [only] = methods.as_slice() {
                    ui.add_space(SPACING.sm);
                    amount_row(
                        ui,
                        &method_label(&only.method, only.sales),
                        &state.money(only.amount_minor),
                        true,
                    );
                } else {
                    for method in &methods {
                        amount_row(
                            ui,
                            &method_label(&method.method, method.sales),
                            &state.money(method.amount_minor),
                            false,
                        );
                    }
                    let taken: i64 = methods.iter().map(|method| method.amount_minor).sum();
                    components::divider(ui);
                    amount_row(ui, "Total", &state.money(taken), true);
                }
            });
        }

        // The shape of the range, as a chart: one bar per day, oldest on the
        // left so the range reads like a calendar, and a round axis above the
        // busiest day. A fixed range draws zero days too; all time draws only
        // the days that sold, and says so.
        ui.add_space(SPACING.lg);
        components::section_title(ui, if shop { "By day" } else { "Your sales by day" });
        if let Some(caption) = chart_caption(state.range) {
            components::caption(ui, caption);
        }
        // Keyed on the sale count, not on the bars: a zero-filled week is still
        // seven bars, and "no sales" is a friendlier answer than seven empty
        // ones.
        if count == 0 {
            components::empty_state_card(
                ui,
                "No sales yet in this range",
                "Each day that takes money grows a bar here.",
                160.0,
            );
        } else {
            let days = crate::db::by_day(&state.store, state.range, user);
            components::card(ui, |ui| {
                ui.set_min_width(ui.available_width());
                day_chart(ui, state, &days);
            });
        }

        ui.add_space(SPACING.lg);
        components::section_title(
            ui,
            if shop {
                "Latest sales"
            } else {
                "Your latest sales"
            },
        );
        let sales = crate::db::sales(&state.store, state.range, 5, user);
        if sales.is_empty() {
            components::empty_state_card(
                ui,
                "No sales recorded",
                "Completed sales appear here, newest first, as soon as the cart is paid.",
                200.0,
            );
            return;
        }

        components::card(ui, |ui| {
            ui.set_min_width(ui.available_width());
            table_row(ui, "Sale", "When", "By", "Total", true, false);
            for (index, sale) in sales.iter().enumerate() {
                if table_row(
                    ui,
                    &format!("#{}", sale.id),
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

        // End-of-shift is not about the shop: it is about who took what. A
        // cashier has no business reading a colleague's takings, and their own
        // row would only repeat the cards above, so the block belongs to the
        // shop view.
        if shop {
            ui.add_space(SPACING.lg);
            components::section_title(ui, "By person");
            let people = crate::db::by_person(&state.store, state.range, user);
            if people.is_empty() {
                components::empty_state_card(
                    ui,
                    "No sales yet in this range",
                    "Whoever takes the money shows up here, with their share.",
                    160.0,
                );
            } else {
                components::card(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    tally_row(ui, "Person", "Sales", "Total", true, false);
                    for (index, person) in people.iter().enumerate() {
                        tally_row(
                            ui,
                            &person.name,
                            &person.sales.to_string(),
                            &state.money(person.revenue),
                            false,
                            index % 2 == 1,
                        );
                    }
                });
            }
        }

        // What actually sells. Ranked by the money each name took, with the
        // units beside it — neither number alone is the story: many cheap
        // units can out-earn a few dear ones, and either way the owner wants
        // both.
        ui.add_space(SPACING.lg);
        components::section_title(ui, if shop { "Top items" } else { "Your top items" });
        let items = crate::db::top_items(&state.store, state.range, 5, user);
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

/// Cards laid out by width arithmetic, not by wrapping.
///
/// `horizontal_wrapped` decides where to break from the *parent's* available
/// width, which on a panel that already overflowed stays wrong for the rest of
/// the frame — the row spills past the window edge and drags every centred
/// widget below it out of place. Columns here are a function of the panel width,
/// so the row always sums to what is on screen.
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
    /// The change against the previous window, or what the busiest hour took.
    sub: String,
    trend: Trend,
}

fn kpi_row(ui: &mut egui::Ui, cards: &[Kpi]) {
    let gap = SPACING.sm;
    let available = ui.available_width();
    let columns = (((available + gap) / (140.0 + gap)).floor() as usize).clamp(1, 4);
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

/// What the cards are being compared with, said once under the range title.
///
/// All time has no previous window, so its line says what the figures are
/// instead of promising a comparison.
fn comparison_caption(range: usize, shop: bool) -> &'static str {
    if shop {
        match range {
            0 => "Compared with yesterday",
            1 => "Compared with the previous 7 days",
            2 => "Compared with the previous 30 days",
            _ => "All recorded sales",
        }
    } else {
        match range {
            0 => "Only your sales · compared with yesterday",
            1 => "Only your sales · compared with the previous 7 days",
            2 => "Only your sales · compared with the previous 30 days",
            _ => "Only your sales · all recorded",
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

/// The range, as the payments card states it.
///
/// Repeated inside the card because the range is chosen in the sidebar: a
/// screenshot of the card has to answer "taken when?" without it.
fn taken_caption(range: usize) -> &'static str {
    match range {
        0 => "Taken today",
        1 => "Taken in the last 7 days",
        2 => "Taken in the last 30 days",
        _ => "Taken over all time",
    }
}

/// `cash` as `Cash · 2 sales`: the column stores what the payment code writes,
/// and the card prints it the way a person would, with the count that makes the
/// figure mean something.
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

// ---------------------------------------------------------------------------
// Products
// ---------------------------------------------------------------------------

struct Products;

impl Module for Products {
    fn title(&self) -> &'static str {
        "Products"
    }

    fn icon(&self) -> Icon {
        Icon::PACKAGE
    }

    fn hotkey(&self) -> egui::Key {
        egui::Key::F5
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, state: &mut State) {
        categories_sidebar(ui, state);

        ui.add_space(SPACING.lg);
        components::field_label(ui, "Search");
        ui.add_space(SPACING.xs);
        components::search_field(ui, &mut state.search, "Name or barcode");
    }

    fn workspace(&mut self, ui: &mut egui::Ui, state: &mut State) {
        if !state.permits(crate::auth::Permission::ChangePrice) {
            return locked_card(ui, crate::auth::Permission::ChangePrice);
        }

        let mut new = false;
        let mut import = false;
        let mut export_visible = false;
        let mut export_all = false;
        ui.horizontal(|ui| {
            components::section_title(ui, "Catalog");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if actions::primary(ui, "New product").clicked() {
                    new = true;
                }
                // Import and the two exports live behind one menu: they are
                // occasional setup actions, and three buttons beside the
                // catalog's only everyday action crowded the header.
                actions::menu_button(ui, "More", |ui| {
                    ui.set_min_width(190.0);
                    if ui
                        .button("Import CSV")
                        .on_hover_text("Read name, category, price and barcode from a CSV file")
                        .clicked()
                    {
                        import = true;
                        ui.close();
                    }
                    if ui
                        .button("Export CSV")
                        .on_hover_text(
                            "Save the products on screen with the columns the import reads",
                        )
                        .clicked()
                    {
                        export_visible = true;
                        ui.close();
                    }
                    if ui
                        .button("Export all")
                        .on_hover_text("Save every product with the columns the import reads")
                        .clicked()
                    {
                        export_all = true;
                        ui.close();
                    }
                });
            });
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

        let needle = state.search.trim().to_lowercase();
        let visible = visible_products(&state.products, state.category.as_deref(), &needle);

        components::caption(
            ui,
            "Exports use the same columns the import reads, so a file that went out \
             can come back in. Barcodes that start with 0 may need a text column in \
             Excel.",
        );
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
/// `settings-2` is in the Lucide set but not yet in `ui-kit`'s vocabulary, so
/// it is resolved by name — the same lookup a settings file uses. The fallback
/// means a renamed glyph costs a different icon, never a missing button.
fn edit_icon() -> Icon {
    Icon::named("settings-2").unwrap_or(Icon::SETTINGS)
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
        ui.vertical(|ui| {
            ui.label(components::semibold(&product.name).size(TYPE.body));
            let mut detail = product.category.clone();
            if let Some(barcode) = &product.barcode {
                detail.push_str(" · ");
                detail.push_str(barcode);
            }
            components::caption(ui, &detail);
        });

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
                                if ui.button(name).clicked() {
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
    fn title(&self) -> &'static str {
        "Suppliers"
    }

    fn icon(&self) -> Icon {
        Icon::TRUCK
    }

    fn hotkey(&self) -> egui::Key {
        egui::Key::F6
    }

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

    fn workspace(&mut self, ui: &mut egui::Ui, state: &mut State) {
        if !state.permits(crate::auth::Permission::ManageSuppliers) {
            return locked_card(ui, crate::auth::Permission::ManageSuppliers);
        }

        let mut new = false;
        ui.horizontal(|ui| {
            components::section_title(ui, "Suppliers");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if actions::primary(ui, "New supplier").clicked() {
                    new = true;
                }
            });
        });
        if new {
            state.start_supplier();
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
    fn title(&self) -> &'static str {
        "Inventory"
    }

    fn icon(&self) -> Icon {
        Icon::BOXES
    }

    fn hotkey(&self) -> egui::Key {
        egui::Key::F3
    }

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

    fn workspace(&mut self, ui: &mut egui::Ui, state: &mut State) {
        // Seeing stock is part of selling; changing it is not. A cashier who is
        // asked "do you have any?" must be able to answer, and the screen says
        // who can record a movement instead of hiding the list outright.
        let can_manage = state.permits(crate::auth::Permission::ManageInventory);

        let mut track = false;
        ui.horizontal(|ui| {
            components::section_title(ui, "Inventory");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if can_manage && actions::primary(ui, "Track stock").clicked() {
                    track = true;
                }
            });
        });
        if track {
            state.start_tracking();
        }

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
        && actions::secondary_compact(ui, "Move stock")
            .on_hover_text("Receive, return, waste or adjust this product")
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
        });
        components::caption(
            ui,
            "The opening count is what you counted today, not what the till has sold.",
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

impl Module for Sales {
    fn title(&self) -> &'static str {
        "Sales"
    }

    fn icon(&self) -> Icon {
        Icon::RECEIPT
    }

    fn hotkey(&self) -> egui::Key {
        egui::Key::F2
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, state: &mut State) {
        components::section_title(ui, "Range");
        for (index, range) in state::RANGES.iter().enumerate() {
            if regions::nav_item(ui, state.range == index, range).clicked() {
                state.range = index;
            }
        }
    }

    fn workspace(&mut self, ui: &mut egui::Ui, state: &mut State) {
        let scope = state.report_scope();
        let shop = scope.is_shop();
        let user = scope.user().map(str::to_owned);
        let user = user.as_deref();

        let mut export = false;
        ui.horizontal(|ui| {
            components::section_title(
                ui,
                if shop {
                    "Transactions"
                } else {
                    "Your transactions"
                },
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if actions::secondary(ui, "Export CSV")
                    .on_hover_text(
                        "Export sales as sale_id, when, cashier, total, rounding, paid, method",
                    )
                    .clicked()
                {
                    export = true;
                }
            });
        });
        if !shop {
            // The export reads the same scope as the list, so the caption
            // promises the same thing about both.
            components::caption(ui, "Only your sales · the export matches this list");
        }
        if export {
            state.start_export(ExportKind::Sales);
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
                if table_row(
                    ui,
                    &format!("#{}", sale.id),
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
    fn title(&self) -> &'static str {
        "Settings"
    }

    fn icon(&self) -> Icon {
        Icon::SETTINGS
    }

    fn hotkey(&self) -> egui::Key {
        egui::Key::F7
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, state: &mut State) {
        components::section_title(ui, "Settings");
        for (index, section) in state::SETTINGS_SECTIONS.iter().enumerate() {
            if regions::nav_item(ui, state.settings_section == index, section).clicked() {
                state.settings_section = index;
            }
        }
    }

    fn workspace(&mut self, ui: &mut egui::Ui, state: &mut State) {
        if !state.permits(crate::auth::Permission::EditSettings) {
            return locked_card(ui, crate::auth::Permission::EditSettings);
        }

        let section = state::SETTINGS_SECTIONS
            .get(state.settings_section)
            .copied()
            .unwrap_or("Terminal");

        // Appearance belongs to the terminal, not to the shop: it is a property
        // of the screen in front of the person. On Staff or Sync it would be an
        // unrelated card at the top of an unrelated page.
        if section == "Terminal" {
            components::section_title(ui, "Appearance");
            components::card(ui, |ui| {
                ui.set_min_width(ui.available_width());
                components::caption(
                    ui,
                    "Light is the default: a till sits under shop lighting and is read from \
                     the customer's side of the counter. Dark suits a dim back office.",
                );
                ui.add_space(SPACING.md);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = SPACING.sm;
                    if components::choice_card(
                        ui,
                        Icon::SUN,
                        theme::Theme::Light.label(),
                        state.theme == theme::Theme::Light,
                        132.0,
                    )
                    .clicked()
                    {
                        state.set_theme(theme::Theme::Light);
                    }
                    if components::choice_card(
                        ui,
                        Icon::MOON,
                        theme::Theme::Dark.label(),
                        state.theme == theme::Theme::Dark,
                        132.0,
                    )
                    .clicked()
                    {
                        state.set_theme(theme::Theme::Dark);
                    }
                });
            });
            ui.add_space(SPACING.lg);
        }

        components::section_title(ui, section);

        match section {
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
            "Tax" => tax_card(ui, state),
            _ => components::empty_state_card(
                ui,
                "Not connected",
                "This till runs alone. A hub or cloud endpoint can be configured once sync exists.",
                160.0,
            ),
        }
    }
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

/// The tax rate the till charges, as a whole percentage.
///
/// `0` means no tax — some small stores are not VAT-registered. Prices on the
/// till are tax-inclusive: the shelf price is what the customer pays, and the
/// receipt backs the tax out underneath the total. Changing the rate does not
/// alter past sales; each sale records the tax it collected.
fn tax_card(ui: &mut egui::Ui, state: &mut State) {
    let dirty = state.tax_draft.trim() != state.settings.tax_rate_percent.to_string();

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
            if dirty && actions::primary(ui, "Save").clicked() {
                state.save_tax_rate();
            }
        });

        ui.add_space(SPACING.xs);
        components::caption(ui, tax_caption(state));
    });
}

/// What the card says under the field.
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
        info_row(ui, "Theme", state.theme.label());
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

/// Which kind of shop this till is. The choice only affects which category
/// names the product form offers as suggestions: it never inserts categories
/// automatically, and the shop can rename or add to them at any time.
fn business_type_card(ui: &mut egui::Ui, state: &mut State) {
    let dirty = state.business_draft.trim()
        != state.settings.business_type.as_deref().unwrap_or("");

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
            text_field(ui, &mut state.business_draft, "supermarket", 220.0);
            components::caption(ui, "e.g. supermarket, pharmacy, stationery");
            if dirty && actions::primary(ui, "Save").clicked() {
                state.save_business_type();
            }
        });

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

    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        components::caption(
            ui,
            "A copy of the database, written beside it. The newest 10 are kept.",
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

    /// The F-band is the till's whole keyboard contract, and `COUNT` is what
    /// keeps a persisted index inside the list. Both are easy to break when a
    /// module is added, and neither breakage is visible until a cashier hits a
    /// stale key — so pin them here.
    #[test]
    fn the_module_table_matches_its_index_and_hotkey_band() {
        let modules = all();

        assert_eq!(
            modules.len(),
            COUNT,
            "COUNT must equal the number of modules or a saved index can point past the end"
        );

        let mut keys: Vec<u8> = modules.iter().map(|module| module.hotkey() as u8).collect();
        let f1 = egui::Key::F1 as u8;
        let f8 = egui::Key::F8 as u8;

        for key in &keys {
            assert!(
                (*key >= f1) && (*key <= f8),
                "hotkeys live in F1-F8; F8 is reserved for purchase orders"
            );
        }
        assert!(
            !keys.contains(&f8),
            "F8 stays free until purchase orders land"
        );

        let before = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(before, keys.len(), "two modules must never share a hotkey");
    }

    #[test]
    fn the_rail_puts_settings_last_and_keeps_hotkeys_in_order() {
        let modules = all();
        let manager = rail_order(crate::auth::Role::Manager);

        assert_eq!(
            manager,
            &[
                SELL, SALES, INVENTORY, DASHBOARD, PRODUCTS, SUPPLIERS, SETTINGS
            ][..]
        );
        assert_eq!(manager.last().copied(), Some(SETTINGS));

        // The rail's order and the F-keys are one contract: the first row is
        // F1, and a manager can walk the whole bar without looking for a gap.
        for (offset, &index) in manager.iter().enumerate() {
            assert_eq!(
                modules[index].hotkey() as u8,
                egui::Key::F1 as u8 + offset as u8,
                "{} should be F{}",
                modules[index].title(),
                offset + 1
            );
        }

        let cashier = rail_order(crate::auth::Role::Cashier);
        assert_eq!(cashier, &[SELL, SALES, INVENTORY, DASHBOARD][..]);
        for admin_only in [PRODUCTS, SUPPLIERS, SETTINGS] {
            assert!(
                !cashier.contains(&admin_only),
                "a cashier rail must not offer an admin-only module"
            );
        }
    }

    #[test]
    fn every_module_has_a_title() {
        // The rail and the palette both read `title`; an empty one is a blank
        // button that is impossible to diagnose from a screenshot.
        for module in all() {
            assert!(!module.title().is_empty());
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
        assert_eq!(comparison_caption(0, true), "Compared with yesterday");
        assert_eq!(
            comparison_caption(1, true),
            "Compared with the previous 7 days"
        );
        assert_eq!(comparison_caption(3, true), "All recorded sales");
        assert_eq!(
            comparison_caption(1, false),
            "Only your sales · compared with the previous 7 days"
        );
        assert_eq!(
            comparison_caption(3, false),
            "Only your sales · all recorded"
        );
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
