//! The POS modules: what each one puts in the sidebar and in the workspace.
//!
//! A module is a *pair of closures with a name*, not a page with its own state.
//! The frame (rail, sidebar, status bar) lives in `shell.rs`; the data lives in
//! [`crate::state::State`]. That split is what lets the palette, the hotkeys and
//! the rail all agree on what "module 2" means without a registry.
//!
//! Adding a module: add a struct here, add it to [`all`] in the same position as
//! its index constant, and pick a key in the F1–F8 band. Everything else — rail
//! entry, tooltip, palette entry, persistence index — follows from those two
//! edits.
//!
//! F6–F8 are reserved for Inventory, Suppliers and Purchase orders. They are
//! documented here and nowhere else so the reservation cannot drift.

use ui_kit::components;
use ui_kit::icons::Icon;
use ui_kit::theme;
use ui_kit::tokens::{SPACING, TYPE};
use ui_shell::egui;
use ui_shell::regions;

use crate::db::Product;
use crate::state::{self, State};

// Indices into `all()`. Order is the activity-bar order and the persisted value,
// so these numbers are as permanent as the module list.
pub const SELL: usize = 0;
pub const DASHBOARD: usize = 1;
pub const PRODUCTS: usize = 2;
pub const SALES: usize = 3;
pub const SETTINGS: usize = 4;

/// Number of modules. Used to clamp a persisted index from an older or newer
/// build before it is used as an array index.
pub const COUNT: usize = 5;

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

    fn sidebar(&mut self, ui: &mut egui::Ui, state: &mut State);

    fn workspace(&mut self, ui: &mut egui::Ui, state: &mut State);
}

/// Every module, in rail order. Must stay in sync with the index constants above.
pub fn all() -> Vec<Box<dyn Module>> {
    vec![
        Box::new(Sell),
        Box::new(Dashboard),
        Box::new(Products),
        Box::new(Sales),
        Box::new(Settings),
    ]
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
        self.grid(&mut grid_ui, state);

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
    fn grid(&mut self, ui: &mut egui::Ui, state: &mut State) {
        components::field_label(ui, "Scan or search");
        ui.add_space(SPACING.xs);
        components::search_field(ui, &mut state.search, "Name or barcode");
        ui.add_space(SPACING.md);

        // A till with an empty catalog is not a bug in the grid: it is a shop
        // that has not entered its products yet. Point at the screen that fixes
        // it instead of showing an empty grid with no explanation.
        if state.products.is_empty() {
            components::empty_state_card(
                ui,
                "No products yet",
                "Add the shop's first product and it appears here as a key.",
                200.0,
            );
            ui.add_space(SPACING.sm);
            ui.vertical_centered(|ui| {
                if components::primary_button(ui, "Add a product").clicked() {
                    // Straight into the form: an empty catalog is a dead end
                    // otherwise, and the operator has already said what they want.
                    state.select(PRODUCTS);
                    state.start_product();
                }
            });
            return;
        }

        let needle = state.search.trim().to_lowercase();
        let visible: Vec<usize> = state
            .products
            .iter()
            .enumerate()
            .filter(|(_, product)| matches(product, state.category.as_deref(), &needle))
            .map(|(index, _)| index)
            .collect();

        if visible.is_empty() {
            components::empty_state_card(
                ui,
                "Nothing matches",
                "Clear the search or pick another category.",
                160.0,
            );
            return;
        }

        // Tile by rectangle, not by wrapping. The column count is arithmetic on
        // the panel width, so a product keeps its place as the window resizes and
        // a two-line label can never push its neighbour onto the next row. Cell
        // width follows the count, so a narrow panel fills instead of leaving a
        // stub of dead space next to every key.
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

        if let Some(index) = clicked {
            let product = state.products[index].clone();
            state.add_to_cart(&product);
        }
    }
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
                let mut steps: Vec<(usize, components::Step)> = Vec::new();

                for (index, line) in state.cart.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(components::semibold(&line.name).size(TYPE.body));
                            ui.label(
                                components::numeric(format!(
                                    "{} × {}",
                                    line.qty,
                                    state.money(line.unit_minor)
                                ))
                                .size(TYPE.small)
                                .color(palette.text_muted),
                            );
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            // Registered right-to-left: the line total is the
                            // right edge, the stepper sits inboard of it.
                            ui.label(
                                components::numeric(state.money(line.total_minor()))
                                    .size(TYPE.body),
                            );
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
        let (count, revenue) = crate::db::totals(&state.store, state.range);

        components::section_title(ui, state::range_caption(state.range));
        // Derived, not stored: revenue over sales, same range. A range with
        // no sales shows "-", never a zero that could be read as "we made
        // nothing today".
        let average = if count > 0 {
            state.money(revenue / count)
        } else {
            "-".to_owned()
        };
        kpi_row(
            ui,
            &[
                ("Sales", count.to_string()),
                ("Revenue", state.money(revenue)),
                ("Average", average),
                ("In cart", state.cart_count().to_string()),
                ("Cart value", state.money(state.cart_total_minor())),
            ],
        );

        // The shape of the range, as a chart: one bar per selling day, oldest
        // on the left so the range reads like a calendar, and a round axis
        // above the busiest day.
        ui.add_space(SPACING.lg);
        components::section_title(ui, "By day");
        let days = crate::db::by_day(&state.store, state.range);
        if days.is_empty() {
            components::empty_state_card(
                ui,
                "No sales yet in this range",
                "Each day that takes money grows a bar here.",
                160.0,
            );
        } else {
            components::card(ui, |ui| {
                ui.set_min_width(ui.available_width());
                day_chart(ui, state, &days);
            });
        }

        ui.add_space(SPACING.lg);
        components::section_title(ui, "Latest sales");
        let sales = crate::db::sales(&state.store, state.range, 5);
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

        // End-of-shift is not about the shop: it is about who took what. The
        // summary follows the range on the sidebar, so "today" already asks
        // the question the owner came to answer.
        ui.add_space(SPACING.lg);
        components::section_title(ui, "By person");
        let people = crate::db::by_person(&state.store, state.range);
        if people.is_empty() {
            components::empty_state_card(
                ui,
                "No sales yet in this range",
                "Whoever takes the money shows up here, with their share.",
                160.0,
            );
            return;
        }

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

        // What actually sells. Ranked by the money each name took, with the
        // units beside it — neither number alone is the story: many cheap
        // units can out-earn a few dear ones, and either way the owner wants
        // both.
        ui.add_space(SPACING.lg);
        components::section_title(ui, "Top items");
        let items = crate::db::top_items(&state.store, state.range, 5);
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
fn kpi_row(ui: &mut egui::Ui, cards: &[(&str, String)]) {
    let gap = SPACING.sm;
    let available = ui.available_width();
    let columns = (((available + gap) / (140.0 + gap)).floor() as usize).clamp(1, 4);
    let card = ((available - (columns as f32 - 1.0) * gap) / columns as f32).floor();

    for chunk in cards.chunks(columns) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (label, value) in chunk {
                kpi(ui, card, label, value);
            }
        });
        ui.add_space(gap);
    }
}

fn kpi(ui: &mut egui::Ui, width: f32, label: &str, value: &str) {
    components::card(ui, |ui| {
        // The frame adds its margin around this width, so the card lands on the
        // column width instead of one margin too wide.
        ui.set_width((width - 2.0 * SPACING.lg).max(60.0));
        components::field_label(ui, label);
        ui.add_space(SPACING.xs);
        ui.label(components::numeric(value).size(TYPE.kpi));
    });
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

    let peak = days.iter().map(|day| day.revenue).max().unwrap_or_default();
    let (axis_max, step) = nice_axis(peak);

    // A gutter for the value labels and a strip under the bars for the dates.
    let plot = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 46.0, rect.top() + 8.0),
        egui::pos2(rect.right(), rect.bottom() - 24.0),
    );

    let muted = egui::Color32::from(palette.text_muted);

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
                egui::Color32::from(palette.accent_soft),
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
            egui::Color32::from(palette.accent),
        );
    }

    // As many dates as fit without colliding, counted from the newest day so
    // the right edge is always labelled. A week is read as a rhythm, so its
    // days are named; a month is read as a calendar, so it is dated. The step
    // comes from the width of a real label rather than a guess, so a long
    // range thins out instead of running the dates together.
    let label_width = painter
        .layout_no_wrap(
            axis_label(&days[0], count),
            egui::FontId::new(TYPE.small, mono.clone()),
            muted,
        )
        .size()
        .x
        + 10.0;
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
        response.on_hover_ui(|ui| {
            ui.label(
                egui::RichText::new(format!("{} {}", weekday_name(day.weekday), day.day))
                    .family(theme::semibold())
                    .size(TYPE.small),
            );
            ui.label(
                egui::RichText::new(format!("{} sales", day.sales))
                    .size(TYPE.small)
                    .color(ui.visuals().weak_text_color()),
            );
            ui.label(
                egui::RichText::new(state.money(day.revenue))
                    .family(mono)
                    .size(TYPE.body),
            );
        });
    }
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
/// month is read as a calendar, so its days are dated. A lone day carries
/// both, because "Sun" alone does not say which Sunday.
fn axis_label(day: &crate::db::DayTotal, count: usize) -> String {
    let weekday = weekday_name(day.weekday);
    match count {
        0 | 1 => format!("{weekday} {}", day.day),
        2..=8 => weekday.to_owned(),
        _ => day.day.clone(),
    }
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
        egui::Key::F3
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
        ui.horizontal(|ui| {
            components::section_title(ui, "Catalog");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if components::primary_button(ui, "New product").clicked() {
                    new = true;
                }
                if ui.button("Import CSV").clicked() {
                    import = true;
                }
            });
        });
        if new {
            state.start_product();
        }
        if import {
            state.start_import();
        }

        if state.products.is_empty() {
            components::empty_state_card(
                ui,
                "No products yet",
                "Add the first product. Products become keys on the Sell screen.",
                200.0,
            );
            return;
        }

        let needle = state.search.trim().to_lowercase();
        let visible: Vec<usize> = state
            .products
            .iter()
            .enumerate()
            .filter(|(_, product)| matches(product, state.category.as_deref(), &needle))
            .map(|(index, _)| index)
            .collect();

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
        egui::ScrollArea::vertical()
            .id_salt("products_rows")
            .auto_shrink([false, false])
            .show(ui, |ui| {
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
    // Suggested category names, computed before the form is borrowed for
    // editing: the list comes from the catalog read-only, the chips are filled
    // into the draft being edited.
    let suggestions: Vec<String> = match state.product_editor.as_ref() {
        Some(draft) => crate::state::category_suggestions(&state.categories, &draft.category),
        None => Vec::new(),
    };
    let Some(draft) = state.product_editor.as_mut() else {
        return;
    };

    let mut save = false;
    let mut cancel = false;

    egui::Modal::new(egui::Id::new("pos_product_editor")).show(ctx, |ui| {
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
                text_field(ui, &mut draft.category, "General", 200.0);
            });
        });

        // Category names the till knows, tapped instead of typed: the
        // catalog's own first, then a starter set for a shop that has not
        // named its shelves yet. Typos are why this exists — "Drinks" and
        // "drinks" must not become two filters because they were typed twice.
        if !suggestions.is_empty() {
            ui.add_space(SPACING.sm);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(SPACING.xs, SPACING.xs);
                for name in &suggestions {
                    if name.eq_ignore_ascii_case(draft.category.trim()) {
                        continue;
                    }
                    if ui.add(egui::Button::new(name.clone()).small()).clicked() {
                        draft.category = name.clone();
                    }
                }
            });
        }

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

        ui.add_space(SPACING.md);
        ui.horizontal(|ui| {
            if components::primary_button(ui, "Save").clicked() {
                save = true;
            }
            if ui.button("Cancel").clicked() {
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

/// The CSV import, drawn with the other foreground layers in `shell.rs`.
///
/// The path is typed rather than picked from a dialog: importing is a one-time
/// setup job for the owner, not a daily flow, and a file-dialog dependency is
/// not worth what it costs a till this small. The modal closes on a successful
/// import and stays open on anything the till could not read, with the reason
/// it could not.
pub fn import_modal(ctx: &egui::Context, state: &mut State) {
    let Some(draft) = state.import.as_mut() else {
        return;
    };

    let mut import = false;
    let mut cancel = false;

    egui::Modal::new(egui::Id::new("pos_import")).show(ctx, |ui| {
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

        ui.add_space(SPACING.md);
        ui.horizontal(|ui| {
            if components::primary_button(ui, "Import").clicked() {
                import = true;
            }
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
        });
    });

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
///
/// The receipt behind a list row. Every figure on it is what was recorded at
/// the time — `unit × qty` as charged, the rounding the customer actually
/// paid — because a customer conversation starts from "this is what the till
/// says", not from a re-derivation that can disagree with the ledger.
pub fn sale_detail_modal(ctx: &egui::Context, state: &mut State) {
    let Some(detail) = state.sale_open.as_ref() else {
        return;
    };

    let mut close = false;

    egui::Modal::new(egui::Id::new("pos_sale_detail")).show(ctx, |ui| {
        ui.set_width(460.0);

        ui.label(components::semibold(format!("Sale #{}", detail.id)).size(TYPE.heading));
        components::caption(
            ui,
            &format!(
                "By {} — {}",
                detail.cashier.as_deref().unwrap_or("-"),
                detail.created_at
            ),
        );

        ui.add_space(SPACING.md);
        components::card(ui, |ui| {
            ui.set_min_width(ui.available_width());
            if detail.lines.is_empty() {
                components::caption(ui, "No line items were recorded on this sale.");
                return;
            }
            for line in &detail.lines {
                // A paper receipt prints "4 × Sugar @ USh 500"; a single item
                // needs neither count nor unit, so the label is just the name.
                let label = if line.qty == 1 {
                    line.name.clone()
                } else {
                    format!(
                        "{} × {} @ {}",
                        line.qty,
                        line.name,
                        state.money(line.unit_minor)
                    )
                };
                components::receipt_line(ui, &label, &state.money(line.total_minor));
            }
        });

        ui.add_space(SPACING.md);
        components::card(ui, |ui| {
            ui.set_min_width(ui.available_width());
            components::receipt_line(ui, "Total", &state.money(detail.total_minor));
            if detail.rounding_minor != 0 {
                components::receipt_line(ui, "Cash rounding", &state.money(detail.rounding_minor));
            }
            // "Paid" is the answer the owner is after: total plus rounding,
            // the figure that entered the drawer.
            crate::modules::amount_row(ui, "Paid", &state.money(detail.paid_minor), true);

            ui.add_space(SPACING.sm);
            components::receipt_line(ui, "Method", &detail.method);
            if detail.tendered_minor != 0 {
                components::receipt_line(
                    ui,
                    "Tendered (cash)",
                    &state.money(detail.tendered_minor),
                );
            }
            if detail.change_minor != 0 {
                components::receipt_line(ui, "Change", &state.money(detail.change_minor));
            }
        });

        ui.add_space(SPACING.md);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if components::primary_button(ui, "Close").clicked() {
                close = true;
            }
        });
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
        SELL | DASHBOARD | SALES => Permission::Sell,
        PRODUCTS => Permission::ChangePrice,
        _ => Permission::EditSettings,
    }
}

/// Switch module, asking for approval when the person cannot open it.
pub fn open(state: &mut State, index: usize) {
    let permission = permission(index);
    state.open(index, permission);
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
        components::section_title(ui, "Transactions");

        let sales = crate::db::sales(&state.store, state.range, 100);
        if sales.is_empty() {
            components::empty_state_card(
                ui,
                "No transactions",
                "Every completed sale in this range is listed here, newest first.",
                240.0,
            );
            return;
        }

        egui::ScrollArea::vertical()
            .id_salt("sales_rows")
            .auto_shrink([false, false])
            .show(ui, |ui| {
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
        egui::Key::F5
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
            "Terminal" => terminal_card(ui, state),
            "Staff" => {
                if state.permits(crate::auth::Permission::ManageUsers) {
                    crate::auth::staff_card(ui, state);
                } else {
                    locked_card(ui, crate::auth::Permission::ManageUsers);
                }
            }
            "Tax" => components::empty_state_card(
                ui,
                "No tax rules",
                "Rates are applied at checkout; the table that stores them lands with the sale flow.",
                160.0,
            ),
            _ => components::empty_state_card(
                ui,
                "Not connected",
                "This till runs alone. A hub or cloud endpoint can be configured once sync exists.",
                160.0,
            ),
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
                if ui.button("Change currency").clicked() {
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

fn info_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        components::field_label(ui, label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(components::semibold(value).size(TYPE.body));
        });
    });
    ui.add_space(SPACING.xs);
}

/// A sample receipt: the one place the receipt face and its 13 px column width
/// can be judged before a printer exists.
fn receipt_preview(ui: &mut egui::Ui, state: &State) {
    components::card(ui, |ui| {
        ui.set_width(300.0);
        ui.vertical_centered(|ui| {
            ui.label(components::semibold(crate::APP_NAME).size(TYPE.body));
            components::caption(ui, "Sample receipt");
        });
        ui.add_space(SPACING.sm);
        components::divider(ui);
        components::receipt_line(ui, "2 x Flat white", &state.money(700));
        components::receipt_line(ui, "1 x Cinnamon roll", &state.money(325));
        components::receipt_line(ui, "1 x Orange juice", &state.money(450));
        components::divider(ui);
        components::receipt_line(ui, "Subtotal", &state.money(1_475));
        components::receipt_line(ui, "Tax", &state.money(0));
        components::divider(ui);
        components::receipt_line(ui, "Total", &state.money(1_475));
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
                "hotkeys live in F1-F8; F6-F8 are reserved for inventory, suppliers and POs"
            );
        }

        let before = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(before, keys.len(), "two modules must never share a hotkey");
    }

    #[test]
    fn every_module_has_a_title() {
        // The rail and the palette both read `title`; an empty one is a blank
        // button that is impossible to diagnose from a screenshot.
        for module in all() {
            assert!(!module.title().is_empty());
        }
    }
}
