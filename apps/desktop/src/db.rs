//! POS schema and the few reads the shell needs.
//!
//! Forward-only: append migrations with a higher version. Never edit one that
//! has shipped, because some till somewhere already applied it.

use anyhow::Context as _;
use std::path::{Path, PathBuf};
use storage::Migration;

use crate::sync::protocol as sync_proto;

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
    /// The row's portable identity: what a peer uses to recognise a replay.
    pub uid: String,
    /// The exact sum of the lines, tax-inclusive.
    pub total_minor: i64,
    /// The cash adjustment, signed: `paid = total + rounding`.
    pub rounding_minor: i64,
    /// The tax the sale collected, included in `total_minor`: `pre_tax =
    /// total - tax`. Zero when the till charges no tax.
    pub tax_minor: i64,
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
///
/// `category` is the denormalized name printed on receipts and reports — it is
/// copied at save time so a receipt never changes when a category is renamed.
/// `category_id` links to [`Category`] so the product form can offer a real
/// dropdown and renaming a category cascades to every product that uses it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Product {
    pub id: i64,
    pub name: String,
    pub price_minor: i64,
    pub category: String,
    pub category_id: Option<i64>,
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

/// A category the shop uses to group products.
///
/// Categories live in their own table so renaming one updates every product
/// that points at it, and so the product form can offer a real dropdown
/// instead of re-deriving the list from the catalog each frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Category {
    pub id: i64,
    pub name: String,
    pub sort_order: i64,
    pub color: Option<String>,
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

/// Why a movement happened.
///
/// The stored spelling is stable: it is in a shipped migration and in every row
/// already written. The label is the till's wording and may change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StockKind {
    /// The count taken when tracking started.
    Opening,
    /// Written by [`record_sale`], one row per sold line.
    Sale,
    /// Goods arriving from a supplier.
    Receive,
    /// Stock coming back onto the shelf.
    Return,
    /// Spoiled, broken, or otherwise unsellable.
    Waste,
    /// A correction after a physical count, positive or negative.
    Adjust,
}

impl StockKind {
    pub const ALL: [StockKind; 6] = [
        StockKind::Opening,
        StockKind::Sale,
        StockKind::Receive,
        StockKind::Return,
        StockKind::Waste,
        StockKind::Adjust,
    ];

    pub fn key(self) -> &'static str {
        match self {
            StockKind::Opening => "opening",
            StockKind::Sale => "sale",
            StockKind::Receive => "receive",
            StockKind::Return => "return",
            StockKind::Waste => "waste",
            StockKind::Adjust => "adjust",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            StockKind::Opening => "Opening count",
            StockKind::Sale => "Sale",
            StockKind::Receive => "Received",
            StockKind::Return => "Returned",
            StockKind::Waste => "Waste",
            StockKind::Adjust => "Adjustment",
        }
    }
}

/// One tracked product and what the ledger says is on the shelf.
///
/// `on_hand` is always `SUM(delta)`, never a stored count. Negative is allowed:
/// the till must keep selling when the shelf and the database disagree, and an
/// adjustment is how the shop says so afterwards.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StockItem {
    pub product_id: i64,
    pub name: String,
    pub category: String,
    pub on_hand: i64,
    pub low_stock: i64,
}

impl StockItem {
    pub fn is_out(&self) -> bool {
        self.on_hand <= 0
    }

    pub fn is_low(&self) -> bool {
        !self.is_out() && self.on_hand <= self.low_stock
    }
}

/// One line of a product's stock history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StockMovement {
    pub id: i64,
    /// Signed: positive puts stock on the shelf, negative takes it off.
    pub delta: i64,
    pub kind: StockKind,
    pub note: Option<String>,
    pub created_at: String,
    /// Who recorded it, when the movement names a staff account.
    pub user: Option<String>,
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
    // Stock, as a ledger rather than a number.
    //
    // `stock_movements` is append-only: every receive, sale, return, waste and
    // adjustment is a signed `delta`, and what the shop has is the sum. That is
    // the only shape that survives corrections — an "on hand" column has to be
    // repaired by hand the first time somebody mistypes a count, while a wrong
    // movement can be answered with another one that says why.
    //
    // Tracking is opt-in per product. A catalog imported from a spreadsheet has
    // no counts, and inventing zero for every row would claim the shop is out
    // of everything. `tracked` marks the rows that have been counted.
    //
    // `low_stock` is a per-product warning line, not a reorder policy: units are
    // whole things on a shelf, and a shop that sells crates and single sweets
    // needs two different lines. Five is a default, not a truth.
    //
    // Deleting a product deletes its movements. This is inventory, not the
    // ledger: there is no valuation or cost to preserve, and a receipt keeps the
    // name and price of what was sold. A purchasing module that needs history
    // past a deleted catalog row will have to carry its own snapshot.
    Migration {
        version: 7,
        name: "create_stock_ledger",
        sql: r#"
            ALTER TABLE products ADD COLUMN tracked INTEGER NOT NULL DEFAULT 0
                CHECK (tracked IN (0, 1));
            ALTER TABLE products ADD COLUMN low_stock INTEGER NOT NULL DEFAULT 5
                CHECK (low_stock >= 0);

            CREATE TABLE IF NOT EXISTS stock_movements (
                id         INTEGER PRIMARY KEY,
                product_id INTEGER NOT NULL REFERENCES products(id) ON DELETE CASCADE,
                delta      INTEGER NOT NULL CHECK (delta <> 0),
                kind       TEXT NOT NULL CHECK
                    (kind IN ('opening', 'sale', 'receive', 'return', 'waste', 'adjust')),
                note       TEXT,
                sale_id    INTEGER REFERENCES sales(id),
                user_id    TEXT REFERENCES users(id),
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE INDEX IF NOT EXISTS stock_movements_product
                ON stock_movements (product_id, id);
            CREATE INDEX IF NOT EXISTS stock_movements_sale
                ON stock_movements (sale_id) WHERE sale_id IS NOT NULL;
        "#,
    },
    // What the sale collected for the tax authority. The rate is a terminal
    // setting that can change, so the amount is written onto the sale itself:
    // a receipt reprinted after the rate moves must show the same tax it did
    // the day the sale happened. `0` means no tax — some small stores are not
    // VAT-registered, and `DEFAULT 0` keeps old rows honest without a rewrite.
    Migration {
        version: 8,
        name: "sale_tax",
        sql: r#"
            ALTER TABLE sales ADD COLUMN tax_minor INTEGER NOT NULL DEFAULT 0
                CHECK (tax_minor >= 0);
        "#,
    },
    // Categories: a proper table so the product form can offer a dropdown and
    // renaming a shelf cascades to every product on it. `category_id` is added
    // to `products` and the old free-text `category` column stays as the
    // denormalized name printed on receipts.
    Migration {
        version: 9,
        name: "create_categories",
        sql: r#"
            CREATE TABLE IF NOT EXISTS categories (
                id          INTEGER PRIMARY KEY,
                name        TEXT    NOT NULL CHECK (length(trim(name)) > 0),
                sort_order  INTEGER NOT NULL DEFAULT 0,
                color       TEXT
            );

            CREATE UNIQUE INDEX IF NOT EXISTS categories_name
                ON categories (name COLLATE NOCASE);

            ALTER TABLE products ADD COLUMN category_id INTEGER
                REFERENCES categories(id) ON DELETE SET NULL;

            CREATE INDEX IF NOT EXISTS products_category
                ON products (category_id);
        "#,
    },
    // The bones of till-to-till replication.
    //
    // A rowid is only meaningful inside the database that assigned it, so every
    // row that travels gets a `uid` beside its integer key. Rows written before
    // this migration keep `uid = NULL`: they are history, and there is no honest
    // way to invent an identity for them. Every new row is a UUIDv7, which sorts
    // by creation time without being a clock the peers have to agree on.
    //
    // `change_seq` is the local feed's cursor, not a timestamp. It is bumped once
    // per transaction that creates a replicable sale, so a peer can ask "what
    // happened after 41?" and a restart or a clock change cannot make it skip or
    // repeat a row. `origin_device` says which install wrote it; `received_at`
    // is set only when a peer's sale arrives here.
    //
    // A product is archived rather than deleted, because a sale line and a stock
    // movement both point at it. The barcode index therefore covers only live
    // products: an archived barcode can be reused, and the operator is not told
    // "taken" by a product they cannot see.
    Migration {
        version: 10,
        name: "sync_foundation",
        sql: r#"
            INSERT INTO meta (key, value) VALUES ('device_id', '')
                ON CONFLICT(key) DO NOTHING;
            INSERT INTO meta (key, value) VALUES ('change_seq', '0')
                ON CONFLICT(key) DO NOTHING;
            INSERT INTO meta (key, value) VALUES ('catalog_version', '1')
                ON CONFLICT(key) DO NOTHING;

            ALTER TABLE sales ADD COLUMN uid TEXT;
            ALTER TABLE sales ADD COLUMN origin_device TEXT;
            ALTER TABLE sales ADD COLUMN is_offline INTEGER NOT NULL DEFAULT 0
                CHECK (is_offline IN (0, 1));
            ALTER TABLE sales ADD COLUMN clock_skew INTEGER NOT NULL DEFAULT 0;
            ALTER TABLE sales ADD COLUMN received_at TEXT;
            ALTER TABLE sales ADD COLUMN change_seq INTEGER;

            ALTER TABLE sale_lines ADD COLUMN uid TEXT;
            ALTER TABLE payments ADD COLUMN uid TEXT;
            ALTER TABLE stock_movements ADD COLUMN uid TEXT;

            ALTER TABLE products ADD COLUMN archived INTEGER NOT NULL DEFAULT 0
                CHECK (archived IN (0, 1));

            CREATE UNIQUE INDEX IF NOT EXISTS sales_uid
                ON sales (uid) WHERE uid IS NOT NULL;
            CREATE UNIQUE INDEX IF NOT EXISTS sale_lines_uid
                ON sale_lines (uid) WHERE uid IS NOT NULL;
            CREATE UNIQUE INDEX IF NOT EXISTS payments_uid
                ON payments (uid) WHERE uid IS NOT NULL;
            CREATE UNIQUE INDEX IF NOT EXISTS stock_movements_uid
                ON stock_movements (uid) WHERE uid IS NOT NULL;

            CREATE INDEX IF NOT EXISTS sales_change_seq
                ON sales (change_seq) WHERE change_seq IS NOT NULL;
            CREATE INDEX IF NOT EXISTS products_archived
                ON products (archived);

            DROP INDEX IF EXISTS products_barcode;
            CREATE UNIQUE INDEX IF NOT EXISTS products_barcode
                ON products (barcode) WHERE barcode IS NOT NULL AND archived = 0;

            CREATE TABLE IF NOT EXISTS sync_peers (
                id                   TEXT PRIMARY KEY,
                name                 TEXT NOT NULL DEFAULT '',
                kind                 TEXT NOT NULL CHECK (kind IN ('till', 'phone')),
                host                 TEXT,
                port                 INTEGER,
                token                TEXT NOT NULL,
                last_pulled_seq      INTEGER NOT NULL DEFAULT 0,
                last_catalog_version INTEGER NOT NULL DEFAULT 0,
                last_seen_at         TEXT,
                created_at           TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS sync_rejects (
                uid           TEXT PRIMARY KEY,
                origin_device TEXT,
                reason        TEXT NOT NULL,
                payload       TEXT NOT NULL,
                rejected_at   TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TRIGGER IF NOT EXISTS products_catalog_version_insert
            AFTER INSERT ON products
            BEGIN
                UPDATE meta SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
                WHERE key = 'catalog_version';
            END;

            CREATE TRIGGER IF NOT EXISTS products_catalog_version_update
            AFTER UPDATE ON products
            BEGIN
                UPDATE meta SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
                WHERE key = 'catalog_version';
            END;

            CREATE TRIGGER IF NOT EXISTS categories_catalog_version_insert
            AFTER INSERT ON categories
            BEGIN
                UPDATE meta SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
                WHERE key = 'catalog_version';
            END;

            CREATE TRIGGER IF NOT EXISTS categories_catalog_version_update
            AFTER UPDATE ON categories
            BEGIN
                UPDATE meta SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
                WHERE key = 'catalog_version';
            END;

            CREATE TRIGGER IF NOT EXISTS users_catalog_version_insert
            AFTER INSERT ON users
            BEGIN
                UPDATE meta SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
                WHERE key = 'catalog_version';
            END;

            CREATE TRIGGER IF NOT EXISTS users_catalog_version_update
            AFTER UPDATE ON users
            BEGIN
                UPDATE meta SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
                WHERE key = 'catalog_version';
            END;
        "#,
    },
    // A peer that refused a pull leaves a reason the owner can read. Kept out of
    // migration 10 because that migration had already shipped to tills in the
    // field; editing an applied migration is the one thing this table forbids.
    Migration {
        version: 11,
        name: "sync_peer_errors",
        sql: r#"
            ALTER TABLE sync_peers ADD COLUMN last_error TEXT;
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

/// This install's stable name, generated once and kept in `meta`.
///
/// It is what `sales.origin_device` records and what a peer uses to tell two
/// tills apart. Generating it lazily means an upgraded database gains an
/// identity on the first sale rather than needing a setup screen; the empty
/// value seeded by migration 10 is not an identity, so it is replaced.
pub fn ensure_device_id(store: &storage::Store) -> anyhow::Result<String> {
    if let Some(id) = meta(store, "device_id").filter(|id| !id.is_empty()) {
        return Ok(id);
    }

    let id = uuid::Uuid::now_v7().to_string();
    set_meta(store, "device_id", &id)?;
    Ok(id)
}

/// Move the local change feed forward one step and return the new cursor.
///
/// Called as the first statement of a transaction that is about to write a
/// replicable sale. Doing it first matters: SQLite upgrades a deferred
/// transaction to a write lock on the first write, and bumping the cursor
/// before reading the catalog avoids discovering a concurrent writer after this
/// transaction already has a snapshot.
pub fn bump_change_seq(transaction: &rusqlite::Transaction<'_>) -> anyhow::Result<i64> {
    let seq = transaction.query_row(
        "INSERT INTO meta (key, value) VALUES ('change_seq', '1')
         ON CONFLICT(key) DO UPDATE
             SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
         RETURNING CAST(value AS INTEGER)",
        [],
        |row| row.get(0),
    )?;
    Ok(seq)
}

/// Mark the catalog as changed for peers that compare versions before pulling.
///
/// Table writes bump it through triggers in migration 10. Values that live
/// outside a table — the tax rate and cash step in settings — call this when
/// they are saved, because the snapshot a phone receives carries them too.
pub fn bump_catalog_version(store: &storage::Store) -> anyhow::Result<()> {
    store.conn().execute(
        "UPDATE meta SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
         WHERE key = 'catalog_version'",
        [],
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

/// A peer this till pulls from or a phone this till remembers.
///
/// The token is the peer's own pairing code. It is stored as the canonical ten
/// characters so the client can pass it to `lan-http` without normalising on
/// every request.
#[derive(Clone, Debug)]
pub struct SyncPeer {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub token: String,
    pub last_pulled_seq: i64,
    pub last_catalog_version: i64,
    pub last_seen_at: Option<String>,
    pub last_error: Option<String>,
}

impl SyncPeer {
    /// What the peer list prints as the address, or why it cannot be dialled.
    pub fn address(&self) -> String {
        match (&self.host, self.port) {
            (Some(host), Some(port)) => format!("{host}:{port}"),
            _ => "No address".to_owned(),
        }
    }

    pub fn display_name(&self) -> &str {
        let name = self.name.trim();
        if name.is_empty() { &self.id } else { name }
    }
}

/// Every peer, oldest first so the list does not reshuffle as rows are touched.
pub fn sync_peers(store: &storage::Store) -> anyhow::Result<Vec<SyncPeer>> {
    let mut statement = store.conn().prepare(
        "SELECT id, name, kind, host, port, token,
                last_pulled_seq, last_catalog_version, last_seen_at, last_error
         FROM sync_peers
         ORDER BY created_at, name COLLATE NOCASE",
    )?;
    let rows = statement.query_map([], |row| {
        let port: Option<i64> = row.get(4)?;
        Ok(SyncPeer {
            id: row.get(0)?,
            name: row.get(1)?,
            kind: row.get(2)?,
            host: row.get(3)?,
            port: port.and_then(|port| u16::try_from(port).ok()),
            token: row.get(5)?,
            last_pulled_seq: row.get(6)?,
            last_catalog_version: row.get(7)?,
            last_seen_at: row.get(8)?,
            last_error: row.get(9)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Add a till to pull from.
///
/// The address and token are checked here rather than in the form so the same
/// rule serves the UI and any future import. A malformed token is rejected
/// instead of being stored and failing at the next sync with a client error.
pub fn add_sync_peer(
    store: &storage::Store,
    name: &str,
    host: &str,
    port: u16,
    token: &str,
) -> anyhow::Result<()> {
    let host = host.trim();
    anyhow::ensure!(!host.is_empty(), "Enter the other till's address.");
    anyhow::ensure!(port != 0, "Enter a port between 1 and 65535.");
    let token = lan_http::pairing::normalize(token)
        .ok_or_else(|| anyhow::anyhow!("That is not a pairing code."))?;

    store.conn().execute(
        "INSERT INTO sync_peers (id, name, kind, host, port, token)
         VALUES (?1, ?2, 'till', ?3, ?4, ?5)",
        rusqlite::params![
            uuid::Uuid::now_v7().to_string(),
            name.trim(),
            host,
            i64::from(port),
            token
        ],
    )?;
    Ok(())
}

pub fn remove_sync_peer(store: &storage::Store, id: &str) -> anyhow::Result<()> {
    store
        .conn()
        .execute("DELETE FROM sync_peers WHERE id = ?1", [id])?;
    Ok(())
}

/// Remember how far this till has read a peer, and that the peer answered.
pub fn save_sync_cursor(
    store: &storage::Store,
    id: &str,
    since_seq: i64,
    catalog_version: i64,
) -> anyhow::Result<()> {
    store.conn().execute(
        "UPDATE sync_peers
         SET last_pulled_seq = ?2, last_catalog_version = ?3,
             last_seen_at = datetime('now'), last_error = NULL
         WHERE id = ?1",
        rusqlite::params![id, since_seq, catalog_version],
    )?;
    Ok(())
}

/// Remember why a peer could not be reached, without moving its cursor.
pub fn set_sync_error(store: &storage::Store, id: &str, error: &str) -> anyhow::Result<()> {
    store.conn().execute(
        "UPDATE sync_peers SET last_error = ?2 WHERE id = ?1",
        rusqlite::params![id, error],
    )?;
    Ok(())
}

/// Store the install's currency, replacing a previous choice.
pub fn set_currency(store: &storage::Store, currency: money::Currency) -> anyhow::Result<()> {
    set_meta(store, "currency", currency.code())
}

/// One row of the change feed while its children are being gathered.
struct PulledSale {
    id: i64,
    change_seq: i64,
    sale: sync_proto::SaleDto,
}

/// A page of the local change feed, newest cursor included.
///
/// Only sales with a portable identity are eligible: rows written before this
/// migration have `uid = NULL` and must not be copied. The cursor is
/// `change_seq`, not `created_at`, because a phone may have been offline for a
/// week and its sale still needs to arrive here exactly once. The seller's clock
/// ordering is local and monotonic; the buyer does not care what the timestamps
/// say beyond what it stores on each row.
///
/// Child rows are fetched per sale. At a page size of one hundred this is a
/// handful of indexed lookups, and it keeps the function readable; it is not
/// the checkout path.
pub fn pull_sales(
    store: &storage::Store,
    since: i64,
    limit: usize,
) -> anyhow::Result<(Vec<sync_proto::SaleDto>, i64, bool)> {
    if limit == 0 {
        return Ok((Vec::new(), since, false));
    }

    let conn = store.conn();
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);

    let rows = {
        let mut statement = conn.prepare(
            "SELECT id, change_seq, uid, COALESCE(origin_device, ''),
                    is_offline, user_id, total_minor, rounding_minor, tax_minor,
                    COALESCE(strftime('%Y-%m-%dT%H:%M:%fZ', created_at), '')
             FROM sales
             WHERE uid IS NOT NULL AND change_seq IS NOT NULL AND change_seq > ?1
             ORDER BY change_seq ASC
             LIMIT ?2",
        )?;
        let rows = statement.query_map(rusqlite::params![since, limit], |row| {
            Ok(PulledSale {
                id: row.get(0)?,
                change_seq: row.get(1)?,
                sale: sync_proto::SaleDto {
                    uid: row.get(2)?,
                    origin_device: row.get(3)?,
                    is_offline: row.get(4)?,
                    occurred_at: row.get(9)?,
                    user_id: row.get(5)?,
                    total_minor: row.get(6)?,
                    rounding_minor: row.get(7)?,
                    tax_minor: row.get(8)?,
                    lines: Vec::new(),
                    payments: Vec::new(),
                    movements: Vec::new(),
                },
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    let has_more = i64::try_from(rows.len()).unwrap_or(i64::MAX) >= limit;
    let mut next_seq = since;
    let mut sales = Vec::with_capacity(rows.len());

    for mut pulled in rows {
        pulled.sale.lines = {
            let mut statement = conn.prepare(
                "SELECT uid, product_id, name, qty, unit_minor, total_minor
                 FROM sale_lines
                 WHERE sale_id = ?1 AND uid IS NOT NULL
                 ORDER BY id",
            )?;
            let rows = statement.query_map([pulled.id], |row| {
                Ok(sync_proto::LineDto {
                    uid: row.get(0)?,
                    product_id: row.get(1)?,
                    name: row.get(2)?,
                    qty: row.get(3)?,
                    unit_minor: row.get(4)?,
                    total_minor: row.get(5)?,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        pulled.sale.payments = {
            let mut statement = conn.prepare(
                "SELECT uid, method, amount_minor, tendered_minor, change_minor
                 FROM payments
                 WHERE sale_id = ?1 AND uid IS NOT NULL
                 ORDER BY id",
            )?;
            let rows = statement.query_map([pulled.id], |row| {
                Ok(sync_proto::PaymentDto {
                    uid: row.get(0)?,
                    method: row.get(1)?,
                    amount_minor: row.get(2)?,
                    tendered_minor: row.get(3)?,
                    change_minor: row.get(4)?,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        pulled.sale.movements = {
            let mut statement = conn.prepare(
                "SELECT uid, product_id, delta, kind, note
                 FROM stock_movements
                 WHERE sale_id = ?1 AND uid IS NOT NULL
                 ORDER BY id",
            )?;
            let rows = statement.query_map([pulled.id], |row| {
                Ok(sync_proto::MovementDto {
                    uid: row.get(0)?,
                    product_id: row.get(1)?,
                    delta: row.get(2)?,
                    kind: row.get(3)?,
                    note: row.get(4)?,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        next_seq = pulled.change_seq;
        sales.push(pulled.sale);
    }

    Ok((sales, next_seq, has_more))
}

/// The catalog as this till would publish it.
///
/// `tax_rate_percent` is passed in because it lives in `settings.json`, not in
/// the database; the snapshot is the one place where the two sources of truth
/// are joined for the phone. Products include archived rows on purpose: a peer
/// learns that a product is gone by seeing `archived = 1`, and the local ledger
/// keeps pointing at the same id.
///
/// Staff is a read-only directory. No credential or PIN is included, and a
/// desktop replica deliberately does not write these rows into `users`: a login
/// is local policy, and inventing a password hash so a remote person can exist
/// in the table would be a security bug dressed as a feature.
pub fn catalog_snapshot(
    store: &storage::Store,
    tax_rate_percent: i64,
) -> anyhow::Result<sync_proto::CatalogSnapshot> {
    anyhow::ensure!(tax_rate_percent >= 0, "the tax rate cannot be negative");

    let tender = currency(store);

    let products = {
        let mut statement = store.conn().prepare(
            "SELECT id, name, price_minor, barcode, category, category_id,
                    tracked, archived
             FROM products
             ORDER BY id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(sync_proto::ProductDto {
                id: row.get(0)?,
                name: row.get(1)?,
                price_minor: row.get(2)?,
                barcode: row.get(3)?,
                category: row.get(4)?,
                category_id: row.get(5)?,
                tracked: row.get(6)?,
                archived: row.get(7)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    let categories = categories_list(store)
        .into_iter()
        .map(|category| sync_proto::CategoryDto {
            id: category.id,
            name: category.name,
            sort_order: category.sort_order,
            color: category.color,
        })
        .collect();

    let staff = users(store)
        .into_iter()
        .filter(|user| user.active)
        .map(|user| sync_proto::StaffDto {
            id: user.id,
            name: user.name,
            role: user.role.key().to_owned(),
        })
        .collect();

    let version = meta(store, "catalog_version")
        .and_then(|value| value.parse().ok())
        .unwrap_or(1);

    Ok(sync_proto::CatalogSnapshot {
        protocol: sync_proto::PROTOCOL,
        version,
        currency: tender.map(|c| c.code().to_owned()).unwrap_or_default(),
        currency_exponent: tender
            .map(|c| i64::from(c.exponent().unwrap_or(0)))
            .unwrap_or(0),
        cash_step_minor: tender.map(crate::currencies::cash_step).unwrap_or(1),
        tax_rate_percent,
        products,
        categories,
        staff,
    })
}

/// Apply an owner's catalog to a replica.
///
/// The snapshot is an upsert by id, not a replace: `DELETE FROM products` would
/// cascade into `stock_movements` and null `sale_lines.product_id`, turning a
/// catalog refresh into a loss of sales history. Products the owner no longer
/// lists are archived, and products the owner marks archived are archived here;
/// nothing is deleted. Categories are only upserted for the same reason: a
/// category row may still be the free-text shelf of a local product.
///
/// The transaction covers the whole snapshot, so a peer sees either the old
/// catalog or the new one, never half a price change.
pub fn apply_catalog_snapshot(
    store: &storage::Store,
    snapshot: &sync_proto::CatalogSnapshot,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        snapshot.protocol == 0 || snapshot.protocol == sync_proto::PROTOCOL,
        "unsupported catalog protocol {}",
        snapshot.protocol
    );
    anyhow::ensure!(snapshot.version > 0, "a catalog needs a version");
    anyhow::ensure!(
        snapshot.cash_step_minor >= 1,
        "the cash step must be positive"
    );
    anyhow::ensure!(
        snapshot.tax_rate_percent >= 0,
        "the tax rate cannot be negative"
    );

    let mut category_ids = std::collections::HashSet::new();
    let mut category_names = std::collections::HashSet::new();
    for category in &snapshot.categories {
        anyhow::ensure!(category.id > 0, "a category needs a positive id");
        anyhow::ensure!(
            category_ids.insert(category.id),
            "the catalog lists category {} twice",
            category.id
        );
        let name = category.name.trim();
        anyhow::ensure!(!name.is_empty(), "a category needs a name");
        anyhow::ensure!(
            category_names.insert(name.to_lowercase()),
            "the catalog lists the category name {name} twice"
        );
    }

    let mut product_ids = std::collections::HashSet::new();
    let mut barcodes: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for product in &snapshot.products {
        anyhow::ensure!(product.id > 0, "a product needs a positive id");
        anyhow::ensure!(
            product_ids.insert(product.id),
            "the catalog lists product {} twice",
            product.id
        );
        anyhow::ensure!(!product.name.trim().is_empty(), "a product needs a name");
        anyhow::ensure!(
            product.price_minor >= 0,
            "a product price cannot be negative"
        );
        anyhow::ensure!(
            !product.category.trim().is_empty(),
            "a product needs a category"
        );
        anyhow::ensure!(
            product.tracked == 0 || product.tracked == 1,
            "tracked must be 0 or 1"
        );
        anyhow::ensure!(
            product.archived == 0 || product.archived == 1,
            "archived must be 0 or 1"
        );
        if let Some(category_id) = product.category_id {
            anyhow::ensure!(
                category_ids.contains(&category_id),
                "product {} names category {category_id}, which is not in the snapshot",
                product.id
            );
        }
        if product.archived == 0
            && let Some(barcode) = product
                .barcode
                .as_deref()
                .map(str::trim)
                .filter(|barcode| !barcode.is_empty())
        {
            anyhow::ensure!(
                !barcodes.contains_key(barcode),
                "the catalog puts barcode {barcode} on two active products"
            );
            barcodes.insert(barcode.to_owned(), product.id);
        }
    }

    let local_currency = currency(store);
    let incoming_currency = if snapshot.currency.trim().is_empty() {
        None
    } else {
        Some(
            money::tender_from_code(snapshot.currency.trim()).with_context(|| {
                format!(
                    "the catalog names unknown currency {}",
                    snapshot.currency.trim()
                )
            })?,
        )
    };

    if let (Some(local), Some(incoming)) = (local_currency, incoming_currency) {
        // Prices and totals cannot be renumbered by a catalog refresh. A
        // mismatched currency means this till was paired with the wrong shop,
        // or was already selling; either way it must not silently switch.
        anyhow::ensure!(
            local.code() == incoming.code(),
            "this till prices in {}, the catalog owner prices in {}",
            local.code(),
            incoming.code()
        );
    }

    let transaction = store.conn().unchecked_transaction()?;

    if local_currency.is_none()
        && let Some(incoming) = incoming_currency
    {
        transaction.execute(
            "INSERT INTO meta (key, value) VALUES ('currency', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [incoming.code()],
        )?;
    }

    for category in &snapshot.categories {
        let color = category
            .color
            .as_deref()
            .map(str::trim)
            .filter(|color| !color.is_empty());
        transaction.execute(
            "INSERT INTO categories (id, name, sort_order, color)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
                 name = excluded.name,
                 sort_order = excluded.sort_order,
                 color = excluded.color",
            rusqlite::params![
                category.id,
                category.name.trim(),
                category.sort_order,
                color,
            ],
        )?;
    }

    // Two products may swap barcodes in one snapshot. Clearing the barcodes of
    // every product in the snapshot first avoids a transient unique-index
    // conflict that has nothing to do with the final state.
    for product in &snapshot.products {
        transaction.execute(
            "UPDATE products SET barcode = NULL WHERE id = ?1",
            [product.id],
        )?;
    }

    for product in &snapshot.products {
        let barcode = product
            .barcode
            .as_deref()
            .map(str::trim)
            .filter(|barcode| !barcode.is_empty());
        transaction.execute(
            "INSERT INTO products
                 (id, name, price_minor, category, category_id, barcode,
                  tracked, archived)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
                 name = excluded.name,
                 price_minor = excluded.price_minor,
                 category = excluded.category,
                 category_id = excluded.category_id,
                 barcode = excluded.barcode,
                 tracked = excluded.tracked,
                 archived = excluded.archived",
            rusqlite::params![
                product.id,
                product.name.trim(),
                product.price_minor,
                product.category.trim(),
                product.category_id,
                barcode,
                product.tracked,
                product.archived,
            ],
        )?;
    }

    // A product the owner stopped listing is archived here. This is the only
    // path that changes a local row behind the owner's explicit list, and it
    // still never deletes it.
    let local_active = {
        let mut statement = transaction.prepare("SELECT id FROM products WHERE archived = 0")?;
        let rows = statement.query_map([], |row| row.get::<_, i64>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for id in local_active {
        if !product_ids.contains(&id) {
            transaction.execute("UPDATE products SET archived = 1 WHERE id = ?1", [id])?;
        }
    }

    transaction.commit()?;
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

/// Every live product, by name.
///
/// The list is small enough to keep in memory and filter there, which is why
/// the search box does not run a query per keystroke. A catalog big enough to
/// change that is a catalog that needs the query, and that is the moment to
/// move the filter into SQL.
///
/// Archived products are excluded here and still reachable by the sync snapshot,
/// which reads the table directly: the shop hid them, a peer still has to learn
/// that they are gone.
pub fn products(store: &storage::Store) -> Vec<Product> {
    let mut statement = match store.conn().prepare(
        "SELECT id, name, price_minor, category, category_id, barcode
         FROM products WHERE archived = 0 ORDER BY name COLLATE NOCASE",
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
            category_id: row.get(4)?,
            barcode: row.get(5)?,
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
    let mut statement = match store.conn().prepare(
        "SELECT DISTINCT category FROM products
         WHERE archived = 0 ORDER BY category COLLATE NOCASE",
    ) {
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

/// Every row in the `categories` table, by name.
///
/// Used by the product form's dropdown: these are the categories the shop has
/// actually created, not the seed list the app invented.
pub fn categories_list(store: &storage::Store) -> Vec<Category> {
    let mut statement = match store
        .conn()
        .prepare("SELECT id, name, sort_order, color FROM categories ORDER BY sort_order, name COLLATE NOCASE")
    {
        Ok(statement) => statement,
        Err(err) => {
            tracing::warn!(error = %err, "categories_list prepare failed");
            return Vec::new();
        }
    };

    let rows = statement.query_map([], |row| {
        Ok(Category {
            id: row.get(0)?,
            name: row.get(1)?,
            sort_order: row.get(2)?,
            color: row.get(3)?,
        })
    });

    match rows {
        Ok(rows) => rows.filter_map(Result::ok).collect(),
        Err(err) => {
            tracing::warn!(error = %err, "categories_list query failed");
            Vec::new()
        }
    }
}

/// Look up a category by name, inserting it when it does not exist.
///
/// Returns the category id. Case-insensitive lookup because "Vitamins" and
/// "vitamins" are the same shelf, and a second product typed by hand should not
/// create a duplicate.
pub fn find_or_insert_category(store: &storage::Store, name: &str) -> anyhow::Result<i64> {
    let name = name.trim();
    anyhow::ensure!(!name.is_empty(), "a category needs a name");

    // Try to find an existing one (case-insensitive).
    if let Ok(found) = store.conn().query_row(
        "SELECT id FROM categories WHERE name = ?1 COLLATE NOCASE",
        [name],
        |row| row.get::<_, i64>(0),
    ) {
        return Ok(found);
    }

    // Insert it.
    store.conn().execute(
        "INSERT INTO categories (name, sort_order) VALUES (?1, 0)",
        [name],
    )?;
    Ok(store.conn().last_insert_rowid())
}

/// Add a product and return its id.
///
/// `category_id` links to the `categories` table when the product was saved
/// with a known shelf; `None` when it was saved with free text that was never
/// added to the categories table.
pub fn insert_product(
    store: &storage::Store,
    name: &str,
    price_minor: i64,
    category: &str,
    category_id: Option<i64>,
    barcode: Option<&str>,
) -> anyhow::Result<i64> {
    store.conn().execute(
        "INSERT INTO products (name, price_minor, category, category_id, barcode)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        (name, price_minor, category, category_id, barcode),
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
    category_id: Option<i64>,
    barcode: Option<&str>,
) -> anyhow::Result<()> {
    let changed = store.conn().execute(
        "UPDATE products
         SET name = ?2, price_minor = ?3, category = ?4, category_id = ?5, barcode = ?6
         WHERE id = ?1",
        (id, name, price_minor, category, category_id, barcode),
    )?;
    anyhow::ensure!(changed == 1, "product {id} is no longer in the catalog");
    Ok(())
}

/// Archive a product.
///
/// The row stays: sale lines and stock movements point at it, and those are the
/// record of what happened. "Delete" is what the operator asked for, so the
/// screen says removed and the catalog list stops showing it; the database
/// keeps the identity a report or a peer needs.
pub fn delete_product(store: &storage::Store, id: i64) -> anyhow::Result<()> {
    store.conn().execute(
        "UPDATE products SET archived = 1 WHERE id = ?1 AND archived = 0",
        [id],
    )?;
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
            "SELECT EXISTS(
                 SELECT 1 FROM products
                 WHERE barcode = ?1 AND id IS NOT ?2 AND archived = 0
             )",
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

/// Every product whose stock is tracked, with the ledger's sum.
///
/// One query rather than a count kept on `products`: the ledger is the truth,
/// and a cached number is the thing that silently disagrees with it. The list is
/// small enough to read whole, like the catalog.
pub fn stock(store: &storage::Store) -> Vec<StockItem> {
    let mut statement = match store.conn().prepare(
        "SELECT p.id, p.name, p.category, p.low_stock, COALESCE(SUM(m.delta), 0)
         FROM products p
         LEFT JOIN stock_movements m ON m.product_id = p.id
         WHERE p.tracked = 1 AND p.archived = 0
         GROUP BY p.id
         ORDER BY p.name COLLATE NOCASE",
    ) {
        Ok(statement) => statement,
        Err(err) => {
            tracing::warn!(error = %err, "stock prepare failed");
            return Vec::new();
        }
    };

    let rows = statement.query_map([], |row| {
        Ok(StockItem {
            product_id: row.get(0)?,
            name: row.get(1)?,
            category: row.get(2)?,
            low_stock: row.get(3)?,
            on_hand: row.get(4)?,
        })
    });

    match rows {
        Ok(rows) => rows.filter_map(Result::ok).collect(),
        Err(err) => {
            tracing::warn!(error = %err, "stock query failed");
            Vec::new()
        }
    }
}

/// Start tracking a product, with the count on the shelf right now.
///
/// The opening count is one movement, not a quantity column: later sales and
/// deliveries append to the same ledger, and a wrong opening count is corrected
/// by another movement like any other mistake. A zero opening count writes no
/// movement — nothing moved — but still marks the product tracked, which is what
/// puts it on the screen and makes sales decrement it.
pub fn track_product(
    store: &storage::Store,
    product_id: i64,
    opening: i64,
    low_stock: i64,
    user_id: Option<&str>,
) -> anyhow::Result<()> {
    anyhow::ensure!(opening >= 0, "the opening count cannot be negative");
    anyhow::ensure!(low_stock >= 0, "the low-stock level cannot be negative");

    let transaction = store.conn().unchecked_transaction()?;

    let exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM products WHERE id = ?1 AND archived = 0)",
        [product_id],
        |row| row.get(0),
    )?;
    anyhow::ensure!(exists, "that product is no longer in the catalog");

    let already: bool = transaction.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM products WHERE id = ?1 AND tracked = 1 AND archived = 0
         )",
        [product_id],
        |row| row.get(0),
    )?;
    anyhow::ensure!(!already, "that product is already tracked");

    transaction.execute(
        "UPDATE products SET tracked = 1, low_stock = ?2
         WHERE id = ?1 AND archived = 0",
        (product_id, low_stock),
    )?;

    if opening != 0 {
        transaction.execute(
            "INSERT INTO stock_movements (product_id, delta, kind, user_id)
             VALUES (?1, ?2, 'opening', ?3)",
            (product_id, opening, user_id),
        )?;
    }

    transaction.commit()?;
    Ok(())
}

/// Append a manual movement to a tracked product's ledger.
///
/// Signs are fixed by the kind, not left to the caller: `Receive` and `Return`
/// add stock, `Waste` removes it, and only `Adjust` may go either way. A UI that
/// computes the wrong sign is a bug this function refuses rather than stores.
pub fn record_stock_movement(
    store: &storage::Store,
    product_id: i64,
    delta: i64,
    kind: StockKind,
    note: &str,
    user_id: Option<&str>,
) -> anyhow::Result<()> {
    anyhow::ensure!(delta != 0, "a stock movement must change the count");
    match kind {
        StockKind::Receive | StockKind::Return => {
            anyhow::ensure!(delta > 0, "{} must add stock", kind.label())
        }
        StockKind::Waste => anyhow::ensure!(delta < 0, "{} must remove stock", kind.label()),
        StockKind::Adjust => {}
        StockKind::Opening | StockKind::Sale => {
            anyhow::bail!("{} is written by its own flow", kind.label())
        }
    }

    let tracked: bool = store.conn().query_row(
        "SELECT EXISTS(
             SELECT 1 FROM products WHERE id = ?1 AND tracked = 1 AND archived = 0
         )",
        [product_id],
        |row| row.get(0),
    )?;
    anyhow::ensure!(tracked, "that product's stock is not tracked");

    store.conn().execute(
        "INSERT INTO stock_movements (product_id, delta, kind, note, user_id)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        (product_id, delta, kind.key(), optional_text(note), user_id),
    )?;
    Ok(())
}

/// A product's movements, newest first.
///
/// Capped because the history is a question about recent stock, not an archive:
/// a modal that has to paint ten thousand deliveries is slower than the answer
/// is useful. The cap is stated on screen when it cuts the list.
pub const MOVEMENT_HISTORY: i64 = 200;

pub fn stock_movements(store: &storage::Store, product_id: i64) -> Vec<StockMovement> {
    let mut statement = match store.conn().prepare(
        "SELECT m.id, m.delta, m.kind, m.note,
                strftime('%d/%m %H:%M', m.created_at, 'localtime'), u.name
         FROM stock_movements m
         LEFT JOIN users u ON u.id = m.user_id
         WHERE m.product_id = ?1
         ORDER BY m.id DESC
         LIMIT ?2",
    ) {
        Ok(statement) => statement,
        Err(err) => {
            tracing::warn!(error = %err, "stock history prepare failed");
            return Vec::new();
        }
    };

    let rows = statement.query_map((product_id, MOVEMENT_HISTORY), |row| {
        let kind: String = row.get(2)?;
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            kind,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, Option<String>>(5)?,
        ))
    });

    let Ok(rows) = rows else {
        tracing::warn!("stock history query failed");
        return Vec::new();
    };

    rows.filter_map(Result::ok)
        .filter_map(|(id, delta, kind, note, created_at, user)| {
            Some(StockMovement {
                id,
                delta,
                kind: StockKind::from_key(&kind)?,
                note,
                created_at,
                user,
            })
        })
        .collect()
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
///
/// Each line for a tracked product also writes a negative `sale` movement to
/// the stock ledger, inside the same transaction.
pub fn record_sale(
    store: &storage::Store,
    lines: &[SaleLine],
    rounding_minor: i64,
    tax_minor: i64,
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

    anyhow::ensure!(tax_minor <= total_minor, "tax cannot exceed the sale total");

    let paid_minor = total_minor
        .checked_add(rounding_minor)
        .context("the amount due does not fit in 64 bits")?;
    anyhow::ensure!(paid_minor >= 0, "the amount due cannot be negative");
    anyhow::ensure!(
        tendered_minor >= paid_minor,
        "cash received is less than the amount due"
    );
    let change_minor = tendered_minor - paid_minor;

    // Read the install identity before opening the transaction: on an upgraded
    // database this may write `meta`, and the transaction's first write must be
    // the cursor itself.
    let origin_device = ensure_device_id(store)?;

    // `unchecked_transaction` because `Store` hands out a shared connection: the
    // transaction cannot borrow it mutably, and there is one till per process.
    let transaction = store.conn().unchecked_transaction()?;

    // The cursor moves before anything else is written: it is the first write
    // of the transaction, and the sale below records the value it returned.
    let change_seq = bump_change_seq(&transaction)?;

    let sale_uid = uuid::Uuid::now_v7().to_string();

    transaction.execute(
        "INSERT INTO sales
             (total_minor, rounding_minor, tax_minor, user_id, uid, origin_device, change_seq)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        (
            total_minor,
            rounding_minor,
            tax_minor,
            user_id,
            sale_uid.as_str(),
            origin_device.as_str(),
            change_seq,
        ),
    )?;
    let id = transaction.last_insert_rowid();

    for line in lines {
        let line_uid = uuid::Uuid::now_v7().to_string();
        transaction.execute(
            "INSERT INTO sale_lines
                 (sale_id, product_id, name, qty, unit_minor, total_minor, uid)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                id,
                line.product_id,
                line.name.as_str(),
                line.qty,
                line.unit_minor,
                line.qty * line.unit_minor,
                line_uid.as_str(),
            ),
        )?;

        // A tracked product's shelf count follows the sale in the same
        // transaction. A sale that committed while the ledger did not is a
        // discrepancy nobody can explain later, and the ledger is append-only,
        // so the only clean moment to write the decrement is here.
        //
        // Untracked products are skipped: the catalog does not claim to know
        // their stock, and writing a movement would make the Inventory screen
        // claim it does. Negative results are allowed; the till must keep
        // selling when the shelf and the database disagree.
        if let Some(product_id) = line.product_id {
            let tracked: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM products WHERE id = ?1 AND tracked = 1)",
                [product_id],
                |row| row.get(0),
            )?;
            if tracked {
                let movement_uid = uuid::Uuid::now_v7().to_string();
                transaction.execute(
                    "INSERT INTO stock_movements
                         (product_id, delta, kind, sale_id, user_id, uid)
                     VALUES (?1, ?2, 'sale', ?3, ?4, ?5)",
                    (product_id, -line.qty, id, user_id, movement_uid.as_str()),
                )?;
            }
        }
    }

    let payment_uid = uuid::Uuid::now_v7().to_string();
    transaction.execute(
        "INSERT INTO payments
             (sale_id, method, amount_minor, tendered_minor, change_minor, uid)
         VALUES (?1, 'cash', ?2, ?3, ?4, ?5)",
        (
            id,
            paid_minor,
            tendered_minor,
            change_minor,
            payment_uid.as_str(),
        ),
    )?;

    transaction.commit()?;

    Ok(RecordedSale {
        id,
        uid: sale_uid,
        total_minor,
        rounding_minor,
        tax_minor,
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

/// The window immediately before a range, for the change on the KPI cards.
///
/// Whole windows, not "the same distance back from this minute": comparing
/// today with yesterday means yesterday, and a boundary that only moves at
/// midnight is one a shopkeeper can predict. All time has nothing before it and
/// returns `None`, so the cards can say so instead of inventing a zero.
fn previous_clause(range: usize, column: &str) -> Option<String> {
    match range {
        0 => Some(format!(
            "date({column}, 'localtime') = date('now', 'localtime', '-1 day')"
        )),
        // Seven days ending the day before the current week opens.
        1 => Some(format!(
            "{column} >= datetime(date('now', 'localtime', '-13 days'), 'utc')
             AND {column} < datetime(date('now', 'localtime', '-6 days'), 'utc')"
        )),
        // Thirty days ending the day before the current month opens.
        2 => Some(format!(
            "{column} >= datetime(date('now', 'localtime', '-59 days'), 'utc')
             AND {column} < datetime(date('now', 'localtime', '-29 days'), 'utc')"
        )),
        _ => None,
    }
}

/// The range's `WHERE` fragment, narrowed to one cashier when `user` is set.
///
/// Every report shares this: the dashboard and the transaction list must never
/// disagree about where the window starts, or about whose sales it contains.
/// The cashier is bound as `?1` on every query, `NULL` meaning the whole shop.
/// Binding the value even when it is `NULL` keeps one placeholder shape
/// throughout; `?1 IS NULL` is a constant true that SQLite folds away.
fn window_clause(range: usize, column: &str) -> String {
    match range_clause(range, column) {
        Some(clause) => format!("({clause}) AND (?1 IS NULL OR s.user_id = ?1)"),
        None => "(?1 IS NULL OR s.user_id = ?1)".to_owned(),
    }
}

/// [`window_clause`] for the period before the range, or `None` for all time.
fn previous_window_clause(range: usize, column: &str) -> Option<String> {
    previous_clause(range, column)
        .map(|clause| format!("({clause}) AND (?1 IS NULL OR s.user_id = ?1)"))
}

/// How many days back the zero-filled calendar opens, as a SQLite modifier.
///
/// Ranges with a fixed length get a calendar so a day that sold nothing is a
/// zero bar, not a missing one. All time has no fixed calendar — it is every
/// day that sold, capped for display — and returns `None`.
fn calendar_start(range: usize) -> Option<&'static str> {
    match range {
        0 => Some("0"),
        1 => Some("-6"),
        2 => Some("-29"),
        _ => None,
    }
}

/// Revenue and sale count for a range.
///
/// A failed query yields zeros and a logged warning rather than an error the
/// shell has to render — the dashboard is a summary, not the source of truth.
pub fn totals(store: &storage::Store, range: usize, user: Option<&str>) -> (i64, i64) {
    totals_in(store, &window_clause(range, "s.created_at"), user)
}

/// [`totals`] for the window before the range: what the cards compare against.
pub fn previous_totals(
    store: &storage::Store,
    range: usize,
    user: Option<&str>,
) -> Option<(i64, i64)> {
    let clause = previous_window_clause(range, "s.created_at")?;
    Some(totals_in(store, &clause, user))
}

fn totals_in(store: &storage::Store, clause: &str, user: Option<&str>) -> (i64, i64) {
    let sql = format!(
        "SELECT COUNT(*), COALESCE(SUM(s.total_minor + s.rounding_minor), 0)
         FROM sales s WHERE {clause}"
    );

    let result = store.conn().query_row(&sql, [user], |row| {
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

/// Units sold in a range: the `qty` on every line, not the number of sales.
pub fn items_sold(store: &storage::Store, range: usize, user: Option<&str>) -> i64 {
    items_sold_in(store, &window_clause(range, "s.created_at"), user)
}

/// [`items_sold`] for the window before the range.
pub fn previous_items_sold(
    store: &storage::Store,
    range: usize,
    user: Option<&str>,
) -> Option<i64> {
    let clause = previous_window_clause(range, "s.created_at")?;
    Some(items_sold_in(store, &clause, user))
}

fn items_sold_in(store: &storage::Store, clause: &str, user: Option<&str>) -> i64 {
    let sql = format!(
        "SELECT COALESCE(SUM(l.qty), 0)
         FROM sale_lines l JOIN sales s ON s.id = l.sale_id
         WHERE {clause}"
    );

    match store
        .conn()
        .query_row(&sql, [user], |row| row.get::<_, i64>(0))
    {
        Ok(items) => items,
        Err(err) => {
            tracing::warn!(error = %err, "items sold query failed");
            0
        }
    }
}

/// The hour that rang the most sales, and how many.
///
/// Ranked by sale count, because "busiest" on a till means the queue, not the
/// takings; revenue breaks a tie so the answer is stable when two hours drew
/// the same number of customers.
pub struct PeakHour {
    /// `0`–`23`, terminal clock.
    pub hour: u8,
    pub sales: i64,
}

pub fn peak_hour(store: &storage::Store, range: usize, user: Option<&str>) -> Option<PeakHour> {
    let sql = format!(
        "SELECT CAST(strftime('%H', s.created_at, 'localtime') AS INTEGER) AS hour,
                COUNT(*) AS sales,
                COALESCE(SUM(s.total_minor + s.rounding_minor), 0) AS revenue
         FROM sales s
         WHERE {}
         GROUP BY hour
         ORDER BY sales DESC, revenue DESC
         LIMIT 1",
        window_clause(range, "s.created_at")
    );

    let mut statement = match store.conn().prepare(&sql) {
        Ok(statement) => statement,
        Err(err) => {
            tracing::warn!(error = %err, "peak hour prepare failed");
            return None;
        }
    };

    match statement.query_map([user], |row| {
        Ok(PeakHour {
            hour: row.get(0)?,
            sales: row.get(1)?,
        })
    }) {
        // An empty range has no busiest hour; that is an answer, not an error.
        Ok(mut rows) => rows.next().and_then(Result::ok),
        Err(err) => {
            tracing::warn!(error = %err, "peak hour query failed");
            None
        }
    }
}

/// Most recent sales in a range, newest first.
///
/// `user` narrows the list to one cashier; `None` is the shop. The same window
/// and the same scope as the dashboard totals, so the list cannot show a sale
/// the cards above it did not count.
pub fn sales(store: &storage::Store, range: usize, limit: i64, user: Option<&str>) -> Vec<Sale> {
    let sql = format!(
        "SELECT s.id, s.total_minor + s.rounding_minor, strftime('%d/%m %H:%M', s.created_at, 'localtime'), u.name
         FROM sales s LEFT JOIN users u ON u.id = s.user_id
         WHERE {}
         ORDER BY s.id DESC LIMIT ?2",
        window_clause(range, "s.created_at")
    );

    let mut statement = match store.conn().prepare(&sql) {
        Ok(statement) => statement,
        Err(err) => {
            tracing::warn!(error = %err, "sales prepare failed");
            return Vec::new();
        }
    };

    let rows = statement.query_map((user, limit), |row| {
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
///
/// `user` narrows the file to one cashier, so an export cannot become a way
/// around the scope the screen enforces. `None` is the shop.
pub fn export_sales(
    store: &storage::Store,
    range: usize,
    user: Option<&str>,
) -> anyhow::Result<Vec<ExportSale>> {
    let sql = format!(
        "SELECT s.id, strftime('%Y-%m-%d %H:%M:%S', s.created_at, 'localtime'), u.name,
                s.total_minor, s.rounding_minor, s.total_minor + s.rounding_minor,
                COALESCE(GROUP_CONCAT(DISTINCT p.method), '')
         FROM sales s
         LEFT JOIN users u ON u.id = s.user_id
         LEFT JOIN payments p ON p.sale_id = s.id
         WHERE {}
         GROUP BY s.id ORDER BY s.id",
        window_clause(range, "s.created_at")
    );

    let mut statement = store
        .conn()
        .prepare(&sql)
        .context("preparing the sales export")?;
    let rows = statement
        .query_map([user], |row| {
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
    /// The pre-rounding amount, tax-inclusive.
    pub total_minor: i64,
    /// Cash rounding on top of `total` — `0` most of the time, and what the
    /// customer paid above the exact total otherwise.
    pub rounding_minor: i64,
    /// `total + rounding`: what actually entered the drawer for this sale.
    pub paid_minor: i64,
    /// The tax the sale collected, included in `total_minor`. `0` when the
    /// till charges no tax.
    pub tax_minor: i64,
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
        "SELECT s.total_minor, s.rounding_minor, s.tax_minor, \
         strftime('%d/%m/%Y %H:%M', s.created_at, 'localtime'), u.name
         FROM sales s LEFT JOIN users u ON u.id = s.user_id
         WHERE s.id = ?1",
        [id],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
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
        cashier: head.4,
        created_at: head.3,
        total_minor: head.0,
        rounding_minor: head.1,
        tax_minor: head.2,
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

pub fn by_person(store: &storage::Store, range: usize, user: Option<&str>) -> Vec<PersonTotal> {
    let sql = format!(
        "SELECT COALESCE(u.name, '-'), COUNT(*), SUM(s.total_minor + s.rounding_minor)
         FROM sales s LEFT JOIN users u ON u.id = s.user_id
         WHERE {}
         GROUP BY s.user_id
         ORDER BY 3 DESC",
        window_clause(range, "s.created_at")
    );

    let mut statement = match store.conn().prepare(&sql) {
        Ok(statement) => statement,
        Err(err) => {
            tracing::warn!(error = %err, "by person prepare failed");
            return Vec::new();
        }
    };

    let rows = statement.query_map([user], |row| {
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
/// already agree on. Days are newest first. Fixed ranges carry every day in the
/// window, zero days included, so the bars are a calendar; all time carries the
/// thirty most recent days that sold, because it has no calendar to draw.
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

pub fn by_day(store: &storage::Store, range: usize, user: Option<&str>) -> Vec<DayTotal> {
    const MAX_DAYS: i64 = 30;

    let conn = store.conn();

    // Ranges with a fixed calendar get every day, including the ones that sold
    // nothing. A missing bar is not a zero day, and a chart that closes the gap
    // lies about the week.
    if let Some(start) = calendar_start(range) {
        let sql = "WITH RECURSIVE days(d) AS (
                       SELECT date('now', 'localtime', ?2 || ' days')
                       UNION ALL
                       SELECT date(d, '+1 day') FROM days
                       WHERE d < date('now', 'localtime')
                   )
                   SELECT strftime('%d/%m', days.d),
                          CAST(strftime('%w', days.d) AS INTEGER),
                          COUNT(s.id),
                          COALESCE(SUM(s.total_minor + s.rounding_minor), 0)
                   FROM days
                   LEFT JOIN sales s
                          ON date(s.created_at, 'localtime') = days.d
                         AND (?1 IS NULL OR s.user_id = ?1)
                   GROUP BY days.d
                   ORDER BY days.d DESC";

        let mut statement = match conn.prepare(sql) {
            Ok(statement) => statement,
            Err(err) => {
                tracing::warn!(error = %err, "by day prepare failed");
                return Vec::new();
            }
        };

        let rows = statement.query_map((user, start), |row| {
            Ok(DayTotal {
                day: row.get(0)?,
                weekday: row.get(1)?,
                sales: row.get(2)?,
                revenue: row.get(3)?,
            })
        });

        return match rows {
            Ok(rows) => rows.filter_map(Result::ok).collect(),
            Err(err) => {
                tracing::warn!(error = %err, "by day query failed");
                Vec::new()
            }
        };
    }

    // All time has no calendar: it is every day that sold, capped to the recent
    // ones so the chart cannot outgrow the dashboard.
    let sql = format!(
        "SELECT strftime('%d/%m', s.created_at, 'localtime'),
                CAST(strftime('%w', s.created_at, 'localtime') AS INTEGER),
                COUNT(*), COALESCE(SUM(s.total_minor + s.rounding_minor), 0)
         FROM sales s
         WHERE {}
         GROUP BY date(s.created_at, 'localtime')
         ORDER BY date(s.created_at, 'localtime') DESC
         LIMIT ?2",
        window_clause(range, "s.created_at")
    );

    let mut statement = match conn.prepare(&sql) {
        Ok(statement) => statement,
        Err(err) => {
            tracing::warn!(error = %err, "by day prepare failed");
            return Vec::new();
        }
    };

    let rows = statement.query_map((user, MAX_DAYS), |row| {
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

pub fn top_items(
    store: &storage::Store,
    range: usize,
    limit: i64,
    user: Option<&str>,
) -> Vec<ItemTotal> {
    let sql = format!(
        "SELECT l.name, SUM(l.qty), SUM(l.total_minor)
         FROM sale_lines l JOIN sales s ON s.id = l.sale_id
         WHERE {}
         GROUP BY l.name
         ORDER BY 3 DESC
         LIMIT ?2",
        window_clause(range, "s.created_at")
    );

    let mut statement = match store.conn().prepare(&sql) {
        Ok(statement) => statement,
        Err(err) => {
            tracing::warn!(error = %err, "top items prepare failed");
            return Vec::new();
        }
    };

    let rows = statement.query_map((user, limit), |row| {
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
pub fn by_method(store: &storage::Store, range: usize, user: Option<&str>) -> Vec<MethodTotal> {
    let sql = format!(
        "SELECT p.method, COUNT(DISTINCT p.sale_id), COALESCE(SUM(p.amount_minor), 0)
         FROM payments p JOIN sales s ON s.id = p.sale_id
         WHERE {}
         GROUP BY p.method ORDER BY 3 DESC",
        window_clause(range, "s.created_at")
    );

    let mut statement = match store.conn().prepare(&sql) {
        Ok(statement) => statement,
        Err(err) => {
            tracing::warn!(error = %err, "payments by method prepare failed");
            return Vec::new();
        }
    };

    let rows = statement.query_map([user], |row| {
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

        assert_eq!(totals(&store, 0, None), (1, 500), "Today");
        assert_eq!(totals(&store, 1, None), (2, 1200), "7 days");
        assert_eq!(totals(&store, 2, None), (3, 2300), "30 days");
        assert_eq!(totals(&store, 3, None), (4, 4300), "All time");

        assert_eq!(sales(&store, 1, 10, None).len(), 2);

        let two = sales(&store, 3, 2, None);
        assert_eq!(two.len(), 2, "the limit applies after the filter");
        assert!(two[0].id > two[1].id, "newest first");

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn the_previous_window_is_whole_and_sits_before_the_range() {
        let (store, path) = store("previous-window");
        insert_sale(&store, "-0 days", &[("Tea", 1, 500)]);
        insert_sale(&store, "-3 days", &[("Tea", 1, 700)]);
        insert_sale(&store, "-10 days", &[("Tea", 2, 550)]);
        insert_sale(&store, "-40 days", &[("Tea", 4, 500)]);

        assert_eq!(
            previous_totals(&store, 0, None),
            Some((0, 0)),
            "yesterday was quiet, not missing"
        );
        assert_eq!(
            previous_totals(&store, 1, None),
            Some((1, 1_100)),
            "the whole week before this one"
        );
        assert_eq!(
            previous_totals(&store, 2, None),
            Some((1, 2_000)),
            "the whole month before this one"
        );
        assert_eq!(
            previous_totals(&store, 3, None),
            None,
            "nothing came before all time"
        );

        assert_eq!(previous_items_sold(&store, 1, None), Some(2));
        assert_eq!(previous_items_sold(&store, 2, None), Some(4));
        assert_eq!(previous_items_sold(&store, 3, None), None);

        let _ = std::fs::remove_file(path);
    }

    /// A sale that knows whose it was — the scope tests need a cashier to
    /// narrow to, and `insert_sale` deliberately leaves that column empty.
    fn insert_sale_for(
        store: &storage::Store,
        offset: &str,
        user: &str,
        lines: &[(&str, i64, i64)],
    ) -> i64 {
        let conn = store.conn();
        let total: i64 = lines.iter().map(|(_, qty, unit)| qty * unit).sum();
        conn.execute(
            "INSERT INTO sales (total_minor, created_at, user_id)
             VALUES (?1, datetime('now', ?2), ?3)",
            (total, offset, user),
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
    fn a_scope_narrows_every_report_to_one_person() {
        let (store, path) = store("report-scope");
        let alice = insert_user(
            &store,
            "Alice",
            Role::Cashier,
            CredentialKind::Pin,
            "$argon2id$placeholder",
        )
        .expect("insert alice");
        let bob = insert_user(
            &store,
            "Bob",
            Role::Cashier,
            CredentialKind::Pin,
            "$argon2id$placeholder",
        )
        .expect("insert bob");

        let alice_today = insert_sale_for(&store, "-0 days", &alice.id, &[("Tea", 2, 100)]);
        let bob_today = insert_sale_for(&store, "-0 days", &bob.id, &[("Rice", 1, 500)]);
        insert_sale_for(&store, "-3 days", &alice.id, &[("Tea", 3, 100)]);

        // One payment each, so the method card has figures to scope.
        for (sale, amount) in [(alice_today, 200), (bob_today, 500)] {
            store
                .conn()
                .execute(
                    "INSERT INTO payments (sale_id, method, amount_minor, tendered_minor, change_minor)
                     VALUES (?1, 'cash', ?2, ?2, 0)",
                    (sale, amount),
                )
                .expect("insert payment");
        }

        assert_eq!(totals(&store, 3, None), (3, 1_000));
        assert_eq!(totals(&store, 3, Some(&alice.id)), (2, 500));
        assert_eq!(totals(&store, 3, Some(&bob.id)), (1, 500));

        assert_eq!(items_sold(&store, 3, None), 6);
        assert_eq!(items_sold(&store, 3, Some(&alice.id)), 5);

        assert_eq!(sales(&store, 3, 10, Some(&alice.id)).len(), 2);
        assert!(
            sales(&store, 3, 10, Some(&alice.id))
                .iter()
                .all(|sale| sale.cashier.as_deref() == Some("Alice"))
        );

        let people = by_person(&store, 3, None);
        assert_eq!(people.len(), 2, "the shop view names both cashiers");
        assert_eq!(by_person(&store, 3, Some(&alice.id)).len(), 1);

        let items = top_items(&store, 3, 10, Some(&alice.id));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].qty, 5);

        let methods = by_method(&store, 3, Some(&alice.id));
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].amount_minor, 200, "only Alice's payment");
        assert_eq!(
            by_method(&store, 3, None)
                .iter()
                .map(|method| method.amount_minor)
                .sum::<i64>(),
            700
        );

        let rows = export_sales(&store, 3, Some(&alice.id)).expect("export");
        assert_eq!(rows.len(), 2, "an export cannot outrun the scope");

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn the_busiest_hour_is_the_one_with_the_most_sales() {
        let (store, path) = store("peak-hour");
        insert_sale(&store, "-0 days", &[("Tea", 1, 100)]);
        insert_sale(&store, "-0 days", &[("Tea", 1, 100)]);
        // A third sale, well away from this hour so it cannot tie the peak.
        store
            .conn()
            .execute(
                "INSERT INTO sales (total_minor, created_at)
                 VALUES (100, datetime('now', '-3 days', '-5 hours'))",
                [],
            )
            .expect("insert sale");

        let peak = peak_hour(&store, 1, None).expect("two sales share an hour");
        assert_eq!(peak.sales, 2);
        assert!(peak.hour <= 23);

        // Narrowed to a window with no sales, there is no busiest hour.
        assert!(peak_hour(&store, 0, Some("nobody")).is_none());

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

        assert_eq!(totals(&store, 0, None), (0, 0));
        assert_eq!(previous_totals(&store, 0, None), Some((0, 0)));
        assert_eq!(items_sold(&store, 0, None), 0);
        assert!(sales(&store, 3, 5, None).is_empty());
        assert!(by_day(&store, 3, None).is_empty());
        assert_eq!(
            by_day(&store, 1, None).len(),
            7,
            "a quiet week is still seven bars"
        );
        assert_eq!(by_day(&store, 1, None)[0].revenue, 0);
        assert!(peak_hour(&store, 0, None).is_none());
        assert!(top_items(&store, 3, 5, None).is_empty());
        assert!(by_person(&store, 3, None).is_empty());
        assert!(by_method(&store, 3, None).is_empty());

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

        let week = by_day(&store, 1, None);
        assert_eq!(week.len(), 7, "a week is seven bars, zero days included");
        assert_eq!(
            week.iter().filter(|day| day.sales > 0).count(),
            2,
            "today and six days back"
        );
        assert_eq!(totals(&store, 1, None), (2, 200));
        let month = by_day(&store, 2, None);
        assert_eq!(month.len(), 30, "a month is its calendar, not its sales");
        assert_eq!(
            month.iter().filter(|day| day.sales > 0).count(),
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

        let today = by_day(&store, 0, None);
        assert_eq!(
            today.len(),
            1,
            "today is one bar, however many sales it had"
        );
        assert_eq!(today[0].sales, 1, "two sales on one day share its row");
        let month = by_day(&store, 2, None);
        assert_eq!(month.len(), 30, "a month is its calendar");
        assert_eq!(
            month.iter().filter(|day| day.sales > 0).count(),
            3,
            "the forty-day-old sale is outside a 30-day range"
        );

        let days = by_day(&store, 3, None);
        assert_eq!(days.len(), 4);
        assert_eq!(
            days[0].revenue, 3_500,
            "newest first, like every other list"
        );
        assert_eq!(days[0].sales, 1, "two sales on one day share its row");
        assert_eq!(days[1].revenue, 1_000);
        assert_eq!(days[3].revenue, 500);

        let items = top_items(&store, 3, 5, None);
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
            top_items(&store, 0, 5, None)[0].name,
            "Sugar",
            "the ranking follows the chosen range, not all history"
        );
        assert_eq!(
            top_items(&store, 3, 2, None).len(),
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
            insert_product(&store, "Flat white", 3_400, "Coffee", None, None).expect("insert");
        insert_product(&store, "Croissant", 3_200, "Bakery", None, Some("4001")).expect("insert");
        insert_product(&store, "Espresso", 2_800, "Coffee", None, None).expect("insert");

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
            None,
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
        let espresso = insert_product(&store, "Espresso", 2_800, "Coffee", None, Some("4001"))
            .expect("insert");

        assert!(barcode_taken(&store, "4001", None));
        assert!(
            !barcode_taken(&store, "4001", Some(espresso)),
            "a product keeps its own barcode when it is edited"
        );
        assert!(!barcode_taken(&store, "4002", None));

        assert!(
            insert_product(&store, "Croissant", 3_200, "Bakery", None, Some("4001")).is_err(),
            "the unique index is the real guard, not the pre-check"
        );

        // Products without a barcode are not competing for one.
        insert_product(&store, "Filter", 2_600, "Coffee", None, None).expect("insert");
        insert_product(&store, "Lemonade", 3_000, "Cold", None, None).expect("insert");

        // Archiving frees the barcode: the old product is no longer sellable,
        // and refusing a live one because of a hidden one is a dead end.
        delete_product(&store, espresso).expect("archive");
        assert!(!barcode_taken(&store, "4001", None));
        insert_product(&store, "Espresso", 2_800, "Coffee", None, Some("4001"))
            .expect("the archived product's barcode is reusable");

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn the_database_refuses_a_negative_price() {
        let (store, path) = store("negative-price");

        assert!(
            insert_product(&store, "Refund", -100, "Coffee", None, None).is_err(),
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
        let espresso = insert_product(&store, "Espresso", 2_800, "Coffee", None, None)
            .expect("insert product");

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
        let sale = record_sale(&store, &lines, -20, 0, 4_000, None).expect("record");

        assert_eq!(sale.total_minor, 3_800, "the exact total is kept exact");
        assert_eq!(sale.paid_minor(), 3_780);
        assert_eq!(sale.change_minor, 220, "4000 tendered against 3780 due");

        let (count, revenue) = totals(&store, 3, None);
        assert_eq!(count, 1);
        assert_eq!(revenue, 3_780, "revenue is what the drawer took");

        let history = sales(&store, 3, 10, None);
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
    fn tracking_writes_an_opening_movement_and_manual_kinds_keep_their_signs() {
        let (store, path) = store("stock-track");
        let soap = insert_product(&store, "Soap", 1_200, "Household", None, None).expect("product");

        track_product(&store, soap, 12, 3, None).expect("track");
        let tracked = stock(&store);
        assert_eq!(tracked.len(), 1);
        assert_eq!(tracked[0].on_hand, 12);
        assert_eq!(tracked[0].low_stock, 3);
        assert!(!tracked[0].is_out());
        assert!(!tracked[0].is_low());

        record_stock_movement(&store, soap, 5, StockKind::Receive, "Delivery", None)
            .expect("receive");
        record_stock_movement(&store, soap, 2, StockKind::Return, "", None).expect("return");
        record_stock_movement(&store, soap, -4, StockKind::Waste, "Broken", None).expect("waste");
        record_stock_movement(&store, soap, -2, StockKind::Adjust, "Counted", None)
            .expect("adjust");

        assert_eq!(stock(&store)[0].on_hand, 13);
        assert!(!stock(&store)[0].is_out());
        assert!(!stock(&store)[0].is_low());

        let history = stock_movements(&store, soap);
        assert_eq!(history.len(), 5, "opening plus four moves");
        assert_eq!(history[0].delta, -2, "newest first");
        assert_eq!(history[0].kind, StockKind::Adjust);
        assert_eq!(history[0].note.as_deref(), Some("Counted"));
        assert_eq!(history[4].kind, StockKind::Opening);

        // Archiving hides the product but keeps its ledger: the movements are
        // the history of stock that was really on the shelf, and removing them
        // would erase the only record of where the count went.
        delete_product(&store, soap).expect("archive");
        assert!(stock(&store).is_empty());
        assert_eq!(stock_movements(&store, soap).len(), 5);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_sale_subtracts_only_the_tracked_lines() {
        let (store, path) = store("stock-sale");
        let soap = insert_product(&store, "Soap", 1_000, "Household", None, None).expect("soap");
        let bun = insert_product(&store, "Bun", 500, "Bakery", None, None).expect("bun");
        track_product(&store, soap, 10, 5, None).expect("track soap");

        let cashier = insert_user(
            &store,
            "Amina",
            Role::Owner,
            CredentialKind::Password,
            "$argon2id$stock",
        )
        .expect("cashier");

        let sale = record_sale(
            &store,
            &[
                SaleLine {
                    product_id: Some(soap),
                    name: "Soap".to_owned(),
                    qty: 3,
                    unit_minor: 1_000,
                },
                SaleLine {
                    product_id: Some(bun),
                    name: "Bun".to_owned(),
                    qty: 2,
                    unit_minor: 500,
                },
            ],
            0,
            0,
            4_000,
            Some(&cashier.id),
        )
        .expect("sale");

        let tracked = stock(&store);
        assert_eq!(tracked.len(), 1, "untracked products stay out of inventory");
        assert_eq!(tracked[0].on_hand, 7);

        let movements = stock_movements(&store, soap);
        assert_eq!(movements[0].kind, StockKind::Sale);
        assert_eq!(movements[0].delta, -3);
        assert_eq!(
            movements[0].user.as_deref(),
            Some("Amina"),
            "the ledger names who took the money"
        );
        assert!(
            stock_movements(&store, bun).is_empty(),
            "a product with no ledger never gets a movement"
        );

        let sale_id: Option<i64> = store
            .conn()
            .query_row(
                "SELECT sale_id FROM stock_movements WHERE product_id = ?1 AND kind = 'sale'",
                [soap],
                |row| row.get(0),
            )
            .expect("sale id");
        assert_eq!(sale_id, Some(sale.id));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_zero_opening_count_tracks_without_pretending_anything_moved() {
        let (store, path) = store("stock-zero");
        let soap = insert_product(&store, "Soap", 1_200, "Household", None, None).expect("product");

        track_product(&store, soap, 0, 5, None).expect("track");
        let tracked = stock(&store);
        assert_eq!(tracked.len(), 1);
        assert_eq!(tracked[0].on_hand, 0);
        assert!(tracked[0].is_out());
        assert!(stock_movements(&store, soap).is_empty());

        // Tracking is a decision, not a button: starting twice is refused.
        assert!(track_product(&store, soap, 4, 5, None).is_err());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_movement_whose_sign_contradicts_its_kind_is_refused() {
        let (store, path) = store("stock-signs");
        let soap = insert_product(&store, "Soap", 1_200, "Household", None, None).expect("product");
        track_product(&store, soap, 5, 2, None).expect("track");

        for (delta, kind) in [
            (-1, StockKind::Receive),
            (-1, StockKind::Return),
            (1, StockKind::Waste),
            (1, StockKind::Opening),
            (-1, StockKind::Sale),
            (0, StockKind::Adjust),
        ] {
            assert!(
                record_stock_movement(&store, soap, delta, kind, "", None).is_err(),
                "{kind:?} with a delta of {delta} should be refused"
            );
        }
        assert_eq!(stock(&store)[0].on_hand, 5, "nothing was written");

        let untracked = insert_product(&store, "Bun", 500, "Bakery", None, None).expect("bun");
        assert!(
            record_stock_movement(&store, untracked, 1, StockKind::Receive, "", None).is_err(),
            "an untracked product has no ledger to append to"
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn stock_may_go_negative_when_the_shelf_and_the_ledger_disagree() {
        let (store, path) = store("stock-negative");
        let soap = insert_product(&store, "Soap", 1_000, "Household", None, None).expect("product");
        track_product(&store, soap, 1, 2, None).expect("track");

        record_sale(
            &store,
            &[SaleLine {
                product_id: Some(soap),
                name: "Soap".to_owned(),
                qty: 4,
                unit_minor: 1_000,
            }],
            0,
            0,
            4_000,
            None,
        )
        .expect("the till sells even when the count says it cannot");

        let tracked = stock(&store);
        assert_eq!(tracked[0].on_hand, -3);
        assert!(tracked[0].is_out());

        // Receiving does not hide the debt: the count stays negative until the
        // shelf has caught up with what was sold.
        record_stock_movement(&store, soap, 2, StockKind::Receive, "", None).expect("receive");
        assert_eq!(stock(&store)[0].on_hand, -1);
        assert!(stock(&store)[0].is_out());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn history_is_newest_first_and_capped() {
        let (store, path) = store("stock-history");
        let soap = insert_product(&store, "Soap", 1_200, "Household", None, None).expect("product");
        track_product(&store, soap, 0, 5, None).expect("track");

        for _ in 0..(MOVEMENT_HISTORY + 5) {
            record_stock_movement(&store, soap, 1, StockKind::Receive, "", None).expect("receive");
        }

        let history = stock_movements(&store, soap);
        assert_eq!(history.len() as i64, MOVEMENT_HISTORY);
        assert!(history[0].id > history[1].id, "newest first");
        assert_eq!(stock(&store)[0].on_hand, MOVEMENT_HISTORY + 5);

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
            record_sale(&store, &lines, 0, 0, 2_000, None).is_err(),
            "2000 does not cover 2800"
        );

        // The sale row is inserted first, so this proves the transaction rolled
        // back rather than leaving a sale with no payment behind.
        assert_eq!(totals(&store, 3, None), (0, 0));
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

        assert!(record_sale(&store, &[], 0, 0, 0, None).is_err());
        assert_eq!(totals(&store, 3, None), (0, 0));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn archiving_a_product_does_not_rewrite_its_receipt() {
        let (store, path) = store("sale-then-archive");
        let flat_white = insert_product(&store, "Flat white", 3_400, "Coffee", None, None)
            .expect("insert product");

        let sale = record_sale(
            &store,
            &[SaleLine {
                product_id: Some(flat_white),
                name: "Flat white".to_owned(),
                qty: 1,
                unit_minor: 3_400,
            }],
            0,
            0,
            3_400,
            None,
        )
        .expect("record");

        delete_product(&store, flat_white).expect("archive");

        let (product_id, name, total_minor): (Option<i64>, String, i64) = store
            .conn()
            .query_row(
                "SELECT product_id, name, total_minor FROM sale_lines WHERE sale_id = ?1",
                [sale.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("the line survives the product");
        assert_eq!(
            product_id,
            Some(flat_white),
            "the link survives, so the report still knows which product it was"
        );
        assert_eq!(name, "Flat white");
        assert_eq!(total_minor, 3_400);
        assert!(products(&store).is_empty(), "the catalog hides it");

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_sale_is_stamped_with_an_identity_and_moves_the_cursor() {
        let (store, path) = store("sale-identity");
        let soap = insert_product(&store, "Soap", 1_000, "Household", None, None).expect("product");
        track_product(&store, soap, 10, 3, None).expect("track");

        let line = SaleLine {
            product_id: Some(soap),
            name: "Soap".to_owned(),
            qty: 1,
            unit_minor: 1_000,
        };
        let first =
            record_sale(&store, std::slice::from_ref(&line), 0, 0, 1_000, None).expect("first");
        let second = record_sale(&store, &[line], 0, 0, 1_000, None).expect("second");

        assert_ne!(first.uid, second.uid);
        assert!(uuid::Uuid::parse_str(&first.uid).is_ok(), "{}", first.uid);
        assert!(uuid::Uuid::parse_str(&second.uid).is_ok());

        let device = ensure_device_id(&store).expect("device id");
        assert_eq!(
            ensure_device_id(&store).expect("stable"),
            device,
            "the install keeps one name"
        );

        let (origin, seq, received_at, offline): (String, i64, Option<String>, i64) = store
            .conn()
            .query_row(
                "SELECT origin_device, change_seq, received_at, is_offline
                 FROM sales WHERE id = ?1",
                [first.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("the sale is stamped");
        assert_eq!(origin, device);
        assert_eq!(seq, 1);
        assert_eq!(
            received_at, None,
            "a local sale was not received from a peer"
        );
        assert_eq!(offline, 0);

        let second_seq: i64 = store
            .conn()
            .query_row(
                "SELECT change_seq FROM sales WHERE id = ?1",
                [second.id],
                |row| row.get(0),
            )
            .expect("second cursor");
        assert_eq!(second_seq, 2, "the cursor counts sales, not seconds");

        let line_uid: String = store
            .conn()
            .query_row(
                "SELECT uid FROM sale_lines WHERE sale_id = ?1",
                [first.id],
                |row| row.get(0),
            )
            .expect("line uid");
        assert!(uuid::Uuid::parse_str(&line_uid).is_ok());

        let payment_uid: String = store
            .conn()
            .query_row(
                "SELECT uid FROM payments WHERE sale_id = ?1",
                [first.id],
                |row| row.get(0),
            )
            .expect("payment uid");
        assert!(uuid::Uuid::parse_str(&payment_uid).is_ok());

        let movement_uid: String = store
            .conn()
            .query_row(
                "SELECT uid FROM stock_movements WHERE sale_id = ?1",
                [first.id],
                |row| row.get(0),
            )
            .expect("movement uid");
        assert!(uuid::Uuid::parse_str(&movement_uid).is_ok());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn catalog_writes_advance_the_catalog_version() {
        let (store, path) = store("catalog-version");
        let version = || {
            meta(&store, "catalog_version")
                .and_then(|value| value.parse::<i64>().ok())
                .expect("catalog_version is an integer")
        };

        let start = version();
        let product =
            insert_product(&store, "Soap", 1_000, "Household", None, None).expect("product");
        let after_insert = version();
        assert!(after_insert > start, "a peer must see the catalog move");

        update_product(&store, product, "Soap", 1_200, "Household", None, None).expect("rename");
        assert!(version() > after_insert, "a rename is a catalog change too");

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
            0,
            500,
            Some(&brian.id),
        )
        .expect("record");

        assert_eq!(sale.paid_minor(), 500);
        let history = sales(&store, 3, 10, None);
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
        assert_eq!(
            sales(&store, 3, 10, None)[0].cashier.as_deref(),
            Some("Brian")
        );

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
            0,
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
            0,
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
            0,
            12_000,
            Some(&amina.id),
        )
        .expect("record");

        let people = by_person(&store, 3, None);
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
            0,
            500,
            None,
        )
        .expect("record");

        let people = by_person(&store, 3, None);
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
            0,
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
            0,
            500,
            None,
        )
        .expect("record an exact sale");

        let methods = by_method(&store, 3, None);
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

        assert!(by_method(&store, 0, None).is_empty());

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

        let rows = export_sales(&store, 3, None).expect("export");
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
