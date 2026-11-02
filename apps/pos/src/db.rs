//! POS schema and the few reads the shell needs.
//!
//! Forward-only: append migrations with a higher version. Never edit one that
//! has shipped, because some till somewhere already applied it.

use anyhow::Context as _;
use std::path::{Path, PathBuf};
use storage::Migration;

/// A completed sale, as the history table shows it.
pub struct Sale {
    pub id: i64,
    /// What the customer actually paid: the exact total plus cash rounding.
    pub paid_minor: i64,
    /// Rendered in the till's local time, minute precision (`01/11 09:15`):
    /// the stored value stays UTC and the query translates and trims. A
    /// cashier reading the list wants "when the sale happened" at a glance,
    /// not the database's epoch count.
    pub created_at: String,
    /// The account that took the money, by name.
    ///
    /// Read through the join, not copied onto the sale: renaming a person should
    /// rename their history. Sales written before the till had staff, and sales
    /// whose account was later removed, are `None`.
    pub cashier: Option<String>,
}

/// One sale as the CSV export writes it.
///
/// Separate from [`Sale`] because the two read different things: the table caps
/// at a hundred rows and trims the timestamp to fit a column, while a file is
/// meant to be complete and re-readable. Sharing one query would mean exporting
/// whatever the screen happened to show.
pub struct ExportSale {
    pub id: i64,
    /// Local time, whole seconds: a file is compared against another system's
    /// books, so the minute the screen shows is not precise enough.
    pub created_at: String,
    pub cashier: Option<String>,
    /// The exact sum of the lines.
    pub total_minor: i64,
    /// The cash adjustment, signed.
    pub rounding_minor: i64,
    /// `total_minor + rounding_minor`: what the customer paid.
    pub paid_minor: i64,
    /// One method, or several joined with `+` once a sale can be split.
    pub method: String,
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

/// One of the shops or people the shop buys from.
///
/// The contact fields are optional because a till should not refuse a supplier
/// just because nobody wrote down an email. `name` is the only thing that has
/// to exist: it is what a purchase order would print.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Supplier {
    pub id: String,
    pub name: String,
    pub contact_name: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub address: Option<String>,
    pub notes: Option<String>,
    pub archived: bool,
}

/// The editable fields of a supplier, as the form holds them.
///
/// One struct rather than six string arguments: the insert and update paths
/// must agree on which column is which, and a caller that transposes phone and
/// email is a bug the compiler cannot see in a positional list.
///
/// The values are raw form text; [`insert_supplier`] and [`update_supplier`]
/// trim them and store a blank optional field as `NULL`.
pub struct SupplierFields<'a> {
    pub name: &'a str,
    pub contact_name: &'a str,
    pub phone: &'a str,
    pub email: &'a str,
    pub address: &'a str,
    pub notes: &'a str,
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
    // Who works here, and who took each sale.
    //
    // The id is a UUIDv7 rather than a rowid because this table is the first one
    // that will be shared between tills: a person is the same person at the
    // counter and in the back office, and merging two installs must not need a
    // renumbering. `sales.user_id` is nullable on purpose — sales written before
    // this migration, and any sale whose cashier row is removed, keep their
    // amounts and lose only the name.
    //
    // `credential` holds a PHC string, never a secret, and `kind` is which
    // keyboard the lock screen shows. Neither is exposed to the UI.
    //
    // `login_attempts` is local wall-clock policy: it must never sync, because a
    // lockout is about a person standing at *this* counter, and a peer's clock
    // has no business deciding when this till opens.
    Migration {
        version: 5,
        name: "staff_and_sale_owner",
        sql: r#"
            CREATE TABLE IF NOT EXISTS users (
                id         TEXT PRIMARY KEY,
                name       TEXT NOT NULL CHECK (length(trim(name)) > 0),
                role       TEXT NOT NULL CHECK (role IN ('cashier', 'manager', 'owner')),
                credential TEXT NOT NULL,
                kind       TEXT NOT NULL CHECK (kind IN ('pin', 'password')),
                active     INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0, 1)),
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE UNIQUE INDEX IF NOT EXISTS users_name ON users (name COLLATE NOCASE);

            -- One owner per install: the recovery code and the currency belong
            -- to exactly one person.
            CREATE UNIQUE INDEX IF NOT EXISTS users_one_owner
                ON users (role) WHERE role = 'owner';

            ALTER TABLE sales ADD COLUMN user_id TEXT REFERENCES users(id);

            CREATE TABLE IF NOT EXISTS login_attempts (
                user_id      TEXT PRIMARY KEY REFERENCES users(id),
                failures     INTEGER NOT NULL DEFAULT 0 CHECK (failures >= 0),
                locked_until INTEGER
            );
        "#,
    },
    // Who the shop buys from.
    //
    // A supplier is a party, not a catalog row: the id is a UUIDv7 for the same
    // reason the staff ids are, because the same supplier is the same supplier
    // in the back office and on another till. Sync metadata is still absent
    // until the sync layer lands; the id is the part that would be expensive to
    // change later.
    //
    // Suppliers are archived rather than deleted. A purchase order will point
    // at one, and "who did we buy this from" must survive the shop changing
    // hands or a rep moving on.
    Migration {
        version: 6,
        name: "create_suppliers",
        sql: r#"
            CREATE TABLE IF NOT EXISTS suppliers (
                id           TEXT PRIMARY KEY,
                name         TEXT NOT NULL CHECK (length(trim(name)) > 0),
                contact_name TEXT,
                phone        TEXT,
                email        TEXT,
                address      TEXT,
                notes        TEXT,
                archived     INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
                created_at   TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE UNIQUE INDEX IF NOT EXISTS suppliers_name
                ON suppliers (name COLLATE NOCASE);
        "#,
    },
];

/// Repair a `suppliers` table left behind by an intermediate build of the
/// directory.
///
/// Migration 6 creates the table with `IF NOT EXISTS`, which is the right rule
/// for a forward-only schema. A database that already held an earlier,
/// unfinished `suppliers` table therefore kept the old columns while recording
/// version 6, and the first save failed with "no column named contact_name".
/// `CREATE TABLE IF NOT EXISTS` cannot fix that, and an applied migration must
/// never be edited, so the repair runs before the migrations.
///
/// The old shape was `(id INTEGER, name, contact, phone, email)`. Rows are
/// preserved: `contact` becomes `contact_name`, ids become the strings the rest
/// of the schema uses, and names are made case-insensitively unique before the
/// index goes back on. On every other database this is a cheap no-op: it reads
/// the column list and returns without writing.
pub fn repair_suppliers(store: &storage::Store) -> anyhow::Result<()> {
    // Never touch a database a newer build has already moved past this schema.
    let newest = MIGRATIONS.last().map_or(0, |migration| migration.version);
    if store.user_version()? > newest {
        return Ok(());
    }

    let columns: Vec<String> = {
        let mut statement = store.conn().prepare("PRAGMA table_info(suppliers)")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
        rows.collect::<Result<_, _>>()?
    };
    if columns.is_empty() || columns.iter().any(|column| column == "contact_name") {
        return Ok(());
    }

    let transaction = store.conn().unchecked_transaction()?;
    transaction.execute_batch(
        r#"
            ALTER TABLE suppliers RENAME TO suppliers_intermediate;

            CREATE TABLE suppliers (
                id           TEXT PRIMARY KEY,
                name         TEXT NOT NULL CHECK (length(trim(name)) > 0),
                contact_name TEXT,
                phone        TEXT,
                email        TEXT,
                address      TEXT,
                notes        TEXT,
                archived     INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
                created_at   TEXT NOT NULL DEFAULT (datetime('now'))
            );

            INSERT INTO suppliers (id, name, contact_name, phone, email)
                SELECT CAST(id AS TEXT), name, contact, phone, email
                FROM suppliers_intermediate;

            DROP TABLE suppliers_intermediate;
        "#,
    )?;

    // The old table only checked that a name was non-empty, so it may hold
    // duplicates. Rename them rather than refusing to open the till: a
    // directory with "Acme" and "Acme (2)" is usable, a till that will not
    // start is not.
    let mut taken = std::collections::HashSet::new();
    let rows: Vec<(String, String)> = {
        let mut statement = transaction.prepare("SELECT id, name FROM suppliers ORDER BY rowid")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    for (id, name) in rows {
        let mut candidate = name.clone();
        let mut suffix = 2;
        while !taken.insert(candidate.to_lowercase()) {
            candidate = format!("{name} ({suffix})");
            suffix += 1;
        }
        if candidate != name {
            transaction.execute(
                "UPDATE suppliers SET name = ?2 WHERE id = ?1",
                (&id, &candidate),
            )?;
        }
    }

    transaction.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS suppliers_name
             ON suppliers (name COLLATE NOCASE);",
    )?;
    transaction.commit()?;
    Ok(())
}

/// Someone who works the till.
///
/// The credential is carried around because the lock screen needs to check it;
/// nothing renders it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct User {
    pub id: String,
    pub name: String,
    pub role: crate::auth::Role,
    pub credential: String,
    pub kind: crate::auth::CredentialKind,
    pub active: bool,
}

/// Every account, owner first, then managers, then cashiers.
///
/// Switched-off accounts are included: Settings has to list them to switch them
/// back on, and the lock screen filters them out itself.
pub fn users(store: &storage::Store) -> Vec<User> {
    let mut statement = match store.conn().prepare(
        "SELECT id, name, role, credential, kind, active
         FROM users
         ORDER BY CASE role WHEN 'owner' THEN 0 WHEN 'manager' THEN 1 ELSE 2 END,
                  name COLLATE NOCASE",
    ) {
        Ok(statement) => statement,
        Err(err) => {
            tracing::warn!(error = %err, "users prepare failed");
            return Vec::new();
        }
    };

    let rows = statement.query_map([], |row| {
        let role: String = row.get(2)?;
        let kind: String = row.get(4)?;
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            role,
            row.get::<_, String>(3)?,
            kind,
            row.get::<_, bool>(5)?,
        ))
    });

    let Ok(rows) = rows else {
        tracing::warn!("users query failed");
        return Vec::new();
    };

    rows.filter_map(Result::ok)
        .filter_map(|(id, name, role, credential, kind, active)| {
            Some(User {
                id,
                name,
                role: crate::auth::Role::from_key(&role)?,
                credential,
                kind: crate::auth::CredentialKind::from_key(&kind)?,
                active,
            })
        })
        .collect()
}

/// The owner account, which exists once the till has been set up.
pub fn owner(store: &storage::Store) -> Option<User> {
    users(store)
        .into_iter()
        .find(|user| user.role == crate::auth::Role::Owner)
}

/// Add an account and return it, id included.
pub fn insert_user(
    store: &storage::Store,
    name: &str,
    role: crate::auth::Role,
    kind: crate::auth::CredentialKind,
    credential: &str,
) -> anyhow::Result<User> {
    let id = uuid::Uuid::now_v7().to_string();
    store.conn().execute(
        "INSERT INTO users (id, name, role, credential, kind) VALUES (?1, ?2, ?3, ?4, ?5)",
        (id.as_str(), name, role.key(), credential, kind.key()),
    )?;

    Ok(User {
        id,
        name: name.to_owned(),
        role,
        credential: credential.to_owned(),
        kind,
        active: true,
    })
}

/// Switch an account on or off. Accounts are never deleted: the sales they took
/// still name them.
pub fn set_user_active(store: &storage::Store, id: &str, active: bool) -> anyhow::Result<()> {
    let changed = store
        .conn()
        .execute("UPDATE users SET active = ?2 WHERE id = ?1", (id, active))?;
    anyhow::ensure!(changed == 1, "that account no longer exists");
    Ok(())
}

/// Replace an account's secret. The kind does not change with it.
pub fn set_user_credential(
    store: &storage::Store,
    id: &str,
    credential: &str,
) -> anyhow::Result<()> {
    let changed = store.conn().execute(
        "UPDATE users SET credential = ?2 WHERE id = ?1",
        (id, credential),
    )?;
    anyhow::ensure!(changed == 1, "that account no longer exists");
    Ok(())
}

/// Failures on record for an account, and the second it is locked until.
pub fn attempts(store: &storage::Store, user_id: &str) -> (u32, Option<i64>) {
    store
        .conn()
        .query_row(
            "SELECT failures, locked_until FROM login_attempts WHERE user_id = ?1",
            [user_id],
            |row| Ok((row.get::<_, i64>(0)? as u32, row.get(1)?)),
        )
        .unwrap_or((0, None))
}

/// Remember a failed attempt and the lock it earned.
pub fn record_attempt(
    store: &storage::Store,
    user_id: &str,
    failures: u32,
    locked_until: Option<i64>,
) {
    if let Err(err) = store.conn().execute(
        "INSERT INTO login_attempts (user_id, failures, locked_until) VALUES (?1, ?2, ?3)
         ON CONFLICT(user_id) DO UPDATE SET failures = excluded.failures,
                                            locked_until = excluded.locked_until",
        (user_id, failures as i64, locked_until),
    ) {
        // A till that cannot remember failures still lets the right PIN in; it
        // only loses the throttle, which is better than refusing everyone.
        tracing::error!(error = %err, user = %user_id, "could not record a failed sign-in");
    }
}

/// A correct secret clears the record, so yesterday's mistakes do not count.
pub fn clear_attempts(store: &storage::Store, user_id: &str) {
    if let Err(err) = store
        .conn()
        .execute("DELETE FROM login_attempts WHERE user_id = ?1", [user_id])
    {
        tracing::error!(error = %err, user = %user_id, "could not clear failed sign-ins");
    }
}

/// One value from `meta`, or `None` when it has never been set.
pub fn meta(store: &storage::Store, key: &str) -> Option<String> {
    store
        .conn()
        .query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .ok()
}

/// Write one `meta` value, replacing it if it exists.
pub fn set_meta(store: &storage::Store, key: &str, value: &str) -> anyhow::Result<()> {
    store.conn().execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        (key, value),
    )?;
    Ok(())
}

/// The currency this install prices in, or `None` before first-run setup.
///
/// An unreadable or unknown value is treated as "not chosen yet": the picker
/// opens, and the operator cannot end up selling in a currency the data no
/// longer recognises.
pub fn currency(store: &storage::Store) -> Option<money::Currency> {
    let code = meta(store, "currency")?;

    let currency = money::tender_from_code(&code);
    if currency.is_none() {
        tracing::warn!(code = %code, "stored currency is not a tender");
    }
    currency
}

/// Store the install's currency, replacing a previous choice.
pub fn set_currency(store: &storage::Store, currency: money::Currency) -> anyhow::Result<()> {
    set_meta(store, "currency", currency.code())
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

/// Every supplier, by name. Archived rows are included; the screen decides
/// whether to show them, which keeps one read for both views.
pub fn suppliers(store: &storage::Store) -> Vec<Supplier> {
    let mut statement = match store.conn().prepare(
        "SELECT id, name, contact_name, phone, email, address, notes, archived
         FROM suppliers ORDER BY name COLLATE NOCASE",
    ) {
        Ok(statement) => statement,
        Err(err) => {
            tracing::warn!(error = %err, "suppliers prepare failed");
            return Vec::new();
        }
    };

    let rows = statement.query_map([], |row| {
        Ok(Supplier {
            id: row.get(0)?,
            name: row.get(1)?,
            contact_name: row.get(2)?,
            phone: row.get(3)?,
            email: row.get(4)?,
            address: row.get(5)?,
            notes: row.get(6)?,
            archived: row.get(7)?,
        })
    });

    match rows {
        Ok(rows) => rows.filter_map(Result::ok).collect(),
        Err(err) => {
            tracing::warn!(error = %err, "suppliers query failed");
            Vec::new()
        }
    }
}

/// Add a supplier and return it, id included.
///
/// A blank optional field is stored as `NULL`, so "no phone" is one value
/// rather than an empty string every reader has to remember to trim.
pub fn insert_supplier(
    store: &storage::Store,
    fields: &SupplierFields<'_>,
) -> anyhow::Result<Supplier> {
    let name = fields.name.trim();
    anyhow::ensure!(!name.is_empty(), "a supplier needs a name");

    let id = uuid::Uuid::now_v7().to_string();
    let contact_name = optional_text(fields.contact_name);
    let phone = optional_text(fields.phone);
    let email = optional_text(fields.email);
    let address = optional_text(fields.address);
    let notes = optional_text(fields.notes);

    store.conn().execute(
        "INSERT INTO suppliers (id, name, contact_name, phone, email, address, notes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        (
            id.as_str(),
            name,
            contact_name,
            phone,
            email,
            address,
            notes,
        ),
    )?;

    Ok(Supplier {
        id,
        name: name.to_owned(),
        contact_name: contact_name.map(str::to_owned),
        phone: phone.map(str::to_owned),
        email: email.map(str::to_owned),
        address: address.map(str::to_owned),
        notes: notes.map(str::to_owned),
        archived: false,
    })
}

/// Replace a supplier's editable fields.
///
/// Returns an error when the row is gone: the list on screen is a cache, and
/// "Saved" over a supplier that no longer exists is a lie the operator would
/// only discover on the next restart.
pub fn update_supplier(
    store: &storage::Store,
    id: &str,
    fields: &SupplierFields<'_>,
) -> anyhow::Result<()> {
    let name = fields.name.trim();
    anyhow::ensure!(!name.is_empty(), "a supplier needs a name");

    let changed = store.conn().execute(
        "UPDATE suppliers
         SET name = ?2, contact_name = ?3, phone = ?4, email = ?5,
             address = ?6, notes = ?7
         WHERE id = ?1",
        (
            id,
            name,
            optional_text(fields.contact_name),
            optional_text(fields.phone),
            optional_text(fields.email),
            optional_text(fields.address),
            optional_text(fields.notes),
        ),
    )?;
    anyhow::ensure!(changed == 1, "supplier {id} is no longer in the directory");
    Ok(())
}

/// Hide a supplier from the active list, or bring it back.
///
/// The row stays because a purchase order may point at it; archival is the
/// supplier equivalent of a staff account being switched off.
pub fn set_supplier_archived(
    store: &storage::Store,
    id: &str,
    archived: bool,
) -> anyhow::Result<()> {
    let changed = store.conn().execute(
        "UPDATE suppliers SET archived = ?2 WHERE id = ?1",
        (id, archived),
    )?;
    anyhow::ensure!(changed == 1, "supplier {id} is no longer in the directory");
    Ok(())
}

/// Whether a name already belongs to a supplier other than `except`.
///
/// Checked before the write so the operator gets "that name is taken" instead
/// of a constraint error. The unique index still enforces it.
pub fn supplier_name_taken(store: &storage::Store, name: &str, except: Option<&str>) -> bool {
    store
        .conn()
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM suppliers
                 WHERE name = ?1 COLLATE NOCASE AND id IS NOT ?2
             )",
            (name.trim(), except),
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(true)
}

/// A blank optional field is the absence of a value, not a value.
fn optional_text(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
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
///
/// `user_id` is the cashier the money is attributed to. It is `None` only for a
/// sale recorded while the till has no staff accounts, which cannot happen once
/// the owner exists.
pub fn record_sale(
    store: &storage::Store,
    lines: &[SaleLine],
    rounding_minor: i64,
    tendered_minor: i64,
    user_id: Option<&str>,
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
        "INSERT INTO sales (total_minor, rounding_minor, user_id) VALUES (?1, ?2, ?3)",
        (total_minor, rounding_minor, user_id),
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
/// where "7 days" starts. The column is passed in because `sales` joins `users`,
/// and both tables have a `created_at`.
fn range_clause(range: usize, column: &str) -> Option<String> {
    match range {
        // `localdate` on purpose: the cashier's "today" is the terminal's day,
        // not UTC. Any other choice moves the day boundary at the wrong hour for
        // half the world, and the till's report disagrees with the drawer.
        0 => Some(format!(
            "date({column}, 'localtime') = date('now', 'localtime')"
        )),
        // Whole calendar days, not a rolling 168 hours: the last week is seven
        // bars, not eight with the two ends cut off at the clock, and the chip
        // says what the chart shows. The boundary is a UTC instant computed
        // once, so the column still answers from its index.
        1 => Some(format!(
            "{column} >= datetime(date('now', 'localtime', '-6 days'), 'utc')"
        )),
        2 => Some(format!(
            "{column} >= datetime(date('now', 'localtime', '-29 days'), 'utc')"
        )),
        _ => None,
    }
}

/// Revenue and sale count for a range.
///
/// A failed query yields zeros and a logged warning rather than an error the
/// shell has to render — the dashboard is a summary, not the source of truth.
pub fn totals(store: &storage::Store, range: usize) -> (i64, i64) {
    let sql = match range_clause(range, "created_at") {
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
    let sql = match range_clause(range, "s.created_at") {
        Some(clause) => format!(
            "SELECT s.id, s.total_minor + s.rounding_minor, strftime('%d/%m %H:%M', s.created_at, 'localtime'), u.name
             FROM sales s LEFT JOIN users u ON u.id = s.user_id
             WHERE {clause} ORDER BY s.id DESC LIMIT ?1"
        ),
        None => "SELECT s.id, s.total_minor + s.rounding_minor, strftime('%d/%m %H:%M', s.created_at, 'localtime'), u.name
             FROM sales s LEFT JOIN users u ON u.id = s.user_id
             ORDER BY s.id DESC LIMIT ?1"
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
            cashier: row.get(3)?,
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

/// Every sale in the range, for the CSV export.
///
/// No `LIMIT`: the screen shows the newest hundred, but a file that silently
/// stops at a hundred is a file somebody reconciles against and finds short.
/// Errors are returned rather than logged: writing a header-only file and
/// calling it a successful export would hide a broken query.
///
/// `GROUP BY s.id` keeps one row per sale when a sale is paid in parts. The
/// method string is sorted in Rust so two orders of the same split cannot
/// produce two different books.
pub fn export_sales(store: &storage::Store, range: usize) -> anyhow::Result<Vec<ExportSale>> {
    let sql = match range_clause(range, "s.created_at") {
        Some(clause) => format!(
            "SELECT s.id, strftime('%Y-%m-%d %H:%M:%S', s.created_at, 'localtime'), u.name,
                    s.total_minor, s.rounding_minor, s.total_minor + s.rounding_minor,
                    COALESCE(GROUP_CONCAT(DISTINCT p.method), '')
             FROM sales s
             LEFT JOIN users u ON u.id = s.user_id
             LEFT JOIN payments p ON p.sale_id = s.id
             WHERE {clause}
             GROUP BY s.id ORDER BY s.id"
        ),
        None => "SELECT s.id, strftime('%Y-%m-%d %H:%M:%S', s.created_at, 'localtime'), u.name,
                    s.total_minor, s.rounding_minor, s.total_minor + s.rounding_minor,
                    COALESCE(GROUP_CONCAT(DISTINCT p.method), '')
             FROM sales s
             LEFT JOIN users u ON u.id = s.user_id
             LEFT JOIN payments p ON p.sale_id = s.id
             GROUP BY s.id ORDER BY s.id"
            .to_owned(),
    };

    let mut statement = store
        .conn()
        .prepare(&sql)
        .context("preparing the sales export")?;
    let rows = statement
        .query_map([], |row| {
            Ok(ExportSale {
                id: row.get(0)?,
                created_at: row.get(1)?,
                cashier: row.get(2)?,
                total_minor: row.get(3)?,
                rounding_minor: row.get(4)?,
                paid_minor: row.get(5)?,
                method: row.get(6)?,
            })
        })
        .context("reading the sales export")?;
    let mut sales: Vec<ExportSale> = rows.collect::<Result<_, _>>().context("reading a sale")?;

    for sale in &mut sales {
        let method = {
            let mut methods: Vec<&str> = sale
                .method
                .split(',')
                .filter(|method| !method.is_empty())
                .collect();
            methods.sort_unstable();
            methods.join("+")
        };
        sale.method = method;
    }

    Ok(sales)
}

/// One line of a sale, as recorded at the time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetailLine {
    /// The name on the receipt. The catalog may have moved on since.
    pub name: String,
    /// Units sold. Always at least one; respected rather than implied.
    pub qty: i64,
    /// Per-unit price in minor units, as charged.
    pub unit_minor: i64,
    /// `qty * unit` as recorded, not recomputed: the sale is the record of
    /// what was charged, and recomputing would only invite disagreement.
    pub total_minor: i64,
}

/// The whole record of one sale: its lines, its money, and who took it.
///
/// The list answers "how much came in"; this answers "what exactly was for
/// that" — the receipt a customer might come back about, or the discrepancy an
/// owner is looking for at the end of a shift.
///
/// Problems `IO`: today the till pays cash only, so `method` is always cash.
/// Payments are already summed in Rust (not `SUM` in SQL) because a split
/// payment — part cash, part something else one day — must read honestly, and
/// each row's method tells the story; a combined string handles both.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaleDetail {
    pub id: i64,
    pub cashier: Option<String>,
    /// Local time, like the list shows: the stored value stays UTC.
    pub created_at: String,
    /// The pre-rounding amount.
    pub total_minor: i64,
    /// Cash rounding on top of `total` — `0` most of the time, and what the
    /// customer paid above the exact total otherwise.
    pub rounding_minor: i64,
    /// `total + rounding`: what actually entered the drawer for this sale.
    pub paid_minor: i64,
    /// " + "-joined payment methods. One sale; one day, "cash + mobile".
    pub method: String,
    /// What was handed over, summed across payments. Cash only today.
    pub tendered_minor: i64,
    /// What went back, summed across payments.
    pub change_minor: i64,
    pub lines: Vec<DetailLine>,
}

/// The full record of one sale, or `None` when the id points nowhere. Rows are
/// never edited — a wrong checkout is a new sale — so the head and its lines
/// always describe one recorded transaction, not a moving target.
pub fn sale_detail(store: &storage::Store, id: i64) -> Option<SaleDetail> {
    let conn = store.conn();

    let head = match conn.query_row(
        "SELECT s.total_minor, s.rounding_minor, strftime('%d/%m/%Y %H:%M', s.created_at, 'localtime'), u.name
         FROM sales s LEFT JOIN users u ON u.id = s.user_id
         WHERE s.id = ?1",
        [id],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        },
    ) {
        Ok(head) => head,
        Err(err) => {
            tracing::warn!(error = %err, sale = id, "sale detail not found");
            return None;
        }
    };

    let lines = {
        let mut statement = match conn
            .prepare("SELECT name, qty, unit_minor, total_minor FROM sale_lines WHERE sale_id = ?1")
        {
            Ok(statement) => statement,
            Err(err) => {
                tracing::warn!(error = %err, "sale lines prepare failed");
                return None;
            }
        };
        let rows = statement.query_map([id], |row| {
            Ok(DetailLine {
                name: row.get(0)?,
                qty: row.get(1)?,
                unit_minor: row.get(2)?,
                total_minor: row.get(3)?,
            })
        });
        match rows {
            Ok(rows) => rows.filter_map(Result::ok).collect(),
            Err(err) => {
                tracing::warn!(error = %err, "sale lines query failed");
                Vec::new()
            }
        }
    };

    // Payments are summed in Rust rather than `SUM`-ed in SQL because a sale
    // settled across two methods must show both, not one pile. Today the till
    // writes one cash row per sale, so the loop runs once and is ready for the
    // day a second method lands.
    let mut method_parts = Vec::new();
    let mut tendered_minor = 0;
    let mut change_minor = 0;
    if let Ok(mut statement) =
        conn.prepare("SELECT method, tendered_minor, change_minor FROM payments WHERE sale_id = ?1")
    {
        if let Ok(rows) = statement.query_map([id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        }) {
            for payment in rows.filter_map(Result::ok) {
                method_parts.push(payment.0);
                tendered_minor += payment.1;
                change_minor += payment.2;
            }
        }
    } else {
        tracing::warn!("payments prepare failed");
    };

    Some(SaleDetail {
        id,
        cashier: head.3,
        created_at: head.2,
        total_minor: head.0,
        rounding_minor: head.1,
        paid_minor: head.0 + head.1,
        method: if method_parts.is_empty() {
            "-".to_owned()
        } else {
            method_parts.join(" + ")
        },
        tendered_minor,
        change_minor,
        lines,
    })
}

/// Revenue and sale count per person, for the range — the end-of-shift view.
///
/// Revenue is what the drawer took, `total + rounding`, the same figure the
/// list shows per sale. The name is joined not copied, so a renamed person's
/// history renames with them; "system" sales that predate staff, and sales
/// whose account was removed, land in one unpromising row under `-`.
pub struct PersonTotal {
    pub name: String,
    pub sales: i64,
    pub revenue: i64,
}

pub fn by_person(store: &storage::Store, range: usize) -> Vec<PersonTotal> {
    let sql = match range_clause(range, "s.created_at") {
        Some(clause) => format!(
            "SELECT COALESCE(u.name, '-'), COUNT(*), SUM(s.total_minor + s.rounding_minor)
             FROM sales s LEFT JOIN users u ON u.id = s.user_id
             WHERE {clause}
             GROUP BY s.user_id
             ORDER BY 3 DESC"
        ),
        None => "SELECT COALESCE(u.name, '-'), COUNT(*), SUM(s.total_minor + s.rounding_minor)
             FROM sales s LEFT JOIN users u ON u.id = s.user_id
             GROUP BY s.user_id
             ORDER BY 3 DESC"
            .to_owned(),
    };

    let mut statement = match store.conn().prepare(&sql) {
        Ok(statement) => statement,
        Err(err) => {
            tracing::warn!(error = %err, "by person prepare failed");
            return Vec::new();
        }
    };

    let rows = statement.query_map([], |row| {
        Ok(PersonTotal {
            name: row.get(0)?,
            sales: row.get(1)?,
            revenue: row.get(2)?,
        })
    });

    match rows {
        Ok(rows) => rows.filter_map(Result::ok).collect(),
        Err(err) => {
            tracing::warn!(error = %err, "by person query failed");
            Vec::new()
        }
    }
}

/// One local day's totals — the trend behind the range.
///
/// Revenue is `total + rounding`, the drawer figure `totals` and the list
/// already agree on. Days are newest first and capped at the most recent
/// thirty, so "All time" cannot grow a list taller than the rest of the
/// dashboard; the cap is on display, not on what the database keeps.
pub struct DayTotal {
    /// `dd/mm`, terminal clock — SQLite's `strftime` has no month names.
    /// Grouped on the full date, so two sales on the same day share one row.
    pub day: String,
    /// `0`–`6`, Sunday first, as `strftime('%w')` numbers them. Carried beside
    /// the date because a week reads better as names, and only the calendar
    /// knows which name a date wears.
    pub weekday: u8,
    pub sales: i64,
    pub revenue: i64,
}

pub fn by_day(store: &storage::Store, range: usize) -> Vec<DayTotal> {
    const MAX_DAYS: i64 = 30;

    let sql = match range_clause(range, "s.created_at") {
        Some(clause) => format!(
            "SELECT strftime('%d/%m', s.created_at, 'localtime'),
                    CAST(strftime('%w', s.created_at, 'localtime') AS INTEGER),
                    COUNT(*), COALESCE(SUM(s.total_minor + s.rounding_minor), 0)
             FROM sales s
             WHERE {clause}
             GROUP BY date(s.created_at, 'localtime')
             ORDER BY date(s.created_at, 'localtime') DESC
             LIMIT ?1"
        ),
        None => "SELECT strftime('%d/%m', s.created_at, 'localtime'),
                    CAST(strftime('%w', s.created_at, 'localtime') AS INTEGER),
                    COUNT(*), COALESCE(SUM(s.total_minor + s.rounding_minor), 0)
             FROM sales s
             GROUP BY date(s.created_at, 'localtime')
             ORDER BY date(s.created_at, 'localtime') DESC
             LIMIT ?1"
            .to_owned(),
    };

    let mut statement = match store.conn().prepare(&sql) {
        Ok(statement) => statement,
        Err(err) => {
            tracing::warn!(error = %err, "by day prepare failed");
            return Vec::new();
        }
    };

    let rows = statement.query_map([MAX_DAYS], |row| {
        Ok(DayTotal {
            day: row.get(0)?,
            weekday: row.get(1)?,
            sales: row.get(2)?,
            revenue: row.get(3)?,
        })
    });

    match rows {
        Ok(rows) => rows.filter_map(Result::ok).collect(),
        Err(err) => {
            tracing::warn!(error = %err, "by day query failed");
            Vec::new()
        }
    }
}

/// The range's bestselling lines, ranked by revenue.
///
/// Ranked by the recorded line total, before rounding: rounding belongs to
/// the sale as a whole, not to any one item. Grouped by the name on the line,
/// not by product id — a deleted product's receipts keep their history, and
/// there is no join to break when the catalog changes.
pub struct ItemTotal {
    pub name: String,
    pub qty: i64,
    pub revenue: i64,
}

pub fn top_items(store: &storage::Store, range: usize, limit: i64) -> Vec<ItemTotal> {
    let sql = match range_clause(range, "s.created_at") {
        Some(clause) => format!(
            "SELECT l.name, SUM(l.qty), SUM(l.total_minor)
             FROM sale_lines l JOIN sales s ON s.id = l.sale_id
             WHERE {clause}
             GROUP BY l.name
             ORDER BY 3 DESC
             LIMIT ?1"
        ),
        None => "SELECT l.name, SUM(l.qty), SUM(l.total_minor)
             FROM sale_lines l JOIN sales s ON s.id = l.sale_id
             GROUP BY l.name
             ORDER BY 3 DESC
             LIMIT ?1"
            .to_owned(),
    };

    let mut statement = match store.conn().prepare(&sql) {
        Ok(statement) => statement,
        Err(err) => {
            tracing::warn!(error = %err, "top items prepare failed");
            return Vec::new();
        }
    };

    let rows = statement.query_map([limit], |row| {
        Ok(ItemTotal {
            name: row.get(0)?,
            qty: row.get(1)?,
            revenue: row.get(2)?,
        })
    });

    match rows {
        Ok(rows) => rows.filter_map(Result::ok).collect(),
        Err(err) => {
            tracing::warn!(error = %err, "top items query failed");
            Vec::new()
        }
    }
}

// ---------------------------------------------------------------------------
// Payments taken
// ---------------------------------------------------------------------------

/// What one payment method brought in over a range.
pub struct MethodTotal {
    /// `cash` today; the column exists so card and mobile money need no change
    /// here when they arrive.
    pub method: String,
    /// Sales that used the method, not payments: one sale may be split across
    /// two methods, and counting payment rows would report it twice.
    pub sales: i64,
    /// What stayed in the drawer, rounding included. Summing `tendered_minor`
    /// would count the change that went back out.
    pub amount_minor: i64,
}

/// Money in, grouped by how it was paid, in the same range the dashboard shows.
///
/// This is *taken*, not *expected in the drawer*: without an opening float and a
/// counted close there is nothing to be expected against. The word stays out of
/// the query and out of the screen copy.
pub fn by_method(store: &storage::Store, range: usize) -> Vec<MethodTotal> {
    let sql = match range_clause(range, "s.created_at") {
        Some(clause) => format!(
            "SELECT p.method, COUNT(DISTINCT p.sale_id), COALESCE(SUM(p.amount_minor), 0)
             FROM payments p JOIN sales s ON s.id = p.sale_id
             WHERE {clause}
             GROUP BY p.method ORDER BY 3 DESC"
        ),
        None => "SELECT p.method, COUNT(DISTINCT p.sale_id), COALESCE(SUM(p.amount_minor), 0)
             FROM payments p JOIN sales s ON s.id = p.sale_id
             GROUP BY p.method ORDER BY 3 DESC"
            .to_owned(),
    };

    let mut statement = match store.conn().prepare(&sql) {
        Ok(statement) => statement,
        Err(err) => {
            tracing::warn!(error = %err, "payments by method prepare failed");
            return Vec::new();
        }
    };

    let rows = statement.query_map([], |row| {
        Ok(MethodTotal {
            method: row.get(0)?,
            sales: row.get(1)?,
            amount_minor: row.get(2)?,
        })
    });

    match rows {
        Ok(rows) => rows.filter_map(Result::ok).collect(),
        Err(err) => {
            tracing::warn!(error = %err, "payments by method query failed");
            Vec::new()
        }
    }
}

// ---------------------------------------------------------------------------
// Backup
// ---------------------------------------------------------------------------

/// How many backups are kept before the oldest is deleted.
const BACKUP_KEEP: usize = 10;

/// The prefix every backup name carries. Also what tells a backup apart from
/// the live `pos.db` should the two ever share a directory.
const BACKUP_PREFIX: &str = "pos-";

/// A filename-safe local timestamp, e.g. `20261101-184212`.
///
/// Asked of SQLite rather than the OS so a backup is named by the same clock
/// that stamps sales: two clocks an hour apart would make a file look newer or
/// older than the rows inside it, which is the one thing a restore cannot
/// afford.
pub fn stamp(store: &storage::Store) -> anyhow::Result<String> {
    store
        .conn()
        .query_row(
            "SELECT strftime('%Y%m%d-%H%M%S', 'now', 'localtime')",
            [],
            |row| row.get::<_, String>(0),
        )
        .context("reading the clock")
}

/// A consistent copy of the live database, pruned to the newest ten.
///
/// `VACUUM INTO` rather than a file copy: the till runs in WAL mode, and a plain
/// copy of the main file can miss committed pages still sitting in the log. The
/// snapshot it writes is a complete database on its own and is taken without
/// stopping the till.
///
/// Returns the file it wrote. Errors are returned rather than logged because the
/// operator pressed a button and is owed an answer.
pub fn backup(store: &storage::Store, dir: &Path) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(dir)
        .with_context(|| format!("creating backup directory {}", dir.display()))?;

    let target = next_backup_path(store, dir)?;
    store
        .conn()
        .execute("VACUUM INTO ?1", [target.to_string_lossy().as_ref()])
        .with_context(|| format!("writing backup {}", target.display()))?;

    prune(dir);
    Ok(target)
}

/// The next name in this second's run.
///
/// The stamp has second resolution, so two backups in one second would otherwise
/// collide and `VACUUM INTO` refuses an existing file. A zero-padded sequence is
/// always present — not only on collision — because it is also what makes the
/// names sort: `…-0002` after `…-0001`, and every file of a later second after
/// every file of an earlier one. Pruning can then trust the name alone; a file
/// system's modification time is not precise enough to separate two backups
/// taken a moment apart.
///
/// The sequence continues past the highest number already used rather than
/// taking the first free one. Pruning leaves gaps at the start of the run, and
/// filling a gap would file the newest backup where the name sort expects the
/// oldest — exactly the file the next prune would delete.
fn next_backup_path(store: &storage::Store, dir: &Path) -> anyhow::Result<PathBuf> {
    let stem = format!("{BACKUP_PREFIX}{}", stamp(store)?);
    let next = highest_sequence(dir, &stem)?.map_or(1, |highest| highest.saturating_add(1));
    if next > 9999 {
        anyhow::bail!("more than 9999 backups in one second; the clock or directory is wrong");
    }
    Ok(dir.join(format!("{stem}-{next:04}.db")))
}

/// The highest sequence already used by files named `<stem>-NNNN.db`.
fn highest_sequence(dir: &Path, stem: &str) -> anyhow::Result<Option<u32>> {
    let entries = std::fs::read_dir(dir).with_context(|| format!("listing {}", dir.display()))?;
    Ok(entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let rest = name.to_str()?.strip_prefix(stem)?.strip_prefix('-')?;
            rest.strip_suffix(".db")?.parse().ok()
        })
        .max())
}

/// Delete everything past the newest [`BACKUP_KEEP`].
///
/// Ordered by name, which is chronological by construction (see
/// [`next_backup_path`]). Deletion failures are logged, not returned: the backup
/// succeeded, and reporting it as failed because an old file would not go away
/// would send the operator looking for a file that is there.
fn prune(dir: &Path) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            tracing::warn!(error = %err, dir = %dir.display(), "could not list backups");
            return;
        }
    };

    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_backup(path))
        .collect();

    // Newest first: the last entry of a name sort is the most recent backup.
    files.sort();
    files.reverse();

    for path in files.into_iter().skip(BACKUP_KEEP) {
        if let Err(err) = std::fs::remove_file(&path) {
            tracing::warn!(error = %err, path = %path.display(), "could not delete an old backup");
        }
    }
}

fn is_backup(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name.strip_prefix(BACKUP_PREFIX).is_some() && path.extension().is_some_and(|ext| ext == "db")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{CredentialKind, Role};
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
        assert!(by_day(&store, 3).is_empty());
        assert!(top_items(&store, 3, 5).is_empty());

        let _ = std::fs::remove_file(path);
    }

    /// A sale with its lines, dated relative to now — `sale_lines` carries the
    /// name, so nothing has to exist in the catalog for this to be real.
    fn insert_sale(store: &storage::Store, offset: &str, lines: &[(&str, i64, i64)]) -> i64 {
        let conn = store.conn();
        let total: i64 = lines.iter().map(|(_, qty, unit)| qty * unit).sum();
        conn.execute(
            "INSERT INTO sales (total_minor, created_at) VALUES (?1, datetime('now', ?2))",
            (total, offset),
        )
        .expect("insert sale");
        let id = conn.last_insert_rowid();
        for (name, qty, unit) in lines {
            conn.execute(
                "INSERT INTO sale_lines (sale_id, product_id, name, qty, unit_minor, total_minor)
                 VALUES (?1, NULL, ?2, ?3, ?4, ?5)",
                (id, name, *qty, *unit, qty * unit),
            )
            .expect("insert line");
        }
        id
    }

    #[test]
    fn a_week_is_seven_calendar_days_not_168_hours() {
        let (store, path) = store("week-window");

        insert_sale(&store, "-0 days", &[("Tea", 1, 100)]);
        insert_sale(&store, "-6 days", &[("Tea", 1, 100)]);
        insert_sale(&store, "-7 days", &[("Tea", 1, 100)]);

        assert_eq!(
            by_day(&store, 1).len(),
            2,
            "today and six days back, so a week is seven bars"
        );
        assert_eq!(totals(&store, 1), (2, 200));
        assert_eq!(
            by_day(&store, 2).len(),
            3,
            "the seventh day is still inside the month"
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn the_trend_and_the_top_items_follow_their_sales() {
        let (store, path) = store("analytics");

        // 3500 today, 1000 three days ago, 5000 ten days ago, 500 forty days ago.
        insert_sale(&store, "-0 days", &[("Tea", 3, 500), ("Sugar", 1, 2_000)]);
        insert_sale(&store, "-3 days", &[("Tea", 2, 500)]);
        insert_sale(&store, "-10 days", &[("Rice", 1, 5_000)]);
        insert_sale(&store, "-40 days", &[("Tea", 1, 500)]);

        assert_eq!(
            by_day(&store, 0).len(),
            1,
            "today is one row, however many sales it had"
        );
        assert_eq!(
            by_day(&store, 2).len(),
            3,
            "the forty-day-old sale is outside a 30-day range"
        );

        let days = by_day(&store, 3);
        assert_eq!(days.len(), 4);
        assert_eq!(
            days[0].revenue, 3_500,
            "newest first, like every other list"
        );
        assert_eq!(days[0].sales, 1, "two sales on one day share its row");
        assert_eq!(days[1].revenue, 1_000);
        assert_eq!(days[3].revenue, 500);

        let items = top_items(&store, 3, 5);
        assert_eq!(
            items.len(),
            3,
            "the same product on different days is one item, not three"
        );
        assert_eq!(items[0].name, "Rice", "revenue ranks first, not units");
        assert_eq!(items[0].revenue, 5_000);
        assert_eq!(items[1].name, "Tea");
        assert_eq!(items[1].qty, 6, "units sum across every sale in the range");
        assert_eq!(items[1].revenue, 3_000);
        assert_eq!(items[2].name, "Sugar");
        assert_eq!(items[2].qty, 1);

        assert_eq!(
            top_items(&store, 0, 5)[0].name,
            "Sugar",
            "the ranking follows the chosen range, not all history"
        );
        assert_eq!(
            top_items(&store, 3, 2).len(),
            2,
            "the limit ranks, not hides"
        );

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

    fn supplier_fields(name: &str) -> SupplierFields<'_> {
        SupplierFields {
            name,
            contact_name: "",
            phone: "",
            email: "",
            address: "",
            notes: "",
        }
    }

    #[test]
    fn suppliers_round_trip_and_archive() {
        let (store, path) = store("suppliers");
        assert!(
            suppliers(&store).is_empty(),
            "a fresh till has no suppliers"
        );

        let mut acme_fields = supplier_fields("  Acme  ");
        acme_fields.contact_name = "Amina";
        acme_fields.phone = "+256 700 000";
        acme_fields.email = "orders@acme.test";
        acme_fields.address = "Kampala";
        acme_fields.notes = "Delivers on Tuesdays";
        let acme = insert_supplier(&store, &acme_fields).expect("insert");
        assert_eq!(acme.name, "Acme", "the name is trimmed before it is stored");
        assert_eq!(acme.phone.as_deref(), Some("+256 700 000"));
        assert!(!acme.archived);

        insert_supplier(&store, &supplier_fields("Zebra")).expect("insert");
        let names: Vec<String> = suppliers(&store)
            .into_iter()
            .map(|supplier| supplier.name)
            .collect();
        assert_eq!(
            names,
            ["Acme", "Zebra"],
            "the list is by name, not by insertion order"
        );
        let zebra = suppliers(&store)
            .into_iter()
            .find(|supplier| supplier.name == "Zebra")
            .expect("the supplier was written");
        assert_eq!(
            zebra.phone, None,
            "a blank optional field is stored as NULL"
        );
        assert_eq!(zebra.notes, None);

        let mut updated_fields = supplier_fields("Acme Ltd");
        updated_fields.contact_name = "Amina";
        updated_fields.email = "orders@acme.test";
        update_supplier(&store, &acme.id, &updated_fields).expect("update");
        let updated = suppliers(&store)
            .into_iter()
            .find(|supplier| supplier.id == acme.id)
            .expect("the supplier survived the update");
        assert_eq!(updated.name, "Acme Ltd");
        assert_eq!(updated.phone, None, "clearing a field clears it");

        set_supplier_archived(&store, &acme.id, true).expect("archive");
        assert!(
            suppliers(&store)
                .iter()
                .find(|supplier| supplier.id == acme.id)
                .expect("archived rows stay readable")
                .archived
        );
        set_supplier_archived(&store, &acme.id, false).expect("restore");
        assert!(!suppliers(&store)[0].archived);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_supplier_name_belongs_to_one_supplier() {
        let (store, path) = store("supplier-names");
        let acme = insert_supplier(&store, &supplier_fields("Acme")).expect("insert");

        assert!(supplier_name_taken(&store, "acme", None), "case is ignored");
        assert!(
            !supplier_name_taken(&store, "ACME", Some(&acme.id)),
            "a supplier keeps its own name when it is edited"
        );
        assert!(!supplier_name_taken(&store, "Other", None));
        assert!(
            insert_supplier(&store, &supplier_fields("ACME")).is_err(),
            "the unique index is the real guard, not the pre-check"
        );

        assert!(
            insert_supplier(&store, &supplier_fields("   ")).is_err(),
            "a supplier with no name is not a supplier"
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn editing_a_missing_supplier_fails() {
        let (store, path) = store("missing-supplier");

        assert!(
            update_supplier(&store, "missing", &supplier_fields("Name")).is_err(),
            "'Saved' over a row that is gone is a lie"
        );
        assert!(set_supplier_archived(&store, "missing", true).is_err());

        let _ = std::fs::remove_file(path);
    }

    /// The bug that sent us here: migration 6 was recorded as applied while a
    /// previous build's `suppliers` table was still in place, so every write
    /// failed on the missing `contact_name`. The repair must keep the rows and
    /// leave a table the new code can use.
    #[test]
    fn an_intermediate_supplier_table_is_repaired_without_losing_rows() {
        let (store, path) = store("supplier-repair");

        // The shape the unfinished build left behind: integer ids, one
        // `contact` column, and no unique index.
        store
            .conn()
            .execute_batch(
                "DROP TABLE suppliers;
                 CREATE TABLE suppliers (
                     id      INTEGER PRIMARY KEY,
                     name    TEXT NOT NULL CHECK (length(name) > 0),
                     contact TEXT,
                     phone   TEXT,
                     email   TEXT
                 );
                 INSERT INTO suppliers (id, name, contact, phone, email)
                     VALUES (7, 'Acme', 'Jane', '0700', 'jane@acme.test'),
                            (8, 'acme', 'John', '0800', NULL);",
            )
            .expect("intermediate schema");

        repair_suppliers(&store).expect("repair");

        let repaired = suppliers(&store);
        assert_eq!(repaired.len(), 2, "rows are kept");
        assert_eq!(
            repaired[0].id, "7",
            "an integer id becomes the string the schema uses"
        );
        assert_eq!(repaired[0].contact_name.as_deref(), Some("Jane"));
        assert_eq!(repaired[0].phone.as_deref(), Some("0700"));
        assert_eq!(repaired[0].email.as_deref(), Some("jane@acme.test"));
        assert_eq!(
            repaired[1].name, "acme (2)",
            "a duplicate name is made unique"
        );
        assert!(supplier_name_taken(&store, "ACME", None));

        // Running it again changes nothing: boot calls it every time.
        repair_suppliers(&store).expect("second repair");
        assert_eq!(suppliers(&store).len(), 2);
        insert_supplier(&store, &supplier_fields("Beta"))
            .expect("the repaired table accepts writes");

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
        let sale = record_sale(&store, &lines, -20, 4_000, None).expect("record");

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
            record_sale(&store, &lines, 0, 2_000, None).is_err(),
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

        assert!(record_sale(&store, &[], 0, 0, None).is_err());
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
            None,
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

    #[test]
    fn a_sale_names_the_person_who_took_it() {
        let (store, path) = store("sale-cashier");

        let owner = insert_user(
            &store,
            "Amina",
            Role::Owner,
            CredentialKind::Password,
            "$argon2id$owner",
        )
        .expect("owner");
        let brian = insert_user(
            &store,
            "Brian",
            Role::Cashier,
            CredentialKind::Pin,
            "$argon2id$brian",
        )
        .expect("cashier");

        // One owner, and one person per name whatever the capitals: the lock
        // screen is a list of names, and two Amimas would be a way to steal a
        // shift.
        assert!(
            insert_user(
                &store,
                "Amina",
                Role::Owner,
                CredentialKind::Password,
                "$argon2id$other"
            )
            .is_err()
        );
        assert!(
            insert_user(
                &store,
                "brian",
                Role::Cashier,
                CredentialKind::Pin,
                "$argon2id$x"
            )
            .is_err()
        );

        let sale = record_sale(
            &store,
            &[SaleLine {
                product_id: None,
                name: "Tea".to_owned(),
                qty: 1,
                unit_minor: 500,
            }],
            0,
            500,
            Some(&brian.id),
        )
        .expect("record");

        assert_eq!(sale.paid_minor(), 500);
        let history = sales(&store, 3, 10);
        assert_eq!(history[0].cashier.as_deref(), Some("Brian"));

        // Switching somebody off keeps their name on their history: the money
        // was still taken by that person.
        set_user_active(&store, &brian.id, false).expect("switch off");
        assert_eq!(
            users(&store)
                .iter()
                .find(|user| user.id == brian.id)
                .map(|user| user.active),
            Some(false)
        );
        assert_eq!(sales(&store, 3, 10)[0].cashier.as_deref(), Some("Brian"));

        // Owner first, then managers, then cashiers.
        assert_eq!(users(&store).first().map(|user| &user.id), Some(&owner.id));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_lockout_survives_the_app_closing() {
        let (store, path) = store("lockout");
        let cashier = insert_user(
            &store,
            "Brian",
            Role::Cashier,
            CredentialKind::Pin,
            "$argon2id$brian",
        )
        .expect("cashier");

        assert_eq!(attempts(&store, &cashier.id), (0, None));

        record_attempt(&store, &cashier.id, 5, Some(1_000));
        assert_eq!(attempts(&store, &cashier.id), (5, Some(1_000)));

        // A further failure overwrites rather than stacking a second row.
        record_attempt(&store, &cashier.id, 6, Some(2_000));
        assert_eq!(attempts(&store, &cashier.id), (6, Some(2_000)));

        clear_attempts(&store, &cashier.id);
        assert_eq!(attempts(&store, &cashier.id), (0, None));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_sale_detail_reads_back_its_receipt() {
        let (store, path) = store("sale-detail");
        let brian = insert_user(
            &store,
            "Brian",
            Role::Cashier,
            CredentialKind::Pin,
            "$argon2id$brian",
        )
        .expect("cashier");

        let sale = record_sale(
            &store,
            &[
                SaleLine {
                    product_id: None,
                    name: "Sugar 1kg".to_owned(),
                    qty: 2,
                    unit_minor: 8000,
                },
                SaleLine {
                    product_id: None,
                    name: "Tea".to_owned(),
                    qty: 1,
                    unit_minor: 500,
                },
            ],
            // Total is 16 500; UGX coins end in 100, so the till rounds up 100.
            100,
            16_600,
            Some(&brian.id),
        )
        .expect("record");

        let detail = sale_detail(&store, sale.id).expect("detail");
        assert_eq!(detail.total_minor, 16_500);
        assert_eq!(detail.rounding_minor, 100);
        assert_eq!(detail.paid_minor, 16_600);
        assert_eq!(detail.cashier.as_deref(), Some("Brian"));
        assert_eq!(detail.tendered_minor, 16_600);
        assert_eq!(detail.change_minor, 0);
        assert_eq!(detail.method, "cash");

        assert_eq!(detail.lines.len(), 2);
        assert_eq!(detail.lines[0].name, "Sugar 1kg");
        assert_eq!(detail.lines[0].qty, 2);
        assert_eq!(detail.lines[0].unit_minor, 8000);
        assert_eq!(detail.lines[0].total_minor, 16_000);
        assert_eq!(detail.lines[1].name, "Tea");
        assert_eq!(detail.lines[1].total_minor, 500);

        // An id that points nowhere reads as nothing, not as an empty record
        // pretending to be sale 99.
        assert!(sale_detail(&store, 999_999).is_none());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_change_returns_in_the_sale_detail() {
        let (store, path) = store("sale-change");

        let sale = record_sale(
            &store,
            &[SaleLine {
                product_id: None,
                name: "Tea".to_owned(),
                qty: 1,
                unit_minor: 500,
            }],
            0,
            1_000,
            None,
        )
        .expect("record");

        let detail = sale_detail(&store, sale.id).expect("detail");
        assert_eq!(detail.tendered_minor, 1_000);
        assert_eq!(detail.change_minor, 500);
        // A sale written before the till had staff still opens: the receipt is
        // the record, and "someone" is honest enough for its header.
        assert_eq!(detail.cashier, None);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn the_shift_totals_reach_the_person_who_took_the_money() {
        let (store, path) = store("by-person");
        let brian = insert_user(
            &store,
            "Brian",
            Role::Cashier,
            CredentialKind::Pin,
            "$argon2id$brian",
        )
        .expect("cashier");
        let amina = insert_user(
            &store,
            "Amina",
            Role::Owner,
            CredentialKind::Password,
            "$argon2id$amina",
        )
        .expect("owner");

        // Brian, two sales: 5 000 exact and 340 with rounding up 60 — the
        // UGX-denomination case that made this column worth having.
        record_sale(
            &store,
            &[SaleLine {
                product_id: None,
                name: "Tea".to_owned(),
                qty: 1,
                unit_minor: 5_000,
            }],
            0,
            5_000,
            Some(&brian.id),
        )
        .expect("record");
        record_sale(
            &store,
            &[SaleLine {
                product_id: None,
                name: "Sugar".to_owned(),
                qty: 1,
                unit_minor: 340,
            }],
            60,
            400,
            Some(&brian.id),
        )
        .expect("record");
        // Amina, one sale, before staff existed this would land under "-".
        record_sale(
            &store,
            &[SaleLine {
                product_id: None,
                name: "Rice".to_owned(),
                qty: 1,
                unit_minor: 12_000,
            }],
            0,
            12_000,
            Some(&amina.id),
        )
        .expect("record");

        let people = by_person(&store, 3);
        // Ordered by revenue, not by count: Amina takes fewer, bigger sales.
        assert_eq!(people.len(), 2);
        assert_eq!(people[0].name, "Amina");
        assert_eq!(people[0].sales, 1);
        assert_eq!(people[0].revenue, 12_000);
        assert_eq!(people[1].name, "Brian");
        assert_eq!(people[1].sales, 2);
        assert_eq!(
            people[1].revenue, 5_400,
            "5 000 exact plus 400 that the drawer took for a 340 sale"
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_sale_that_predates_staff_totals_under_a_dash() {
        let (store, path) = store("by-person-dash");

        record_sale(
            &store,
            &[SaleLine {
                product_id: None,
                name: "Tea".to_owned(),
                qty: 1,
                unit_minor: 500,
            }],
            0,
            500,
            None,
        )
        .expect("record");

        let people = by_person(&store, 3);
        assert_eq!(people.len(), 1);
        assert_eq!(people[0].name, "-");
        assert_eq!(people[0].sales, 1);
        assert_eq!(people[0].revenue, 500);

        let _ = std::fs::remove_file(path);
    }

    /// Cash, two sales, one of them rounded. The method total is what the drawer
    /// kept, so the rounded amount counts — not the pre-rounding total the
    /// customer's items added up to, and not the note that was handed over.
    #[test]
    fn payments_taken_are_the_amounts_that_stayed() {
        let (store, path) = store("by-method");

        record_sale(
            &store,
            &[SaleLine {
                product_id: None,
                name: "Bread".to_owned(),
                qty: 1,
                unit_minor: 340,
            }],
            60,
            1_000,
            None,
        )
        .expect("record a rounded sale");
        record_sale(
            &store,
            &[SaleLine {
                product_id: None,
                name: "Milk".to_owned(),
                qty: 1,
                unit_minor: 500,
            }],
            0,
            500,
            None,
        )
        .expect("record an exact sale");

        let methods = by_method(&store, 3);
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].method, "cash");
        assert_eq!(methods[0].sales, 2);
        assert_eq!(methods[0].amount_minor, 900, "400 rounded + 500 exact");

        let _ = std::fs::remove_file(path);
    }

    /// A range nobody sold in is an empty list, not a row of zeros: the card
    /// draws "no money taken" from emptiness, and a zero row would draw a table.
    #[test]
    fn a_range_with_no_sales_reports_no_methods() {
        let (store, path) = store("by-method-empty");

        assert!(by_method(&store, 0).is_empty());

        let _ = std::fs::remove_file(path);
    }

    /// A sale paid in two parts is one line in the books, not two: an ungrouped
    /// join would double every split sale and nobody would notice until a
    /// reconciliation came up short.
    #[test]
    fn a_split_sale_exports_as_one_row_with_both_methods() {
        let (store, path) = store("export-split");

        store
            .conn()
            .execute(
                "INSERT INTO sales (total_minor, rounding_minor) VALUES (1000, 0)",
                [],
            )
            .expect("insert sale");
        let id = store.conn().last_insert_rowid();
        for (method, amount) in [("mobile", 400_i64), ("cash", 600)] {
            store
                .conn()
                .execute(
                    "INSERT INTO payments
                        (sale_id, method, amount_minor, tendered_minor, change_minor)
                     VALUES (?1, ?2, ?3, ?3, 0)",
                    (id, method, amount),
                )
                .expect("insert payment");
        }

        let rows = export_sales(&store, 3).expect("export");
        assert_eq!(rows.len(), 1, "a split sale is one sale");
        assert_eq!(
            rows[0].method, "cash+mobile",
            "sorted, so the file does not depend on insert order"
        );
        assert_eq!(rows[0].total_minor, 1_000);
        assert_eq!(rows[0].rounding_minor, 0);
        assert_eq!(rows[0].paid_minor, 1_000);
        assert!(rows[0].cashier.is_none());

        let _ = std::fs::remove_file(path);
    }

    /// A backup is only a backup if it opens: the file has to be a complete
    /// database with this binary's schema, not a copy of whatever pages
    /// happened to be flushed.
    #[test]
    fn a_backup_is_a_database_that_opens() {
        let (store, path) = store("backup");
        let dir = std::env::temp_dir().join(format!("pos-backup-open-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let file = backup(&store, &dir).expect("backup");

        assert!(file.starts_with(&dir), "{}", file.display());
        assert!(
            file.metadata().expect("metadata").len() > 0,
            "an empty file is not a backup"
        );
        let copy = storage::Store::open(&file).expect("a backup opens");
        assert_eq!(
            copy.user_version().expect("a version"),
            store.user_version().expect("a version")
        );

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(path);
    }

    /// Twelve presses do not mean twelve files: exactly the newest ten remain,
    /// and the newest — the one somebody would actually restore — stays.
    ///
    /// The expected set is derived from the same names the backup chose rather
    /// than from "the first two went", because the run can cross a second
    /// boundary — and then the eleventh file is older than the first of the new
    /// second, which is the ordering the prune has to get right anyway.
    #[test]
    fn only_the_newest_ten_backups_are_kept() {
        let (store, path) = store("backup-prune");
        let dir = std::env::temp_dir().join(format!("pos-backup-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let written: Vec<PathBuf> = (0..12)
            .map(|_| backup(&store, &dir).expect("backup"))
            .collect();

        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .expect("list backups")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect();

        let mut expected = written.clone();
        expected.sort();
        expected.drain(..written.len() - BACKUP_KEEP);

        files.sort();
        assert_eq!(files, expected);
        assert!(files.contains(&written[11]), "the newest survives");

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(path);
    }

    /// A sequence that has been pruned is still used up: filling the gap would
    /// hand the newest backup a name the sort reads as the oldest, and the next
    /// prune would delete it. Planted files make this independent of how fast
    /// twelve real backups run.
    #[test]
    fn a_pruned_sequence_is_not_reused() {
        let (store, path) = store("backup-gap");
        let dir = std::env::temp_dir().join(format!("pos-backup-gap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("make directory");

        let stem = format!("{BACKUP_PREFIX}{}", stamp(&store).expect("clock"));
        for sequence in [3, 12] {
            std::fs::write(dir.join(format!("{stem}-{sequence:04}.db")), b"")
                .expect("planted backup");
        }

        let next = next_backup_path(&store, &dir).expect("next name");
        assert_eq!(
            next.file_name().and_then(|name| name.to_str()),
            Some(format!("{stem}-0013.db").as_str()),
            "the first free number is a gap that must stay empty"
        );

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(path);
    }
}
