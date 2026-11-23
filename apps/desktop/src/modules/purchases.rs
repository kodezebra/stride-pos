//! Purchases: what the shop means to buy, and what came back.
//!
//! A purchase is a list, not a stock movement. Nothing on the shelf changes
//! until the trip is received, so a list can be made at home, adjusted at the
//! market, or abandoned without leaving a trace in the ledger.

use super::*;
// ---------------------------------------------------------------------------
// Purchases
// ---------------------------------------------------------------------------

/// Restocking trips: what the shop means to buy, and what came back.
///
/// A list is not a stock movement. Nothing on the shelf changes until the trip
/// is received, so a list can be made at home, adjusted at the market, or
/// abandoned without leaving a trace in the ledger. Planning and counting in are
/// the same page: the quantity column is the count sheet.
pub(super) struct Purchases;

/// The name of the trip on screen: its supplier, or what kind of trip it is.
pub(super) fn purchase_title(state: &State) -> String {
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

pub(super) fn purchase_header(ui: &mut egui::Ui) {
    components::table_header(ui, "Purchase");
    components::divider(ui);
}

/// One trip: what it is and where it stands, with the one action at the edge.
pub(super) fn purchase_row(
    ui: &mut egui::Ui,
    purchase: &Purchase,
    currency: money::Currency,
) -> Option<i64> {
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
pub(super) fn purchase_matches(purchase: &Purchase, needle: &str) -> bool {
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

pub(super) fn visible_purchases(purchases: &[Purchase], needle: &str, filter: usize) -> Vec<usize> {
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

/// How many products the picker shows before asking the user to search.
const PICKER_LIMIT: usize = 6;

/// The picker owns a pane beside the list, so adding a line never pushes it
/// down. The share and floors follow the till's cart: wide enough to read a
/// product row, narrow enough to leave the cost/qty fields a real width.
const PICKER_PANE_SHARE: f32 = 0.34;
const PICKER_PANE_MIN: f32 = 300.0;
const PICKER_PANE_MAX: f32 = 400.0;
const LINES_PANE_MIN: f32 = 380.0;

/// The read-only lists a purchase form is built from, assembled once per frame.
///
/// The supplier dropdown, the product picker and the expiry hints all ask the
/// same questions of `state`. Doing that work inside the draw closure made a
/// click add a line from a list assembled a few lines earlier in the same
/// frame; this snapshot keeps every part of the form reading one answer.
pub(super) struct PurchaseDerived {
    suppliers: Vec<(String, String)>,
    saved_supplier: Option<String>,
    candidates: Vec<StockItem>,
    perishable: Vec<i64>,
    already: Vec<i64>,
    low_to_add: Vec<i64>,
}

impl PurchaseDerived {
    pub(super) fn suppliers(&self) -> &[(String, String)] {
        &self.suppliers
    }

    pub(super) fn saved_supplier(&self) -> Option<&str> {
        self.saved_supplier.as_deref()
    }

    pub(super) fn candidates(&self) -> &[StockItem] {
        &self.candidates
    }

    pub(super) fn perishable(&self) -> &[i64] {
        &self.perishable
    }

    /// The picker matches follow the live search box, so they are filtered when
    /// the box is drawn rather than when the snapshot is taken. With an empty
    /// box, products not yet on the list come first; search keeps source order
    /// so a typed match reads the same way every time.
    pub(super) fn matches(&self, search: &str) -> Vec<StockItem> {
        let needle = search.trim().to_lowercase();
        let mut matches: Vec<_> = self
            .candidates
            .iter()
            .filter(|item| {
                needle.is_empty()
                    || item.name.to_lowercase().contains(&needle)
                    || item.category.to_lowercase().contains(&needle)
            })
            .cloned()
            .collect();
        if needle.is_empty() {
            // Stable sort: alphabetical order survives inside each group.
            matches.sort_by_key(|item| self.already.contains(&item.product_id));
        }
        matches
    }

    pub(super) fn already_on_list(&self, product_id: i64) -> bool {
        self.already.contains(&product_id)
    }

    /// Whether `Add low stock` would add anything to this list. A product that
    /// is low but already here is not a reason to show the action.
    pub(super) fn has_low_stock_to_add(&self) -> bool {
        !self.low_to_add.is_empty()
    }
}

/// The action on a picker row: a second line reads differently from a first.
pub(super) fn product_picker_action(already_on_list: bool) -> &'static str {
    if already_on_list { "Add again" } else { "Add" }
}

pub(super) fn derive_purchase(state: &State) -> PurchaseDerived {
    derive_purchase_from(
        &state.suppliers,
        &state.stock,
        state.purchase_editor.as_ref(),
    )
}

pub(super) fn derive_purchase_from(
    suppliers: &[Supplier],
    stock: &[StockItem],
    draft: Option<&state::PurchaseDraft>,
) -> PurchaseDerived {
    let active = suppliers
        .iter()
        .filter(|supplier| !supplier.archived)
        .map(|supplier| (supplier.id.clone(), supplier.name.clone()))
        .collect();
    // An old list can point at a supplier that has since been archived. Keep
    // showing that name rather than silently rewriting the trip to "none".
    let saved_supplier = draft
        .and_then(|draft| draft.supplier_id.as_deref())
        .and_then(|id| suppliers.iter().find(|supplier| supplier.id == id))
        .map(|supplier| supplier.name.clone());
    // Keep the whole stock item: the picker draws the same stock pill and
    // tooltip the Catalog page draws for a product, not a name/category tuple.
    let candidates = stock.to_vec();
    // Which lines ask for a date. A product that keeps is not dated; a
    // perishable delivery must be, and the field says so.
    let perishable = stock
        .iter()
        .filter(|item| item.perishable)
        .map(|item| item.product_id)
        .collect();
    let already: Vec<i64> = draft
        .map(|draft| draft.lines.iter().map(|line| line.product_id).collect())
        .unwrap_or_default();
    let low_to_add = stock
        .iter()
        .filter(|item| item.needs_restock() && !already.contains(&item.product_id))
        .map(|item| item.product_id)
        .collect();

    PurchaseDerived {
        suppliers: active,
        saved_supplier,
        candidates,
        perishable,
        already,
        low_to_add,
    }
}

/// Plan a trip: choose a supplier if there is one, list what to buy, and put
/// an expected price beside each line so the total is known before leaving.
///
/// The same page counts the goods in. A quantity is a number of units, and the
/// trip is over when it is marked received, so there is no second screen to
/// learn at the market with one hand full.
pub(super) fn purchase_page(ui: &mut egui::Ui, state: &mut State) {
    let currency = state.currency;
    // The lists a trip is built from do not change while its form is open.
    // Assemble them once, before the draft is borrowed mutably; the picker's
    // matches still follow the live search box, but read this one snapshot.
    let derived = derive_purchase(state);

    let Some(draft) = state.purchase_editor.as_mut() else {
        return;
    };
    let mut delete = false;
    let mut add_low = false;
    let mut add_product = None;
    let mut remove_line = None;

    // The list and the picker own one pane each. Adding a line grows only the
    // left pane's scroll content; the search box and its matches stay exactly
    // where the cashier left them, on a window-height card.
    let gap = SPACING.md;
    let full = ui.available_rect_before_wrap();
    let picker_width = (full.width() * PICKER_PANE_SHARE)
        .clamp(PICKER_PANE_MIN, PICKER_PANE_MAX)
        .min((full.width() - LINES_PANE_MIN - gap).max(PICKER_PANE_MIN));
    let picker_rect =
        egui::Rect::from_min_max(egui::pos2(full.max.x - picker_width, full.min.y), full.max);
    let lines_rect = egui::Rect::from_min_max(
        full.min,
        egui::pos2((picker_rect.min.x - gap).max(full.min.x), full.max.y),
    );

    let mut lines_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(lines_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    components::card(&mut lines_ui, |ui| {
        ui.set_min_width(ui.available_width());

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.md;
            ui.vertical(|ui| {
                components::field_label(ui, "Supplier (optional)");
                ui.add_space(SPACING.xs);
                let selected = derived.saved_supplier().unwrap_or("No supplier").to_owned();
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
                        for (id, name) in derived.suppliers() {
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
                if derived.has_low_stock_to_add()
                    && actions::secondary_compact(ui, "Add low stock").clicked()
                {
                    add_low = true;
                }
            });
        });
        ui.add_space(SPACING.xs);

        // Only this pane scrolls. The total sits with the lines so a long list
        // never steals the picker's height or pushes Save out of the toolbar.
        egui::ScrollArea::vertical()
            .id_salt("pos_purchase_lines")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if draft.lines.is_empty() {
                    components::caption(
                        ui,
                        if derived.has_low_stock_to_add() {
                            "Nothing on the list yet. Add a product on the right, or fill it from low stock."
                        } else {
                            "Nothing on the list yet. Add a product on the right."
                        },
                    );
                } else {
                    components::divider(ui);
                    for (index, line) in draft.lines.iter_mut().enumerate() {
                        // The numbers are a quantity and a unit cost, so the row
                        // says so with the one symbol that reads as arithmetic:
                        // 12 x 2,000. The name keeps whatever is left.
                        let (_, removed) = egui::containers::Sides::new()
                            .height(44.0)
                            .shrink_left()
                            .show(
                                ui,
                                |ui| {
                                    ui.label(components::semibold(&line.name).size(TYPE.body));
                                },
                                |ui| {
                                    let mut removed = false;
                                    if components::icon_button(ui, Icon::TRASH, 44.0)
                                        .on_hover_text("Remove")
                                        .clicked()
                                    {
                                        removed = true;
                                    }
                                    text_field(ui, &mut line.cost, "Cost", 110.0)
                                        .on_hover_text("Cost of one purchase unit");
                                    components::caption(ui, "×");
                                    text_field(ui, &mut line.qty, "Qty", 70.0)
                                        .on_hover_text("How many purchase units");
                                    components::caption(ui, "units/pack");
                                    text_field(ui, &mut line.pack_size, "1", 54.0)
                                        .on_hover_text("Base units in one purchase unit");
                                    removed
                                },
                            );
                        // The date is optional on a plan: the shop may not know it
                        // until the goods are in hand. Receiving asks again for a
                        // perishable product, and a date typed there wins. Two
                        // printed dates on one delivery are two lines.
                        if derived.perishable().contains(&line.product_id) {
                            ui.horizontal(|ui| {
                                components::caption(ui, "Expiry");
                                text_field(ui, &mut line.expires_on, "YYYY-MM-DD", 130.0)
                                    .on_hover_text(
                                        "The date printed on the pack — required when you receive it. \
                                     Mixed dates: add the product twice.",
                                    );
                            });
                        }
                        if removed {
                            remove_line = Some(index);
                        }
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

                if draft.id.is_some() {
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
    });

    let mut picker_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(picker_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    components::card(&mut picker_ui, |ui| {
        ui.set_min_width(ui.available_width());
        components::field_label(ui, "Add a product");
        ui.add_space(SPACING.xs);
        components::search_field(ui, &mut draft.search, "Name or category");

        let needle = draft.search.trim();
        // Matching follows the live search box, so it happens here rather than
        // in the frame snapshot. Both read the same candidate list.
        let matches = derived.matches(&draft.search);

        ui.add_space(SPACING.xs);
        if derived.candidates().is_empty() {
            components::caption(
                ui,
                "No tracked products yet. Track stock first, then plan a purchase.",
            );
        } else if matches.is_empty() {
            components::caption(ui, "No other tracked product matches.");
        } else {
            if needle.is_empty() {
                components::caption(
                    ui,
                    "Tracked products — type to find one by name or category.",
                );
            }
            product_picker_header(ui, "Add");
            egui::ScrollArea::vertical()
                .id_salt("pos_purchase_picker")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for item in matches.iter().take(PICKER_LIMIT) {
                        let already = derived.already_on_list(item.product_id);
                        let detail = if item.category.is_empty() {
                            "No category".to_owned()
                        } else {
                            item.category.clone()
                        };
                        let detail = if already {
                            format!("{detail} · already on list")
                        } else {
                            detail
                        };
                        if product_picker_row(
                            ui,
                            &item.name,
                            &detail,
                            Some(item),
                            product_picker_action(already),
                            false,
                        ) {
                            add_product = Some(item.product_id);
                        }
                    }
                    if matches.len() > PICKER_LIMIT {
                        let more_hint = if needle.is_empty() {
                            format!(
                                "{} more — type a name or category to see them.",
                                matches.len() - PICKER_LIMIT
                            )
                        } else {
                            format!(
                                "{} more — keep typing to narrow it.",
                                matches.len() - PICKER_LIMIT
                            )
                        };
                        components::caption(ui, &more_hint);
                    }
                });
        }

        if let Some(error) = &draft.error {
            ui.add_space(SPACING.sm);
            ui.label(
                egui::RichText::new(error)
                    .size(TYPE.small)
                    .color(ui.visuals().error_fg_color),
            );
        }
    });

    // Claim the whole row so the workspace grows by the pane's real height.
    ui.advance_cursor_after_rect(full);

    if add_low {
        state.purchase_add_low_stock();
    }
    if let Some(product_id) = add_product {
        state.purchase_add_product(product_id);
    }
    if let Some(index) = remove_line {
        state.purchase_remove_line(index);
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
pub(super) fn purchase_estimate(
    lines: &[state::PurchaseLineDraft],
    currency: money::Currency,
) -> (i64, bool) {
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
pub(super) fn purchase_received_page(ui: &mut egui::Ui, state: &mut State) {
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
                    egui::containers::Sides::new()
                        .height(44.0)
                        .shrink_left()
                        .show(
                            ui,
                            |ui| {
                                ui.vertical(|ui| {
                                    ui.label(components::semibold(&line.name).size(TYPE.body));
                                    components::caption(ui, &bought_label(line));
                                    if let Some(date) = line.expires_on.as_deref() {
                                        components::caption(ui, &format!("Expires {date}"));
                                    }
                                });
                            },
                            |ui| {
                                if let Some(cost) = line.unit_cost_minor {
                                    components::caption(
                                        ui,
                                        &format!("{} each", Money::new(cost, currency).format()),
                                    );
                                }
                            },
                        );
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
pub(super) fn bought_label(line: &crate::db::PurchaseLine) -> String {
    if line.qty_received == 0 {
        "not bought".to_owned()
    } else if line.qty_received < line.qty_ordered {
        format!("{} of {} bought", line.qty_received, line.qty_ordered)
    } else {
        format!("{} bought", line.qty_received)
    }
}
impl Module for Purchases {
    /// The editor owns a two-column frame with its own pane scrolls, the
    /// received view owns its table scroll, and the list wraps itself below.
    /// A page-level scroll would make the picker's height unbounded again.
    fn scrolls(&self) -> bool {
        false
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, state: &mut State) {
        components::section_title(ui, "Buying");
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
            // Receiving writes stock and cannot be undone, so the button asks
            // twice: the first press turns it into a confirmation.
            let receive_label = match state.purchase_editor.as_ref() {
                Some(draft) if draft.confirm_receive => "Confirm received",
                _ => "Received",
            };
            let mut back = false;
            let mut save = false;
            let mut receive = false;
            action_bar(ui, PageLeft::Title(&title), |ui| {
                if editing && can_manage {
                    if actions::primary_compact(ui, receive_label).clicked() {
                        receive = true;
                    }
                    if actions::secondary_compact(ui, "Save").clicked() {
                        save = true;
                    }
                }
                if actions::quiet_compact(ui, "Back").clicked() {
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

        if !can_manage {
            return;
        }

        let mut new = false;
        let mut low = false;
        let any_low = state.stock.iter().any(|item| item.needs_restock());
        action_bar(
            ui,
            PageLeft::Search {
                text: &mut state.purchase_search,
                hint: "Supplier or note",
                count: None,
            },
            |ui| {
                if actions::primary_compact(ui, "New list").clicked() {
                    new = true;
                }
                if any_low && actions::secondary_compact(ui, "Add low stock").clicked() {
                    low = true;
                }
            },
        );
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

        let mut open = None;
        egui::ScrollArea::vertical()
            .id_salt("pos_purchase_list")
            .auto_shrink([false, false])
            .show(ui, |ui| {
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
                ui.add_space(SPACING.xl);
            });

        if let Some(id) = open {
            state.edit_purchase(id);
        }
    }
}
