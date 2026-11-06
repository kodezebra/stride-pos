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
use crate::settings::ReceiptDetails;

/// The paper around a sale: whose shop it is, how to reach them, and the lines
/// that close the slip.
///
/// Kept apart from [`SaleDetail`] because the same sale can be reprinted after
/// the shop moves premises or changes its returns policy. The sale is the
/// record; this is the letterhead it is printed on.
pub struct Paper<'a> {
    pub shop: &'a str,
    pub address: &'a str,
    pub phone: &'a str,
    pub tax_number: &'a str,
    pub return_policy: &'a str,
    pub footer: &'a str,
    pub show_cashier: bool,
}

impl<'a> Paper<'a> {
    pub fn from_receipt(shop: &'a str, receipt: &'a ReceiptDetails) -> Self {
        Self {
            shop,
            address: &receipt.address,
            phone: &receipt.phone,
            tax_number: &receipt.tax_number,
            return_policy: &receipt.return_policy,
            footer: receipt.footer_text(),
            show_cashier: receipt.show_cashier,
        }
    }

    fn footer_text(&self) -> &str {
        let footer = self.footer.trim();
        if footer.is_empty() {
            "Thank you."
        } else {
            footer
        }
    }
}

/// A short, realistic sale for Settings → Receipt. Deliberately not a real
/// sale: a preview that borrowed the last sale would disappear on a fresh till
/// and change under the owner while they typed.
pub fn sample() -> SaleDetail {
    SaleDetail {
        id: 1042,
        cashier: Some("Amina".to_owned()),
        created_at: "02/11/2026 14:03".to_owned(),
        total_minor: 14_750,
        rounding_minor: 0,
        tax_minor: 0,
        paid_minor: 14_750,
        method: "cash".to_owned(),
        tendered_minor: 20_000,
        change_minor: 5_250,
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
                unit_minor: 3_250,
                total_minor: 3_250,
            },
            DetailLine {
                name: "Orange juice".to_owned(),
                qty: 1,
                unit_minor: 4_500,
                total_minor: 4_500,
            },
        ],
    }
}

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
    paper: &Paper<'_>,
    currency: Currency,
    with_change: bool,
) {
    let palette = theme::current(ui.ctx());
    let text = egui::Color32::from(palette.text);
    let muted = egui::Color32::from(palette.text_muted);

    // The merchant is the one centred thing on the page: on paper it is the
    // letterhead, and it is what makes the slip identifiable in a drawer full
    // of them.
    let shop = non_empty(paper.shop).unwrap_or(crate::APP_NAME);
    ui.vertical_centered(|ui| {
        ui.label(components::semibold(shop).size(TYPE.body).color(text));
        for line in [
            non_empty(paper.address).map(str::to_owned),
            non_empty(paper.phone).map(|phone| format!("Tel: {phone}")),
            non_empty(paper.tax_number).map(|tax| format!("Tax/VAT: {tax}")),
        ]
        .into_iter()
        .flatten()
        {
            ui.add(
                egui::Label::new(egui::RichText::new(line).size(TYPE.small).color(muted)).wrap(),
            );
        }
    });
    ui.add_space(SPACING.xs);

    // Two compact key-value rows instead of one label per fact. Four stacked
    // lines of metadata cost a third of a narrow slip before the first item,
    // and the customer only needs to find the receipt number and the cashier.
    let (date, time) = stamp_parts(&detail.created_at);
    meta_row(ui, &format!("Receipt #{}", detail.id), &date);
    let left = meta_left(&time, detail);
    let right = if paper.show_cashier {
        cashier(detail)
    } else {
        String::new()
    };
    if !left.is_empty() || !right.is_empty() {
        meta_row(ui, &left, &right);
    }
    ui.add_space(SPACING.xs);

    components::divider(ui);

    if detail.lines.is_empty() {
        components::caption(ui, "No line items were recorded on this sale.");
    }
    for line in &detail.lines {
        item(ui, line, currency);
    }

    components::divider(ui);

    // When the till charges tax, the shelf price is tax-inclusive: the receipt
    // backs the tax out of the total so the customer can see what the shop
    // keeps and what goes to the authority. Zero tax means no line — some small
    // stores are not VAT-registered.
    if detail.tax_minor > 0 {
        row(
            ui,
            "Pre-tax",
            &amount(detail.total_minor - detail.tax_minor, currency),
            Weight::Plain,
        );
        row(
            ui,
            &format!("Tax (@ {}%)", detail.tax_minor * 100 / detail.total_minor),
            &amount(detail.tax_minor, currency),
            Weight::Plain,
        );
    }

    // Subtotal earns a line only when rounding moved the figure: otherwise it
    // would repeat the total a centimetre above it. The rounding is named by
    // the direction it went, the same words the payment screen uses.
    if detail.rounding_minor != 0 {
        row(
            ui,
            "Subtotal",
            &amount(detail.total_minor, currency),
            Weight::Plain,
        );
        row(
            ui,
            if detail.rounding_minor < 0 {
                "Rounded down"
            } else {
                "Rounded up"
            },
            &amount(detail.rounding_minor, currency),
            Weight::Plain,
        );
    }
    row(
        ui,
        &format!("Total ({})", currency.code()),
        &amount(detail.paid_minor, currency),
        Weight::Strong,
    );
    if detail.tendered_minor != 0 {
        row(
            ui,
            &format!("Paid by {}", detail.method),
            &amount(detail.tendered_minor, currency),
            Weight::Plain,
        );
    }
    if with_change && detail.change_minor != 0 {
        row(
            ui,
            "Change",
            &amount(detail.change_minor, currency),
            Weight::Change,
        );
    }

    ui.add_space(SPACING.sm);
    if let Some(policy) = non_empty(paper.return_policy) {
        ui.vertical_centered(|ui| {
            ui.add(
                egui::Label::new(egui::RichText::new(policy).size(TYPE.small).color(muted)).wrap(),
            );
        });
    }
    ui.vertical_centered(|ui| components::caption(ui, paper.footer_text()));
}

/// The same slip as plain text, for the clipboard.
///
/// It is the receipt a customer can be sent rather than shown: a messaging app
/// is where a disputed total actually gets discussed, and "look at my screen"
/// does not travel. The order matches [`body`] so the two cannot tell different
/// stories — the difference is only that text must carry the change itself,
/// because there is no big green figure above it to do that job.
pub fn plain(detail: &SaleDetail, paper: &Paper<'_>, currency: Currency) -> String {
    let mut out = String::new();

    out.push_str(non_empty(paper.shop).unwrap_or(crate::APP_NAME));
    out.push('\n');
    if let Some(address) = non_empty(paper.address) {
        out.push_str(address);
        out.push('\n');
    }
    if let Some(phone) = non_empty(paper.phone) {
        out.push_str(&format!("Tel: {phone}\n"));
    }
    if let Some(tax) = non_empty(paper.tax_number) {
        out.push_str(&format!("Tax/VAT: {tax}\n"));
    }
    out.push('\n');
    let (date, time) = stamp_parts(&detail.created_at);
    push_pair(&mut out, &format!("Receipt #{}", detail.id), &date);
    let left = meta_left(&time, detail);
    let right = if paper.show_cashier {
        cashier(detail)
    } else {
        String::new()
    };
    push_pair(&mut out, &left, &right);
    out.push('\n');

    if detail.lines.is_empty() {
        out.push_str("No line items were recorded on this sale.\n");
    }
    for line in &detail.lines {
        out.push_str(&text_row(
            &item_label(line),
            &amount(line.total_minor, currency),
        ));
        out.push('\n');
    }

    out.push('\n');
    if detail.tax_minor > 0 {
        out.push_str(&text_row(
            "Pre-tax",
            &amount(detail.total_minor - detail.tax_minor, currency),
        ));
        out.push('\n');
        out.push_str(&text_row(
            &format!("Tax (@ {}%)", detail.tax_minor * 100 / detail.total_minor),
            &amount(detail.tax_minor, currency),
        ));
        out.push('\n');
    }
    if detail.rounding_minor != 0 {
        out.push_str(&text_row("Subtotal", &amount(detail.total_minor, currency)));
        out.push('\n');
        let direction = if detail.rounding_minor < 0 {
            "Rounded down"
        } else {
            "Rounded up"
        };
        out.push_str(&text_row(
            direction,
            &amount(detail.rounding_minor, currency),
        ));
        out.push('\n');
    }
    out.push_str(&text_row(
        &format!("Total ({})", currency.code()),
        &amount(detail.paid_minor, currency),
    ));
    out.push('\n');
    if detail.tendered_minor != 0 {
        out.push_str(&text_row(
            &format!("Paid by {}", detail.method),
            &amount(detail.tendered_minor, currency),
        ));
        out.push('\n');
    }
    if detail.change_minor != 0 {
        out.push_str(&text_row("Change", &amount(detail.change_minor, currency)));
        out.push('\n');
    }

    out.push('\n');
    if let Some(policy) = non_empty(paper.return_policy) {
        out.push_str(policy);
        out.push('\n');
    }
    out.push_str(paper.footer_text());
    out.push('\n');
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

/// A metadata row where either side may be absent.
///
/// A two-column header is only worth its line when both columns have something
/// to say; an empty right column would otherwise leave trailing spaces in the
/// copied text or an empty cell on screen.
fn text_pair(label: &str, value: &str) -> String {
    match (label.trim().is_empty(), value.trim().is_empty()) {
        (true, true) => String::new(),
        (true, false) => value.to_owned(),
        (false, true) => label.to_owned(),
        (false, false) => text_row(label, value),
    }
}

fn push_pair(out: &mut String, label: &str, value: &str) {
    let pair = text_pair(label, value);
    if !pair.is_empty() {
        out.push_str(&pair);
        out.push('\n');
    }
}

/// One line item: the quantity with the name, the line total on the right.
///
/// The count sits in the name rather than on a second line. On narrow paper a
/// `2 × 3,500` row under every multi-unit item doubles the height of the slip,
/// and the unit price is already derivable from the line total. The exception
/// belongs to the day the sale model can carry a discount or a weighed item:
/// then the second line explains a figure that no longer divides cleanly.
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
                components::numeric(amount(line.total_minor, currency))
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
                            egui::RichText::new(item_label(line))
                                .size(TYPE.body)
                                .color(text),
                        )
                        .wrap(),
                    );
                },
            );
        });
    });
    ui.add_space(SPACING.xs);
}

/// The item as it prints: the quantity, then the name.
///
/// The count is always shown, including `1×`, so the eye gets a consistent
/// quantity column down the slip. A lone item without a count makes the name
/// column shift left on every other row, which is exactly the misalignment a
/// narrow thermal receipt cannot afford.
///
/// The multiplication sign rather than `x` keeps the count visually separate
/// from a product whose name happens to start with a letter.
fn item_label(line: &DetailLine) -> String {
    format!("{}× {}", line.qty, line.name)
}

/// How many units are on the slip, not how many rows: a customer checking a
/// bag counts three things when one line says two flat whites and another says
/// one roll.
fn item_count(detail: &SaleDetail) -> i64 {
    detail.lines.iter().map(|line| line.qty).sum()
}

fn item_count_text(detail: &SaleDetail) -> String {
    let count = item_count(detail);
    if count == 1 {
        "1 item".to_owned()
    } else {
        format!("{count} items")
    }
}

/// The lower-left metadata cell: the time, then how many units were sold.
///
/// Either half can be missing on an old or hand-edited sale, so the cell
/// degrades to the half that exists rather than printing a separator around
/// nothing.
fn meta_left(time: &str, detail: &SaleDetail) -> String {
    let count = item_count(detail);
    match (time.is_empty(), count) {
        (true, 0) => String::new(),
        (true, _) => item_count_text(detail),
        (false, 0) => time.to_owned(),
        (false, _) => format!("{time} · {}", item_count_text(detail)),
    }
}

/// A date a customer can read without knowing the database's format. The till
/// stores ISO or `DD/MM/YYYY`; paper gets `2 Nov 2026, 14:03`, which cannot be
/// mistaken for a month-first date in a conversation about a return.
fn date_time(raw: &str) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];

    let (date, time) = raw.split_once(' ').unwrap_or((raw, ""));
    let (day, month, year) = if date.len() >= 10 && date.as_bytes().get(4) == Some(&b'-') {
        (&date[8..10], &date[5..7], &date[0..4])
    } else if date.len() >= 10 && date.as_bytes().get(2) == Some(&b'/') {
        (&date[0..2], &date[3..5], &date[6..10])
    } else {
        return raw.to_owned();
    };

    let (Ok(day), Ok(month)) = (day.parse::<u32>(), month.parse::<usize>()) else {
        return raw.to_owned();
    };
    if !(1..=12).contains(&month) {
        return raw.to_owned();
    }
    let month = MONTHS[month - 1];
    let time = time.get(0..5).unwrap_or("");

    if time.is_empty() {
        format!("{day} {month} {year}")
    } else {
        format!("{day} {month} {year}, {time}")
    }
}

/// The display date split where a compact header can put it: the day on the
/// receipt-number row, the clock on the item-count row.
fn stamp_parts(raw: &str) -> (String, String) {
    let stamp = date_time(raw);
    match stamp.split_once(", ") {
        Some((date, time)) => (date.to_owned(), time.to_owned()),
        None => (stamp, String::new()),
    }
}

/// `Some` only when a field would print something; a line of spaces is blank.
fn non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.is_empty() { None } else { Some(value) }
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

/// A compact two-column metadata line, such as the receipt number and date.
///
/// Fixed height for the same reason as [`row`]: a right-to-left child inside an
/// auto-sized receipt would otherwise stretch the line to the bottom of the
/// pane.
fn meta_row(ui: &mut egui::Ui, left: &str, right: &str) {
    let palette = theme::current(ui.ctx());
    let muted = egui::Color32::from(palette.text_muted);
    let left = egui::RichText::new(left).size(TYPE.small).color(muted);
    let right = egui::RichText::new(right).size(TYPE.small).color(muted);

    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), TYPE.small * 1.7),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.label(left);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(right);
            });
        },
    );
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
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.label(label_text);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(value_text);
            });
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

/// The amount as the receipt prints it: grouped, without the currency symbol.
///
/// Repeating `USh` on every line spends width on a narrow slip and tells the
/// customer what they already know. The currency is named once, on the total
/// row, and the figures beneath it stay a clean right-aligned column.
fn amount(minor: i64, currency: Currency) -> String {
    let scale = Money::new(minor, currency).scale() as usize;
    let sign = if minor < 0 { "-" } else { "" };
    let digits = minor.unsigned_abs().to_string();
    // Pad with leading zeroes so a sub-unit amount still has a whole part:
    // 5 KES cents is "0.05", not ".5".
    let digits = format!("{digits:0>width$}", width = scale + 1);
    let whole = &digits[..digits.len() - scale];
    let fraction = &digits[digits.len() - scale..];

    let mut grouped = String::with_capacity(whole.len() + whole.len() / 3);
    for (index, ch) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }

    if scale == 0 {
        format!("{sign}{grouped}")
    } else {
        format!("{sign}{grouped}.{fraction}")
    }
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
            tax_minor: 0,
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

    fn paper(shop: &str) -> Paper<'_> {
        Paper {
            shop,
            address: "",
            phone: "",
            tax_number: "",
            return_policy: "",
            footer: "",
            show_cashier: true,
        }
    }

    /// The clipboard slip has to be able to answer a customer on its own: who
    /// sold it, when, what it held, what it came to, and what went back. Any of
    /// those missing turns the copy into a list of numbers with no seller.
    #[test]
    fn the_text_receipt_carries_the_whole_slip() {
        let text = plain(&sale(), &paper("Nakawa Grocers"), Currency::UGX);

        assert!(text.starts_with("Nakawa Grocers\n"), "{text}");
        assert!(text.contains("Receipt #12"), "{text}");
        assert!(text.contains("2 Nov 2026"), "{text}");
        assert!(text.contains("14:03 · 3 items"), "{text}");
        assert!(text.contains("Cashier: Amina"), "{text}");
        assert!(text.contains("2× Flat white"), "{text}");
        assert!(text.contains("1× Cinnamon roll"), "{text}");
        assert!(text.contains("7,000"), "{text}");
        assert!(text.contains("12,500"), "{text}");
        assert!(text.contains("7,500"), "{text}");
        assert!(text.contains("Total (UGX)"), "{text}");
        assert!(!text.contains("USh"), "{text}");
        assert!(text.contains("Paid by cash"), "{text}");
        assert!(text.trim_end().ends_with("Thank you."), "{text}");
    }

    /// The order of the money block is the order on paper: the subtotal and the
    /// rounding explain the total, and the change comes after the total is read.
    #[test]
    fn the_text_receipt_explains_the_total_before_it_prints_the_change() {
        let text = plain(&sale(), &paper("Nakawa Grocers"), Currency::UGX);

        let subtotal = text.find("Subtotal").expect("subtotal line");
        let rounding = text.find("Rounded up").expect("rounding line");
        let total = text.find("Total").expect("total line");
        let change = text.find("Change").expect("change line");

        assert!(subtotal < rounding, "{text}");
        assert!(rounding < total, "{text}");
        assert!(total < change, "{text}");
    }

    /// A multi-unit line keeps its count in the name and spends no second line
    /// on the unit price. The customer checks the line total; a narrow slip has
    /// no height to spare repeating a figure that divides back out.
    #[test]
    fn a_multi_unit_line_prints_its_count_without_a_unit_price_line() {
        let text = plain(&sale(), &paper("Nakawa Grocers"), Currency::UGX);

        assert!(text.contains("2× Flat white"), "{text}");
        assert!(text.contains("1× Cinnamon roll"), "{text}");
        assert!(!text.contains("2 × 3,500"), "{text}");
        assert!(!text.contains("3,500"), "{text}");

        let item_lines = text
            .lines()
            .filter(|line| line.contains("Flat white") || line.contains("Cinnamon roll"))
            .count();
        assert_eq!(item_lines, 2, "one printed line per item: {text}");
    }

    #[test]
    fn a_single_item_uses_the_singular_count() {
        let mut one = sale();
        one.lines.truncate(1);
        one.lines[0].qty = 1;
        one.lines[0].total_minor = 3_500;

        let text = plain(&one, &paper("Nakawa Grocers"), Currency::UGX);

        assert!(text.contains("1 item"), "{text}");
        assert!(!text.contains("1 items"), "{text}");
        assert!(text.contains("1× Flat white"), "{text}");
    }

    #[test]
    fn a_date_reads_as_a_day_and_a_month_name() {
        assert_eq!(date_time("2026-11-02 14:03"), "2 Nov 2026, 14:03");
        assert_eq!(date_time("02/11/2026 14:03"), "2 Nov 2026, 14:03");
        assert_eq!(date_time("2026-11-02"), "2 Nov 2026");
        assert_eq!(date_time("not a date"), "not a date");
    }

    /// Metadata is two rows, not five: the receipt number and date share the
    /// first, the clock and item count share the second, and the cashier rides
    /// on the right of the second. That is the whole vertical saving on 58 mm.
    #[test]
    fn the_header_keeps_metadata_to_two_rows() {
        let text = plain(&sale(), &paper("Nakawa Grocers"), Currency::UGX);
        let header = text
            .lines()
            .skip_while(|line| !line.contains("Receipt #12"))
            .take(2)
            .collect::<Vec<_>>();

        assert_eq!(header.len(), 2, "{text}");
        assert!(header[0].contains("2 Nov 2026"), "{text}");
        assert!(header[1].contains("14:03 · 3 items"), "{text}");
        assert!(header[1].contains("Cashier: Amina"), "{text}");
    }

    /// The currency belongs on the total, not on every line. Figures are
    /// grouped so a six-figure shilling total can be read at a glance.
    #[test]
    fn amounts_are_grouped_and_the_currency_is_named_once() {
        let text = plain(&sale(), &paper("Nakawa Grocers"), Currency::UGX);

        assert!(text.contains("2× Flat white"), "{text}");
        assert!(text.contains("7,000"), "{text}");
        assert!(text.contains("Total (UGX)"), "{text}");
        assert_eq!(text.matches("UGX").count(), 1, "{text}");
        assert!(!text.contains("USh"), "{text}");
    }

    #[test]
    fn grouping_keeps_the_decimal_part_and_the_sign() {
        assert_eq!(amount(1_234_567, Currency::UGX), "1,234,567");
        assert_eq!(amount(123_456, Currency::KES), "1,234.56");
        assert_eq!(amount(5, Currency::KES), "0.05");
        assert_eq!(amount(-500, Currency::UGX), "-500");
    }

    /// The optional fields are the shop's, and the draft preview and the saved
    /// slip must print the same block. A blank field is absent, not an empty
    /// line with a label.
    #[test]
    fn the_text_receipt_prints_the_shop_details_and_closing_lines() {
        let receipt = ReceiptDetails {
            address: "12 Kampala Road".to_owned(),
            phone: "+256 700 123 456".to_owned(),
            tax_number: "TIN 1000123456".to_owned(),
            return_policy: "Exchanges within 7 days with this receipt.".to_owned(),
            footer: "Thank you for shopping with us.".to_owned(),
            show_cashier: true,
        };
        let text = plain(
            &sale(),
            &Paper::from_receipt("Nakawa Grocers", &receipt),
            Currency::UGX,
        );

        assert!(text.contains("12 Kampala Road"), "{text}");
        assert!(text.contains("Tel: +256 700 123 456"), "{text}");
        assert!(text.contains("Tax/VAT: TIN 1000123456"), "{text}");
        assert!(
            text.contains("Exchanges within 7 days with this receipt."),
            "{text}"
        );
        assert!(
            text.trim_end().ends_with("Thank you for shopping with us."),
            "{text}"
        );
    }

    #[test]
    fn a_hidden_cashier_is_not_printed() {
        let receipt = ReceiptDetails {
            show_cashier: false,
            ..ReceiptDetails::default()
        };
        let text = plain(
            &sale(),
            &Paper::from_receipt("Nakawa Grocers", &receipt),
            Currency::UGX,
        );

        assert!(!text.contains("Cashier:"), "{text}");
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
