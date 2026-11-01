//! Importing products from a CSV file.
//!
//! The catalog is the shop's, typed in once with real prices — nobody retypes
//! hundreds of rows through a small form. A spreadsheet saved from Excel or a
//! file from a supplier imports the same products, and anything a row says
//! that the till cannot read is skipped and counted, never half-imported.
//!
//! Rows are flexible rather than a fixed dialect: a file with the shop's own
//! column headings (`product_name`, `category_name`, `retail_price_ugx`,
//! `barcode`) and a plain `name, category, price, barcode` file both work.
//! Weights, cost prices and units are columns the till cannot price yet; they
//! are ignored, not a reason to refuse the rest of the file.

use money::{Currency, Money};

/// One product row ready for the catalog.
pub struct Row {
    pub name: String,
    /// Never empty: `General` is the till's answer to a blank cell, matching
    /// what the product form does when its category is left out.
    pub category: String,
    /// Read with the till's currency, exactly the way the product form reads
    /// its price. The file is paid for in whatever currency the till prices,
    /// and the import caption says so.
    pub price_minor: i64,
    pub barcode: Option<String>,
}

/// Parse CSV text into importable rows plus per-line errors.
///
/// Returns the rows the till will actually take and one message per line it
/// could not, so an operator learns "9 added, 2 skipped — line 3: the price
/// is not a number" instead of watching a parser refuse the whole file.
pub fn parse(text: &str, currency: Currency) -> (Vec<Row>, Vec<String>) {
    let Some(first) = text.lines().next() else {
        return (Vec::new(), vec!["The file is empty.".to_owned()]);
    };

    let first_fields = split_line(first);
    // A first row made of recognized headings is a header; anything else is
    // already data, and the plain four-column order is assumed.
    let is_header = first_fields.iter().any(|field| is_heading(field));
    let columns = if is_header {
        columns_from_headings(&first_fields)
    } else {
        Columns::plain()
    };

    let mut rows = Vec::new();
    let mut errors = Vec::new();

    for (index, raw) in text.lines().enumerate() {
        if index == 0 && is_header {
            continue;
        }
        let number = index + 1;
        let fields = if index == 0 {
            first_fields.clone()
        } else {
            split_line(raw)
        };
        if fields.iter().all(|field| field.trim().is_empty()) {
            continue;
        }

        let name = cell(&fields, columns.name);
        if name.is_empty() {
            errors.push(format!("line {number}: the product needs a name."));
            continue;
        }

        let mut category = cell(&fields, columns.category);
        if category.is_empty() {
            category = "General".to_owned();
        }

        let price_text = cell(&fields, columns.price);
        let price_minor = match Money::parse(&price_text, currency) {
            Ok(price) if price.minor() >= 0 => price.minor(),
            Ok(_) => {
                errors.push(format!("line {number}: the price cannot be negative."));
                continue;
            }
            Err(error) => {
                errors.push(format!("line {number}: {error}"));
                continue;
            }
        };

        let barcode = columns
            .barcode
            .map(|column| cell(&fields, column))
            .filter(|code| !code.is_empty());

        rows.push(Row {
            name: name.to_owned(),
            category,
            price_minor,
            barcode,
        });
    }

    (rows, errors)
}

/// Where each field lives, by column number.
struct Columns {
    name: usize,
    category: usize,
    price: usize,
    /// `None` when there is no barcode column at all, so a file without one
    /// imports cleanly rather than reading a name as a barcode.
    barcode: Option<usize>,
}

impl Columns {
    /// Columns of a file with no heading row.
    fn plain() -> Self {
        Columns {
            name: 0,
            category: 1,
            price: 2,
            barcode: Some(3),
        }
    }
}

/// A heading we accept, matched without case so Excel's "PRODUCT NAME" works.
fn is_heading(field: &str) -> bool {
    const HEADINGS: &[&str] = &[
        "name",
        "product_name",
        "category",
        "category_name",
        "price",
        "retail_price",
        "retail_price_ugx",
        "barcode",
    ];
    HEADINGS
        .iter()
        .any(|heading| field.eq_ignore_ascii_case(heading))
}

/// Map headings to columns, falling back to the plain order per column when a
/// heading is not one we know, so an unknown file keeps importing as best it can.
fn columns_from_headings(headings: &[String]) -> Columns {
    let position = |candidates: &[&str]| {
        headings.iter().position(|heading| {
            candidates
                .iter()
                .any(|name| heading.eq_ignore_ascii_case(name))
        })
    };
    Columns {
        name: position(&["product_name", "name"]).unwrap_or(0),
        category: position(&["category_name", "category"]).unwrap_or(1),
        price: position(&["retail_price_ugx", "retail_price", "price"]).unwrap_or(2),
        barcode: position(&["barcode"]),
    }
}

/// The text of cell `index`, trimmed; a short line reads as an empty cell.
fn cell(fields: &[String], index: usize) -> String {
    fields
        .get(index)
        .map(|field| field.trim().to_owned())
        .unwrap_or_default()
}

/// Split one CSV line on commas, honouring double quotes. `""` inside a quoted
/// field is a literal quote; a comma inside quotes never splits. An
/// unterminated quote keeps the rest of the line, which is what a spreadsheet
/// writes when a pasted cell happened to contain a quote.
pub fn split_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();

    while let Some(character) = chars.next() {
        match character {
            '"' if quoted => {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            }
            '"' => quoted = true,
            ',' if !quoted => fields.push(std::mem::take(&mut field)),
            character => field.push(character),
        }
    }
    fields.push(field);
    fields
}

#[cfg(test)]
mod tests {
    use super::*;
    use money::Country;

    fn currency(country: Country) -> Currency {
        money::tender_for(country).expect("East Africa always has a currency")
    }

    /// The header row of a file built from the shop's catalog spreadsheet.
    const SHOP: &str = "category_code,category_name,product_name,unit_of_measure,\
cost_price_ugx,retail_price_ugx,is_weighted,barcode";

    fn ugx() -> Currency {
        currency(Country::UG)
    }

    #[test]
    fn reads_the_shop_csv_by_its_headings() {
        let text =
            format!("{SHOP}\n01,Grains & Oils,\"Posho mill, 1kg\",PC,4500,5000,,770208000001");
        let (rows, errors) = parse(&text, ugx());

        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Posho mill, 1kg");
        assert_eq!(rows[0].category, "Grains & Oils");
        assert_eq!(rows[0].price_minor, 5000, "UGX has no minor decimals");
        assert_eq!(rows[0].barcode.as_deref(), Some("770208000001"));
    }

    #[test]
    fn reads_a_plain_four_column_file() {
        let text = "name,category,price,barcode\nOmo 500g,Cleaning,3500";
        let (rows, errors) = parse(text, ugx());

        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Omo 500g");
        assert_eq!(rows[0].category, "Cleaning");
        assert_eq!(rows[0].price_minor, 3500);
        assert_eq!(rows[0].barcode, None);
    }

    #[test]
    fn prices_follow_the_tills_currency_not_the_column_name() {
        // The column is named for Uganda, but a Nairobi till prices in KES:
        // the number is read the same way the product form reads it.
        let text = format!("{SHOP}\n01,Snacks,Gloria biscuits,PC,800,1000,,");
        let (rows, _) = parse(&text, currency(Country::KE));

        assert_eq!(rows[0].price_minor, 100_000, "1000 KES = 100 000 minor");
    }

    #[test]
    fn a_bad_row_is_skipped_with_its_line_number() {
        let text = "name,category,price,barcode\nSoap,Household,1200\nSoap lite,Household,free\nSugar 1kg,Pantry,6000";
        let (rows, errors) = parse(text, ugx());

        assert_eq!(rows.len(), 2, "the good rows import");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].starts_with("line 3"), "{}", errors[0]);
    }

    #[test]
    fn no_barcode_column_imports_cleanly() {
        let text = "name,category,price\nRice 1kg,Pantry,8000";
        let (rows, errors) = parse(text, ugx());

        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(rows[0].barcode, None);
    }

    #[test]
    fn blank_cells_and_blank_lines() {
        let text = "name,category,price,barcode\n\n\"\",,100,\nBun, Bakery ,500,";
        let (rows, errors) = parse(text, ugx());

        assert_eq!(
            errors[0], "line 3: the product needs a name.",
            "a nameless row is reported where it is, not dropped"
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Bun");
        assert_eq!(rows[0].category, "Bakery");
        assert_eq!(rows[0].barcode, None);
    }

    #[test]
    fn an_empty_file_reports_itself() {
        let (rows, errors) = parse("", ugx());
        assert!(rows.is_empty());
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn quotes_survive_the_splitter() {
        assert_eq!(
            split_line(r#"01,"Say ""yes"" twice, please",5000"#),
            vec![
                "01".to_owned(),
                r#"Say "yes" twice, please"#.to_owned(),
                "5000".to_owned(),
            ]
        );
        assert_eq!(
            split_line(" soap ,  loofah ,1500"),
            vec![" soap ", "  loofah ", "1500"]
        );
    }
}
