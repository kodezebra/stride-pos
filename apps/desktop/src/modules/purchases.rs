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

/// Plan a trip: choose a supplier if there is one, list what to buy, and put
/// an expected price beside each line so the total is known before leaving.
///
/// The same page counts the goods in. A quantity is a number of units, and the
/// trip is over when it is marked received, so there is no second screen to
/// learn at the market with one hand full.
pub(super) fn purchase_page(ui: &mut egui::Ui, state: &mut State) {
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
    // Which lines ask for a date. A product that keeps is not dated; a
    // perishable delivery must be, and the field says so.
    let perishable: Vec<i64> = state
        .stock
        .iter()
        .filter(|item| item.perishable)
        .map(|item| item.product_id)
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
                    if perishable.contains(&line.product_id) {
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

            ui.add_space(SPACING.lg);
            components::field_label(ui, "Add a product");
            ui.add_space(SPACING.xs);
            components::search_field(ui, &mut draft.search, "Name or category");

            let needle = draft.search.trim().to_lowercase();
            // A product already on the list stays findable: a delivery that
            // arrives with two printed dates is two lines, and the plan needs
            // a way to add the second one.
            let already: Vec<i64> = draft.lines.iter().map(|line| line.product_id).collect();
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
            if candidates.is_empty() {
                components::caption(
                    ui,
                    "No tracked products yet. Track stock first, then plan a purchase.",
                );
            } else if needle.is_empty() {
                // The catalog is long and the page is short. A list that only
                // appears once there is something to match keeps the whole
                // list — and the delete route — on one screen.
                components::caption(ui, "Type a name or category to add it.");
            } else if matches.is_empty() {
                components::caption(ui, "No other tracked product matches.");
            } else {
                for (id, name, category) in matches.iter().take(6) {
                    let label = if category.is_empty() {
                        name.clone()
                    } else {
                        format!("{name} — {category}")
                    };
                    let label = if already.contains(id) {
                        format!("{label} · another line")
                    } else {
                        label
                    };
                    if ui.selectable_label(false, label).clicked() {
                        add_product = Some(*id);
                    }
                }
                if matches.len() > 6 {
                    components::caption(
                        ui,
                        &format!("{} more — keep typing to narrow it.", matches.len() - 6),
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
        });

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
