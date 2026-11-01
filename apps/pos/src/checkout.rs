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

    // A finished sale is a different screen and needs no mutable payment.
    if let Some(payment) = state.payment.as_mut()
        && let Some(sale) = payment.done.clone()
    {
        let just_finished = payment.focus;
        payment.focus = false;
        // Cloned out of the borrow: the receipt renderer needs the state's
        // currency, and holding `payment` open for the whole closure would
        // fight it. A receipt is a handful of lines, not a table.
        let receipt = payment.detail.clone();

        let change = state.money(sale.change_minor);
        let mut close = false;

        crate::modules::modal(ctx, egui::Id::new("pos_payment")).show(ctx, |ui| {
            ui.set_width(420.0);
            let palette = theme::current(ui.ctx());
            let success = egui::Color32::from(palette.success);

            ui.label(
                components::semibold("Paid")
                    .size(TYPE.heading)
                    .color(success),
            );
            components::caption(ui, &format!("Sale #{}", sale.id));

            ui.add_space(SPACING.md);
            // The change stays the big figure on this screen: it is the number
            // the cashier acts on. The receipt below is what the customer
            // keeps, and it repeats the change because paper has to.
            change_row(ui, &change, success);
            ui.add_space(SPACING.md);

            if let Some(detail) = receipt.as_ref() {
                // A twenty-line sale scrolls inside the modal rather than
                // growing it past the bottom of the window.
                egui::ScrollArea::vertical()
                    .max_height(420.0)
                    .show(ui, |ui| {
                        crate::receipt::body(ui, detail, currency, false);
                    });
            } else {
                // The row was just written, so this should not happen; but a
                // cashier standing over a paid sale is owed the figures either
                // way, and the four we hold are better than a blank screen.
                components::receipt_line(ui, "Cash", &state.money(sale.tendered_minor));
                if sale.rounding_minor != 0 {
                    components::receipt_line(
                        ui,
                        if sale.rounding_minor < 0 {
                            "Rounded down"
                        } else {
                            "Rounded up"
                        },
                        &Money::new(sale.rounding_minor, currency).format(),
                    );
                }
                components::receipt_line(ui, "Total", &state.money(sale.paid_minor()));
            }

            ui.add_space(SPACING.md);
            ui.horizontal(|ui| {
                if components::primary_button(ui, "New sale").clicked() {
                    close = true;
                }
                // The receipt is already written, so this cannot undo the sale;
                // it dismisses the screen. It exists because "New sale" is a
                // commitment, and the cashier may only want the receipt gone.
                if ui
                    .button("Cancel")
                    .on_hover_text("Close this receipt")
                    .clicked()
                {
                    close = true;
                }
            });
            ui.add_space(SPACING.xs);
            components::caption(ui, "Enter or Space starts the next sale. Escape closes.");
        });

        if close
            || (!just_finished
                && ctx.input(|input| {
                    input.key_pressed(egui::Key::Escape)
                        || input.key_pressed(egui::Key::Enter)
                        || input.key_pressed(egui::Key::Space)
                }))
        {
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
    let mut field_submitted = false;

    crate::modules::modal(ctx, egui::Id::new("pos_payment")).show(ctx, |ui| {
        ui.set_width(420.0);
        let palette = theme::current(ui.ctx());

        ui.label(components::semibold("Take payment").size(TYPE.heading));

        ui.add_space(SPACING.md);
        components::receipt_line(ui, "Subtotal", &subtotal);
        if rounding_minor != 0 {
            // Named by the direction the drawer moved, not by the mechanism: a
            // cashier can act on "rounded down" without knowing what a rounding
            // step is, and the receipt says the same words.
            let label = if rounding_minor < 0 {
                "Rounded down"
            } else {
                "Rounded up"
            };
            components::receipt_line(ui, label, &rounding);
        }
        components::divider(ui);
        crate::modules::amount_row(ui, "To pay", &due, true);

        if rounding_minor != 0 {
            // The line a shop owner stops at, so it gets the whole of it: why
            // the figure moved, and that the sale still carries the exact total.
            ui.add_space(SPACING.xs);
            components::caption(
                ui,
                &format!(
                    "Cash is rounded to the nearest {}, the smallest coin in circulation. \
                     The customer pays {due}; the sale still records {subtotal}.",
                    Money::new(crate::currencies::cash_step(currency), currency).format(),
                ),
            );
        }

        ui.add_space(SPACING.md);
        components::field_label(ui, "Cash received");
        ui.add_space(SPACING.xs);

        let mut edited = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.sm;

            let field =
                crate::modules::text_field(ui, &mut payment.tendered, "Exact amount", 210.0);
            if payment.focus {
                field.request_focus();
                payment.focus = false;
            }
            if field.changed() {
                edited = true;
            }
            if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                field_submitted = true;
            }

            if ui
                .button("Clear")
                .on_hover_text("Clear cash received")
                .clicked()
            {
                payment.tendered.clear();
                edited = true;
            }
        });

        // Exact first, then the notes that can actually cover the bill. A chip
        // smaller than the total can only make the drawer come up short, so it
        // is not offered.
        ui.add_space(SPACING.xs);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(SPACING.xs, SPACING.xs);
            for &amount in &quick_amounts(currency, due_minor) {
                let exact_amount = amount == due_minor;
                let label = if exact_amount {
                    "Exact".to_owned()
                } else {
                    Money::new(amount, currency).format()
                };
                let btn = egui::Button::new(
                    egui::RichText::new(&label)
                        .size(TYPE.small)
                        .family(theme::semibold()),
                )
                .min_size(egui::vec2(0.0, 30.0));

                let hint = if exact_amount {
                    format!("Tender the exact {due}")
                } else {
                    format!("Tender {label}")
                };
                if ui.add(btn).on_hover_text(hint).clicked() {
                    apply_denomination(payment, amount, due_minor, currency);
                    edited = true;
                }
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

    if (!opening && ctx.input(|input| input.key_pressed(egui::Key::Enter))) || field_submitted {
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

/// How many one-tap amounts the payment screen offers, exact amount included.
///
/// Three handovers above the bill is what a customer's pocket realistically
/// holds; past that the strip stops being a shortcut and becomes the currency.
const QUICK_AMOUNTS: usize = 5;

/// The amounts worth one tap, in order: the exact total first, then the bill
/// rounded up to each of the biggest notes, smallest first.
///
/// A chip is an amount, not a note. A 27,500 bill is settled with a twenty and
/// a ten (30,000) or with two twenties (40,000) at least as often as with a
/// single 50,000, so each big note is a step the bill is rounded up to rather
/// than a chip in its own right. That is also why a total that lands on a note
/// still gets the round-ups above it: the customer holding two notes of the
/// exact amount is not the common case.
///
/// Only the biggest notes are steps. Below them rounding turns into change
/// counting — nobody is helped by a chip that turns 12,400 into 12,500 — and
/// nothing under the total is offered at all, because it can only produce a
/// "Short by" line.
fn quick_amounts(currency: money::Currency, due_minor: i64) -> Vec<i64> {
    let mut amounts = Vec::with_capacity(QUICK_AMOUNTS);
    amounts.push(due_minor);
    // Denominations are ascending, so the biggest notes are at the end.
    for &step in crate::currencies::denominations(currency)
        .iter()
        .rev()
        .take(QUICK_AMOUNTS - 1)
    {
        let rounded = due_minor - due_minor.rem_euclid(step) + step;
        if !amounts.contains(&rounded) {
            amounts.push(rounded);
        }
    }
    amounts.sort_unstable();
    amounts
}

/// Tap a denomination button: fill the note if empty or already sufficient,
/// or accumulate notes if the cashier is counting in multiple bills.
fn apply_denomination(
    payment: &mut state::Payment,
    denom: i64,
    due_minor: i64,
    currency: money::Currency,
) {
    let current_minor = if payment.tendered.trim().is_empty() {
        0
    } else {
        Money::parse(payment.tendered.trim(), currency)
            .map(|amount| amount.minor())
            .unwrap_or(0)
    };

    let next_minor = if current_minor == 0 || current_minor >= due_minor {
        denom
    } else {
        current_minor.saturating_add(denom)
    };

    payment.tendered = state::plain_amount(next_minor, currency);
    payment.error = None;
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

#[cfg(test)]
mod tests {
    use super::*;
    use money::Currency;

    #[test]
    fn quick_amounts_start_at_the_total_and_round_up_to_the_big_notes() {
        // 12,440 rounds to 12,400. A customer reaches for a 15,000 or a 20,000
        // long before they reach for 600 shillings to make it 13,000.
        assert_eq!(
            quick_amounts(Currency::UGX, 12_400),
            vec![12_400, 15_000, 20_000, 50_000]
        );

        // The case the strip exists for: 27,500 is paid with a twenty and a
        // ten, with two twenties, or with one fifty — never with one note.
        assert_eq!(
            quick_amounts(Currency::UGX, 27_500),
            vec![27_500, 30_000, 40_000, 50_000]
        );

        // A total that lands on a note keeps the round-ups above it.
        assert_eq!(
            quick_amounts(Currency::UGX, 10_000),
            vec![10_000, 15_000, 20_000, 50_000]
        );

        // A decimal currency rounds to the same ladder: $12.34 to $20, $50,
        // $100, and no chip for the two cents that would make it $12.36.
        assert_eq!(
            quick_amounts(Currency::USD, 1_234),
            vec![1_234, 2_000, 5_000, 10_000]
        );
    }

    #[test]
    fn denomination_clicks_accumulate_when_short_and_replace_when_sufficient() {
        let mut payment = state::Payment {
            tendered: String::new(),
            error: Some("Previous error".into()),
            done: None,
            detail: None,
            focus: false,
        };

        // 1. First note on empty field sets the amount
        apply_denomination(&mut payment, 2_000, 3_400, Currency::UGX);
        assert_eq!(payment.tendered, "2000");
        assert_eq!(payment.error, None);

        // 2. 2,000 < 3,400: tapping another 2,000 accumulates to 4,000
        apply_denomination(&mut payment, 2_000, 3_400, Currency::UGX);
        assert_eq!(payment.tendered, "4000");

        // 3. 4,000 >= 3,400: tapping 10,000 replaces it with 10,000 note
        apply_denomination(&mut payment, 10_000, 3_400, Currency::UGX);
        assert_eq!(payment.tendered, "10000");

        // 4. Works for decimal currencies (USD: due is $15.00 = 1500 minor)
        payment.tendered.clear();
        apply_denomination(&mut payment, 1_000, 1_500, Currency::USD); // $10
        assert_eq!(payment.tendered, "10.00");
        apply_denomination(&mut payment, 1_000, 1_500, Currency::USD); // + $10 = $20.00
        assert_eq!(payment.tendered, "20.00");
        apply_denomination(&mut payment, 5_000, 1_500, Currency::USD); // replace with $50.00
        assert_eq!(payment.tendered, "50.00");
    }
}
