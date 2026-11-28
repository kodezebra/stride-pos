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
use printing::{Align, Document, MM_TO_PT, Page, text_width};

use crate::db::SaleDetail;
use crate::receipt::{Paper, Row};

const LEFT: f64 = 15.0;
const RIGHT: f64 = 195.0;
const WIDTH: f64 = RIGHT - LEFT;
const CENTRE: f64 = (LEFT + RIGHT) / 2.0;
const LAST_Y: f64 = 270.0;

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
) -> Vec<u8> {
    let note = reprint.then_some("as of printing, not the sale date");
    let rows = crate::receipt::rows(detail, paper, currency, note);
    let mut pages = vec![Page::a4()];
    let mut y = 18.0;

    for row in rows {
        let height = match &row {
            Row::Shop(line) => wrap(line, WIDTH, 16.0, true).len() as f64 * 7.0 + 2.0,
            Row::Centered(line) => wrap(line, WIDTH, 9.5, false).len() as f64 * 4.6,
            Row::Pair { label, value, .. } => {
                let size = pair_size(label, value);
                let value_mm = text_width(value, size, false) / MM_TO_PT;
                wrap(label, (WIDTH - value_mm - 2.0).max(20.0), size, false).len() as f64 * 5.0
            }
            Row::Bare(line) => wrap(line, WIDTH, 9.5, false).len() as f64 * 5.0,
            Row::Blank => 3.0,
            Row::Divider => 5.0,
            Row::Credit { note, .. } => 22.0 + if note.is_some() { 5.0 } else { 0.0 },
        };
        if y + height > LAST_Y {
            let mut next = Page::a4();
            let label = detail
                .receipt_no
                .clone()
                .unwrap_or_else(|| format!("#{}", detail.id));
            next.text(
                LEFT,
                18.0,
                8.0,
                false,
                Align::Left,
                &format!("{label} — continued"),
            );
            next.rule(LEFT, 23.0, RIGHT, 23.0);
            pages.push(next);
            y = 30.0;
        }

        let page = pages.last_mut().expect("at least one page");
        match row {
            Row::Shop(line) => {
                for text in wrap(&line, WIDTH, 16.0, true) {
                    page.text(CENTRE, y + 5.0, 16.0, true, Align::Center, &text);
                    y += 7.0;
                }
                y += 2.0;
            }
            Row::Centered(line) => {
                for text in wrap(&line, WIDTH, 9.5, false) {
                    page.text(CENTRE, y + 3.5, 9.5, false, Align::Center, &text);
                    y += 4.6;
                }
            }
            Row::Pair {
                label,
                value,
                strong,
            } => {
                let size = pair_size(&label, &value);
                let value_mm = text_width(&value, size, false) / MM_TO_PT;
                let lines = wrap(&label, (WIDTH - value_mm - 2.0).max(20.0), size, false);
                for (i, text) in lines.iter().enumerate() {
                    page.text(LEFT, y + 4.0, size, strong, Align::Left, text);
                    y += 5.0;
                    if i == 0 && !value.is_empty() {
                        page.text(RIGHT, y - 1.0, size, strong, Align::Right, &value);
                    }
                }
            }
            Row::Bare(line) => {
                for text in wrap(&line, WIDTH, 9.5, false) {
                    page.text(LEFT, y + 4.0, 9.5, false, Align::Left, &text);
                    y += 5.0;
                }
            }
            Row::Blank => y += 3.0,
            Row::Divider => {
                page.rule(LEFT, y, RIGHT, y);
                y += 5.0;
            }
            Row::Credit {
                settled,
                balance,
                note,
            } => {
                let height = 22.0 + if note.is_some() { 5.0 } else { 0.0 };
                page.rect(LEFT, y - 2.0, WIDTH, height);
                page.text(CENTRE, y + 4.0, 11.0, true, Align::Center, "CREDIT SALE");
                page.text(LEFT, y + 10.0, 9.5, false, Align::Left, "Settled so far");
                page.text(RIGHT, y + 10.0, 9.5, false, Align::Right, &settled);
                page.text(LEFT, y + 15.0, 10.0, true, Align::Left, "Balance due");
                page.text(RIGHT, y + 15.0, 10.0, true, Align::Right, &balance);
                if let Some(note) = note {
                    page.text(CENTRE, y + 20.0, 8.0, false, Align::Center, &note);
                }
                y += height + 4.0;
            }
        }
    }

    let mut pages = pages.into_iter();
    let mut document = Document::new(pages.next().expect("at least one page"));
    for page in pages {
        document.push(page);
    }
    document.to_pdf()
}

/// A size that lets the label and the figure share one line when they can.
fn pair_size(label: &str, value: &str) -> f64 {
    for size in [10.0, 9.0, 8.0] {
        if (text_width(label, size, false) + text_width(value, size, false)) / MM_TO_PT
            <= WIDTH - 2.0
        {
            return size;
        }
    }
    8.0
}

fn wrap(text: &str, max_mm: f64, size_pt: f64, bold: bool) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let candidate = if current.is_empty() {
            word.to_owned()
        } else {
            format!("{current} {word}")
        };
        if current.is_empty() || text_width(&candidate, size_pt, bold) / MM_TO_PT <= max_mm {
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

    fn paper() -> Paper<'static> {
        Paper {
            shop: "Nakawa Grocers",
            address: "",
            phone: "",
            tax_number: "",
            return_policy: "",
            footer: "",
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
        let pdf = sale_pdf(&sale(), &paper(), Currency::UGX, false);
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/MediaBox [0 0 595.276 841.890]"));
        assert!(text.contains("/Count 1"));
        assert!(content(&pdf).contains("Nakawa Grocers"));
    }

    #[test]
    fn a_credit_sale_is_marked_and_shows_both_amounts() {
        let mut detail = sale();
        detail.is_on_credit = true;
        detail.settled_minor = 2_790;
        detail.balance_minor = 12_000;
        let content = content(&sale_pdf(&detail, &paper(), Currency::UGX, true));

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
        let content = content(&sale_pdf(&detail, &paper(), Currency::UGX, false));
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
        let pdf = sale_pdf(&detail, &paper(), Currency::UGX, false);
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/Count 2"), "{text}");
        assert!(content(&pdf).contains("continued"));
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
        let text = content(&sale_pdf(&detail, &paper(), Currency::UGX, false));
        assert!(text.contains("Paid by Cash"), "{text}");
        assert!(text.contains("Paid by MTN MoMo"), "{text}");
        assert!(text.contains("TXN-4471"), "{text}");
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
            sale_pdf(&detail, &paper(), Currency::UGX, true),
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
