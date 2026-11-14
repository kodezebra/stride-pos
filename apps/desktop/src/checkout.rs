//! Cash in, change out, one sale written — inside the cart column.
//!
//! Paying is not a screen. The cart's footer grows into the tender panel in
//! place, so the amount the cashier was already reading does not move, and the
//! product grid behind it dims but stays where it was. A till is operated by
//! muscle memory; a modal that covers the workspace throws that memory away
//! every single sale.
//!
//! One state, one panel. It asks what is due, what the customer handed over,
//! and what comes back. Completing the sale closes the panel and empties the
//! cart in the same frame, so the next customer starts from a clean till with
//! no "new sale" button to press.
//!
//! Cash only. The `payments` table already has a `method` column and mobile money
//! is a new row rather than a migration, but a method picker with one method in
//! it is a choice the cashier has to make for no reason.

use money::Money;
use ui_kit::components;
use ui_kit::theme;
use ui_kit::tokens::{SPACING, TYPE};
use ui_shell::egui;

use crate::actions;
use crate::state::{self, State};

/// Draw the tender panel into the cart column.
///
/// Called from the Sell module's cart, at the point where the totals and the
/// Pay button would otherwise be. It owns the footer for as long as
/// [`State::payment`] is set: the panel is the payment, not an overlay on it.
pub fn tender(ui: &mut egui::Ui, state: &mut State) {
    if state.payment.is_none() {
        return;
    }

    // Read the amounts before borrowing the payment: they come from the cart,
    // which the payment does not own, and `Money::format` is the one rendering.
    let currency = state.currency;
    let rounding_minor = state.cart_rounding_minor();
    let due_minor = state.due_minor();
    let rounding = state.money(rounding_minor);
    let due = state.money(due_minor);

    // Everything below draws the form; the keys are routed by `shell`, so a
    // focused field and a bare counter top behave the same.
    let Some(payment) = state.payment.as_mut() else {
        return;
    };

    // The frame that opened the panel still has the keystroke that opened it —
    // Enter or Ctrl+Enter — in its input, and Enter here means "complete the
    // sale". A guard on the focus flag keeps that keystroke from selling the
    // basket the cashier only meant to start paying.
    let opening = payment.focus;

    let palette = theme::current(ui.ctx());
    let success = egui::Color32::from(palette.success);
    let mut complete = false;
    let mut close = false;
    let mut field_submitted = false;

    // The caller has already put a gap between the lines and the footer.
    components::divider(ui);
    ui.add_space(SPACING.xs);

    // The exact total, then what the drawer takes. The cashier already read
    // these on the cart; they are repeated because the rounding line only
    // matters at the moment money changes hands.
    if rounding_minor != 0 {
        let label = if rounding_minor < 0 {
            "Rounded down"
        } else {
            "Rounded up"
        };
        components::receipt_line(ui, label, &rounding);
    }
    crate::modules::amount_row(ui, "To pay", &due, true);
    ui.add_space(SPACING.sm);

    components::field_label(ui, "Cash received");
    ui.add_space(SPACING.xs);

    let mut edited = false;
    let field = crate::modules::text_field(ui, &mut payment.tendered, &due, ui.available_width());
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

    // Exact first, then the notes that can actually cover the bill. A chip
    // smaller than the total can only make the drawer come up short, so it is
    // not offered. Four is the strip's limit: past that it stops being a
    // shortcut and becomes the currency.
    ui.add_space(SPACING.xs);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(SPACING.xs, SPACING.xs);
        for &amount in quick_amounts(currency, due_minor).iter().take(4) {
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
            .min_size(egui::vec2(0.0, 34.0));

            let hint = if exact_amount {
                format!("Tender the exact {due}")
            } else {
                format!("Tender {label}")
            };
            if ui.add(btn).on_hover_text(hint).clicked() {
                apply_denomination(payment, amount, due_minor, currency);
                edited = true;
                // Hand the keyboard back to the amount: the next thing a
                // cashier does is type, not click again.
                field.request_focus();
            }
        }
    });

    // An error about the previous attempt is not about the new one.
    if edited {
        payment.error = None;
    }

    // The change updates as the cashier types: it is the number they read out,
    // and waiting for Complete to show it invites a second count.
    let tendered = if payment.tendered.trim().is_empty() {
        Some(due_minor)
    } else {
        Money::parse(payment.tendered.trim(), currency)
            .ok()
            .map(|amount| amount.minor())
    };

    ui.add_space(SPACING.xs);
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
            success,
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

    ui.add_space(SPACING.sm);
    let label = if tendered.is_some_and(|t| t >= due_minor) {
        "Complete sale"
    } else {
        "Cash is short"
    };
    let complete_btn = ui.add_sized(
        [ui.available_width(), 52.0],
        egui::Button::new(
            egui::RichText::new(label)
                .size(TYPE.button)
                .family(theme::semibold())
                .color(palette.accent_text),
        )
        .fill(palette.accent)
        .stroke(egui::Stroke::NONE)
        .corner_radius(egui::CornerRadius::same(ui_kit::tokens::RADII.sm as u8)),
    );
    if complete_btn.clicked() {
        complete = true;
    }

    // "Cancel" would read as cancelling the sale. The basket is intact; this
    // only returns to it. Same row as the key hint, because the two are the
    // same thought: nothing here is committed until Complete.
    ui.horizontal(|ui| {
        if actions::quiet(ui, "Back to cart")
            .on_hover_text("Escape")
            .clicked()
        {
            close = true;
        }
        // The hint gives way to an error rather than stacking under it: the
        // panel is the cart's footer and has a fixed height to stay inside.
        if payment.error.is_none() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                components::caption(ui, "Enter pays · Esc back");
            });
        }
    });

    if (!opening && ui.input(|i| i.key_pressed(egui::Key::Enter))) || field_submitted {
        complete = true;
    }

    if complete {
        state.confirm_payment();
    } else if close {
        state.close_payment();
    }
}

/// How many one-tap amounts the tender panel offers, exact amount included.
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
