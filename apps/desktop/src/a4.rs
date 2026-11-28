//! One sale on A4 paper.
//!
//! The receipt's [`crate::receipt::rows`] are the content; this module decides
//! where each row sits and asks `printing` for the PDF bytes. Keeping the two
//! apart means a shop can move its prices, its credit rules or its footer
//! without touching the page geometry, and a page-margin fix cannot change what
//! the slip says.
//!
//! A4 is the first paper size. Thermal 80 mm and an operating-system print
//! dialog are deliberately not here yet.

use std::io;
use std::path::Path;

use money::Currency;
use printing::{Align, Color, Document, MM_TO_PT, Page, Rect, Text};

use crate::db::SaleDetail;
use crate::receipt::{Paper, Row};

const LEFT: f64 = 15.0;
const RIGHT: f64 = 195.0;
const WIDTH: f64 = RIGHT - LEFT;
const CENTRE: f64 = (LEFT + RIGHT) / 2.0;
const BODY_BOTTOM: f64 = 256.0;

const INK: Color = Color::hex(0x0f_17_2a);
const MUTED: Color = Color::hex(0x64_70_80);
const LINE: Color = Color::hex(0xd8_de_e7);
const WASH: Color = Color::hex(0xf3_f6_f9);

/// One text style. Styles are values so a page can mix a quiet reference
/// number and a loud total without either renderer guessing at the other's
/// typography.
struct Style {
    size: f64,
    bold: bool,
    color: Color,
}

impl Style {
    fn put(&self, page: &mut Page, x_mm: f64, y_mm: f64, align: Align, text: &str) {
        page.push(Text {
            x_mm,
            y_mm,
            size_pt: self.size,
            bold: self.bold,
            align,
            text: text.to_owned(),
            color: self.color,
        });
    }

    fn width(&self, text: &str) -> f64 {
        printing::text_width(text, self.size, self.bold) / MM_TO_PT
    }

    fn wrap(&self, text: &str, max_mm: f64) -> Vec<String> {
        let mut lines = Vec::new();
        let mut current = String::new();
        for word in text.split_whitespace() {
            let candidate = if current.is_empty() {
                word.to_owned()
            } else {
                format!("{current} {word}")
            };
            if current.is_empty() || self.width(&candidate) <= max_mm {
                current = candidate;
            } else {
                lines.push(std::mem::take(&mut current));
                current = word.to_owned();
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
        if lines.is_empty() {
            lines.push(String::new());
        }
        lines
    }
}

/// Lay one sale out on A4 and return the PDF bytes.
///
/// `reprint` says the slip is being printed from the archive rather than handed
/// to the customer at the till. The credit block then carries a note, because
/// its balance is the balance now and not the balance on the sale date.
pub fn sale_pdf(
    detail: &SaleDetail,
    paper: &Paper<'_>,
    currency: Currency,
    reprint: bool,
    accent: Color,
) -> Vec<u8> {
    let note = reprint.then_some("as of printing, not the sale date");
    let rows = crate::receipt::rows(detail, paper, currency, note);
    // rows() ends with a blank before the policy and footer. Those belong at
    // the foot of every page, not wherever the last item happened to land.
    let split = rows
        .iter()
        .rposition(|row| matches!(row, Row::Blank))
        .unwrap_or(rows.len());
    let (body, tail) = rows.split_at(split);
    let footer: Vec<String> = tail
        .iter()
        .filter_map(|row| match row {
            Row::Centered(line) => Some(line.clone()),
            _ => None,
        })
        .collect();

    let title = Style {
        size: 19.0,
        bold: true,
        color: INK,
    };
    let small = Style {
        size: 8.5,
        bold: false,
        color: MUTED,
    };
    let body_style = Style {
        size: 9.5,
        bold: false,
        color: INK,
    };
    let strong = Style {
        size: 10.0,
        bold: true,
        color: INK,
    };
    let loud = Style {
        size: 11.0,
        bold: true,
        color: accent,
    };

    let mut pages = vec![Page::a4()];
    let mut y = 14.0;
    let mut i = 0;

    // Letterhead: one accent rule, the shop, and a quiet document label.
    pages[0].rect_filled(LEFT, y, WIDTH, 1.4, accent);
    y += 11.0;
    if let Some(Row::Shop(name)) = body.first() {
        for line in title.wrap(name, WIDTH) {
            title.put(&mut pages[0], CENTRE, y, Align::Center, &line);
            y += 7.0;
        }
        i = 1;
    }
    small.put(&mut pages[0], CENTRE, y + 1.0, Align::Center, "RECEIPT");
    y += 7.0;
    while i < body.len() {
        match &body[i] {
            Row::Centered(line) => {
                small.put(&mut pages[0], CENTRE, y, Align::Center, line);
                y += 4.6;
            }
            Row::Blank => y += 2.0,
            _ => break,
        }
        i += 1;
    }
    y += 2.0;
    pages[0].rule_colored(LEFT, y, RIGHT, y, accent);
    pages[0].rule_colored(LEFT, y + 0.6, RIGHT, y + 0.6, LINE);
    y += 5.0;

    // The metadata block shares a pale panel: receipt number, date, time,
    // cashier and customer are one fact each, not five lines of table.
    let meta_start = i;
    while matches!(body.get(i), Some(Row::Pair { .. })) {
        i += 1;
    }
    let meta = &body[meta_start..i];
    let panel_height = meta.len() as f64 * 5.0 + 5.0;
    pages[0].rect_filled(LEFT, y - 3.5, WIDTH, panel_height, WASH);
    for (index, row) in meta.iter().enumerate() {
        if let Row::Pair { label, value, .. } = row {
            small.put(
                &mut pages[0],
                LEFT + 3.0,
                y + index as f64 * 5.0,
                Align::Left,
                label,
            );
            if !value.is_empty() {
                body_style.put(
                    &mut pages[0],
                    RIGHT - 3.0,
                    y + index as f64 * 5.0,
                    Align::Right,
                    value,
                );
            }
        }
    }
    y += panel_height + 1.0;
    if matches!(body.get(i), Some(Row::Divider)) {
        i += 1;
    }

    // Item table: column heads, one row per line, hairlines between them.
    if y > BODY_BOTTOM {
        y = new_page(&mut pages, detail, accent);
    }
    small.put(pages.last_mut().unwrap(), LEFT, y, Align::Left, "ITEM");
    small.put(pages.last_mut().unwrap(), RIGHT, y, Align::Right, "AMOUNT");
    y += 2.0;
    pages
        .last_mut()
        .unwrap()
        .rule_colored(LEFT, y, RIGHT, y, LINE);
    y += 5.0;
    while i < body.len() && !matches!(body[i], Row::Divider) {
        let line_height = match &body[i] {
            Row::Pair { label, value, .. } => {
                body_style
                    .wrap(label, WIDTH - body_style.width(value) - 3.0)
                    .len() as f64
                    * 5.0
            }
            Row::Bare(text) => body_style.wrap(text, WIDTH).len() as f64 * 5.0,
            _ => 5.0,
        };
        if y + line_height > BODY_BOTTOM {
            y = new_page(&mut pages, detail, accent);
        }
        draw_row(
            pages.last_mut().unwrap(),
            &body[i],
            y,
            &body_style,
            &small,
            LEFT + 3.0,
        );
        y += line_height.max(5.0);
        pages
            .last_mut()
            .unwrap()
            .rule_colored(LEFT, y - 2.0, RIGHT, y - 2.0, LINE);
        i += 1;
    }
    if matches!(body.get(i), Some(Row::Divider)) {
        i += 1;
        y += 5.0;
    }

    // Money, tender and credit. The total gets a tinted panel in the shop's
    // accent; the credit block gets a left bar so it reads as a callout.
    let mut payment_heading = false;
    while i < body.len() {
        let row = &body[i];
        let height = match row {
            Row::Pair {
                label,
                value,
                strong: is_total,
            } => {
                if *is_total {
                    12.0
                } else {
                    let style = if label.starts_with("Paid by") {
                        &strong
                    } else {
                        &body_style
                    };
                    style.wrap(label, WIDTH - style.width(value) - 3.0).len() as f64 * 5.5
                }
            }
            Row::Credit { note, .. } => 22.0 + if note.is_some() { 5.0 } else { 0.0 },
            Row::Blank => 3.0,
            Row::Divider => 5.0,
            _ => 5.0,
        };
        if y + height > BODY_BOTTOM {
            y = new_page(&mut pages, detail, accent);
        }
        match row {
            Row::Pair {
                label,
                value,
                strong: is_total,
            } => {
                if label.starts_with("Paid by") && !payment_heading {
                    payment_heading = true;
                    y += 3.0;
                    small.put(pages.last_mut().unwrap(), LEFT, y, Align::Left, "PAYMENT");
                    y += 5.0;
                }
                if *is_total {
                    let page = pages.last_mut().unwrap();
                    page.push(Rect {
                        x_mm: LEFT,
                        y_mm: y - 4.0,
                        width_mm: WIDTH,
                        height_mm: 10.0,
                        width_pt: 0.0,
                        fill: Some(WASH),
                        stroke: None,
                    });
                    page.rect_filled(LEFT, y - 4.0, 1.2, 10.0, accent);
                    loud.put(page, LEFT + 4.0, y + 2.0, Align::Left, label);
                    loud.put(page, RIGHT - 3.0, y + 2.0, Align::Right, value);
                } else {
                    draw_row(pages.last_mut().unwrap(), row, y, &body_style, &small, LEFT);
                }
            }
            Row::Credit {
                settled,
                balance,
                note,
            } => {
                let page = pages.last_mut().unwrap();
                // Air above the panel: the row before it has a baseline here.
                y += 3.0;
                let height = 22.0 + if note.is_some() { 5.0 } else { 0.0 };
                page.push(Rect {
                    x_mm: LEFT,
                    y_mm: y - 3.0,
                    width_mm: WIDTH,
                    height_mm: height,
                    width_pt: 0.0,
                    fill: Some(WASH),
                    stroke: None,
                });
                page.rect_filled(LEFT, y - 3.0, 1.2, height, accent);
                loud.put(page, LEFT + 4.0, y + 2.0, Align::Left, "CREDIT SALE");
                small.put(page, LEFT + 4.0, y + 9.0, Align::Left, "Settled so far");
                body_style.put(page, RIGHT - 3.0, y + 9.0, Align::Right, settled);
                strong.put(page, LEFT + 4.0, y + 14.0, Align::Left, "Balance due");
                loud.put(page, RIGHT - 3.0, y + 14.0, Align::Right, balance);
                if let Some(note) = note {
                    small.put(page, LEFT + 4.0, y + 19.0, Align::Left, note);
                }
            }
            Row::Blank => y += 3.0,
            Row::Divider => {
                pages
                    .last_mut()
                    .unwrap()
                    .rule_colored(LEFT, y, RIGHT, y, LINE);
                y += 5.0;
            }
            _ => {}
        }
        y += match row {
            Row::Pair { strong: true, .. } => 12.0,
            Row::Credit { note, .. } => 22.0 + if note.is_some() { 5.0 } else { 0.0 },
            Row::Blank => 3.0,
            Row::Divider => 5.0,
            _ => 5.5,
        };
        i += 1;
    }

    // Footer on every page, with the page count a reader can check.
    let total = pages.len();
    for (index, page) in pages.iter_mut().enumerate() {
        page.rule_colored(LEFT, 268.0, RIGHT, 268.0, LINE);
        for (line, text) in footer.iter().enumerate() {
            small.put(page, CENTRE, 275.0 + line as f64 * 5.0, Align::Center, text);
        }
        small.put(
            page,
            RIGHT,
            289.0,
            Align::Right,
            &format!("Page {} of {}", index + 1, total),
        );
    }

    let mut pages = pages.into_iter();
    let mut document = Document::new(pages.next().expect("at least one page"));
    for page in pages {
        document.push(page);
    }
    document.to_pdf()
}

/// A continuation page keeps the receipt number and a light header rule.
fn new_page(pages: &mut Vec<Page>, detail: &SaleDetail, accent: Color) -> f64 {
    let mut page = Page::a4();
    page.rect_filled(LEFT, 14.0, WIDTH, 1.4, accent);
    let label = detail
        .receipt_no
        .clone()
        .unwrap_or_else(|| format!("#{}", detail.id));
    Style {
        size: 8.5,
        bold: false,
        color: MUTED,
    }
    .put(
        &mut page,
        LEFT,
        26.0,
        Align::Left,
        &format!("{label} — continued"),
    );
    page.rule_colored(LEFT, 30.0, RIGHT, 30.0, LINE);
    pages.push(page);
    38.0
}

fn draw_row(page: &mut Page, row: &Row, y: f64, style: &Style, muted: &Style, x: f64) {
    match row {
        Row::Pair { label, value, .. } => {
            style.put(page, x, y + 4.0, Align::Left, label);
            if !value.is_empty() {
                style.put(page, RIGHT - 3.0, y + 4.0, Align::Right, value);
            }
        }
        Row::Bare(text) => muted.put(page, x, y + 4.0, Align::Left, text),
        _ => {}
    }
}

/// A filename a shop can find again: the printed receipt number, with anything
/// a filesystem might object to replaced.
pub fn file_name(detail: &SaleDetail) -> String {
    let printed = detail
        .receipt_no
        .clone()
        .unwrap_or_else(|| format!("#{}", detail.id));
    let safe: String = printed
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    format!("receipt-{safe}.pdf")
}

/// Open the saved file in the computer's own PDF viewer. A machine without one
/// still keeps the file; only the convenience of opening it is lost.
pub fn open_in_viewer(path: &Path) -> io::Result<()> {
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("cmd")
        .args(["/C", "start", ""])
        .arg(path)
        .status();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(path).status();
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(path).status();

    result.and_then(|status| {
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other("the PDF viewer could not open the file"))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{DetailLine, PaidBy};
    use money::Currency;

    const ACCENT: Color = Color::hex(0x0f_76_6e);

    fn paper() -> Paper<'static> {
        Paper {
            shop: "Nakawa Grocers",
            address: "12 Main Rd, Kampala",
            phone: "0700 123 456",
            tax_number: "TIN 1000123456",
            return_policy: "Exchanges within 7 days with this receipt.",
            footer: "Thank you for shopping with us.",
            show_cashier: true,
        }
    }

    fn sale() -> SaleDetail {
        let mut detail = crate::receipt::sample();
        detail.receipt_no = Some("18CA-20261102-042".to_owned());
        detail
    }

    fn content(pdf: &[u8]) -> String {
        String::from_utf8_lossy(pdf)
            .split("\nstream\n")
            .skip(1)
            .map(|chunk| chunk.split("endstream").next().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn a_sale_prints_on_one_a4_page() {
        let pdf = sale_pdf(&sale(), &paper(), Currency::UGX, false, ACCENT);
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/MediaBox [0 0 595.276 841.890]"));
        assert!(text.contains("/Count 1"));
        assert!(content(&pdf).contains("Nakawa Grocers"));
    }

    #[test]
    fn the_layout_uses_an_accent_colour_and_table_headings() {
        let content = content(&sale_pdf(&sale(), &paper(), Currency::UGX, false, ACCENT));
        assert!(content.contains("0.059 0.463 0.431 rg"), "{content}");
        assert!(content.contains("RECEIPT"), "{content}");
        assert!(content.contains("ITEM"), "{content}");
        assert!(content.contains("AMOUNT"), "{content}");
        assert!(content.contains("Page 1 of 1"), "{content}");
    }

    #[test]
    fn a_credit_sale_is_marked_and_shows_both_amounts() {
        let mut detail = sale();
        detail.is_on_credit = true;
        detail.settled_minor = 2_790;
        detail.balance_minor = 12_000;
        let content = content(&sale_pdf(&detail, &paper(), Currency::UGX, true, ACCENT));

        assert!(content.contains("CREDIT SALE"), "{content}");
        assert!(content.contains("Settled so far"), "{content}");
        assert!(content.contains("2,790"), "{content}");
        assert!(content.contains("12,000"), "{content}");
        assert!(content.contains("as of printing"), "{content}");
    }

    #[test]
    fn a_settled_credit_sale_is_still_marked() {
        let mut detail = sale();
        detail.is_on_credit = true;
        detail.settled_minor = detail.paid_minor;
        detail.balance_minor = 0;
        let content = content(&sale_pdf(&detail, &paper(), Currency::UGX, false, ACCENT));
        assert!(content.contains("CREDIT SALE"));
        assert!(content.contains("Balance due"));
        assert!(!content.contains("as of printing"));
    }

    #[test]
    fn a_long_sale_continues_on_a_second_page() {
        let mut detail = sale();
        detail.lines = (0..60)
            .map(|i| DetailLine {
                name: format!("Item {i}"),
                qty: 1,
                unit_minor: 1_000,
                total_minor: 1_000,
                unit_factor: 1,
                unit_label: None,
            })
            .collect();
        let pdf = sale_pdf(&detail, &paper(), Currency::UGX, false, ACCENT);
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/Count 2"), "{text}");
        let content = content(&pdf);
        assert!(content.contains("continued"), "{content}");
        assert!(content.contains("Page 1 of 2"), "{content}");
        assert!(content.contains("Page 2 of 2"), "{content}");
    }

    #[test]
    fn two_tenders_and_a_reference_stay_on_the_pdf() {
        let mut detail = sale();
        detail.payments = vec![
            PaidBy {
                method: "cash".to_owned(),
                amount_minor: 4_000,
                tendered_minor: 5_000,
                change_minor: 1_000,
                reference: None,
            },
            PaidBy {
                method: "mtn_momo".to_owned(),
                amount_minor: 8_750,
                tendered_minor: 8_750,
                change_minor: 0,
                reference: Some("TXN-4471".to_owned()),
            },
        ];
        let text = content(&sale_pdf(&detail, &paper(), Currency::UGX, false, ACCENT));
        assert!(text.contains("Paid by Cash"), "{text}");
        assert!(text.contains("Paid by MTN MoMo"), "{text}");
        assert!(text.contains("TXN-4471"), "{text}");
        assert!(text.contains("PAYMENT"), "{text}");
    }

    /// Write a real sale to a file for `pdfinfo` / `pdftoppm`; no printer needed.
    /// Ignored by the normal suite because it writes outside the test directory.
    #[test]
    #[ignore = "writes a sample PDF to /tmp/opencode"]
    fn dump_a4_sample() {
        let mut detail = sale();
        detail.is_on_credit = true;
        detail.settled_minor = 2_790;
        detail.balance_minor = 12_000;
        std::fs::write(
            "/tmp/opencode/pos-a4-receipt.pdf",
            sale_pdf(&detail, &paper(), Currency::UGX, true, ACCENT),
        )
        .expect("sample PDF");
    }

    #[test]
    fn the_filename_uses_the_printed_number_and_is_safe() {
        let mut detail = sale();
        assert_eq!(file_name(&detail), "receipt-18CA-20261102-042.pdf");
        detail.receipt_no = Some("18/CA 42".to_owned());
        assert_eq!(file_name(&detail), "receipt-18-CA-42.pdf");
    }
}
