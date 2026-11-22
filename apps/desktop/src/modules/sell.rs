//! Sell: the till. Pick products, build a cart, take the money.
//!
//! The two views (tiles and rows) are the same list drawn two ways, so they
//! share the visible-index list the filters produce and differ only in the
//! widget. The cart is the one screen that mutates a sale in progress.

use super::*;
// ---------------------------------------------------------------------------
// Sell
// ---------------------------------------------------------------------------

/// Horizontal scrolling category pill bar.
///
/// Kept on a single row above the product grid so categories are reachable with
/// a single tap, while scrolling smoothly horizontally if there are many.
pub(super) fn categories_bar(ui: &mut egui::Ui, state: &mut State) {
    let palette = theme::current(ui.ctx());
    egui::ScrollArea::horizontal()
        .id_salt("sell_categories_bar")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = SPACING.xs;
            ui.horizontal(|ui| {
                let all_active = state.category.is_none();
                let mut all_text = egui::RichText::new("All").size(TYPE.small);
                if all_active {
                    all_text = all_text
                        .family(theme::semibold())
                        .color(egui::Color32::from(palette.accent_ink()));
                } else {
                    all_text = all_text.color(egui::Color32::from(palette.text_muted));
                }
                let all_btn = egui::Button::new(all_text)
                    .min_size(egui::vec2(52.0, 32.0))
                    .fill(if all_active {
                        egui::Color32::from(palette.accent_soft)
                    } else {
                        egui::Color32::from(palette.surface)
                    })
                    .stroke(egui::Stroke::new(1.0, palette.border))
                    .corner_radius(egui::CornerRadius::same(RADII.pill as u8));
                if ui.add(all_btn).clicked() {
                    state.category = None;
                }

                let categories = state.categories.clone();
                for cat in &categories {
                    let active = state.category.as_deref() == Some(cat.as_str());
                    let mut text = egui::RichText::new(cat).size(TYPE.small);
                    if active {
                        text = text
                            .family(theme::semibold())
                            .color(egui::Color32::from(palette.accent_ink()));
                    } else {
                        text = text.color(egui::Color32::from(palette.text));
                    }
                    let btn = egui::Button::new(text)
                        .min_size(egui::vec2(0.0, 32.0))
                        .fill(if active {
                            egui::Color32::from(palette.accent_soft)
                        } else {
                            egui::Color32::from(palette.surface)
                        })
                        .stroke(egui::Stroke::new(1.0, palette.border))
                        .corner_radius(egui::CornerRadius::same(RADII.pill as u8));
                    if ui.add(btn).clicked() {
                        state.category = Some(cat.clone());
                    }
                }
            });
        });
}

/// The till: the catalog on the left, the cart on the right.
pub(super) struct Sell;

impl Module for Sell {
    /// Sell has no vertical sidebar; categories live in a horizontal pill bar
    /// directly above the product catalog, maximizing grid real estate.
    fn fixed_sidebar(&self) -> Option<f32> {
        Some(0.0)
    }

    /// The till owns its own frame: grid on the left, cart pinned right. A page
    /// scroll would make the cart's height unbounded and un-pin the Pay button.
    fn scrolls(&self) -> bool {
        false
    }

    fn sidebar(&mut self, _ui: &mut egui::Ui, _state: &mut State) {}

    /// The scan field lives on the page bar, like every other list's search.
    ///
    /// It also owns the scanner's Enter: the field that a barcode types into is
    /// the field that has to submit it, and the grid below draws after this bar,
    /// so it sees the cart and the cleared query in the same frame.
    fn toolbar(&mut self, ui: &mut egui::Ui, state: &mut State) {
        // The bar is part of the till's map, not of the payment: while the
        // tender owns the keyboard it dims with the grid, so a scan cannot land
        // in a basket that is already being paid for.
        ui.scope(|ui| {
            if state.is_tendering() {
                ui.set_opacity(0.35);
                ui.disable();
            }
            let response = action_bar(
                ui,
                PageLeft::Search {
                    text: &mut state.sell_search,
                    hint: "Name or barcode",
                    count: None,
                },
                |ui| sell_view_toggle(ui, &mut state.sell_view),
            )
            .expect("Sell's bar has a search field");

            // The cursor belongs here whenever the selling screen opens: nothing
            // else on Sell takes text, and a scanner types into whatever has
            // focus.
            if std::mem::take(&mut state.scan_focus) {
                response.request_focus();
            }

            // A barcode scanner is a keyboard that types and then presses Enter.
            // The field already filtered the products, but a cashier with a
            // scanner in their hand expects the item in the cart — asking them
            // to click a tile they just scanned is the whole bug. `lost_focus`
            // alone would also fire on a click elsewhere, so the Enter press is
            // what confirms the scan. A phone scan has no keyboard to press
            // Enter, so it sets `scan_submit`; that flag comes through here, on
            // the same path.
            //
            // A scan that arrives mid-payment is consumed and dropped: the
            // basket is frozen until the sale is written. Leaving the flag set
            // would fire it at the next customer, which is a different bug.
            let submitted = (response.lost_focus()
                && ui.input(|input| input.key_pressed(egui::Key::Enter)))
                || std::mem::take(&mut state.scan_submit);
            if !submitted || state.is_tendering() {
                return;
            }

            let scanned = state.sell_search.trim().to_owned();
            // Enter on an empty field is the till's fourth keystroke: scan,
            // scan, Enter, cash. There is no barcode to look up, so it means
            // "that is the basket" and the cart's footer becomes the tender.
            if scanned.is_empty() {
                if !state.cart.is_empty() {
                    state.start_payment();
                }
                response.request_focus();
                return;
            }

            let needle = scanned.to_lowercase();
            let visible = visible_products(&state.products, state.category.as_deref(), &needle);
            match scan_target(&state.products, &scanned, &visible) {
                Scan::One { index, unit_factor } => {
                    // Cloned because the lookup holds `state.products` borrowed
                    // and `add_to_cart` needs the whole state.
                    let product = state.products[index].clone();
                    state.add_to_cart(&product, unit_factor);
                    // Clear the field so the next scan starts clean, and keep the
                    // focus so it lands here without a mouse.
                    state.sell_search.clear();
                    response.request_focus();
                }
                // Say what happened in words the cashier can act on.
                // `add_to_cart` clears `notice`, so these are set after it,
                // never before.
                Scan::Ambiguous(count) => {
                    state.notice = Some(format!(
                        "{count} products match {scanned} — scan a barcode or type more."
                    ));
                }
                Scan::None => {
                    state.notice = Some(format!("No product matches {scanned}."));
                }
            }
        });
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

        // Horizontal Category Pill Bar: smooth single-row scrolling across all categories.
        categories_bar(ui, state);
        ui.add_space(SPACING.xs);

        let needle = state.sell_search.trim().to_lowercase();
        let visible = visible_products(&state.products, state.category.as_deref(), &needle);

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

        if let Some((index, unit_factor)) = clicked {
            let product = state.products[index].clone();
            state.add_to_cart(&product, unit_factor);
        }
    }
}

/// Two buttons that choose how the Sell screen arranges its products.
///
/// One bordered container, not two loose buttons: the pair is a single choice,
/// and a gap between them would read as two independent actions.
pub(super) fn sell_view_toggle(ui: &mut egui::Ui, view: &mut SellView) {
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
pub(super) fn product_tiles(
    ui: &mut egui::Ui,
    state: &State,
    visible: &[usize],
) -> Option<(usize, i64)> {
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
                let response = ui.put(rect, tile);
                let response = if product.pack_size > 1
                    && let Some(pack_price) = product.pack_price_minor
                {
                    response.on_hover_text(format!(
                        "Pack of {}: {}",
                        product.pack_size,
                        state.money(pack_price)
                    ))
                } else {
                    response
                };
                response.context_menu(|ui| {
                    if ui.button("Add loose").clicked() {
                        clicked = Some((product_index, 1));
                        ui.close();
                    }
                    if product.pack_size > 1
                        && product.pack_price_minor.is_some()
                        && ui
                            .button(format!("Add pack of {}", product.pack_size))
                            .clicked()
                    {
                        clicked = Some((product_index, product.pack_size));
                        ui.close();
                    }
                });
                if response.clicked() {
                    clicked = Some((product_index, 1));
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
pub(super) fn product_rows(
    ui: &mut egui::Ui,
    state: &State,
    visible: &[usize],
) -> Option<(usize, i64)> {
    let mut clicked = None;
    egui::ScrollArea::vertical()
        .id_salt("sell_list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = SPACING.xs;
            for &product_index in visible {
                let product = &state.products[product_index];
                let price = state.money(product.price_minor);
                let response = sell_row(ui, &product.name, &price);
                let response = if product.pack_size > 1
                    && let Some(pack_price) = product.pack_price_minor
                {
                    response.on_hover_text(format!(
                        "Pack of {}: {}",
                        product.pack_size,
                        state.money(pack_price)
                    ))
                } else {
                    response
                };
                response.context_menu(|ui| {
                    if ui.button("Add loose").clicked() {
                        clicked = Some((product_index, 1));
                        ui.close();
                    }
                    if product.pack_size > 1
                        && product.pack_price_minor.is_some()
                        && ui
                            .button(format!("Add pack of {}", product.pack_size))
                            .clicked()
                    {
                        clicked = Some((product_index, product.pack_size));
                        ui.close();
                    }
                });
                if response.clicked() {
                    clicked = Some((product_index, 1));
                }
            }
        });

    clicked
}

/// One product in the Sell list: name and price on a row that is one big
/// click target.
pub(super) fn sell_row(ui: &mut egui::Ui, name: &str, price: &str) -> egui::Response {
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

pub(super) fn cart(ui: &mut egui::Ui, state: &mut State) {
    let palette = theme::current(ui.ctx());

    ui.horizontal(|ui| {
        ui.label(components::semibold("Cart").size(TYPE.heading));

        // Parked cart recall button if there are any parked baskets
        let parked_count = state.parked_carts.len();
        if parked_count > 0 && state.payment.is_none() {
            let label = format!("Parked ({parked_count})");
            let resp = ui.add(
                egui::Button::new(
                    egui::RichText::new(&label)
                        .size(TYPE.small)
                        .family(theme::semibold())
                        .color(egui::Color32::from(palette.accent_ink())),
                )
                .fill(egui::Color32::from(palette.accent_soft))
                .stroke(egui::Stroke::NONE)
                .corner_radius(egui::CornerRadius::same(RADII.pill as u8)),
            );
            resp.context_menu(|ui| {
                ui.label(components::semibold("Parked Carts").size(TYPE.small));
                ui.separator();
                for i in 0..parked_count {
                    let p = &state.parked_carts[i];
                    let text = format!("{} · {} ({} items)", p.id, p.created_at, p.lines.len());
                    if ui.button(text).clicked() {
                        state.recall_cart(i);
                        ui.close();
                        break;
                    }
                }
            });
            if resp.clicked() {
                // Quick tap recalls the most recent parked cart
                state.recall_cart(parked_count - 1);
            }
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !state.cart.is_empty() && state.payment.is_none() {
                if actions::quiet_compact(ui, "Clear")
                    .on_hover_text("Clear entire cart")
                    .clicked()
                {
                    state.clear_cart();
                }
                ui.add_space(SPACING.xs);
                if actions::quiet_compact(ui, "Hold")
                    .on_hover_text("Hold basket for later")
                    .clicked()
                {
                    state.hold_cart();
                }
                ui.add_space(SPACING.xs);
            }
            components::caption(ui, &format!("{} item(s)", state.cart_count()));
        });
    });
    ui.add_space(SPACING.sm);
    components::divider(ui);
    ui.add_space(SPACING.sm);
    customer_row(ui, state);
    ui.add_space(SPACING.xs);
    customer_mode_row(ui, state);

    // Reserve the footer first so the line list scrolls instead of pushing the
    // total off the pane. A till whose total can scroll away is broken.
    //
    // The footer grows by one line when cash rounding applies, because that row
    // only exists then and a fixed reserve would push the Pay button off the
    // bottom at the window's narrow end.
    //
    // Paying takes the whole footer, and the tender panel is much taller than
    // the totals it replaces. Its height is the sum of its rows — two 44.8
    // amount rows, a 44 field, 44 chips, a 52 button, and the hint row that
    // shares its line with "Back to cart" — plus 6px gaps and a little slack.
    // An error replaces the hint rather than stacking under it, so that case
    // does not need its own reserve.
    let rounding_minor = state.cart_rounding_minor();
    // A discount row and the wholesale warning are rows the footer does not
    // always have; reserving for them only when they exist keeps a plain cart
    // as tall as it was.
    let discount_minor = state.cart_discount_minor();
    let suppressed = state.customer_mode == CustomerMode::Wholesale && state.customer.is_some();
    let extra = if discount_minor > 0 {
        TYPE.small * 1.8
    } else {
        0.0
    } + if suppressed { TYPE.small * 1.8 } else { 0.0 };
    let footer = match state.payment.as_ref() {
        Some(_) => {
            420.0
                + if rounding_minor != 0 {
                    TYPE.small * 1.8
                } else {
                    0.0
                }
        }
        None if rounding_minor == 0 => 196.0 + extra,
        None => 196.0 + extra + TYPE.small * 1.8,
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
                        let each = format!(
                            "{} × {}{}{}",
                            line.qty,
                            state.money(line.unit_minor),
                            line.unit_label
                                .as_deref()
                                .map(|label| format!(" · {label}"))
                                .unwrap_or_default(),
                            // The rung is not optional: without it the cashier
                            // and the customer read the same amount differently.
                            line.tier_label()
                                .map(|label| format!(" · {label}"))
                                .unwrap_or_default()
                        );
                        ui.label(
                            components::numeric(each)
                                .size(TYPE.small)
                                .color(palette.text_muted),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            // Quick-remove item button (trash). The hit area is a
                            // full 44-point target; the glyph and its hover fill
                            // stay 32 so the cart line keeps its density.
                            let (trash_rect, trash_resp) = ui
                                .allocate_exact_size(egui::vec2(44.0, 44.0), egui::Sense::click());
                            let trash_resp = trash_resp
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .on_hover_text("Remove item");
                            if ui.is_rect_visible(trash_rect) {
                                let hovered = trash_resp.hovered();
                                let visual = egui::Rect::from_center_size(
                                    trash_rect.center(),
                                    egui::vec2(32.0, 32.0),
                                );
                                if hovered {
                                    ui.painter().rect_filled(
                                        visual,
                                        egui::CornerRadius::same(RADII.sm as u8),
                                        ui.visuals().widgets.hovered.weak_bg_fill,
                                    );
                                }
                                let trash_color = if hovered {
                                    egui::Color32::from(palette.danger)
                                } else {
                                    egui::Color32::from(palette.text_muted)
                                };
                                Icon::TRASH.image(16.0, trash_color).paint_at(ui, visual);
                            }
                            if trash_resp.clicked() {
                                remove = Some(index);
                            }

                            ui.add_space(SPACING.xs);

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
                        components::Step::Up => {
                            let qty = line.qty + 1;
                            line.set_qty(qty);
                        }
                        components::Step::Down => {
                            let qty = line.qty - 1;
                            line.set_qty(qty);
                            if qty <= 0 {
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
    // The subtotal is the shelf price of everything in the basket; the total is
    // what the customer actually pays. Showing only the net would hide the
    // discount the cashier just granted, and showing only the gross would
    // disagree with the receipt.
    let subtotal = state.cart_gross_minor();
    let total = state.cart_total_minor();
    amount_row(ui, "Subtotal", &state.money(subtotal), false);
    if discount_minor > 0 {
        let label = format!("Discount ({})", percent_text(state.effective_discount_bp()));
        amount_row(ui, &label, &state.money(-discount_minor), false);
    }
    amount_row(ui, "Tax", "not configured", false);
    ui.add_space(SPACING.xs);
    // "Total" is the exact total, the number the receipt prints. The rounded
    // cash amount is the payment screen's, and it says so there: a cart that
    // showed 300 for a 340 basket would hide the 40 the shop is giving away.
    amount_row(ui, "Total", &state.money(total), true);
    if suppressed {
        let name = state
            .customer
            .as_ref()
            .map(|customer| customer.name.as_str())
            .unwrap_or_default();
        components::caption(
            ui,
            &format!(
                "Wholesale prices — {name}'s {} is not applied",
                percent_text(state.customer.as_ref().map_or(0, |c| c.discount_bp))
            ),
        );
    }
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

/// The regular customer this basket belongs to, above the ladder switch.
///
/// A chooser rather than a text field: the rate is the shop's own term, and a
/// typed name would either match nothing or invent a second "Mama Njeri" at a
/// different rate. Walk-in is the default and needs no permission; attaching a
/// customer with a rate asks for the discount permission, like the switch below.
fn customer_row(ui: &mut egui::Ui, state: &mut State) {
    let selected = state
        .customer
        .as_ref()
        .map(|customer| customer.name.clone())
        .unwrap_or_else(|| "Walk-in".to_owned());
    // The choice is taken out of the loop and applied afterwards: attaching a
    // customer needs `&mut state`, which the list borrow forbids mid-iteration.
    let mut choice: Option<Option<usize>> = None;

    ui.add_enabled_ui(!state.is_tendering(), |ui| {
        egui::ComboBox::from_id_salt("pos_cart_customer")
            .width(ui.available_width())
            .selected_text(selected)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(state.customer.is_none(), "Walk-in")
                    .clicked()
                {
                    choice = Some(None);
                    ui.close();
                }
                if state.customers.is_empty() {
                    components::caption(ui, "No regular customers yet. Add them in Settings.");
                }
                for (index, customer) in state.customers.iter().enumerate() {
                    let label = if customer.discount_bp > 0 {
                        format!("{} · {}", customer.name, percent_text(customer.discount_bp))
                    } else {
                        customer.name.clone()
                    };
                    let attached = state
                        .customer
                        .as_ref()
                        .is_some_and(|current| current.id == customer.id);
                    if ui.selectable_label(attached, label).clicked() {
                        choice = Some(Some(index));
                        ui.close();
                    }
                }
            });
    });

    match choice {
        Some(Some(index)) => {
            let customer = state.customers[index].clone();
            state.set_customer(Some(customer));
        }
        Some(None) => state.set_customer(None),
        None => {}
    }
}

/// The retail/wholesale switch, above the lines it reprices.
///
/// A switch rather than a menu because there are two states and the cashier has
/// to see which one is live before the first item is scanned. Wholesale is a
/// pricing decision, so it asks for the same permission as a discount; the
/// override leaves a minute-long approval and the cashier taps again.
fn customer_mode_row(ui: &mut egui::Ui, state: &mut State) {
    let palette = theme::current(ui.ctx());
    let wholesale = state.customer_mode == CustomerMode::Wholesale;
    let text = egui::RichText::new(state.customer_mode.label())
        .size(TYPE.small)
        .family(theme::semibold());
    let text = if wholesale {
        text.color(egui::Color32::from(palette.accent_ink()))
    } else {
        text.color(egui::Color32::from(palette.text_muted))
    };
    let button = egui::Button::new(text)
        .min_size(egui::vec2(ui.available_width(), 36.0))
        .fill(if wholesale {
            egui::Color32::from(palette.accent_soft)
        } else {
            egui::Color32::TRANSPARENT
        })
        .stroke(egui::Stroke::new(1.0_f32, palette.border))
        .corner_radius(egui::CornerRadius::same(RADII.sm as u8));
    // While the tender panel owns the screen the basket is frozen, so the mode
    // is too: repricing under a payment already on the counter is not a thing
    // the cashier can mean.
    let response = ui
        .add_enabled(!state.is_tendering(), button)
        .on_hover_text("Wholesale prices use each product's deepest quantity break");
    if response.clicked() {
        if state.permits(crate::auth::Permission::Discount) {
            state.toggle_customer_mode();
        } else {
            state.ask_override(crate::auth::Permission::Discount, None);
        }
    }
}

/// Whether `text` is wider than the space its label was given.
///
/// The cart asks before offering a tooltip: a name that already fits needs no
/// second copy on hover, and a truncated one is exactly the case where the full
/// name is worth the hover.
pub(super) fn is_cut(ui: &egui::Ui, text: &str, width: f32) -> bool {
    let font = egui::FontId::new(TYPE.body, theme::semibold());
    let measured = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, egui::Color32::PLACEHOLDER)
        .size()
        .x;
    measured > width
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

        // A pack and a loose unit are two rows for the same product, so the
        // editor names which one. A cashier editing "12" needs to know whether
        // that is twelve tablets or twelve boxes before pressing Set.
        if let Some(editor) = &state.cart_quantity
            && let Some(line) = state.cart.iter().find(|line| {
                line.product_id == editor.product_id && line.unit_factor == editor.unit_factor
            })
        {
            components::caption(
                ui,
                &match line.unit_label.as_deref() {
                    Some(label) => format!("{} · {label}", line.name),
                    None => line.name.clone(),
                },
            );
        }
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
            Some(qty) => state.set_cart_quantity(editor.product_id, editor.unit_factor, qty),
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
pub(super) fn quantity_key(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let text = egui::RichText::new(label)
        .size(TYPE.button)
        .family(theme::semibold());
    ui.add(
        egui::Button::new(text)
            .min_size(egui::vec2(72.0, 40.0))
            .corner_radius(egui::CornerRadius::same(RADII.sm as u8)),
    )
}

/// Width of the cart column inside the Sell workspace.
///
/// Not a shell panel on purpose: the cart belongs to selling, and a cashier who
/// switches to Settings should not drag a cart's worth of dead pixels along.
/// Promoting it to `regions::sidebar`-style right panel later is a local change
/// in [`Sell::workspace`].
///
/// It is a share of the workspace with a floor and a ceiling: a fixed width is
/// too wide on a 960 px till and too narrow on a 4K one.
pub(super) const CART_MIN: f32 = 300.0;

pub(super) const CART_MAX: f32 = 380.0;

pub(super) const CART_SHARE: f32 = 0.34;

/// Height of one product tile. Two lines of name plus a price line fit.
pub(super) const TILE_HEIGHT: f32 = 96.0;

/// Narrowest a tile may get before the grid drops a column.
pub(super) const TILE_MIN: f32 = 140.0;

/// Height of one table row, including its breathing room.
pub(super) const ROW_HEIGHT: f32 = 40.0;
