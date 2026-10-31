//! The payment screen: cash in, change out, one sale written.
//!
//! A modal, like the product form, because paying is a single act that must not
//! compete with the grid behind it. The cashier is counting notes; a mis-click on
//! a product tile at that moment is a sale that did not happen.
//!
//! Two states, one window. Before the sale: what is due, what the customer
//! handed over, what comes back. After: the change, big, and the one button that
//! starts the next sale. The switch is [`crate::state::Payment::done`] rather
//! than a second modal, so the confirmation never has to be found somewhere else.
//!
//! Cash only. The `payments` table already has a `method` column and mobile money
//! is a new row rather than a migration, but a method picker with one method in
//! it is a choice the cashier has to make for no reason.

use money::Money;
use ui_kit::components;
use ui_kit::theme;
use ui_kit::tokens::{SPACING, TYPE};
use ui_shell::egui;

use crate::state::{self, State};

/// Draw the payment modal, if one is open.
///
/// Called from `shell` after the frame, like the product form, so the backdrop
/// covers the cart and the Pay button cannot be pressed twice.
pub fn payment_modal(ctx: &egui::Context, state: &mut State) {
    if state.payment.is_none() {
        return;
    }

    // Read the amounts before borrowing the payment: they come from the cart,
    // which the payment does not own, and `Money::format` is the one rendering.
    let currency = state.currency;
    let subtotal_minor = state.cart_total_minor();
    let rounding_minor = state.cart_rounding_minor();
    let due_minor = state.due_minor();
    let subtotal = state.money(subtotal_minor);
    let rounding = state.money(rounding_minor);
    let due = state.money(due_minor);
    let exact = state::plain_amount(due_minor, currency);

    // A finished sale is a different screen and needs no mutable payment.
    if let Some(sale) = state
        .payment
        .as_ref()
        .and_then(|payment| payment.done.clone())
    {
        let change = state.money(sale.change_minor);
        let tendered = state.money(sale.tendered_minor);
        let paid = state.money(sale.paid_minor());
        let mut close = false;

        egui::Modal::new(egui::Id::new("pos_payment")).show(ctx, |ui| {
            ui.set_width(420.0);
            let palette = theme::current(ui.ctx());

            ui.label(
                components::semibold("Paid")
                    .size(TYPE.heading)
                    .color(egui::Color32::from(palette.success)),
            );
            components::caption(ui, &format!("Sale #{}", sale.id));

            ui.add_space(SPACING.md);
            change_row(ui, &change, egui::Color32::from(palette.success));
            ui.add_space(SPACING.md);
            components::receipt_line(ui, "Cash", &tendered);
            if sale.rounding_minor != 0 {
                components::receipt_line(
                    ui,
                    "Rounding",
                    &Money::new(sale.rounding_minor, currency).format(),
                );
            }
            components::receipt_line(ui, "Total", &paid);

            ui.add_space(SPACING.md);
            if components::primary_button(ui, "New sale").clicked() {
                close = true;
            }
        });

        if close || ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            state.close_payment();
        }
        return;
    }

    let Some(payment) = state.payment.as_mut() else {
        return;
    };

    // The frame that opened the modal still has the keystroke that opened it —
    // Ctrl+Enter — in its input, and Enter here means "complete the sale". A
    // guard on the focus flag keeps that keystroke from selling the basket the
    // cashier only meant to start paying.
    let opening = payment.focus;
    let mut complete = false;
    let mut close = false;

    egui::Modal::new(egui::Id::new("pos_payment")).show(ctx, |ui| {
        ui.set_width(420.0);
        let palette = theme::current(ui.ctx());

        ui.label(components::semibold("Take payment").size(TYPE.heading));
        components::caption(
            ui,
            "The exact total is kept on the sale. Only the cash changes hands in whole coins.",
        );

        ui.add_space(SPACING.md);
        components::receipt_line(ui, "Subtotal", &subtotal);
        if rounding_minor != 0 {
            components::receipt_line(ui, &format!("Rounding ({})", currency.code()), &rounding);
        }
        components::divider(ui);
        crate::modules::amount_row(ui, "To pay", &due, true);

        ui.add_space(SPACING.md);
        components::field_label(ui, "Cash received");
        ui.add_space(SPACING.xs);

        let mut edited = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.sm;

            let field =
                crate::modules::text_field(ui, &mut payment.tendered, "Exact amount", 240.0);
            if payment.focus {
                field.request_focus();
                payment.focus = false;
            }
            if field.changed() {
                edited = true;
            }

            if ui
                .button("Exact")
                .on_hover_text("Fill in the amount due")
                .clicked()
            {
                payment.tendered = exact.clone();
                edited = true;
            }
        });

        // An error about the previous attempt is not about the new one.
        if edited {
            payment.error = None;
        }

        // The change updates as the cashier types: it is the number they read
        // out, and waiting for Complete to show it invites a second count.
        let tendered = if payment.tendered.trim().is_empty() {
            Some(due_minor)
        } else {
            Money::parse(payment.tendered.trim(), currency)
                .ok()
                .map(|amount| amount.minor())
        };

        ui.add_space(SPACING.sm);
        match tendered {
            Some(tendered) if tendered < due_minor => {
                ui.label(
                    egui::RichText::new(format!(
                        "Short by {}",
                        Money::new(due_minor - tendered, currency).format()
                    ))
                    .size(TYPE.small)
                    .color(egui::Color32::from(palette.danger)),
                );
            }
            Some(tendered) => change_row(
                ui,
                &Money::new(tendered - due_minor, currency).format(),
                egui::Color32::from(palette.success),
            ),
            // Mid-typing text that is not yet an amount is not an error.
            None => {
                ui.label(egui::RichText::new(" ").size(TYPE.small));
            }
        }

        if let Some(error) = &payment.error {
            ui.add_space(SPACING.xs);
            ui.label(
                egui::RichText::new(error)
                    .size(TYPE.small)
                    .color(ui.visuals().error_fg_color),
            );
        }

        ui.add_space(SPACING.md);
        ui.horizontal(|ui| {
            if components::primary_button(ui, "Complete sale").clicked() {
                complete = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
        components::caption(
            ui,
            "Enter completes the sale, Escape goes back to the cart.",
        );
    });

    if !opening && ctx.input(|input| input.key_pressed(egui::Key::Enter)) {
        complete = true;
    }
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        close = true;
    }

    if complete {
        state.confirm_payment();
    }
    if close {
        state.close_payment();
    }
}

/// The number the cashier hands back, at the size of the total.
///
/// Same geometry as [`crate::modules::amount_row`], different colour: change is
/// the one amount on the screen that is an instruction rather than a fact.
fn change_row(ui: &mut egui::Ui, amount: &str, color: egui::Color32) {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), TYPE.total * 1.4),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            ui.label(components::numeric(amount).size(TYPE.total).color(color));
            ui.label(components::semibold("Change").size(TYPE.body));
        },
    );
}
