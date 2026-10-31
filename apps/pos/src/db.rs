//! POS schema and the few reads the shell needs.
//!
//! Forward-only: append migrations with a higher version. Never edit one that
//! has shipped, because some till somewhere already applied it.

use anyhow::Context as _;
use storage::Migration;

/// A completed sale, as the history table shows it.
pub struct Sale {
    pub id: i64,
    /// What the customer actually paid: the exact total plus cash rounding.
    pub paid_minor: i64,
    pub created_at: String,
}

/// One line of a sale, on its way into the database.
///
/// The name and the unit price are the ones on the receipt, not a lookup: a
/// sale records what it sold, and the catalog may have moved on since.
pub struct SaleLine {
    /// The catalog row, when there is still one. Reporting only.
    pub product_id: Option<i64>,
    pub name: String,
    pub qty: i64,
    pub unit_minor: i64,
}

/// A sale after it is written.
///
/// The amounts are the ones the cashier reads out, so the confirmation screen
/// does not have to re-derive them from a cart that is now empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedSale {
    pub id: i64,
    /// The exact sum of the lines.
    pub total_minor: i64,
    /// The cash adjustment, signed: `paid = total + rounding`.
    pub rounding_minor: i64,
    pub tendered_minor: i64,
    pub change_minor: i64,
}

impl RecordedSale {
    /// What the customer paid, and what the drawer gained.
    pub fn paid_minor(&self) -> i64 {
        self.total_minor + self.rounding_minor
    }
}

/// One thing the shop sells, as the catalog stores it.
///
/// `price_minor` is in the till's currency, like every other amount: there is no
/// second unit and nothing to convert at checkout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Product {
    pub id: i64,
    pub name: String,
    pub price_minor: i64,
    pub category: String,
    /// Optional, and unique when present: two products claiming one barcode is
    /// a scanning bug rather than a preference.
    pub barcode: Option<String>,
}

/// Owner stamp in the SQLite header. Arbitrary, permanent, and never derived
/// from the app's name — a rename must not orphan a till's database.
pub const APPLICATION_ID: i64 = 1_836_624_513;

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "create_sales",
        sql: r#"
            CREATE TABLE IF NOT EXISTS sales (
                id          INTEGER PRIMARY KEY,
                total_cents INTEGER NOT NULL,
                created_at  TEXT    NOT NULL DEFAULT (datetime('now'))
            );
        "#,
    },
    // The till's currency is a property of the install, not of a settings file:
    // it must travel with the database it prices. `meta` is the general
    // key/value table for that kind of fact.
    //
    // `total_cents` named a unit, not an amount: the same integer is whole
    // shillings in Kampala and cents in Nairobi. `total_minor` says what the
    // column actually holds.
    Migration {
        version: 2,
        name: "currency_and_meta",
        sql: r#"
            ALTER TABLE sales RENAME COLUMN total_cents TO total_minor;

            CREATE TABLE IF NOT EXISTS meta (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
        "#,
    },
    // The catalog. `category` is free text rather than a second table: a shop
    // with forty products needs a grouping, not a taxonomy, and a category row
    // nothing references is one more thing to keep consistent.
    //
    // The price is an integer count of minor units in the till's currency — the
    // same rule as `sales.total_minor`. A price is not rounded to the cash step
    // when it is stored: the exact price is what the receipt and the ledger
    // record, and only the cash tender rounds.
    Migration {
        version: 3,
        name: "create_products",
        sql: r#"
            CREATE TABLE IF NOT EXISTS products (
                id          INTEGER PRIMARY KEY,
                name        TEXT    NOT NULL CHECK (length(name) > 0),
                price_minor INTEGER NOT NULL CHECK (price_minor >= 0),
                category    TEXT    NOT NULL CHECK (length(category) > 0),
                barcode     TEXT,
                created_at  TEXT    NOT NULL DEFAULT (datetime('now'))
            );

            CREATE UNIQUE INDEX IF NOT EXISTS products_barcode
                ON products (barcode) WHERE barcode IS NOT NULL;

            CREATE INDEX IF NOT EXISTS products_name ON products (name);
        "#,
    },
    // What was sold, and how it was paid for.
    //
    // A line copies the name and price it was sold at: the catalog is editable,
    // and a receipt that changes when a product is renamed or repriced is not a
    // receipt. `product_id` is only a link back for reporting, and it is
    // `SET NULL` rather than `CASCADE` so deleting a product never deletes the
    // history of selling it.
    //
    // `sales.rounding_minor` is the cash adjustment, signed: the customer paid
    // `total_minor + rounding_minor`, because 340 shillings cannot be tendered
    // with a 100 shilling coin. The exact total stays exact; only the tender
    // rounds.
    //
    // `payments` is a table rather than three columns on the sale because a
    // sale can be paid in parts, and because it is append-only like `sales`.
    // The method is text, not a CHECK list: mobile money is a new row, not a
    // migration that rebuilds the table.
    Migration {
        version: 4,
        name: "sale_lines_and_payments",
        sql: r#"
            ALTER TABLE sales ADD COLUMN rounding_minor INTEGER NOT NULL DEFAULT 0;

            CREATE TABLE IF NOT EXISTS sale_lines (
                id          INTEGER PRIMARY KEY,
                sale_id     INTEGER NOT NULL REFERENCES sales(id),
                product_id  INTEGER REFERENCES products(id) ON DELETE SET NULL,
                name        TEXT    NOT NULL CHECK (length(name) > 0),
                qty         INTEGER NOT NULL CHECK (qty > 0),
                unit_minor  INTEGER NOT NULL CHECK (unit_minor >= 0),
                total_minor INTEGER NOT NULL CHECK (total_minor >= 0)
            );

            CREATE INDEX IF NOT EXISTS sale_lines_sale ON sale_lines (sale_id);

            CREATE TABLE IF NOT EXISTS payments (
                id             INTEGER PRIMARY KEY,
                sale_id        INTEGER NOT NULL REFERENCES sales(id),
                method         TEXT    NOT NULL CHECK (length(method) > 0),
                amount_minor   INTEGER NOT NULL CHECK (amount_minor >= 0),
                tendered_minor INTEGER NOT NULL CHECK (tendered_minor >= 0),
                change_minor   INTEGER NOT NULL CHECK (change_minor >= 0)
            );

            CREATE INDEX IF NOT EXISTS payments_sale ON payments (sale_id);
        "#,
    },
];

/// The currency this install prices in, or `None` before first-run setup.
///
/// An unreadable or unknown value is treated as "not chosen yet": the picker
/// opens, and the operator cannot end up selling in a currency the data no
/// longer recognises.
pub fn currency(store: &storage::Store) -> Option<money::Currency> {
    let code: String = store
        .conn()
        .query_row("SELECT value FROM meta WHERE key = 'currency'", [], |row| {
            row.get(0)
        })
        .ok()?;

    let currency = money::tender_from_code(&code);
    if currency.is_none() {
        tracing::warn!(code = %code, "stored currency is not a tender");
    }
    currency
}

/// Store the install's currency, replacing a previous choice.
pub fn set_currency(store: &storage::Store, currency: money::Currency) -> anyhow::Result<()> {
    store.conn().execute(
        "INSERT INTO meta (key, value) VALUES ('currency', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [currency.code()],
    )?;
    Ok(())
}

/// Whether this till has recorded a sale.
///
/// Currency is editable only while this is false: a price in one currency and a
/// total in another is not something a report can repair afterwards.
pub fn has_sales(store: &storage::Store) -> bool {
    store
        .conn()
        .query_row("SELECT EXISTS(SELECT 1 FROM sales)", [], |row| {
            row.get::<_, bool>(0)
        })
        // A query that cannot answer must not unlock the setting.
        .unwrap_or(true)
}

/// Every product, by name.
///
/// The list is small enough to keep in memory and filter there, which is why
/// the search box does not run a query per keystroke. A catalog big enough to
/// change that is a catalog that needs the query, and that is the moment to
/// move the filter into SQL.
pub fn products(store: &storage::Store) -> Vec<Product> {
    let mut statement = match store.conn().prepare(
        "SELECT id, name, price_minor, category, barcode
         FROM products ORDER BY name COLLATE NOCASE",
    ) {
        Ok(statement) => statement,
        Err(err) => {
            tracing::warn!(error = %err, "products prepare failed");
            return Vec::new();
        }
    };

    let rows = statement.query_map([], |row| {
        Ok(Product {
            id: row.get(0)?,
            name: row.get(1)?,
            price_minor: row.get(2)?,
            category: row.get(3)?,
            barcode: row.get(4)?,
        })
    });

    match rows {
        Ok(rows) => rows.filter_map(Result::ok).collect(),
        Err(err) => {
            tracing::warn!(error = %err, "products query failed");
            Vec::new()
        }
    }
}

/// The categories in use, so the sidebar lists the shop's groups rather than a
/// list the app invented.
pub fn categories(store: &storage::Store) -> Vec<String> {
    let mut statement = match store
        .conn()
        .prepare("SELECT DISTINCT category FROM products ORDER BY category COLLATE NOCASE")
    {
        Ok(statement) => statement,
        Err(err) => {
            tracing::warn!(error = %err, "categories prepare failed");
            return Vec::new();
        }
    };

    let rows = statement.query_map([], |row| row.get::<_, String>(0));
    match rows {
        Ok(rows) => rows.filter_map(Result::ok).collect(),
        Err(err) => {
            tracing::warn!(error = %err, "categories query failed");
            Vec::new()
        }
    }
}

/// Add a product and return its id.
pub fn insert_product(
    store: &storage::Store,
    name: &str,
    price_minor: i64,
    category: &str,
    barcode: Option<&str>,
) -> anyhow::Result<i64> {
    store.conn().execute(
        "INSERT INTO products (name, price_minor, category, barcode)
         VALUES (?1, ?2, ?3, ?4)",
        (name, price_minor, category, barcode),
    )?;
    Ok(store.conn().last_insert_rowid())
}

/// Replace a product's editable fields.
///
/// Returns an error when the row is gone: the list on screen is a cache, and
/// "Saved" over a product that no longer exists is a lie the operator would
/// only discover on the next restart.
pub fn update_product(
    store: &storage::Store,
    id: i64,
    name: &str,
    price_minor: i64,
    category: &str,
    barcode: Option<&str>,
) -> anyhow::Result<()> {
    let changed = store.conn().execute(
        "UPDATE products
         SET name = ?2, price_minor = ?3, category = ?4, barcode = ?5
         WHERE id = ?1",
        (id, name, price_minor, category, barcode),
    )?;
    anyhow::ensure!(changed == 1, "product {id} is no longer in the catalog");
    Ok(())
}

/// Remove a product.
///
/// Sales do not reference this row: a completed sale keeps the name and price
/// it was sold at, so deleting a product never rewrites a receipt.
pub fn delete_product(store: &storage::Store, id: i64) -> anyhow::Result<()> {
    store
        .conn()
        .execute("DELETE FROM products WHERE id = ?1", [id])?;
    Ok(())
}

/// Whether a barcode already belongs to a product other than `except`.
///
/// Checked before the write so the operator gets "that barcode is taken"
/// instead of a constraint error. The unique index still enforces it.
pub fn barcode_taken(store: &storage::Store, barcode: &str, except: Option<i64>) -> bool {
    store
        .conn()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM products WHERE barcode = ?1 AND id IS NOT ?2)",
            (barcode, except),
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(true)
}

/// Record a sale and its payment in one transaction.
///
/// The amounts are checked here rather than left to the CHECK constraints, so a
/// bad cart fails with a sentence the operator can act on instead of a
/// constraint error. Nothing is written unless every line and the tender are
/// valid: a sale that half-happened is worse than one that did not.
///
/// `rounding_minor` is signed. It exists because the exact total is often not
/// tenderable — 340 shillings against a 100 shilling coin — and the shop
/// absorbs the difference rather than refusing the sale.
pub fn record_sale(
    store: &storage::Store,
    lines: &[SaleLine],
    rounding_minor: i64,
    tendered_minor: i64,
) -> anyhow::Result<RecordedSale> {
    anyhow::ensure!(!lines.is_empty(), "a sale needs at least one line");

    let total_minor = lines.iter().try_fold(0_i64, |sum, line| {
        anyhow::ensure!(line.qty > 0, "{} has no quantity", line.name);
        anyhow::ensure!(line.unit_minor >= 0, "{} has a negative price", line.name);
        anyhow::ensure!(
            !line.name.trim().is_empty(),
            "a sale line needs a name to print on the receipt"
        );
        let line_total = line
            .qty
            .checked_mul(line.unit_minor)
            .context("a line total does not fit in 64 bits")?;
        sum.checked_add(line_total)
            .context("the sale total does not fit in 64 bits")
    })?;

    let paid_minor = total_minor
        .checked_add(rounding_minor)
        .context("the amount due does not fit in 64 bits")?;
    anyhow::ensure!(paid_minor >= 0, "the amount due cannot be negative");
    anyhow::ensure!(
        tendered_minor >= paid_minor,
        "cash received is less than the amount due"
    );
    let change_minor = tendered_minor - paid_minor;

    // `unchecked_transaction` because `Store` hands out a shared connection: the
    // transaction cannot borrow it mutably, and there is one till per process.
    let transaction = store.conn().unchecked_transaction()?;

    transaction.execute(
        "INSERT INTO sales (total_minor, rounding_minor) VALUES (?1, ?2)",
        (total_minor, rounding_minor),
    )?;
    let id = transaction.last_insert_rowid();

    for line in lines {
        transaction.execute(
            "INSERT INTO sale_lines (sale_id, product_id, name, qty, unit_minor, total_minor)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (
                id,
                line.product_id,
                line.name.as_str(),
                line.qty,
                line.unit_minor,
                line.qty * line.unit_minor,
            ),
        )?;
    }

    transaction.execute(
        "INSERT INTO payments (sale_id, method, amount_minor, tendered_minor, change_minor)
         VALUES (?1, 'cash', ?2, ?3, ?4)",
        (id, paid_minor, tendered_minor, change_minor),
    )?;

    transaction.commit()?;

    Ok(RecordedSale {
        id,
        total_minor,
        rounding_minor,
        tendered_minor,
        change_minor,
    })
}

/// The SQL window a range index covers, or `None` for all time.
///
/// One definition for both screens: the dashboard totals and the transaction
/// list are the same question asked twice, and they must never disagree about
/// where "7 days" starts.
fn range_clause(range: usize) -> Option<&'static str> {
    match range {
        // `localdate` on purpose: the cashier's "today" is the terminal's day,
        // not UTC. Any other choice moves the day boundary at the wrong hour for
        // half the world, and the till's report disagrees with the drawer.
        0 => Some("date(created_at, 'localtime') = date('now', 'localtime')"),
        1 => Some("created_at >= datetime('now', '-7 days')"),
        2 => Some("created_at >= datetime('now', '-30 days')"),
        _ => None,
    }
}

/// Revenue and sale count for a range.
///
/// A failed query yields zeros and a logged warning rather than an error the
/// shell has to render — the dashboard is a summary, not the source of truth.
pub fn totals(store: &storage::Store, range: usize) -> (i64, i64) {
    let sql = match range_clause(range) {
        Some(clause) => format!(
            "SELECT COUNT(*), COALESCE(SUM(total_minor + rounding_minor), 0)
             FROM sales WHERE {clause}"
        ),
        None => {
            "SELECT COUNT(*), COALESCE(SUM(total_minor + rounding_minor), 0) FROM sales".to_owned()
        }
    };

    let result = store.conn().query_row(&sql, [], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
    });

    match result {
        Ok(totals) => totals,
        Err(err) => {
            tracing::warn!(error = %err, "totals query failed");
            (0, 0)
        }
    }
}

/// Most recent sales in a range, newest first.
pub fn sales(store: &storage::Store, range: usize, limit: i64) -> Vec<Sale> {
    let sql = match range_clause(range) {
        Some(clause) => format!(
            "SELECT id, total_minor + rounding_minor, created_at
             FROM sales WHERE {clause} ORDER BY id DESC LIMIT ?1"
        ),
        None => "SELECT id, total_minor + rounding_minor, created_at
             FROM sales ORDER BY id DESC LIMIT ?1"
            .to_owned(),
    };

    let mut statement = match store.conn().prepare(&sql) {
        Ok(statement) => statement,
        Err(err) => {
            tracing::warn!(error = %err, "sales prepare failed");
            return Vec::new();
        }
    };

    let rows = statement.query_map([limit], |row| {
        Ok(Sale {
            id: row.get(0)?,
            paid_minor: row.get(1)?,
            created_at: row.get(2)?,
        })
    });

    match rows {
        Ok(rows) => rows.filter_map(Result::ok).collect(),
        Err(err) => {
            tracing::warn!(error = %err, "sales query failed");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A real file, not `:memory:`: the till runs with pragmas that only exist
    /// on disk, and a test that skips them is testing a different database.
    fn store(name: &str) -> (storage::Store, PathBuf) {
        let path = std::env::temp_dir().join(format!("pos-{name}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut store = storage::Store::open(&path).expect("open database");
        store.migrate(MIGRATIONS).expect("migrate");
        (store, path)
    }

    /// Four sales, each dated relative to now so the test never rots: today,
    /// three days ago, ten days ago, forty days ago.
    fn seed(store: &storage::Store) {
        for (offset, total_minor) in [
            ("-0 days", 500_i64),
            ("-3 days", 700),
            ("-10 days", 1100),
            ("-40 days", 2000),
        ] {
            store
                .conn()
                .execute(
                    "INSERT INTO sales (total_minor, created_at) VALUES (?1, datetime('now', ?2))",
                    (total_minor, offset),
                )
                .expect("insert sale");
        }
    }

    #[test]
    fn ranges_select_the_window_they_advertise() {
        let (store, path) = store("ranges");
        seed(&store);

        assert_eq!(totals(&store, 0), (1, 500), "Today");
        assert_eq!(totals(&store, 1), (2, 1200), "7 days");
        assert_eq!(totals(&store, 2), (3, 2300), "30 days");
        assert_eq!(totals(&store, 3), (4, 4300), "All time");

        assert_eq!(sales(&store, 1, 10).len(), 2);

        let two = sales(&store, 3, 2);
        assert_eq!(two.len(), 2, "the limit applies after the filter");
        assert!(two[0].id > two[1].id, "newest first");

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn the_currency_is_stored_once_and_read_back() {
        let (store, path) = store("currency");

        assert_eq!(currency(&store), None, "a fresh install has not chosen yet");
        assert!(!has_sales(&store));

        let ugx = money::tender_from_code("UGX").expect("UGX is a tender");
        let kes = money::tender_from_code("KES").expect("KES is a tender");

        set_currency(&store, ugx).expect("store currency");
        assert_eq!(currency(&store), Some(ugx));

        // A second choice replaces the row rather than adding one.
        set_currency(&store, kes).expect("replace currency");
        assert_eq!(currency(&store), Some(kes));

        seed(&store);
        assert!(has_sales(&store));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_unknown_stored_currency_reads_as_not_chosen() {
        let (store, path) = store("bad-currency");

        store
            .conn()
            .execute(
                "INSERT INTO meta (key, value) VALUES ('currency', 'NOPE')",
                [],
            )
            .expect("insert meta");

        assert_eq!(currency(&store), None);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_fresh_database_reports_zero_rather_than_failing() {
        let (store, path) = store("empty");

        assert_eq!(totals(&store, 0), (0, 0));
        assert!(sales(&store, 3, 5).is_empty());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn products_round_trip_and_stay_sorted_by_name() {
        let (store, path) = store("products");

        assert!(products(&store).is_empty(), "a fresh till has no catalog");
        assert!(categories(&store).is_empty());

        let flat_white =
            insert_product(&store, "Flat white", 3_400, "Coffee", None).expect("insert");
        insert_product(&store, "Croissant", 3_200, "Bakery", Some("4001")).expect("insert");
        insert_product(&store, "Espresso", 2_800, "Coffee", None).expect("insert");

        let names: Vec<String> = products(&store)
            .into_iter()
            .map(|product| product.name)
            .collect();
        assert_eq!(
            names,
            ["Croissant", "Espresso", "Flat white"],
            "the list is by name, not by insertion order"
        );
        assert_eq!(categories(&store), ["Bakery", "Coffee"]);

        update_product(
            &store,
            flat_white,
            "Flat white",
            3_600,
            "Coffee",
            Some("4002"),
        )
        .expect("update");
        let updated = products(&store)
            .into_iter()
            .find(|product| product.id == flat_white)
            .expect("the product survived the update");
        assert_eq!(updated.price_minor, 3_600);
        assert_eq!(updated.barcode.as_deref(), Some("4002"));

        delete_product(&store, flat_white).expect("delete");
        assert_eq!(products(&store).len(), 2);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_barcode_belongs_to_one_product() {
        let (store, path) = store("barcodes");
        let espresso =
            insert_product(&store, "Espresso", 2_800, "Coffee", Some("4001")).expect("insert");

        assert!(barcode_taken(&store, "4001", None));
        assert!(
            !barcode_taken(&store, "4001", Some(espresso)),
            "a product keeps its own barcode when it is edited"
        );
        assert!(!barcode_taken(&store, "4002", None));

        assert!(
            insert_product(&store, "Croissant", 3_200, "Bakery", Some("4001")).is_err(),
            "the unique index is the real guard, not the pre-check"
        );

        // Products without a barcode are not competing for one.
        insert_product(&store, "Filter", 2_600, "Coffee", None).expect("insert");
        insert_product(&store, "Lemonade", 3_000, "Cold", None).expect("insert");

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn the_database_refuses_a_negative_price() {
        let (store, path) = store("negative-price");

        assert!(
            insert_product(&store, "Refund", -100, "Coffee", None).is_err(),
            "a price is a non-negative amount; the CHECK is the guard"
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_sale_writes_its_lines_and_its_payment() {
        let (store, path) = store("record-sale");
        let espresso =
            insert_product(&store, "Espresso", 2_800, "Coffee", None).expect("insert product");

        let lines = [
            SaleLine {
                product_id: Some(espresso),
                name: "Espresso".to_owned(),
                qty: 1,
                unit_minor: 2_800,
            },
            SaleLine {
                product_id: None,
                name: "Unlisted bun".to_owned(),
                qty: 2,
                unit_minor: 500,
            },
        ];

        // 2800 + 1000 = 3800 exact; rounded down 20 to the 100 shilling coin.
        let sale = record_sale(&store, &lines, -20, 4_000).expect("record");

        assert_eq!(sale.total_minor, 3_800, "the exact total is kept exact");
        assert_eq!(sale.paid_minor(), 3_780);
        assert_eq!(sale.change_minor, 220, "4000 tendered against 3780 due");

        let (count, revenue) = totals(&store, 3);
        assert_eq!(count, 1);
        assert_eq!(revenue, 3_780, "revenue is what the drawer took");

        let history = sales(&store, 3, 10);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].paid_minor, 3_780);
        assert_eq!(history[0].id, sale.id);

        let line_count: i64 = store
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM sale_lines WHERE sale_id = ?1",
                [sale.id],
                |row| row.get(0),
            )
            .expect("count lines");
        assert_eq!(line_count, 2);

        let (method, amount, tendered): (String, i64, i64) = store
            .conn()
            .query_row(
                "SELECT method, amount_minor, tendered_minor FROM payments WHERE sale_id = ?1",
                [sale.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("read payment");
        assert_eq!(method, "cash");
        assert_eq!(amount, 3_780);
        assert_eq!(tendered, 4_000);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_short_tender_writes_nothing() {
        let (store, path) = store("short-tender");
        let lines = [SaleLine {
            product_id: None,
            name: "Espresso".to_owned(),
            qty: 1,
            unit_minor: 2_800,
        }];

        assert!(
            record_sale(&store, &lines, 0, 2_000).is_err(),
            "2000 does not cover 2800"
        );

        // The sale row is inserted first, so this proves the transaction rolled
        // back rather than leaving a sale with no payment behind.
        assert_eq!(totals(&store, 3), (0, 0));
        let line_count: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM sale_lines", [], |row| row.get(0))
            .expect("count lines");
        assert_eq!(line_count, 0);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_empty_cart_is_not_a_sale() {
        let (store, path) = store("empty-cart");

        assert!(record_sale(&store, &[], 0, 0).is_err());
        assert_eq!(totals(&store, 3), (0, 0));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn deleting_a_product_does_not_rewrite_its_receipt() {
        let (store, path) = store("sale-then-delete");
        let flat_white =
            insert_product(&store, "Flat white", 3_400, "Coffee", None).expect("insert product");

        let sale = record_sale(
            &store,
            &[SaleLine {
                product_id: Some(flat_white),
                name: "Flat white".to_owned(),
                qty: 1,
                unit_minor: 3_400,
            }],
            0,
            3_400,
        )
        .expect("record");

        delete_product(&store, flat_white).expect("delete");

        let (product_id, name, total_minor): (Option<i64>, String, i64) = store
            .conn()
            .query_row(
                "SELECT product_id, name, total_minor FROM sale_lines WHERE sale_id = ?1",
                [sale.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("the line survives the product");
        assert_eq!(product_id, None, "the link is cleared, not the history");
        assert_eq!(name, "Flat white");
        assert_eq!(total_minor, 3_400);

        let _ = std::fs::remove_file(path);
    }
}
