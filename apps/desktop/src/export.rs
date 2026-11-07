//! Writing the books out as CSV.
//!
//! The import this shop already trusts reads `name, category, price, barcode`,
//! so the catalog export writes exactly that: a file that went out of the till
//! can come back in without a spreadsheet in the middle. Sales are for the
//! owner's accountant, not for re-import, and name their columns the way the
//! ledger does.
//!
//! Amounts are plain decimals with no symbol and no grouping: `1200` for UGX,
//! `12.50` for KES. That is the only spelling both a spreadsheet and
//! [`crate::import::parse`] read back as the same number. It duplicates a few
//! lines of `money`'s display formatting on purpose — promoting that to
//! `stride-libs` would be a PR there plus a rev bump in both products, and the
//! export needs a decimal, not a price tag.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use money::{Currency, Money};

use crate::db::{ExportSale, Product};

/// An amount as a plain decimal the importer can read back.
///
/// `1200` for UGX, `12.50` for KES. No symbol, no grouping separators, and a
/// leading `-` when the rounding adjustment went the other way.
pub fn decimal(minor: i64, currency: Currency) -> String {
    let scale = Money::new(0, currency).scale();
    let sign = if minor < 0 { "-" } else { "" };
    let magnitude = minor.unsigned_abs();

    if scale == 0 {
        format!("{sign}{magnitude}")
    } else {
        let factor = 10_u64.pow(scale);
        format!(
            "{sign}{}.{:0width$}",
            magnitude / factor,
            magnitude % factor,
            width = scale as usize
        )
    }
}

/// One CSV field, quoted only when it has to be (RFC 4180).
fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

/// A row of fields, comma-separated, with a newline of its own.
fn csv_row(fields: impl IntoIterator<Item = String>) -> String {
    let mut row = fields
        .into_iter()
        .map(|field| csv_field(&field))
        .collect::<Vec<_>>()
        .join(",");
    row.push('\n');
    row
}

/// The catalog, in the column order [`crate::import::parse`] assumes.
///
/// Empty input still writes the header: a file that says what it would have
/// contained is a usable file, and an empty one is a mystery.
pub fn products_csv(products: &[Product], currency: Currency) -> String {
    let mut text = csv_row(["name", "category", "price", "barcode"].map(str::to_owned));
    for product in products {
        text.push_str(&csv_row([
            product.name.clone(),
            product.category.clone(),
            decimal(product.price_minor, currency),
            product.barcode.clone().unwrap_or_default(),
        ]));
    }
    text
}

/// A starting point for the import: the same columns as [`products_csv`], with
/// two example rows to replace.
///
/// Built from [`products_csv`] rather than written out by hand, so a change to
/// the columns cannot leave the template behind. The prices are the export's
/// own decimal spelling, which is exactly what the importer reads back.
pub fn template_csv(currency: Currency) -> String {
    products_csv(
        &[
            Product {
                id: 0,
                name: "Sugar 1 kg".to_owned(),
                category: "Groceries".to_owned(),
                category_id: None,
                price_minor: 4_500,
                barcode: None,
            },
            Product {
                id: 0,
                name: "Bread".to_owned(),
                category: "Bakery".to_owned(),
                category_id: None,
                price_minor: 3_400,
                barcode: None,
            },
        ],
        currency,
    )
}

/// Every sale in `rows`, one line each, header first.
pub fn sales_csv(rows: &[ExportSale], currency: Currency) -> String {
    let mut text = csv_row(
        [
            "sale_id", "when", "cashier", "total", "rounding", "paid", "method",
        ]
        .map(str::to_owned),
    );
    for sale in rows {
        text.push_str(&csv_row([
            sale.id.to_string(),
            sale.created_at.clone(),
            sale.cashier.clone().unwrap_or_default(),
            decimal(sale.total_minor, currency),
            decimal(sale.rounding_minor, currency),
            decimal(sale.paid_minor, currency),
            sale.method.clone(),
        ]));
    }
    text
}

/// Write `body` into `dir`, creating it if needed, and say where it landed.
///
/// UTF-8 with no byte-order mark: the exporter's own import expects plain text,
/// and a BOM would make the first heading `\u{feff}name` and hide the column.
pub fn write(dir: &Path, name: &str, body: &str) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(dir)
        .with_context(|| format!("creating export directory {}", dir.display()))?;
    let path = dir.join(name);
    std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ugx() -> Currency {
        money::tender_from_code("UGX").expect("UGX is a tender")
    }

    fn kes() -> Currency {
        money::tender_from_code("KES").expect("KES is a tender")
    }

    fn product(name: &str, category: &str, price_minor: i64, barcode: Option<&str>) -> Product {
        Product {
            id: 0,
            name: name.to_owned(),
            category: category.to_owned(),
            category_id: None,
            price_minor,
            barcode: barcode.map(str::to_owned),
        }
    }

    #[test]
    fn a_field_is_quoted_only_when_it_has_to_be() {
        assert_eq!(csv_field("Bread"), "Bread");
        assert_eq!(csv_field("Bread, white"), "\"Bread, white\"");
        assert_eq!(csv_field("12\" loaf"), "\"12\"\" loaf\"");
        assert_eq!(csv_field("two\nlines"), "\"two\nlines\"");
    }

    #[test]
    fn a_decimal_reads_back_as_the_amount_it_came_from() {
        for (minor, currency) in [(1200, ugx()), (1250, kes()), (-60, ugx()), (5, kes())] {
            let text = decimal(minor, currency);
            assert_eq!(
                Money::parse(&text, currency).expect("parses").minor(),
                minor,
                "{text} did not round-trip"
            );
        }
        assert_eq!(decimal(1200, ugx()), "1200");
        assert_eq!(decimal(1250, kes()), "12.50");
        assert_eq!(decimal(-60, ugx()), "-60");
    }

    #[test]
    fn the_catalog_file_keeps_awkward_names_and_blank_barcodes() {
        let products = [
            product("Bread, white", "Bakery", 3_400, Some("0123")),
            product("Milk", "Dairy", 5_000, None),
        ];

        let text = products_csv(&products, ugx());
        assert_eq!(
            text,
            "name,category,price,barcode\n\
             \"Bread, white\",Bakery,3400,0123\n\
             Milk,Dairy,5000,\n"
        );
    }

    #[test]
    fn an_empty_catalog_still_names_its_columns() {
        assert_eq!(products_csv(&[], ugx()), "name,category,price,barcode\n");
    }

    #[test]
    fn an_empty_range_writes_a_sales_header_and_nothing_else() {
        assert_eq!(
            sales_csv(&[], kes()),
            "sale_id,when,cashier,total,rounding,paid,method\n"
        );
    }

    #[test]
    fn a_sale_row_carries_the_amounts_the_ledger_holds() {
        let rows = [ExportSale {
            id: 7,
            created_at: "2026-11-01 18:42:12".to_owned(),
            cashier: None,
            total_minor: 340,
            rounding_minor: 60,
            paid_minor: 400,
            method: "cash".to_owned(),
        }];

        assert_eq!(
            sales_csv(&rows, ugx()),
            "sale_id,when,cashier,total,rounding,paid,method\n\
             7,2026-11-01 18:42:12,,340,60,400,cash\n"
        );
    }

    #[test]
    fn the_template_is_a_file_the_import_reads_back() {
        let text = template_csv(kes());

        assert!(
            text.starts_with("name,category,price,barcode\n"),
            "the template must lead with the columns the import expects: {text}"
        );
        assert!(
            text.contains("Sugar 1 kg,Groceries,45.00,\n"),
            "the example price must be the export's own spelling: {text}"
        );

        let (rows, errors) = crate::import::parse(&text, kes());

        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].name, "Bread");
        assert_eq!(rows[1].category, "Bakery");
        assert_eq!(rows[1].price_minor, 3_400);
        assert_eq!(rows[1].barcode, None);
    }

    #[test]
    fn a_catalog_export_imports_back_without_losing_a_row() {
        let products = [
            product("Bread, white", "Bakery", 3_400, Some("0123")),
            product("Milk", "Dairy", 5_000, None),
            product("Rice", "General", 12_500, Some("4001")),
        ];

        let (rows, errors) = crate::import::parse(&products_csv(&products, ugx()), ugx());

        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].name, "Bread, white");
        assert_eq!(rows[0].price_minor, 3_400);
        assert_eq!(rows[0].barcode.as_deref(), Some("0123"));
        assert_eq!(rows[2].category, "General");
        assert_eq!(rows[2].price_minor, 12_500);
        assert_eq!(rows[2].barcode.as_deref(), Some("4001"));
    }
}
