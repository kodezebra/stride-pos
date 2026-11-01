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
/// callers can hold their own borrow while the modal is open.
///
/// `with_change` is false on the till's confirmation, which prints the change
/// as its own callout above the slip: the same figure twice on one screen reads
/// as two different amounts, and the cashier needs it where the eye lands, not
/// at the end of a scroll.
pub fn body(ui: &mut egui::Ui, detail: &SaleDetail, currency: Currency, with_change: bool) {
    let palette = theme::current(ui.ctx());
    let text = egui::Color32::from(palette.text);
    let muted = egui::Color32::from(palette.text_muted);

    // The merchant is the one centred thing on the page: on paper it is the
    // letterhead, and it is what makes the slip identifiable in a drawer full
    // of them.
    ui.vertical_centered(|ui| {
        ui.label(
            components::semibold(crate::APP_NAME)
                .size(TYPE.body)
                .color(text),
        );
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
    // would repeat the total a centimetre above it.
    if detail.rounding_minor != 0 {
        row(
            ui,
            "Subtotal",
            &money(detail.total_minor, currency),
            Weight::Plain,
        );
        row(
            ui,
            "Cash rounding",
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
