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

/// The till: quick keys on the left, the cart on the right.
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
        components::section_title(ui, "Categories");
        for (index, category) in state::CATEGORIES.iter().enumerate() {
            if regions::nav_item(ui, state.category == index, category).clicked() {
                state.category = index;
            }
        }
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
        components::search_field(ui, &mut state.search, "Barcode");
        ui.add_space(SPACING.md);

        let needle = state.search.trim().to_lowercase();
        let keys: Vec<(&'static str, i64)> = state::QUICK_KEYS
            .iter()
            .filter(|(name, _)| needle.is_empty() || name.to_lowercase().contains(&needle))
            .copied()
            .collect();

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

        egui::ScrollArea::vertical()
            .id_salt("sell_quick_keys")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let area = ui.available_rect_before_wrap();
                for (index, (name, unit_minor)) in keys.iter().enumerate() {
                    let column = (index % columns) as f32;
                    let row = (index / columns) as f32;
                    let min = egui::pos2(
                        area.left() + column * (cell.x + gap),
                        area.top() + row * (cell.y + gap),
                    );
                    let rect = egui::Rect::from_min_size(min, cell);
                    let price = state.money(*unit_minor);
                    let tile = components::Tile {
                        name,
                        price: &price,
                    };
                    if ui.put(rect, tile).clicked() {
                        state.add_to_cart(name, *unit_minor);
                    }
                }

                let rows = keys.len().div_ceil(columns);
                let height = rows as f32 * cell.y + rows.saturating_sub(1) as f32 * gap;
                ui.advance_cursor_after_rect(egui::Rect::from_min_size(
                    area.min,
                    egui::vec2(area.width(), height),
                ));
            });
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
    const FOOTER: f32 = 196.0;
    let list_height = (ui.available_height() - FOOTER).max(120.0);

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
            "Scan an item or use a quick key.",
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
    amount_row(ui, "Total", &state.money(subtotal), true);
    ui.add_space(SPACING.sm);

    if let Some(notice) = state.notice.as_deref() {
        components::caption(ui, notice);
        ui.add_space(SPACING.xs);
    }

    let pay = ui.add_sized(
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
    );
    if pay.clicked() {
        // Deliberately not a fake success. The button is real, the flow is not,
        // and the user is told which one they hit.
        state.notice = Some("Payments arrive with the sale flow".to_owned());
    }
}

fn amount_row(ui: &mut egui::Ui, label: &str, amount: &str, strong: bool) {
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
        kpi_row(
            ui,
            &[
                ("Sales", count.to_string()),
                ("Revenue", state.money(revenue)),
                ("In cart", state.cart_count().to_string()),
                ("Cart value", state.money(state.cart_total_minor())),
            ],
        );

        ui.add_space(SPACING.lg);
        components::section_title(ui, "Latest sales");
        let sales = crate::db::sales(&state.store, state.range, 5);
        if sales.is_empty() {
            components::empty_state_card(
                ui,
                "No sales recorded",
                "Completed sales appear here as soon as the sale flow exists.",
                200.0,
            );
            return;
        }

        components::card(ui, |ui| {
            ui.set_min_width(ui.available_width());
            table_row(ui, "Sale", "When", "Total", true, false);
            for (index, sale) in sales.iter().enumerate() {
                table_row(
                    ui,
                    &format!("#{}", sale.id),
                    &sale.created_at,
                    &state.money(sale.total_minor),
                    false,
                    index % 2 == 1,
                );
            }
        });
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
fn table_row(ui: &mut egui::Ui, id: &str, when: &str, amount: &str, header: bool, stripe: bool) {
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

    let left = painter.layout_no_wrap(id.to_owned(), egui::FontId::new(size, mono.clone()), color);
    painter.galley(
        egui::pos2(rect.left(), y - left.size().y * 0.5),
        left,
        color,
    );

    let middle = painter.layout_no_wrap(
        when.to_owned(),
        egui::FontId::new(size, proportional),
        color,
    );
    painter.galley(
        egui::pos2(rect.left() + width * 0.30, y - middle.size().y * 0.5),
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
        components::section_title(ui, "Categories");
        for (index, category) in state::CATEGORIES.iter().enumerate() {
            if regions::nav_item(ui, state.category == index, category).clicked() {
                state.category = index;
            }
        }

        ui.add_space(SPACING.lg);
        components::field_label(ui, "Search");
        ui.add_space(SPACING.xs);
        components::search_field(ui, &mut state.search, "Name or barcode");
    }

    fn workspace(&mut self, ui: &mut egui::Ui, _state: &mut State) {
        components::section_title(ui, "Catalog");

        // There is no `products` table yet: the Sell grid is still the
        // placeholder list in `state::QUICK_KEYS`. Saying that plainly is
        // better than an empty table that looks like a query returning nothing.
        components::empty_state_card(
            ui,
            "No catalog yet",
            "Products are not stored yet; the Sell grid is placeholder data.",
            240.0,
        );
    }
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
                    table_row(ui, "Sale", "When", "Total", true, false);
                    for (index, sale) in sales.iter().enumerate() {
                        table_row(
                            ui,
                            &format!("#{}", sale.id),
                            &sale.created_at,
                            &state.money(sale.total_minor),
                            false,
                            index % 2 == 1,
                        );
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

        let section = state::SETTINGS_SECTIONS
            .get(state.settings_section)
            .copied()
            .unwrap_or("Terminal");

        ui.add_space(SPACING.lg);
        components::section_title(ui, section);

        match section {
            "Receipt" => receipt_preview(ui, state),
            "Terminal" => terminal_card(ui, state),
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
            if ui.button("Change currency").clicked() {
                // A failed attempt from last time is not about this one.
                state.currency_error = None;
                state.currency_picker = true;
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
