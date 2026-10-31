//! POS schema and the few reads the shell needs.
//!
//! Forward-only: append migrations with a higher version. Never edit one that
//! has shipped, because some till somewhere already applied it.

use storage::Migration;

/// A completed sale, as the history table shows it.
pub struct Sale {
    pub id: i64,
    pub total_minor: i64,
    pub created_at: String,
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
        Some(clause) => {
            format!("SELECT COUNT(*), COALESCE(SUM(total_minor), 0) FROM sales WHERE {clause}")
        }
        None => "SELECT COUNT(*), COALESCE(SUM(total_minor), 0) FROM sales".to_owned(),
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
            "SELECT id, total_minor, created_at FROM sales WHERE {clause} ORDER BY id DESC LIMIT ?1"
        ),
        None => {
            "SELECT id, total_minor, created_at FROM sales ORDER BY id DESC LIMIT ?1".to_owned()
        }
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
            total_minor: row.get(1)?,
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
}
