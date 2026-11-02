//! The receipt, drawn once.
//!
//! Two screens show a sale: the confirmation after payment and the record
//! behind a row in Sales. They are the same paper, so they share this block —
//! shop, lines, money — in the order a receipt prints it. Two renderers would
//! drift, and a shop whose till and whose archive disagree about a sale has no
//! record at all.
//!
//! The order follows the trade, not the schema: merchant first, because the
//! customer's copy has to say whose shop this is; then what was bought; then
//! the money, subtotal before total and the change last, because that is the
//! figure the customer counts on the way out. Square and Shopify print the
//! same sequence, and a cashier reading ours should not have to learn a second
//! one.
//!
//! Amounts are what the sale recorded — `qty × unit` as charged, the rounding
//! the customer actually paid — never a re-derivation that could disagree with
//! the ledger.

use money::{Currency, Money};
use ui_kit::components;
use ui_kit::theme;
use ui_kit::tokens::{SPACING, TYPE};
use ui_shell::egui;

use crate::db::{DetailLine, SaleDetail};

/// Draw the receipt for one recorded sale.
///
/// Takes the currency rather than the whole [`crate::state::State`] so both
/// callers can hold their own borrow while the modal is open, and the shop name
/// rather than reading settings, so the preview can show a name that has not
/// been saved yet.
///
/// `with_change` is false on the till's confirmation, which prints the change
/// as its own callout above the slip: the same figure twice on one screen reads
/// as two different amounts, and the cashier needs it where the eye lands, not
/// at the end of a scroll.
pub fn body(
    ui: &mut egui::Ui,
    detail: &SaleDetail,
    shop: &str,
    currency: Currency,
    with_change: bool,
) {
    let palette = theme::current(ui.ctx());
    let text = egui::Color32::from(palette.text);
    let muted = egui::Color32::from(palette.text_muted);

    // The merchant is the one centred thing on the page: on paper it is the
    // letterhead, and it is what makes the slip identifiable in a drawer full
    // of them.
    ui.vertical_centered(|ui| {
        ui.label(components::semibold(shop).size(TYPE.body).color(text));
        ui.label(
            components::numeric(format!("Sale #{}", detail.id))
                .size(TYPE.receipt)
                .color(text),
        );
        ui.label(
            egui::RichText::new(&detail.created_at)
                .size(TYPE.small)
                .color(muted),
        );
        ui.label(
            egui::RichText::new(cashier(detail))
                .size(TYPE.small)
                .color(muted),
        );
    });

    components::divider(ui);

    if detail.lines.is_empty() {
        components::caption(ui, "No line items were recorded on this sale.");
    }
    for line in &detail.lines {
        item(ui, line, currency);
    }

    components::divider(ui);

    // Subtotal earns a line only when rounding moved the figure: otherwise it
    // would repeat the total a centimetre above it. The rounding is named by
    // the direction it went, the same words the payment screen uses.
    if detail.rounding_minor != 0 {
        row(
            ui,
            "Subtotal",
            &money(detail.total_minor, currency),
            Weight::Plain,
        );
        row(
            ui,
            if detail.rounding_minor < 0 {
                "Rounded down"
            } else {
                "Rounded up"
            },
            &money(detail.rounding_minor, currency),
            Weight::Plain,
        );
    }
    row(
        ui,
        "Total",
        &money(detail.paid_minor, currency),
        Weight::Strong,
    );
    if detail.tendered_minor != 0 {
        row(
            ui,
            &format!("Paid by {}", detail.method),
            &money(detail.tendered_minor, currency),
            Weight::Plain,
        );
    }
    if with_change && detail.change_minor != 0 {
        row(
            ui,
            "Change",
            &money(detail.change_minor, currency),
            Weight::Change,
        );
    }

    ui.add_space(SPACING.sm);
    ui.vertical_centered(|ui| components::caption(ui, "Thank you."));
}

/// The same slip as plain text, for the clipboard.
///
/// It is the receipt a customer can be sent rather than shown: a messaging app
/// is where a disputed total actually gets discussed, and "look at my screen"
/// does not travel. The order matches [`body`] so the two cannot tell different
/// stories — the difference is only that text must carry the change itself,
/// because there is no big green figure above it to do that job.
pub fn plain(detail: &SaleDetail, shop: &str, currency: Currency) -> String {
    let mut out = String::new();

    out.push_str(shop);
    out.push('\n');
    out.push_str(&format!("Sale #{}\n", detail.id));
    out.push_str(&format!("{}\n", detail.created_at));
    out.push_str(&format!("{}\n\n", cashier(detail)));

    if detail.lines.is_empty() {
        out.push_str("No line items were recorded on this sale.\n");
    }
    for line in &detail.lines {
        out.push_str(&text_row(&line.name, &money(line.total_minor, currency)));
        out.push('\n');
        // A count and unit price only when there was more than one: repeating
        // "1 ×" under every name is noise on paper.
        if line.qty > 1 {
            out.push_str(&format!(
                "  {} × {}\n",
                line.qty,
                money(line.unit_minor, currency)
            ));
        }
    }

    out.push('\n');
    if detail.rounding_minor != 0 {
        out.push_str(&text_row("Subtotal", &money(detail.total_minor, currency)));
        out.push('\n');
        let direction = if detail.rounding_minor < 0 {
            "Rounded down"
        } else {
            "Rounded up"
        };
        out.push_str(&text_row(
            direction,
            &money(detail.rounding_minor, currency),
        ));
        out.push('\n');
    }
    out.push_str(&text_row("Total", &money(detail.paid_minor, currency)));
    out.push('\n');
    if detail.tendered_minor != 0 {
        out.push_str(&text_row(
            &format!("Paid by {}", detail.method),
            &money(detail.tendered_minor, currency),
        ));
        out.push('\n');
    }
    if detail.change_minor != 0 {
        out.push_str(&text_row("Change", &money(detail.change_minor, currency)));
        out.push('\n');
    }

    out.push_str("\nThank you.\n");
    out
}

/// A label on the left and a figure on the right, in a fixed text column.
///
/// Padding rather than tabs: a tab stops at whatever the reader's editor decides
/// is 4 or 8 columns, and a receipt that misaligns in the customer's messaging
/// app is a receipt that looks wrong.
fn text_row(label: &str, value: &str) -> String {
    const COLUMN: usize = 30;
    let pad = COLUMN.saturating_sub(label.chars().count()).max(1);
    format!("{label}{}{value}", " ".repeat(pad))
}

/// One line item: the name, the line total on the right, and — when more than
/// one unit went out — the count and the unit price underneath.
///
/// The name wraps instead of being cut. A cart row may truncate because the
/// full name is one hover away; the customer's copy cannot, because the hover
/// is not printed.
fn item(ui: &mut egui::Ui, line: &DetailLine, currency: Currency) {
    let palette = theme::current(ui.ctx());
    let text = egui::Color32::from(palette.text);

    ui.horizontal_top(|ui| {
        // Right to left, amount first: the figure holds the right edge and the
        // name gets whatever is left, however many lines it needs.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            ui.label(
                components::numeric(money(line.total_minor, currency))
                    .size(TYPE.receipt)
                    .color(text),
            );

            let width = (ui.available_width() - ui.spacing().item_spacing.x).max(0.0);
            ui.allocate_ui_with_layout(
                egui::vec2(width, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&line.name).size(TYPE.body).color(text),
                        )
                        .wrap(),
                    );
                    // A single unit needs no count: the name already says one.
                    if line.qty > 1 {
                        components::caption(
                            ui,
                            &format!("{} × {}", line.qty, money(line.unit_minor, currency)),
                        );
                    }
                },
            );
        });
    });
    ui.add_space(SPACING.xs);
}

/// How much weight a money row carries.
#[derive(Clone, Copy)]
enum Weight {
    /// A supporting figure: subtotal, tender.
    Plain,
    /// The amount the sale came to.
    Strong,
    /// What goes back over the counter: the one number read aloud.
    Change,
}

/// A label on the left, a figure on the right, on one fixed-height line.
///
/// Fixed height because a right-to-left child otherwise claims the rest of the
/// pane, which turns a short receipt into one line and a lot of blank paper.
fn row(ui: &mut egui::Ui, label: &str, value: &str, weight: Weight) {
    let palette = theme::current(ui.ctx());
    let text = egui::Color32::from(palette.text);
    let muted = egui::Color32::from(palette.text_muted);
    let success = egui::Color32::from(palette.success);

    let (height, label_text, value_text) = match weight {
        Weight::Plain => (
            TYPE.receipt * 1.9,
            egui::RichText::new(label).size(TYPE.receipt).color(muted),
            components::numeric(value).size(TYPE.receipt).color(text),
        ),
        Weight::Strong => (
            TYPE.body * 1.9,
            components::semibold(label).size(TYPE.body).color(text),
            components::numeric(value).size(TYPE.body).color(text),
        ),
        Weight::Change => (
            TYPE.body * 1.9,
            components::semibold(label).size(TYPE.body).color(text),
            components::numeric(value).size(TYPE.body).color(success),
        ),
    };

    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), height),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            ui.label(value_text);
            ui.label(label_text);
        },
    );
}

/// Who took the money, as the slip prints it. A sale from before the till
/// asked for a name still has to read as a complete receipt.
fn cashier(detail: &SaleDetail) -> String {
    match detail.cashier.as_deref() {
        Some(name) => format!("Cashier: {name}"),
        None => "Cashier: -".to_owned(),
    }
}

fn money(minor: i64, currency: Currency) -> String {
    Money::new(minor, currency).format()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One sale with everything on it: two units of one thing, one of another,
    /// small rounding, and change coming back. The shape a real slip has.
    fn sale() -> SaleDetail {
        SaleDetail {
            id: 12,
            cashier: Some("Amina".to_owned()),
            created_at: "2026-11-02 14:03".to_owned(),
            total_minor: 12_400,
            rounding_minor: 100,
            paid_minor: 12_500,
            method: "cash".to_owned(),
            tendered_minor: 20_000,
            change_minor: 7_500,
            lines: vec![
                DetailLine {
                    name: "Flat white".to_owned(),
                    qty: 2,
                    unit_minor: 3_500,
                    total_minor: 7_000,
                },
                DetailLine {
                    name: "Cinnamon roll".to_owned(),
                    qty: 1,
                    unit_minor: 5_400,
                    total_minor: 5_400,
                },
            ],
        }
    }

    /// The clipboard slip has to be able to answer a customer on its own: who
    /// sold it, when, what it held, what it came to, and what went back. Any of
    /// those missing turns the copy into a list of numbers with no seller.
    #[test]
    fn the_text_receipt_carries_the_whole_slip() {
        let text = plain(&sale(), "Nakawa Grocers", Currency::UGX);

        assert!(text.starts_with("Nakawa Grocers\n"), "{text}");
        assert!(text.contains("Sale #12"), "{text}");
        assert!(text.contains("2026-11-02 14:03"), "{text}");
        assert!(text.contains("Cashier: Amina"), "{text}");
        assert!(text.contains("Flat white"), "{text}");
        assert!(text.contains("Cinnamon roll"), "{text}");
        assert!(
            text.contains(&Money::new(12_500, Currency::UGX).format()),
            "{text}"
        );
        assert!(
            text.contains(&Money::new(7_500, Currency::UGX).format()),
            "{text}"
        );
        assert!(text.contains("Paid by cash"), "{text}");
        assert!(text.trim_end().ends_with("Thank you."), "{text}");
    }

    /// The order of the money block is the order on paper: the subtotal and the
    /// rounding explain the total, and the change comes after the total is read.
    #[test]
    fn the_text_receipt_explains_the_total_before_it_prints_the_change() {
        let text = plain(&sale(), "Nakawa Grocers", Currency::UGX);

        let subtotal = text.find("Subtotal").expect("subtotal line");
        let rounding = text.find("Rounded up").expect("rounding line");
        let total = text.find("Total").expect("total line");
        let change = text.find("Change").expect("change line");

        assert!(subtotal < rounding, "{text}");
        assert!(rounding < total, "{text}");
        assert!(total < change, "{text}");
    }

    /// A count and unit price under the name only when more than one went out.
    /// Repeating "1 ×" is a line the cashier has to read for nothing.
    #[test]
    fn only_a_multi_unit_line_prints_its_count_and_unit_price() {
        let text = plain(&sale(), "Nakawa Grocers", Currency::UGX);
        let unit = Money::new(3_500, Currency::UGX).format();

        assert!(text.contains(&format!("  2 × {unit}")), "{text}");
        assert!(
            !text.contains("1 ×"),
            "no one-unit line should show a count: {text}"
        );
    }

    /// Labels pad to a column so the figures line up in any monospace reader,
    /// but a name longer than the column must still keep its figure — one space
    /// is enough, and eating the amount is not.
    #[test]
    fn a_long_label_keeps_one_space_before_its_figure() {
        let long = "a".repeat(40);
        let row = text_row(&long, "9,000");

        assert_eq!(row, format!("{long} 9,000"));
        assert_eq!(text_row("Total", "9,000").len(), 30 + "9,000".len());
    }
}
