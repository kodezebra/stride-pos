//! Sell: the till. Pick products, build a cart, take the money.
//!
//! The two views (tiles and rows) are the same list drawn two ways, so they
//! share the visible-index list the filters produce and differ only in the
//! widget. The cart is the one screen that mutates a sale in progress.

use super::*;
use std::collections::{HashMap, hash_map::Entry};

use crate::state::CartSummary;

// ---------------------------------------------------------------------------
// Sell
// ---------------------------------------------------------------------------

/// Sell's read-only view of the catalog, assembled once per frame.
///
/// Both catalog views used to filter independently; they now share this list.
/// The hover hint for each match is derived here too, off the draw path, so a
/// click that adds to the cart never has to borrow the products twice.
pub(super) struct SellDerived {
    /// Indices into `state.products`, in catalog order.
    visible: Vec<usize>,
    /// Hover text for each visible product, in the same order as `visible`.
    details: Vec<Option<String>>,
}

impl SellDerived {
    pub(super) fn visible(&self) -> &[usize] {
        &self.visible
    }

    pub(super) fn detail(&self, slot: usize) -> Option<&str> {
        self.details.get(slot).and_then(Option::as_deref)
    }
}

pub(super) fn derive(state: &State) -> SellDerived {
    let needle = state.sell_search.trim().to_lowercase();
    let visible = visible_products(&state.products, state.category.as_deref(), &needle);
    let expiry = nearest_expiry(&state.expiring);
    let details = visible
        .iter()
        .map(|&index| product_detail(state, &state.products[index], &expiry))
        .collect();
    SellDerived { visible, details }
}

/// The closest dated shelf per product, and how much is on it.
///
/// Undated stock is deliberately absent: "expires in 3 days" must never be
/// inferred from a bucket with no date. Buckets on the same day are summed.
fn nearest_expiry(buckets: &[ExpiryBucket]) -> HashMap<i64, (i64, i64)> {
    let mut nearest = HashMap::new();
    for bucket in buckets {
        let Some(days) = bucket.days_left else {
            continue;
        };
        match nearest.entry(bucket.product_id) {
            Entry::Vacant(entry) => {
                entry.insert((days, bucket.on_hand));
            }
            Entry::Occupied(mut entry) if days < entry.get().0 => {
                entry.insert((days, bucket.on_hand));
            }
            Entry::Occupied(mut entry) if days == entry.get().0 => {
                entry.get_mut().1 += bucket.on_hand;
            }
            Entry::Occupied(_) => {}
        }
    }
    nearest
}

/// The extra facts a cashier may need before choosing a tile: pack price,
/// quantity breaks, and the nearest expiry. Kept as hover text so the grid
/// itself stays at a glance.
fn product_detail(
    state: &State,
    product: &Product,
    expiry: &HashMap<i64, (i64, i64)>,
) -> Option<String> {
    let mut lines = Vec::new();
    if product.pack_size > 1
        && let Some(price) = product.pack_price_minor
    {
        lines.push(format!(
            "Pack of {}: {}",
            product.pack_size,
            state.money(price)
        ));
    }
    if !product.tiers.is_empty() {
        let breaks: Vec<String> = product
            .tiers
            .iter()
            .map(|tier| format!("{}+ {}", tier.min_qty, state.money(tier.price_minor)))
            .collect();
        lines.push(format!("Breaks · {}", breaks.join(" · ")));
    }
    if let Some(&(days, on_hand)) = expiry.get(&product.id) {
        let when = match days {
            0 => "today".to_owned(),
            1 => "tomorrow".to_owned(),
            days if days > 0 => format!("in {days} days"),
            -1 => "yesterday".to_owned(),
            days => format!("{} days ago", -days),
        };
        lines.push(format!("Expires {when} · {on_hand} {}", product.unit_label));
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// Horizontal scrolling category pill bar.
///
/// Kept on a single row above the product grid so categories are reachable with
/// a single tap, while scrolling smoothly horizontally if there are many.
pub(super) fn categories_bar(ui: &mut egui::Ui, state: &mut State) {
    let palette = theme::current(ui.ctx());
    egui::ScrollArea::horizontal()
        .id_salt("sell_categories_bar")
        .max_height(40.0)
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
    /// Sell has no sidebar; categories live in a horizontal pill bar
    /// directly above the product catalog, maximizing grid real estate.
    fn has_sidebar(&self) -> bool {
        false
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

        // One filtered list and one set of hover hints per frame. The shell
        // runs the toolbar first, so the scan path still does its own lookup;
        // everything below the bar shares this snapshot.
        let derived = derive(state);

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
        self.products(&mut grid_ui, state, &derived);

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
    fn products(&mut self, ui: &mut egui::Ui, state: &mut State, derived: &SellDerived) {
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

        let visible = derived.visible();

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
            SellView::Grid => product_tiles(ui, state, derived),
            SellView::List => product_rows(ui, state, derived),
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
    derived: &SellDerived,
) -> Option<(usize, i64)> {
    let visible = derived.visible();
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
                let response = with_product_hint(response, derived.detail(index));
                product_context_menu(&response, product, product_index, &mut clicked);
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
    derived: &SellDerived,
) -> Option<(usize, i64)> {
    let mut clicked = None;
    egui::ScrollArea::vertical()
        .id_salt("sell_list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = SPACING.xs;
            for (slot, &product_index) in derived.visible().iter().enumerate() {
                let product = &state.products[product_index];
                let price = state.money(product.price_minor);
                let response = sell_row(ui, &product.name, &price);
                let response = with_product_hint(response, derived.detail(slot));
                product_context_menu(&response, product, product_index, &mut clicked);
                if response.clicked() {
                    clicked = Some((product_index, 1));
                }
            }
        });

    clicked
}

/// Attach the derived hint to a product response, if it has extra facts.
fn with_product_hint(response: egui::Response, detail: Option<&str>) -> egui::Response {
    match detail {
        Some(detail) => response.on_hover_text(detail.to_owned()),
        None => response,
    }
}

/// The one right-click menu both catalog views use.
///
/// The loose item is always offered; a pack appears only when one is priced.
/// The click is recorded rather than applied so the caller can drop its borrow
/// of the products before touching the cart.
fn product_context_menu(
    response: &egui::Response,
    product: &Product,
    product_index: usize,
    clicked: &mut Option<(usize, i64)>,
) {
    response.context_menu(|ui| {
        if ui.button("Add loose").clicked() {
            *clicked = Some((product_index, 1));
            ui.close();
        }
        if product.pack_size > 1
            && product.pack_price_minor.is_some()
            && ui
                .button(format!("Add pack of {}", product.pack_size))
                .clicked()
        {
            *clicked = Some((product_index, product.pack_size));
            ui.close();
        }
    });
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

/// Space kept below the line list so the total never scrolls out of sight.
///
/// The tender panel replaces the whole footer, so it needs its own reserve.
/// The optional caption rows (discount, wholesale warning, cash rounding) are
/// added only when they exist, keeping a plain cart exactly as tall as before.
fn footer_reserve(summary: &CartSummary, tendering: bool) -> f32 {
    const CART_FOOTER: f32 = 196.0;
    const TENDER_FOOTER: f32 = 420.0;

    let caption = TYPE.small * 1.8;
    let rounding = if summary.rounding_minor != 0 {
        caption
    } else {
        0.0
    };
    if tendering {
        // An error replaces the hint rather than stacking under it, so no
        // extra reserve is needed here.
        TENDER_FOOTER + rounding
    } else {
        let discount = if summary.discount_minor > 0 {
            caption
        } else {
            0.0
        };
        let warning = if summary.suppressed { caption } else { 0.0 };
        CART_FOOTER + rounding + discount + warning
    }
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
                let mut recall_idx = None;
                let mut discard_idx = None;
                for i in 0..parked_count {
                    let p = &state.parked_carts[i];
                    let text = format!("{} · {} ({} items)", p.id, p.created_at, p.lines.len());
                    ui.horizontal(|ui| {
                        if ui.button(text).clicked() {
                            recall_idx = Some(i);
                        }
                        if ui
                            .button("×")
                            .on_hover_text("Discard parked cart")
                            .clicked()
                        {
                            discard_idx = Some(i);
                        }
                    });
                }
                if let Some(i) = recall_idx {
                    state.recall_cart(i);
                    ui.close();
                } else if let Some(i) = discard_idx {
                    state.discard_parked_cart(i);
                    ui.close();
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
    customer_header(ui, state);

    // One arithmetic pass for the reserve and the footer. The header above can
    // change the basket; the line list below can too, so this is rebuilt after
    // that loop before anything is drawn.
    let mut summary = state.cart_summary();

    // Reserve the footer first so the line list scrolls instead of pushing the
    // total off the pane. A till whose total can scroll away is broken.
    let footer = footer_reserve(&summary, state.payment.is_some());
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
                let mut toggles: Vec<(i64, i64)> = Vec::new();

                for (index, line) in state.cart.iter().enumerate() {
                    let key = (line.product_id, line.unit_factor);
                    let expanded = state.is_cart_line_expanded(key.0, key.1);

                    // One compact row by default: count, product and total are
                    // the three things a cashier checks while scanning. The
                    // unit price and its controls live one tap below, so a
                    // packed basket fits on the till without hiding the money.
                    let row_height = 44.0;
                    let (row_rect, row_resp) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), row_height),
                        egui::Sense::click(),
                    );
                    let row_resp = row_resp
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(if expanded {
                            "Hide line details"
                        } else {
                            "Show line details"
                        });

                    if ui.is_rect_visible(row_rect) {
                        let fill = if expanded {
                            egui::Color32::from(palette.accent_soft)
                        } else if row_resp.hovered() {
                            egui::Color32::from(palette.surface_alt)
                        } else {
                            egui::Color32::TRANSPARENT
                        };
                        if fill != egui::Color32::TRANSPARENT {
                            ui.painter().rect_filled(
                                row_rect,
                                egui::CornerRadius::same(RADII.sm as u8),
                                fill,
                            );
                        }
                        ui.painter().line_segment(
                            [row_rect.left_bottom(), row_rect.right_bottom()],
                            egui::Stroke::new(1.0, egui::Color32::from(palette.border)),
                        );
                    }

                    let mut chevron_clicked = false;
                    let mut row_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(row_rect)
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    );
                    row_ui.spacing_mut().item_spacing.x = SPACING.xs;

                    let (chev_rect, chev_resp) = row_ui
                        .allocate_exact_size(egui::vec2(44.0, row_height), egui::Sense::click());
                    let chev_resp = chev_resp
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(if expanded {
                            "Hide line details"
                        } else {
                            "Show line details"
                        });
                    if chev_resp.clicked() {
                        chevron_clicked = true;
                    }
                    if row_ui.is_rect_visible(chev_rect) {
                        let hovered = chev_resp.hovered();
                        let visual = egui::Rect::from_center_size(
                            chev_rect.center(),
                            egui::vec2(32.0, 32.0),
                        );
                        if hovered {
                            row_ui.painter().rect_filled(
                                visual,
                                egui::CornerRadius::same(RADII.sm as u8),
                                row_ui.visuals().widgets.hovered.weak_bg_fill,
                            );
                        }
                        let chevron_color = if hovered {
                            egui::Color32::from(palette.text)
                        } else {
                            egui::Color32::from(palette.text_muted)
                        };
                        let chevron = if expanded {
                            Icon::CHEVRON_DOWN
                        } else {
                            Icon::CHEVRON_RIGHT
                        };
                        chevron.image(16.0, chevron_color).paint_at(&row_ui, visual);
                    }

                    row_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            components::numeric(state.money(line.total_minor())).size(TYPE.body),
                        );

                        // The name gets the remainder. A long name costs an
                        // ellipsis and a tooltip, never the amount on the right.
                        let width = (ui.available_width() - ui.spacing().item_spacing.x).max(0.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(width, row_height),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                let label = match line.unit_label.as_deref() {
                                    Some(unit) if line.unit_factor > 1 => {
                                        format!("{} × {} · {}", line.qty, line.name, unit)
                                    }
                                    _ => format!("{} × {}", line.qty, line.name),
                                };
                                let name = ui.add(
                                    egui::Label::new(components::semibold(&label).size(TYPE.body))
                                        .truncate(),
                                );
                                if is_cut(ui, &label, width) {
                                    name.on_hover_text(&label);
                                }
                            },
                        );
                    });
                    ui.advance_cursor_after_rect(row_rect);

                    // The row and its chevron are separate hit targets, but a
                    // tap on either asks for the same toggle. Collect it here
                    // and apply after the borrow of `state.cart` ends.
                    if row_resp.clicked() || chevron_clicked {
                        toggles.push(key);
                    }

                    if expanded {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = SPACING.xs;
                            // Align the detail with the name, not the chevron.
                            ui.add_space(44.0 + SPACING.xs);
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
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    // Quick-remove item button (trash). The hit area
                                    // is a full 44-point target; the glyph and its
                                    // hover fill stay 32 so the row keeps density.
                                    let (trash_rect, trash_resp) = ui.allocate_exact_size(
                                        egui::vec2(44.0, 44.0),
                                        egui::Sense::click(),
                                    );
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

                                    // The stepper is minus, count, plus everywhere
                                    // else it appears. `horizontal` inherits the
                                    // parent's direction, so inside this
                                    // right-to-left row it would read plus, count,
                                    // minus — the opposite of every other stepper.
                                    let (step, count) = ui
                                        .with_layout(
                                            egui::Layout::left_to_right(egui::Align::Center),
                                            |ui| components::stepper(ui, line.qty),
                                        )
                                        .inner;
                                    // The count in the middle of the stepper is a
                                    // button. A till is a touch screen, so there is
                                    // no hover to reveal that the quantity is
                                    // editable; tapping it opens the keypad.
                                    if count.on_hover_text("Set quantity").clicked() {
                                        edit = Some(index);
                                    }
                                    if step != components::Step::None {
                                        steps.push((index, step));
                                    }
                                },
                            );
                        });
                    }
                    ui.add_space(SPACING.sm);
                }

                for (product_id, unit_factor) in toggles {
                    state.toggle_cart_line_expansion(product_id, unit_factor);
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
                    state.remove_cart_line(index);
                } else if let Some(index) = edit {
                    state.edit_cart_quantity(index);
                }
            });
    }

    // The lines above can change the basket. Re-read it before the footer so
    // the subtotal, discount, Total, and cash rounding all agree.
    summary = state.cart_summary();

    ui.add_space(SPACING.sm);
    // Paying is not a screen: the footer becomes the tender panel in place.
    // The lines above stay visible, frozen, because the customer may still
    // change their mind about one of them.
    if state.payment.is_some() {
        crate::checkout::tender(ui, state, &summary);
        return;
    }
    // The subtotal is the shelf price of everything in the basket; the total is
    // what the customer actually pays. Showing only the net would hide the
    // discount the cashier just granted, and showing only the gross would
    // disagree with the receipt.
    amount_row(ui, "Subtotal", &state.money(summary.gross_minor), false);
    if summary.discount_minor > 0 {
        let label = format!("Discount ({})", percent_text(summary.discount_bp));
        amount_row(ui, &label, &state.money(-summary.discount_minor), false);
    }
    // Prices are tax-inclusive, so this is the tax already inside the total,
    // not an amount to add on top.
    let tax = if summary.tax_configured {
        state.money(summary.tax_minor)
    } else {
        "not configured".to_owned()
    };
    amount_row(ui, "Tax", &tax, false);
    ui.add_space(SPACING.xs);
    // "Total" is the exact total, the number the receipt prints. The rounded
    // cash amount is the payment screen's, and it says so there: a cart that
    // showed 300 for a 340 basket would hide the 40 the shop is giving away.
    amount_row(ui, "Total", &state.money(summary.total_minor), true);
    if summary.suppressed {
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
    if summary.rounding_minor != 0 {
        let due = state.money(summary.due_minor);
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

/// The short, always-visible price mode on the customer chip.
///
/// The popup has words, but the chip has to read at a glance before the first
/// item is scanned. Uppercase keeps it distinct from the customer's name.
fn customer_header_badge(mode: CustomerMode) -> &'static str {
    match mode {
        CustomerMode::Retail => "RETAIL",
        CustomerMode::Wholesale => "WHOLESALE",
    }
}

/// The name shown on the header chip.
fn customer_header_name(customer: Option<&crate::db::Customer>) -> String {
    customer
        .map(|customer| customer.name.clone())
        .unwrap_or_else(|| "Walk-in".to_owned())
}

/// One header chip for the customer and the price ladder.
///
/// Both choices used to own a full row each, which pushed the first cart line
/// down and made the basket look busier than it is. The badge stays outside the
/// popup: mode changes reprice every line already in the basket, so the cashier
/// must be able to read it without opening anything.
///
/// Choices are deferred until the popup closes because both `set_customer` and
/// `set_customer_mode` need `&mut state`, while the menu body holds `&state`.
fn customer_header(ui: &mut egui::Ui, state: &mut State) {
    let palette = theme::current(ui.ctx());
    let wholesale = state.customer_mode == CustomerMode::Wholesale;
    let badge = customer_header_badge(state.customer_mode);
    let name = customer_header_name(state.customer.as_ref());

    // Badge first: `Button::truncate` keeps the leading atoms and drops the
    // later ones, so a long customer name can never hide the price mode.
    let mut job = egui::text::LayoutJob::default();
    job.append(
        badge,
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::new(TYPE.small, theme::semibold()),
            color: egui::Color32::from(if wholesale {
                palette.accent_ink()
            } else {
                palette.text_muted
            }),
            ..Default::default()
        },
    );
    job.append(
        "  ",
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::new(TYPE.body, egui::FontFamily::Proportional),
            color: egui::Color32::from(palette.text),
            ..Default::default()
        },
    );
    job.append(
        &name,
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::new(TYPE.body, egui::FontFamily::Proportional),
            color: egui::Color32::from(palette.text),
            ..Default::default()
        },
    );

    let width = ui.available_width();
    let button = egui::Button::new(job)
        .truncate()
        .min_size(egui::vec2(width, 44.0))
        .fill(if wholesale {
            egui::Color32::from(palette.accent_soft)
        } else {
            egui::Color32::from(palette.surface_alt)
        })
        .stroke(egui::Stroke::new(1.0_f32, palette.border))
        .corner_radius(egui::CornerRadius::same(RADII.sm as u8))
        .right_text(Icon::CHEVRON_DOWN.image(16.0, egui::Color32::from(palette.text_muted)));

    let mut customer_choice: Option<Option<usize>> = None;
    let mut mode_choice: Option<CustomerMode> = None;

    // While the tender panel owns the screen the basket is frozen, so both
    // choices are too: repricing under a payment already on the counter is not
    // a thing the cashier can mean.
    let enabled = ui.add_enabled_ui(!state.is_tendering(), |ui| {
        egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
            ui.set_min_width(260.0);
            components::caption(ui, "Customer");
            ui.add_space(SPACING.xs);
            if ui
                .selectable_label(state.customer.is_none(), "Walk-in")
                .clicked()
            {
                customer_choice = Some(None);
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
                    customer_choice = Some(Some(index));
                    ui.close();
                }
            }

            ui.add_space(SPACING.sm);
            components::divider(ui);
            ui.add_space(SPACING.sm);
            components::caption(ui, "Price mode");
            for mode in [CustomerMode::Retail, CustomerMode::Wholesale] {
                if ui
                    .selectable_label(state.customer_mode == mode, mode.label())
                    .clicked()
                {
                    if mode != state.customer_mode {
                        mode_choice = Some(mode);
                    }
                    ui.close();
                }
            }
            components::caption(ui, "Wholesale uses the deepest quantity break.");
        })
    });

    enabled.inner.0.on_hover_text(
        "Customer and price mode · wholesale uses each product's deepest quantity break",
    );

    match customer_choice {
        Some(Some(index)) => {
            let customer = state.customers[index].clone();
            state.set_customer(Some(customer));
        }
        Some(None) => state.set_customer(None),
        None => {}
    }

    if let Some(mode) = mode_choice {
        if state.permits(crate::auth::Permission::Discount) {
            state.set_customer_mode(mode);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn bucket(product_id: i64, days_left: Option<i64>, on_hand: i64) -> ExpiryBucket {
        ExpiryBucket {
            product_id,
            name: "Milk".to_owned(),
            unit_label: "bottle".to_owned(),
            expires_on: days_left.map(|_| "2026-11-23".to_owned()),
            on_hand,
            days_left,
        }
    }

    fn summary() -> CartSummary {
        CartSummary {
            count: 1,
            gross_minor: 1000,
            discount_minor: 0,
            discount_bp: 0,
            total_minor: 1000,
            tax_minor: 0,
            tax_configured: false,
            rounding_minor: 0,
            due_minor: 1000,
            suppressed: false,
        }
    }

    #[test]
    fn nearest_expiry_keeps_the_closest_dated_bucket_and_sums_ties() {
        let buckets = vec![
            bucket(1, Some(5), 2),
            bucket(1, Some(3), 4),
            bucket(1, Some(3), 1),
            bucket(1, None, 50),
            bucket(2, Some(9), 7),
        ];
        let nearest = nearest_expiry(&buckets);
        assert_eq!(nearest.get(&1), Some(&(3, 5)));
        assert_eq!(nearest.get(&2), Some(&(9, 7)));
        assert_eq!(nearest.len(), 2);
    }

    #[test]
    fn footer_reserve_only_adds_the_rows_that_exist() {
        let caption = TYPE.small * 1.8;
        assert_eq!(footer_reserve(&summary(), false), 196.0);

        let rounding = CartSummary {
            rounding_minor: 5,
            ..summary()
        };
        assert_eq!(footer_reserve(&rounding, false), 196.0 + caption);
        assert_eq!(footer_reserve(&rounding, true), 420.0 + caption);

        let discount = CartSummary {
            discount_minor: 100,
            ..summary()
        };
        assert_eq!(footer_reserve(&discount, false), 196.0 + caption);

        let suppressed = CartSummary {
            suppressed: true,
            ..summary()
        };
        assert_eq!(footer_reserve(&suppressed, false), 196.0 + caption);
        assert_eq!(footer_reserve(&suppressed, true), 420.0);
    }

    #[test]
    fn customer_header_badge_names_the_active_price_mode() {
        assert_eq!(customer_header_badge(CustomerMode::Retail), "RETAIL");
        assert_eq!(customer_header_badge(CustomerMode::Wholesale), "WHOLESALE");
    }

    #[test]
    fn customer_header_name_uses_customer_or_walk_in() {
        let customer = crate::db::Customer {
            id: 1,
            name: "Mama Njeri".to_owned(),
            phone: None,
            discount_bp: 0,
        };
        assert_eq!(customer_header_name(Some(&customer)), "Mama Njeri");
        assert_eq!(customer_header_name(None), "Walk-in");
    }
}
