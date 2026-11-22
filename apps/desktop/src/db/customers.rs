//! Regular customers and the rate each one buys at.
//!
//! A customer row is a pricing term the cashier picks at the counter: a name
//! to show and a discount to take off the basket. It is catalog, not a till's
//! private list — it syncs with products — so the same shape rules apply:
//! archived rather than deleted, and a live name is unique.

use super::*;

/// Every live customer, by name.
///
/// Small enough to keep in memory and filter at the counter, like the product
/// list. Archived customers are excluded here and still reachable by the sync
/// snapshot, which reads the table directly: the shop hid them, a peer still
/// has to learn that they are gone.
pub fn customers(store: &storage::Store) -> Vec<Customer> {
    let mut statement = match store.conn().prepare(
        "SELECT id, name, phone, discount_bp
             FROM customers WHERE archived = 0 ORDER BY name COLLATE NOCASE",
    ) {
        Ok(statement) => statement,
        Err(err) => {
            tracing::warn!(error = %err, "customers prepare failed");
            return Vec::new();
        }
    };

    let rows = statement.query_map([], |row| {
        Ok(Customer {
            id: row.get(0)?,
            name: row.get(1)?,
            phone: row.get(2)?,
            discount_bp: row.get(3)?,
        })
    });

    match rows {
        Ok(rows) => rows.filter_map(Result::ok).collect(),
        Err(err) => {
            tracing::warn!(error = %err, "customers query failed");
            Vec::new()
        }
    }
}

/// Whether a name already belongs to a live customer other than `except`.
///
/// Checked before the write so the operator reads "that name is taken"
/// instead of a constraint error. Case-insensitive, matching the index.
pub fn customer_name_taken(store: &storage::Store, name: &str, except: Option<i64>) -> bool {
    store
        .conn()
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM customers
                 WHERE name = ?1 COLLATE NOCASE
                   AND id IS NOT ?2 AND archived = 0
             )",
            (name.trim(), except),
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(true)
}

/// Add a customer, or replace one's terms.
///
/// Returns the row id. The name is the identity the cashier picks by, so it
/// must be present; the phone is optional and the rate defaults to none.
pub fn save_customer(
    store: &storage::Store,
    id: Option<i64>,
    name: &str,
    phone: Option<&str>,
    discount_bp: i64,
) -> anyhow::Result<i64> {
    let name = name.trim();
    anyhow::ensure!(!name.is_empty(), "a customer needs a name");
    anyhow::ensure!(
        (0..=10_000).contains(&discount_bp),
        "a discount cannot exceed the whole basket"
    );
    let phone = phone.map(str::trim).filter(|phone| !phone.is_empty());
    anyhow::ensure!(
        !customer_name_taken(store, name, id),
        "another customer is already called {name}"
    );

    match id {
        Some(id) => {
            let changed = store.conn().execute(
                "UPDATE customers SET name = ?2, phone = ?3, discount_bp = ?4
                 WHERE id = ?1 AND archived = 0",
                rusqlite::params![id, name, phone, discount_bp],
            )?;
            anyhow::ensure!(changed == 1, "customer {id} is no longer on the list");
            Ok(id)
        }
        None => {
            store.conn().execute(
                "INSERT INTO customers (name, phone, discount_bp) VALUES (?1, ?2, ?3)",
                rusqlite::params![name, phone, discount_bp],
            )?;
            Ok(store.conn().last_insert_rowid())
        }
    }
}

/// Hide a customer from the list, keeping the sales written under them.
pub fn archive_customer(store: &storage::Store, id: i64) -> anyhow::Result<()> {
    store.conn().execute(
        "UPDATE customers SET archived = 1 WHERE id = ?1 AND archived = 0",
        [id],
    )?;
    Ok(())
}
