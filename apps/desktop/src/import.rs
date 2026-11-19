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
//! Weights and cost prices are columns the till cannot price yet; they are
//! ignored, not a reason to refuse the rest of the file.
//!
//! A file may also carry `unit`, `pack_size`, `pack_price` and `pack_barcode`.
//! The price column stays the price of one base unit, so a pack row says how
//! many units the box holds and what the sealed box costs, without turning the
//! row into a second product.
//!
//! `tiers` is the loose-unit price ladder, written `10:475|50:450`: the count a
//! rung starts at, a colon, and the price of one unit from there up. The export
//! writes the same shape, so a ladder makes the round trip through a
//! spreadsheet unchanged. A ladder that climbs is refused on its line, the way
//! the form refuses it, rather than saved and charged at the counter.

use money::{Currency, Money};

use crate::db::PriceTier;

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
    /// What one base unit is called; `unit` when the file says nothing.
    pub unit_label: String,
    /// How many base units one sealed pack holds; `1` when there is no pack.
    pub pack_size: i64,
    /// The sealed pack's price. Only meaningful when `pack_size` is above one.
    pub pack_price_minor: Option<i64>,
    pub pack_barcode: Option<String>,
    /// Loose-unit price breaks, ascending. Empty when the file has none.
    pub tiers: Vec<PriceTier>,
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

        let unit_label = columns
            .unit_label
            .map(|column| cell(&fields, column))
            .filter(|label| !label.is_empty())
            .unwrap_or_else(|| "unit".to_owned());

        let pack_size_text = columns
            .pack_size
            .map(|column| cell(&fields, column))
            .unwrap_or_default();
        let pack_size = if pack_size_text.is_empty() {
            1
        } else {
            match pack_size_text.parse::<i64>() {
                Ok(size) if size >= 1 => size,
                _ => {
                    errors.push(format!(
                        "line {number}: the pack size must be a whole number of one or more."
                    ));
                    continue;
                }
            }
        };

        let pack_price_text = columns
            .pack_price
            .map(|column| cell(&fields, column))
            .unwrap_or_default();
        let pack_barcode = columns
            .pack_barcode
            .map(|column| cell(&fields, column))
            .filter(|code| !code.is_empty());

        // A row that names a pack price or pack barcode but no pack is a
        // contradiction, not a small typo: dropping the pack quietly would
        // import a box at the loose price. Say so and keep the rest of the file.
        if pack_size <= 1 && (!pack_price_text.is_empty() || pack_barcode.is_some()) {
            errors.push(format!(
                "line {number}: a pack price or barcode needs a pack size above one."
            ));
            continue;
        }

        let pack_price_minor = if pack_size <= 1 || pack_price_text.is_empty() {
            None
        } else {
            match Money::parse(&pack_price_text, currency) {
                Ok(price) if price.minor() >= 0 => Some(price.minor()),
                Ok(_) => {
                    errors.push(format!("line {number}: the pack price cannot be negative."));
                    continue;
                }
                Err(error) => {
                    errors.push(format!("line {number}: {error}"));
                    continue;
                }
            }
        };

        if pack_size > 1
            && let Some(pack_code) = pack_barcode.as_deref()
            && barcode.as_deref() == Some(pack_code)
        {
            errors.push(format!(
                "line {number}: the loose and pack barcodes cannot be the same."
            ));
            continue;
        }

        let tiers_text = columns
            .tiers
            .map(|column| cell(&fields, column))
            .unwrap_or_default();
        let tiers = if tiers_text.is_empty() {
            Vec::new()
        } else {
            match parse_tiers(&tiers_text, currency) {
                Ok(tiers) => tiers,
                Err(message) => {
                    errors.push(format!("line {number}: {message}"));
                    continue;
                }
            }
        };
        // The ladder is checked here, against the row's own price, so a file
        // and the product form refuse the same ladders. A rung that climbs is
        // a wrong price, and skipping the row says so where it happened.
        let tiers = match crate::db::clean_tiers(price_minor, &tiers) {
            Ok(tiers) => tiers,
            Err(error) => {
                errors.push(format!("line {number}: {error}."));
                continue;
            }
        };

        rows.push(Row {
            name: name.to_owned(),
            category,
            price_minor,
            barcode,
            unit_label,
            pack_size,
            pack_price_minor,
            pack_barcode,
            tiers,
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
    /// The name of one base unit, when the file names it.
    unit_label: Option<usize>,
    /// How many base units one sealed pack holds, when the file has packs.
    pack_size: Option<usize>,
    /// The sealed pack's price, when the file has packs.
    pack_price: Option<usize>,
    /// The barcode printed on the box, when the file has packs.
    pack_barcode: Option<usize>,
    /// The loose-unit price ladder, when the file prices in breaks.
    tiers: Option<usize>,
}

impl Columns {
    /// Columns of a file with no heading row. The plain order stays the four
    /// original fields: a file that predates packs must keep importing the
    /// same way it always did.
    fn plain() -> Self {
        Columns {
            name: 0,
            category: 1,
            price: 2,
            barcode: Some(3),
            unit_label: None,
            pack_size: None,
            pack_price: None,
            pack_barcode: None,
            tiers: None,
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
        "unit",
        "unit_label",
        "pack_size",
        "pack_price",
        "pack_barcode",
        "tiers",
        "price_breaks",
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
        unit_label: position(&["unit_label", "unit"]),
        pack_size: position(&["pack_size"]),
        pack_price: position(&["pack_price", "pack_retail_price"]),
        pack_barcode: position(&["pack_barcode", "box_barcode"]),
        tiers: position(&["tiers", "price_breaks"]),
    }
}

/// One `tiers` cell: `10:475|50:450`.
///
/// Only the spelling is read here; whether the rungs make a ladder is
/// [`crate::db::clean_tiers`]'s question, so a file and the product form
/// refuse the same thing for the same reason.
fn parse_tiers(text: &str, currency: Currency) -> Result<Vec<PriceTier>, String> {
    let mut tiers = Vec::new();
    for part in text.split('|') {
        let part = part.trim();
        if part.is_empty() {
            return Err("a price break is missing a quantity or a price.".to_owned());
        }
        let Some((qty_text, price_text)) = part.split_once(':') else {
            return Err(format!(
                "the price break `{part}` needs a quantity and a price, as 10:475."
            ));
        };
        let min_qty = match qty_text.trim().parse::<i64>() {
            Ok(qty) if qty >= 2 => qty,
            _ => {
                return Err(format!(
                    "the price break `{part}` must start at two units or more."
                ));
            }
        };
        let price_minor = match Money::parse(price_text.trim(), currency) {
            Ok(price) if price.minor() >= 0 => price.minor(),
            Ok(_) => return Err(format!("the price break `{part}` cannot be negative.")),
            Err(error) => return Err(format!("the price break `{part}`: {error}")),
        };
        tiers.push(PriceTier {
            min_qty,
            price_minor,
        });
    }
    Ok(tiers)
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

    /// A pack row is still one product: the price is per tablet, and the pack
    /// columns describe the box without becoming a second line in the file.
    #[test]
    fn a_pack_row_carries_its_own_size_price_and_code() {
        let text = "name,category,price,unit,pack_size,pack_price,barcode,pack_barcode\n\
                    Amoxicillin,Medicine,30,tablet,100,2800,100100,100200";
        let (rows, errors) = parse(text, ugx());

        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.price_minor, 30, "the tablet price");
        assert_eq!(row.unit_label, "tablet");
        assert_eq!(row.pack_size, 100);
        assert_eq!(row.pack_price_minor, Some(2_800));
        assert_eq!(row.barcode.as_deref(), Some("100100"));
        assert_eq!(row.pack_barcode.as_deref(), Some("100200"));
    }

    /// A pack of one with a box price is a contradiction: importing it would
    /// quietly sell the whole box for the loose price.
    #[test]
    fn a_pack_of_one_with_a_pack_price_is_refused() {
        let text = "name,category,price,unit,pack_size,pack_price,pack_barcode\n\
                    Single,Shop,500,box,1,9000,999";
        let (rows, errors) = parse(text, ugx());

        assert!(rows.is_empty());
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("pack size above one"), "{errors:?}");
    }

    /// The ladder cell is `count:price` rungs joined by `|` — the shape the
    /// export writes, so a file the till wrote comes back unchanged.
    #[test]
    fn a_tiers_cell_becomes_a_price_ladder() {
        let text = "name,category,price,tiers\n\
                    Panadol,Cold,500,50:450|10:475";
        let (rows, errors) = parse(text, ugx());

        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            rows[0].tiers,
            vec![
                PriceTier {
                    min_qty: 10,
                    price_minor: 475,
                },
                PriceTier {
                    min_qty: 50,
                    price_minor: 450,
                },
            ],
            "the ladder is stored in count order however the cell lists it"
        );
    }

    /// A ladder that climbs is a wrong price, not a spelling mistake, and the
    /// row is skipped where it happened rather than saved for the counter to
    /// honour.
    #[test]
    fn a_ladder_that_climbs_is_refused_on_its_line() {
        let text = "name,category,price,tiers\n\
                    Panadol,Cold,500,10:600\n\
                    Brufen,Cold,500,10:475";
        let (rows, errors) = parse(text, ugx());

        assert_eq!(rows.len(), 1, "the good row still imports");
        assert_eq!(rows[0].name, "Brufen");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].starts_with("line 2:"), "{errors:?}");
        assert!(errors[0].contains("cannot be above"), "{errors:?}");
    }

    #[test]
    fn a_malformed_tiers_cell_names_its_line_and_what_is_wrong() {
        let text = "name,category,price,tiers\nSoap,Household,1200,10-1100";
        let (rows, errors) = parse(text, ugx());

        assert!(rows.is_empty());
        assert!(errors[0].starts_with("line 2:"), "{errors:?}");
        assert!(errors[0].contains("quantity and a price"), "{errors:?}");
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
