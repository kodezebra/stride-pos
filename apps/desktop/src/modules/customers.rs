//! Customers: the shop's regulars and the rate each one buys at.
//!
//! This is a Shop page, not a corner of Setup: choosing a regular changes what
//! the till charges, so it sits next to the catalog and the suppliers rather
//! than behind the settings rail. The card stays inline — the list is short,
//! and the row that opens the form is where the operator is already looking.

use super::*;

/// The regular customers and the rate each one buys at.
fn customers_card(ui: &mut egui::Ui, state: &mut State) {
    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        components::caption(
            ui,
            "A regular customer buys at a rate you set here. Pick one on the Sell \
             screen and the rate comes off the basket; wholesale prices already \
             give the shop's floor, so the rate is not applied on top of them.",
        );

        ui.add_space(SPACING.md);
        if state.customers.is_empty() {
            components::caption(ui, "No regular customers yet.");
        }

        let mut edit = None;
        let mut archive = None;
        for customer in &state.customers {
            ui.horizontal(|ui| {
                ui.label(components::semibold(&customer.name).size(TYPE.body));
                if customer.discount_bp > 0 {
                    components::caption(ui, &percent_text(customer.discount_bp));
                } else {
                    components::caption(ui, "no discount");
                }
                if let Some(phone) = customer.phone.as_deref().filter(|p| !p.trim().is_empty()) {
                    components::caption(ui, phone);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if actions::secondary_compact(ui, "Archive").clicked() {
                        archive = Some(customer.id);
                    }
                    if actions::secondary_compact(ui, "Edit").clicked() {
                        edit = Some(customer.clone());
                    }
                });
            });
            ui.add_space(SPACING.xs);
        }

        if let Some(id) = archive {
            state.archive_customer(id);
        }
        if let Some(customer) = edit {
            state.edit_customer(&customer);
        }

        ui.add_space(SPACING.md);
        if state.customer_editor.is_some() {
            customer_form(ui, state);
        } else if actions::secondary(ui, "Add customer").clicked() {
            state.start_customer();
        }
    });
}

/// The add/edit form, drawn inside the customer card.
///
/// The buttons are collected as flags and applied after the draft borrow ends:
/// saving needs the whole state, and the draft lives in it.
fn customer_form(ui: &mut egui::Ui, state: &mut State) {
    let mut save = false;
    let mut cancel = false;

    if let Some(draft) = state.customer_editor.as_mut() {
        let heading = if draft.id.is_some() {
            "Edit customer"
        } else {
            "New customer"
        };
        components::section_title(ui, heading);
        ui.add_space(SPACING.xs);

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.sm;
            components::field_label(ui, "Name");
            text_field(ui, &mut draft.name, "e.g. Mama Njeri", 220.0);
            ui.add_space(SPACING.md);
            components::field_label(ui, "Phone");
            text_field(ui, &mut draft.phone, "optional", 160.0);
        });

        ui.add_space(SPACING.sm);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.sm;
            components::field_label(ui, "Discount");
            text_field(ui, &mut draft.discount, "0", 90.0);
            components::caption(ui, "% off every basket");
        });

        if let Some(error) = &draft.error {
            ui.add_space(SPACING.sm);
            ui.label(
                egui::RichText::new(error)
                    .size(TYPE.small)
                    .color(ui.visuals().error_fg_color),
            );
        }

        ui.add_space(SPACING.sm);
        actions::end(ui, |ui| {
            if actions::primary(ui, "Save").clicked() {
                save = true;
            }
            if actions::secondary(ui, "Cancel").clicked() {
                cancel = true;
            }
        });
    }

    if save {
        state.save_customer();
    }
    if cancel {
        state.customer_editor = None;
    }
}

pub(super) struct Customers;

impl Module for Customers {
    fn sidebar(&mut self, ui: &mut egui::Ui, _state: &mut State) {
        components::section_title(ui, "Regulars");
        components::caption(
            ui,
            "A regular's rate follows them onto the Sell screen. Wholesale prices \
             stay the floor.",
        );
    }

    fn workspace(&mut self, ui: &mut egui::Ui, state: &mut State) {
        if !state.permits(crate::auth::Permission::EditSettings) {
            return locked_card(ui, crate::auth::Permission::EditSettings);
        }
        customers_card(ui, state);
    }
}
