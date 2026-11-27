//! Suppliers: who the shop buys from, and the deliveries that came in.
//!
//! The list is the shopkeeper's own address book, not a ledger. A supplier is
//! only worth a row because a purchase can name them, and a delivery is only
//! worth recording because it moves stock.

use super::*;

/// Short label for the collapsed supplier details section.
///
/// A directory entry is often half-known: the rep's name and phone arrive
/// before the email or the address. The heading says which of those optional
/// fields already have something in them, so folding the form away never hides
/// a detail the operator thought they had saved.
pub(super) fn supplier_detail_summary(draft: &state::SupplierDraft) -> String {
    let filled: Vec<&str> = [
        ("Contact", draft.contact_name.trim()),
        ("Phone", draft.phone.trim()),
        ("Email", draft.email.trim()),
        ("Address", draft.address.trim()),
        ("Notes", draft.notes.trim()),
    ]
    .into_iter()
    .filter(|(_, value)| !value.is_empty())
    .map(|(label, _)| label)
    .collect();

    match filled.len() {
        0 => "Nothing filled yet".to_owned(),
        1 | 2 => filled.join(", "),
        count => format!("{}, {} +{} more", filled[0], filled[1], count - 2),
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

        // Only the name stays on the first screen. The rest is a list a shop
        // fills in as it learns it, so it folds away behind a heading that
        // says what is already there.
        ui.add_space(SPACING.md);
        let detail_heading = components::semibold(format!(
            "Contact & delivery details · {}",
            supplier_detail_summary(draft)
        ))
        .size(TYPE.body);
        egui::CollapsingHeader::new(detail_heading)
            .id_salt("pos_supplier_details")
            .show(ui, |ui| {
                ui.add_space(SPACING.xs);
                // Contact and phone share a row because the modal has the
                // width for it; they are the two things a shop knows first
                // about a rep.
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
            });

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

// ---------------------------------------------------------------------------
// Suppliers
// ---------------------------------------------------------------------------

pub(super) struct Suppliers;

pub(super) enum SupplierAction {
    Edit(String),
    AskArchive(String),
    Archive(String),
    Restore(String),
    CancelArchive,
}

pub(super) fn supplier_header(ui: &mut egui::Ui) {
    components::table_header(ui, "Supplier");
    components::divider(ui);
}

/// One supplier: name and whatever contact details exist, actions on the right.
///
/// The row returns what was clicked instead of mutating the state, because the
/// caller is iterating `state.suppliers` and cannot borrow it mutably yet.
pub(super) fn supplier_row(
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
                if components::icon_button(ui, Icon::CHECK, 44.0)
                    .on_hover_text("Archive")
                    .clicked()
                {
                    action = Some(SupplierAction::Archive(supplier.id.clone()));
                }
                if components::icon_button(ui, Icon::CLOSE, 44.0)
                    .on_hover_text("Cancel")
                    .clicked()
                {
                    action = Some(SupplierAction::CancelArchive);
                }
                components::caption(ui, "Archive this supplier?");
            } else {
                if supplier.archived {
                    if components::icon_button(ui, restore_icon(), 44.0)
                        .on_hover_text("Restore")
                        .clicked()
                    {
                        action = Some(SupplierAction::Restore(supplier.id.clone()));
                    }
                } else if components::icon_button(ui, archive_icon(), 44.0)
                    .on_hover_text("Archive")
                    .clicked()
                {
                    action = Some(SupplierAction::AskArchive(supplier.id.clone()));
                }
                if components::icon_button(ui, edit_icon(), 44.0)
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
pub(super) fn supplier_detail(supplier: &Supplier) -> String {
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
pub(super) fn supplier_matches(supplier: &Supplier, needle: &str) -> bool {
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
pub(super) fn visible_suppliers(
    suppliers: &[Supplier],
    needle: &str,
    show_archived: bool,
) -> Vec<usize> {
    suppliers
        .iter()
        .enumerate()
        .filter(|(_, supplier)| {
            (show_archived || !supplier.archived) && supplier_matches(supplier, needle)
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
            components::card(ui, |ui| {
                ui.set_min_width(ui.available_width());
                product_picker_header(ui, "");
                egui::ScrollArea::vertical()
                    .max_height(180.0)
                    .show(ui, |ui| {
                        for (id, name, category) in &matches {
                            let detail = if category.is_empty() {
                                "No category"
                            } else {
                                category.as_str()
                            };
                            if product_picker_row(
                                ui,
                                name,
                                detail,
                                None,
                                "",
                                draft.product_id == Some(*id),
                            ) {
                                draft.product_id = Some(*id);
                            }
                        }
                    });
                if matches.is_empty() {
                    components::caption(ui, "No untracked product matches.");
                }
            });
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
    let currency = state.currency;
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
            for (kind, cause, label) in [
                (StockKind::Receive, None, "Received"),
                (StockKind::Return, None, "Customer return"),
                (StockKind::Waste, Some(WasteCause::Damage), "Damage"),
                (StockKind::Waste, Some(WasteCause::Spoilage), "Spoilage"),
                (StockKind::Waste, Some(WasteCause::Loss), "Loss"),
                (StockKind::Adjust, None, "Counted"),
            ] {
                let selected = draft.kind == kind && draft.cause == cause;
                if ui.selectable_label(selected, label).clicked() {
                    draft.kind = kind;
                    draft.cause = cause;
                }
            }
        });
        components::caption(ui, movement_hint(draft.kind));

        ui.add_space(SPACING.md);
        components::field_label(ui, movement_quantity_label(draft.kind));
        ui.add_space(SPACING.xs);
        text_field(
            ui,
            &mut draft.quantity,
            movement_quantity_label(draft.kind),
            160.0,
        );

        // Square shows Price, Unit Cost and Profit Margin together at the
        // point of receiving. Here the price is already on the product, so
        // typing the cost is enough to show the margin — no trip to a report.
        if draft.kind == StockKind::Receive {
            ui.add_space(SPACING.md);
            components::field_label(ui, &format!("Unit cost ({}, optional)", currency.code()));
            ui.add_space(SPACING.xs);
            text_field(ui, &mut draft.cost, "0", 130.0)
                .on_hover_text("What one unit cost you on this delivery.");
            if let Ok(cost) = money::Money::parse(draft.cost.trim(), currency)
                && cost.minor() >= 0
                && draft.selling_price_minor > 0
            {
                let price = draft.selling_price_minor;
                let margin = ((price - cost.minor()) * 100 + price / 2) / price;
                components::caption(
                    ui,
                    &format!(
                        "Selling at {} · {margin}% margin at that cost.",
                        money::Money::new(price, currency).format()
                    ),
                );
            } else {
                components::caption(ui, "Used for margin and profit reports.");
            }
        }

        // A date only exists on stock that arrived or left on a specific
        // batch. An adjustment is a correction to a count, not a delivery.
        if draft.kind != StockKind::Adjust {
            let required = draft.kind == StockKind::Receive && draft.perishable;
            ui.add_space(SPACING.md);
            components::field_label(
                ui,
                if required {
                    "Expiry date (required)"
                } else {
                    "Expiry date (optional)"
                },
            );
            ui.add_space(SPACING.xs);
            text_field(ui, &mut draft.expires_on, "YYYY-MM-DD", 170.0);
            if draft.kind == StockKind::Receive
                && let Some(days) = draft.shelf_life_days
            {
                components::caption(
                    ui,
                    &format!("Usually keeps {days} days. The date printed on the pack wins."),
                );
            }
            if draft.kind == StockKind::Waste {
                components::caption(
                    ui,
                    "Date it when pulling one dated batch. Leave it blank to take the \
                     quantity off the oldest date first.",
                );
            }
        }

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

pub(super) fn movement_hint(kind: StockKind) -> &'static str {
    match kind {
        StockKind::Receive => "Goods arriving from a supplier add to the shelf.",
        StockKind::Return => "Stock a customer brought back adds to the shelf.",
        StockKind::Waste => "Stock that was damaged, spoiled or lost leaves the shelf.",
        StockKind::Adjust => "Enter what you counted. The till records the difference.",
        StockKind::Opening | StockKind::Sale => "",
    }
}

pub(super) fn movement_quantity_label(kind: StockKind) -> &'static str {
    match kind {
        StockKind::Adjust => "Counted quantity",
        StockKind::Receive => "Number received",
        _ => "Quantity",
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

pub(super) fn movement_row(ui: &mut egui::Ui, movement: &crate::db::StockMovement) {
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
            ui.label(components::semibold(movement.reason_label()).size(TYPE.body));
            let mut detail = movement.created_at.clone();
            if let Some(user) = &movement.user {
                detail.push_str(" · ");
                detail.push_str(user);
            }
            if let Some(note) = &movement.note {
                detail.push_str(" · ");
                detail.push_str(note);
            }
            if let Some(date) = movement.expires_on.as_deref() {
                detail.push_str(" · dated ");
                detail.push_str(date);
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
/// A single count over the shelf, with a variance review before anything is
/// written. Cycle counts, not full counts: an un counted row is untouched.
pub fn count_modal(ctx: &egui::Context, state: &mut State) {
    if state.count.is_none() {
        return;
    }
    let stage = state.count.as_ref().expect("checked above").stage;
    let categories = state.categories.clone();
    let category = state
        .count
        .as_ref()
        .and_then(|draft| draft.category.clone())
        .unwrap_or_else(|| "All products".to_owned());
    let change_count = state.count_deltas().len();
    let mut new_category: Option<Option<String>> = None;
    let mut review = false;
    let mut back = false;
    let mut confirm = false;
    let mut cancel = false;

    modal(ctx, egui::Id::new("pos_stock_count")).show(ctx, |ui| {
        ui.set_width(640.0);
        ui.label(components::semibold("Count stock").size(TYPE.heading));
        components::caption(
            ui,
            "Count what is on the shelf. Leave a row blank and the till keeps its number.",
        );

        let Some(draft) = state.count.as_mut() else {
            return;
        };

        if stage == crate::state::CountStage::Counting {
            ui.add_space(SPACING.md);
            ui.horizontal(|ui| {
                components::field_label(ui, "Category");
                egui::ComboBox::from_id_salt("pos_count_category")
                    .width(200.0)
                    .selected_text(&category)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(draft.category.is_none(), "All products")
                            .clicked()
                        {
                            new_category = Some(None);
                            ui.close();
                        }
                        for shelf in &categories {
                            let selected = draft.category.as_deref() == Some(shelf.name.as_str());
                            if ui.selectable_label(selected, &shelf.name).clicked() {
                                new_category = Some(Some(shelf.name.clone()));
                                ui.close();
                            }
                        }
                    });
            });

            ui.add_space(SPACING.sm);
            egui::ScrollArea::vertical()
                .max_height(320.0)
                .show(ui, |ui| {
                    if draft.rows.is_empty() {
                        components::caption(ui, "No tracked products in this category.");
                    }
                    for row in &mut draft.rows {
                        let name = if row.unit_label == "unit" {
                            row.name.clone()
                        } else {
                            format!("{} ({})", row.name, row.unit_label)
                        };
                        ui.horizontal(|ui| {
                            ui.set_min_height(34.0);
                            ui.label(components::semibold(&name).size(TYPE.body));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    text_field(ui, &mut row.counted, "—", 90.0)
                                        .on_hover_text("What you counted on the shelf");
                                    components::caption(ui, &format!("{} on hand", row.on_hand));
                                },
                            );
                        });
                        components::divider(ui);
                    }
                });
        } else {
            ui.add_space(SPACING.md);
            let changes: Vec<_> = draft
                .rows
                .iter()
                .filter_map(|row| {
                    let counted = row.counted.trim().parse::<i64>().ok()?;
                    let delta = counted - row.on_hand;
                    (delta != 0).then_some((row, counted, delta))
                })
                .collect();
            components::caption(ui, "Review the differences before they are written.");
            egui::ScrollArea::vertical()
                .max_height(320.0)
                .show(ui, |ui| {
                    for (row, counted, delta) in changes {
                        ui.horizontal(|ui| {
                            ui.set_min_height(34.0);
                            ui.label(components::semibold(&row.name).size(TYPE.body));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let sign = if delta > 0 { "+" } else { "" };
                                    components::caption(
                                        ui,
                                        &format!("{} → {counted} ({sign}{delta})", row.on_hand),
                                    );
                                },
                            );
                        });
                        components::divider(ui);
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

        actions::footer(ui, |ui| {
            if stage == crate::state::CountStage::Counting {
                if actions::primary(ui, "Review counts").clicked() {
                    review = true;
                }
            } else {
                if actions::primary(ui, &format!("Confirm {change_count} changes")).clicked() {
                    confirm = true;
                }
                if actions::secondary(ui, "Back").clicked() {
                    back = true;
                }
            }
            if actions::secondary(ui, "Cancel").clicked() {
                cancel = true;
            }
        });
    });

    if let Some(shelf) = new_category {
        state.recount_category(shelf);
    }
    if review {
        state.review_count();
    }
    if back && let Some(draft) = state.count.as_mut() {
        draft.stage = crate::state::CountStage::Counting;
        draft.error = None;
    }
    if confirm {
        state.confirm_count();
    }
    if cancel {
        state.count = None;
    }
}

/// The shop-wide ledger with reason and date filters. Built as a modal rather
/// than another tab: it answers one question and leaves the catalog in place.
pub fn global_history_modal(ctx: &egui::Context, state: &mut State) {
    if state.global_history.is_none() {
        return;
    }
    let mut close = false;
    let mut days = None;
    let mut reason = None;

    modal(ctx, egui::Id::new("pos_global_history")).show(ctx, |ui| {
        ui.set_width(680.0);
        ui.label(components::semibold("Stock history").size(TYPE.heading));
        components::caption(
            ui,
            "Every change to stock, newest first. Sales appear as negative.",
        );

        let Some(draft) = state.global_history.as_mut() else {
            return;
        };
        ui.add_space(SPACING.md);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.md;
            ui.vertical(|ui| {
                components::field_label(ui, "When");
                egui::ComboBox::from_id_salt("pos_history_days")
                    .width(140.0)
                    .selected_text(["All time", "Last 7 days", "Last 30 days"][draft.days.min(2)])
                    .show_ui(ui, |ui| {
                        for (index, label) in ["All time", "Last 7 days", "Last 30 days"]
                            .into_iter()
                            .enumerate()
                        {
                            if ui.selectable_label(draft.days == index, label).clicked() {
                                days = Some(index);
                                ui.close();
                            }
                        }
                    });
            });
            ui.vertical(|ui| {
                components::field_label(ui, "Reason");
                let labels = reason_labels();
                egui::ComboBox::from_id_salt("pos_history_reason")
                    .width(160.0)
                    .selected_text(labels[draft.reason.min(labels.len() - 1)].to_owned())
                    .show_ui(ui, |ui| {
                        for (index, label) in labels.iter().enumerate() {
                            if ui.selectable_label(draft.reason == index, *label).clicked() {
                                reason = Some(index);
                                ui.close();
                            }
                        }
                    });
            });
        });

        ui.add_space(SPACING.sm);
        let (added, removed) = draft
            .movements
            .iter()
            .fold((0_i64, 0_i64), |(up, down), row| {
                if row.delta > 0 {
                    (up + row.delta, down)
                } else {
                    (up, down + row.delta)
                }
            });
        components::caption(ui, &format!("{added} in · {removed} out on screen."));

        egui::ScrollArea::vertical()
            .max_height(360.0)
            .show(ui, |ui| {
                if draft.movements.is_empty() {
                    components::caption(ui, "Nothing matches these filters.");
                }
                for movement in &draft.movements {
                    ui.horizontal(|ui| {
                        ui.set_min_height(34.0);
                        ui.label(components::semibold(&movement.product_name).size(TYPE.body));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            components::caption(ui, &movement.created_at);
                            let sign = if movement.delta > 0 { "+" } else { "" };
                            components::caption(ui, &format!("{sign}{}", movement.delta));
                            components::caption(ui, movement.reason_label());
                        });
                    });
                    if let Some(note) = &movement.note {
                        components::caption(ui, note);
                    }
                    components::divider(ui);
                }
            });

        actions::footer(ui, |ui| {
            if actions::secondary(ui, "Close").clicked() {
                close = true;
            }
        });
    });

    if let Some(value) = days {
        if let Some(draft) = state.global_history.as_mut() {
            draft.days = value;
        }
        state.reload_global_history();
    }
    if let Some(value) = reason {
        if let Some(draft) = state.global_history.as_mut() {
            draft.reason = value;
        }
        state.reload_global_history();
    }
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) || close {
        state.global_history = None;
    }
}

/// The reason choices for the global ledger, with "all reasons" at zero.
fn reason_labels() -> Vec<&'static str> {
    let mut labels = vec!["All reasons"];
    labels.extend(
        crate::db::StockKind::ALL
            .into_iter()
            .map(|kind| kind.label()),
    );
    labels.extend(
        crate::db::WasteCause::ALL
            .into_iter()
            .map(|cause| cause.label()),
    );
    labels
}

impl Module for Suppliers {
    fn sidebar(&mut self, ui: &mut egui::Ui, state: &mut State) {
        components::section_title(ui, "Directory");
        ui.checkbox(&mut state.show_archived_suppliers, "Show archived");
        components::caption(
            ui,
            "Archived suppliers leave the working list but stay for the record.",
        );
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, state: &mut State) {
        // The workspace is a locked card without the permission, so the page has
        // no bar at all rather than a search field over a card that cannot use it.
        if !state.permits(crate::auth::Permission::ManageSuppliers) {
            return;
        }
        let mut new = false;
        action_bar(
            ui,
            PageLeft::Search {
                text: &mut state.supplier_search,
                hint: "Name, contact or phone",
                count: None,
            },
            |ui| {
                if actions::primary_compact(ui, "New supplier").clicked() {
                    new = true;
                }
            },
        );
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
