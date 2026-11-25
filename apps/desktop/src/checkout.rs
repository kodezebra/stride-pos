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
//! The till takes one or more exact tenders. Cash can cover the remainder and
//! take change; mobile money is recorded only, for the amounts the cashier has
//! confirmed with the customer, and never returns change.

use money::Money;
use ui_kit::components;
use ui_kit::theme;
use ui_kit::tokens::{SPACING, TYPE};
use ui_shell::egui;

use crate::actions;
use crate::state::{self, CartSummary, State};

/// Draw the tender panel into the cart column.
///
/// Called from the Sell module's cart, at the point where the totals and the
/// Pay button would otherwise be. It owns the footer for as long as
/// [`State::payment`] is set: the panel is the payment, not an overlay on it.
pub fn tender(ui: &mut egui::Ui, state: &mut State, summary: &CartSummary) {
    if state.payment.is_none() {
        return;
    }

    // Read the amounts before borrowing the payment. They arrive in one
    // snapshot from the cart, so the amount due cannot disagree with the
    // rounding line even if a quantity edit landed in the same frame.
    let currency = state.currency;
    let rounding_minor = summary.rounding_minor;
    let due_minor = summary.due_minor;
    let rounding = state.money(rounding_minor);
    let due = state.money(due_minor);

    let paid_minor = state.payment_paid_minor();
    let remaining_minor = (due_minor - paid_minor).max(0);
    let remaining = state.money(remaining_minor);
    // One compact read of what is already on the counter. The strip stays one
    // wrapped line: the panel is the cart's footer and cannot grow with a
    // tender count.
    let taken: Vec<(String, String)> = state
        .payment
        .as_ref()
        .map(|payment| {
            payment
                .tenders
                .iter()
                .map(|tender| {
                    (
                        crate::state::payment_method_label(&tender.method),
                        state.money(tender.tendered_minor),
                    )
                })
                .collect()
        })
        .unwrap_or_default();

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
    let danger = egui::Color32::from(palette.danger);
    let mut add = false;
    let mut close = false;
    let mut remove_last = false;
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

    // A split sale keeps the balance in front of the cashier, and the tenders
    // already taken as one line. The undo is on the strip because the last
    // tender is the one a mistyped figure belongs to.
    if !payment.tenders.is_empty() {
        crate::modules::amount_row(ui, "Remaining", &remaining, false);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(SPACING.xs, SPACING.xs);
            components::caption(ui, "Taken:");
            for (label, amount) in &taken {
                components::caption(ui, &format!("{amount} {label}"));
            }
            if actions::quiet(ui, "Undo")
                .on_hover_text("Remove the last payment")
                .clicked()
            {
                remove_last = true;
            }
        });
    }
    ui.add_space(SPACING.sm);

    // The method is a choice only when there is more than one; the row reads
    // left to right in the order the shop takes money.
    components::field_label(ui, "Method");
    ui.add_space(SPACING.xs);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(SPACING.xs, SPACING.xs);
        for (key, label) in state::PAYMENT_METHODS {
            let selected = payment.method.eq_ignore_ascii_case(key);
            let text = if selected {
                egui::RichText::new(*label)
                    .size(TYPE.small)
                    .family(theme::semibold())
                    .color(palette.accent_text)
            } else {
                egui::RichText::new(*label)
                    .size(TYPE.small)
                    .family(theme::semibold())
            };
            let mut button = egui::Button::new(text).min_size(egui::vec2(0.0, 36.0));
            if selected {
                button = button.fill(palette.accent).stroke(egui::Stroke::NONE);
            }
            if ui.add(button).clicked() && !selected {
                payment.method = (*key).to_owned();
                payment.tendered.clear();
                payment.reference.clear();
                payment.error = None;
                // Hand the keyboard back to the amount: the next thing the
                // cashier does is type or tap a note.
                payment.focus = true;
            }
        }
    });

    let is_cash = payment.method.eq_ignore_ascii_case("cash");
    ui.add_space(SPACING.sm);
    components::field_label(ui, if is_cash { "Cash received" } else { "Amount" });
    ui.add_space(SPACING.xs);

    let mut edited = false;
    let amount_hint = crate::state::plain_amount(remaining_minor, currency);
    let field = crate::modules::prefixed_field(
        ui,
        &mut payment.tendered,
        currency.code(),
        &amount_hint,
        ui.available_width(),
    );
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

    if is_cash {
        // Exact first, then the notes that can actually cover the balance. A
        // chip smaller than what is left can only make the drawer come up
        // short, so it is not offered.
        ui.add_space(SPACING.xs);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(SPACING.xs, SPACING.xs);
            if remaining_minor > 0 {
                for &amount in &quick_amounts(currency, remaining_minor) {
                    let exact_amount = amount == remaining_minor;
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
                    .min_size(egui::vec2(0.0, 44.0));

                    let hint = if exact_amount {
                        format!("Tender the exact {remaining}")
                    } else {
                        format!("Tender {label}")
                    };
                    if ui.add(btn).on_hover_text(hint).clicked() {
                        apply_denomination(payment, amount, remaining_minor, currency);
                        edited = true;
                        // Hand the keyboard back to the amount: the next thing a
                        // cashier does is type, not click again.
                        field.request_focus();
                    }
                }
            }
        });
    } else {
        // Mobile money is recorded, not processed. The sentence is on screen
        // because the till cannot check the network, and the cashier is the
        // one holding the customer's confirmation.
        ui.add_space(SPACING.sm);
        components::caption(ui, "Record only — confirm the customer's phone, then add.");
        ui.add_space(SPACING.xs);
        let reference = crate::modules::text_field(
            ui,
            &mut payment.reference,
            "Transaction ID (optional)",
            ui.available_width(),
        );
        if reference.changed() {
            edited = true;
        }
        if reference.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            field_submitted = true;
        }
    }

    // An even split is the common case behind a split tender: two or three
    // people paying one share each. The chip fills the amount field with one
    // share whatever the method; Add still records it, so the cashier sees
    // every tender and a mobile share stays exact.
    ui.add_space(SPACING.xs);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(SPACING.xs, SPACING.xs);
        components::caption(ui, "Split evenly");
        for parts in 2..=4 {
            let share = split_share(due_minor, remaining_minor, parts);
            if share <= 0 {
                continue;
            }
            let btn = egui::Button::new(
                egui::RichText::new(parts.to_string())
                    .size(TYPE.small)
                    .family(theme::semibold()),
            )
            .min_size(egui::vec2(34.0, 30.0));
            if ui
                .add(btn)
                .on_hover_text(format!(
                    "Set this payment to one of {parts} shares ({})",
                    Money::new(share, currency).format()
                ))
                .clicked()
            {
                payment.tendered = crate::state::plain_amount(share, currency);
                payment.error = None;
                edited = true;
                field.request_focus();
            }
        }
    });

    // An error about the previous attempt is not about the new one.
    if edited {
        payment.error = None;
    }

    // The change updates as the cashier types: it is the number they read out,
    // and waiting for Complete to show it invites a second count. A non-cash
    // tender is exact, so it has no short or change line to draw.
    let tendered = if payment.tendered.trim().is_empty() {
        Some(remaining_minor)
    } else {
        Money::parse(payment.tendered.trim(), currency)
            .ok()
            .map(|amount| amount.minor())
    };

    if is_cash {
        ui.add_space(SPACING.xs);
        match tendered {
            Some(tendered) if tendered < remaining_minor => {
                ui.label(
                    egui::RichText::new(format!(
                        "Short by {}",
                        Money::new(remaining_minor - tendered, currency).format()
                    ))
                    .size(TYPE.small)
                    .color(danger),
                );
            }
            Some(tendered) => change_row(
                ui,
                &Money::new(tendered - remaining_minor, currency).format(),
                success,
            ),
            // Mid-typing text that is not yet an amount is not an error.
            None => {
                ui.label(egui::RichText::new(" ").size(TYPE.small));
            }
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

    let settles = match tendered {
        Some(amount) if is_cash => amount >= remaining_minor,
        Some(amount) => amount == remaining_minor,
        None => false,
    };

    ui.add_space(SPACING.sm);
    let label = if settles {
        "Complete sale"
    } else {
        "Add payment"
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
        add = true;
    }

    // "Cancel" would read as cancelling the sale. The basket is intact; this
    // only returns to it. Same row as the key hint, because the two are the
    // same thought: nothing here is committed until the sale writes.
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
                components::caption(ui, "Enter adds · Esc back");
            });
        }
    });

    if (!opening && ui.input(|i| i.key_pressed(egui::Key::Enter))) || field_submitted {
        add = true;
    }

    if remove_last {
        state.remove_last_tender();
    }
    if add {
        state.confirm_payment();
    } else if close {
        state.close_payment();
    }
}

/// How many one-tap amounts the tender panel offers, exact amount included.
///
/// A bill is settled with a handful of notes; past four chips the strip stops
/// being a shortcut and becomes the currency.
const QUICK_AMOUNTS: usize = 4;

/// The amounts worth one tap, in order: the exact total first, then the notes
/// that can cover it, smallest first.
///
/// A chip is a note the drawer holds, not a round-up. A 27,500 bill is settled
/// with a 50,000 note, not with an invented 30,000; a 12,400 bill never offers
/// 15,000. When the bill is larger than every note, the smallest multiple of
/// the largest note that covers it is offered instead — two 50,000s for 60,000.
///
/// Nothing under the total is offered at all, because it can only produce a
/// "Short by" line.
fn quick_amounts(currency: money::Currency, due_minor: i64) -> Vec<i64> {
    let notes = crate::currencies::denominations(currency);
    let mut amounts = Vec::with_capacity(QUICK_AMOUNTS);
    amounts.push(due_minor);

    // Denominations are ascending, so the notes at or above the bill are the
    // ones a single hand can cover it with.
    for &note in notes.iter().filter(|&&note| note >= due_minor) {
        if amounts.len() == QUICK_AMOUNTS {
            break;
        }
        if !amounts.contains(&note) {
            amounts.push(note);
        }
    }

    // A bill bigger than every note is paid with several of the biggest.
    if let Some(&largest) = notes.last()
        && due_minor > largest
        && amounts.len() < QUICK_AMOUNTS
    {
        let multiple = due_minor - due_minor.rem_euclid(largest) + largest;
        if !amounts.contains(&multiple) {
            amounts.push(multiple);
        }
    }

    amounts
}

/// Tap a denomination button: fill the note if empty or already sufficient,
/// or accumulate notes if the cashier is counting in multiple bills.
fn apply_denomination(
    payment: &mut state::Payment,
    denom: i64,
    remaining_minor: i64,
    currency: money::Currency,
) {
    let current_minor = if payment.tendered.trim().is_empty() {
        0
    } else {
        Money::parse(payment.tendered.trim(), currency)
            .map(|amount| amount.minor())
            .unwrap_or(0)
    };

    let next_minor = if current_minor == 0 || current_minor >= remaining_minor {
        denom
    } else {
        current_minor.saturating_add(denom)
    };

    payment.tendered = state::plain_amount(next_minor, currency);
    payment.error = None;
}

/// One share of a sale split evenly between `parts` people.
///
/// The share is read from the whole bill, not from the shrinking balance, so
/// pressing "3" three times gives three equal shares. Integer division can
/// leave a unit or two over; when only that remainder is left, the share is the
/// remainder, so the last person settles the sale rather than leaving a balance
/// no tender can clear.
fn split_share(due_minor: i64, remaining_minor: i64, parts: i64) -> i64 {
    if parts <= 1 {
        return remaining_minor;
    }
    let share = due_minor / parts;
    if share <= 0 || remaining_minor < share.saturating_mul(2) {
        remaining_minor
    } else {
        share
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

#[cfg(test)]
mod tests {
    use super::*;
    use money::Currency;

    #[test]
    fn quick_amounts_offer_the_exact_total_then_the_notes_that_cover_it() {
        // 12,400 is settled with a 20,000 or a 50,000. There is no 15,000 note
        // in the drawer, so the strip does not invent one.
        assert_eq!(
            quick_amounts(Currency::UGX, 12_400),
            vec![12_400, 20_000, 50_000]
        );

        // The case the strip exists for: a 27,500 bill is one 50,000 note, not
        // a made-up 30,000 or 40,000.
        assert_eq!(quick_amounts(Currency::UGX, 27_500), vec![27_500, 50_000]);

        // A total that lands on a note keeps the notes above it.
        assert_eq!(
            quick_amounts(Currency::UGX, 10_000),
            vec![10_000, 20_000, 50_000]
        );

        // A bill larger than every note is the smallest multiple of the
        // biggest: two 50,000s cover 60,000.
        assert_eq!(quick_amounts(Currency::UGX, 60_000), vec![60_000, 100_000]);

        // Four chips is the cap, and the smallest notes win the last places.
        assert_eq!(
            quick_amounts(Currency::UGX, 3_400),
            vec![3_400, 5_000, 10_000, 20_000]
        );

        // A decimal currency walks the same ladder: $12.34 to $20, $50, $100,
        // and no chip for the two cents that would make it $12.36.
        assert_eq!(
            quick_amounts(Currency::USD, 1_234),
            vec![1_234, 2_000, 5_000, 10_000]
        );
    }

    #[test]
    fn denomination_clicks_accumulate_when_short_and_replace_when_sufficient() {
        let mut payment = state::Payment {
            tenders: Vec::new(),
            method: "cash".to_owned(),
            tendered: String::new(),
            reference: String::new(),
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

    #[test]
    fn split_shares_divide_the_bill_and_leave_the_remainder_to_the_last() {
        assert_eq!(split_share(10_000, 10_000, 2), 5_000);
        assert_eq!(split_share(10_000, 5_000, 2), 5_000);

        // Three shares of 10,000: 3,333 twice, then the 3,334 remainder.
        assert_eq!(split_share(10_000, 10_000, 3), 3_333);
        assert_eq!(split_share(10_000, 6_667, 3), 3_333);
        assert_eq!(split_share(10_000, 3_334, 3), 3_334);

        // A balance smaller than a share is tendered whole.
        assert_eq!(split_share(10_000, 100, 4), 100);

        // A bill too small to split still yields the whole balance.
        assert_eq!(split_share(1, 1, 4), 1);
        assert_eq!(split_share(0, 0, 3), 0);
    }
}
