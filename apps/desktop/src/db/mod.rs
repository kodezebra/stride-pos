//! POS schema and the few reads the shell needs.
//!
//! Forward-only: append migrations with a higher version. Never edit one that
//! has shipped, because some till somewhere already applied it.

mod catalog;

pub use catalog::*;

mod sync;

pub use sync::*;

mod staff;

pub use staff::*;

mod schema;

pub use schema::*;

mod types;

pub use types::*;

use anyhow::Context as _;
use rusqlite::OptionalExtension as _;
use std::path::{Path, PathBuf};

use crate::sync::protocol as sync_proto;

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
        "SELECT p.id, p.name, p.category, p.low_stock, p.cost_milli, p.target_stock,
                COALESCE(SUM(m.delta), 0),
                COALESCE((
                    SELECT SUM((pl.qty_ordered - pl.qty_received) * pl.pack_size)
                    FROM purchase_lines pl
                    JOIN purchases pu ON pu.id = pl.purchase_id
                    WHERE pl.product_id = p.id AND pu.status = 'planned'
                ), 0),
                p.pack_size, p.unit_label, p.perishable, p.shelf_life_days
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
            cost_milli: row.get(4)?,
            target_stock: row.get(5)?,
            on_hand: row.get(6)?,
            incoming: row.get(7)?,
            pack_size: row.get(8)?,
            unit_label: row.get(9)?,
            perishable: row.get(10)?,
            shelf_life_days: row.get(11)?,
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
    target_stock: Option<i64>,
    user_id: Option<&str>,
) -> anyhow::Result<()> {
    anyhow::ensure!(opening >= 0, "the opening count cannot be negative");
    anyhow::ensure!(low_stock >= 0, "the low-stock level cannot be negative");
    anyhow::ensure!(
        target_stock.is_none_or(|target| target >= low_stock),
        "the target cannot be below the low-stock level"
    );

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
        "UPDATE products SET tracked = 1, low_stock = ?2, target_stock = ?3
         WHERE id = ?1 AND archived = 0",
        (product_id, low_stock, target_stock),
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

/// Mark a tracked product as perishable, or clear that.
///
/// Perishability is not part of [`ProductFields`]: like tracking, it is a
/// statement about how the shop keeps the product, not something the catalog
/// form rewrites on every save. A product that is not tracked has no ledger, so
/// there is nothing for a date to sit on: perishable is refused rather than
/// stored and silently ignored.
pub fn set_perishability(
    store: &storage::Store,
    product_id: i64,
    perishable: bool,
    shelf_life_days: Option<i64>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        shelf_life_days.is_none_or(|days| days > 0),
        "a shelf life must be a whole number of days"
    );

    if perishable {
        let (tracked, archived): (bool, bool) = store
            .conn()
            .query_row(
                "SELECT tracked, archived FROM products WHERE id = ?1",
                [product_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .context("that product is no longer in the catalog")?;
        anyhow::ensure!(!archived, "that product is archived");
        anyhow::ensure!(
            tracked,
            "only a tracked product can be marked perishable; start tracking it first"
        );
    }

    let changed = store.conn().execute(
        "UPDATE products
            SET perishable = ?2, shelf_life_days = ?3
          WHERE id = ?1 AND archived = 0",
        (
            product_id,
            perishable,
            if perishable { shelf_life_days } else { None },
        ),
    )?;
    anyhow::ensure!(changed == 1, "that product is no longer in the catalog");
    Ok(())
}

/// Read a typed expiry date and write it as `YYYY-MM-DD`.
///
/// The pack prints a day, a month and a year in one of two orders, and a shop
/// types what it sees. Both `31/12/2026` and `2026-12-31` are accepted, a
/// single-digit day or month is fine, and anything that is not a real calendar
/// date is refused. SQLite's own `date()` would quietly turn `2026-02-30` into
/// March, which would hide a typing mistake until a report was wrong.
pub fn parse_expiry_date(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }

    let (year, month, day) = if let Some((year, rest)) = text.split_once('-') {
        let (month, day) = rest.split_once('-')?;
        (
            year.parse::<i64>().ok()?,
            month.parse::<i64>().ok()?,
            day.parse::<i64>().ok()?,
        )
    } else {
        let mut parts = text.split('/');
        let day = parts.next()?.parse::<i64>().ok()?;
        let month = parts.next()?.parse::<i64>().ok()?;
        let year = parts.next()?.parse::<i64>().ok()?;
        if parts.next().is_some() {
            return None;
        }
        (year, month, day)
    };

    if !(1900..=2999).contains(&year) || !(1..=12).contains(&month) {
        return None;
    }
    if day < 1 || day > i64::from(days_in_month(year, month)) {
        return None;
    }
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

/// Normalise an optional typed date, refusing a value that is not a date.
///
/// A blank field means "no date", not an error: returns and stock pulled off a
/// dated bucket are the only movements where the date is optional.
fn normalize_expiry(expires_on: Option<&str>) -> anyhow::Result<Option<String>> {
    match expires_on {
        None => Ok(None),
        Some(text) if text.trim().is_empty() => Ok(None),
        Some(text) => parse_expiry_date(text)
            .map(Some)
            .context("a date must be written like 31/12/2026"),
    }
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: i64) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days since 1970-01-01 for a civil date, after Howard Hinnant's algorithm.
///
/// The FEFO report needs "how many days until this date" without asking SQLite
/// once per bucket, and without a calendar library for one subtraction.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = (month + 9) % 12;
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Days since 1970-01-01 for a canonical `YYYY-MM-DD`, or `None` if malformed.
fn date_days(date: &str) -> Option<i64> {
    let mut parts = date.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(days_from_civil(year, month, day))
}

/// Today's local date in days since the epoch, for the same arithmetic.
fn today_days(store: &storage::Store) -> Option<i64> {
    let today: String = store
        .conn()
        .query_row("SELECT date('now','localtime')", [], |row| row.get(0))
        .ok()?;
    date_days(&today)
}

/// What is left on each dated slice of a product's stock.
///
/// Sales and untagged corrections are not allocated to a date when they are
/// written; they consume the earliest expiry first, which is what FEFO means.
/// Tagged waste comes off its own date, so pulling expired stock does not
/// silently eat a fresher delivery. Undated arrivals (an opening count, an
/// adjustment up, a delivery nobody dated) sit at the end of the queue.
fn derive_buckets(movements: &[(StockKind, Option<String>, i64)]) -> Vec<(Option<String>, i64)> {
    let mut dated: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    let mut undated_in = 0_i64;
    let mut untagged_out = 0_i64;

    for (kind, expires_on, delta) in movements {
        match kind {
            StockKind::Receive | StockKind::Return => match expires_on {
                Some(date) => *dated.entry(date.clone()).or_default() += delta,
                None => undated_in += delta,
            },
            StockKind::Waste => match expires_on {
                Some(date) => *dated.entry(date.clone()).or_default() += delta,
                None => untagged_out += -delta,
            },
            StockKind::Sale => untagged_out += -delta,
            StockKind::Adjust if *delta > 0 => undated_in += delta,
            StockKind::Adjust => untagged_out += -delta,
            StockKind::Opening => undated_in += delta,
        }
    }

    // A date cannot hold less than nothing. A dated waste that asked for more
    // than that batch still had — a typo, or a sale between counting the shelf
    // and tapping the button — took the excess off the rest of the shelf, so
    // the deficit joins the untagged pool instead of vanishing with the date.
    for balance in dated.values_mut() {
        if *balance < 0 {
            untagged_out += -*balance;
            *balance = 0;
        }
    }

    let mut leftover = untagged_out;
    for balance in dated.values_mut() {
        if *balance <= 0 || leftover <= 0 {
            continue;
        }
        let taken = (*balance).min(leftover);
        *balance -= taken;
        leftover -= taken;
    }

    let mut undated = undated_in;
    if leftover > 0 {
        let taken = undated.max(0).min(leftover);
        undated -= taken;
        leftover -= taken;
    }
    // More was sold than the ledger ever held: the shop is negative on hand.
    // Report it against the undated pool rather than inventing a date for it,
    // so the sum of the buckets still matches `SUM(delta)`.
    if leftover > 0 {
        undated -= leftover;
    }

    let mut buckets: Vec<(Option<String>, i64)> = dated
        .into_iter()
        .filter(|(_, balance)| *balance > 0)
        .map(|(date, balance)| (Some(date), balance))
        .collect();
    if undated != 0 {
        buckets.push((None, undated));
    }
    buckets
}

/// One tracked product's movements, oldest first, as FEFO reads them.
fn product_movements(
    store: &storage::Store,
    product_id: i64,
) -> Vec<(StockKind, Option<String>, i64)> {
    let mut statement = match store.conn().prepare(
        "SELECT kind, expires_on, delta
         FROM stock_movements
         WHERE product_id = ?1
         ORDER BY id",
    ) {
        Ok(statement) => statement,
        Err(err) => {
            tracing::warn!(error = %err, "expiry movements prepare failed");
            return Vec::new();
        }
    };

    let rows = statement.query_map([product_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, i64>(2)?,
        ))
    });

    let Ok(rows) = rows else {
        tracing::warn!("expiry movements query failed");
        return Vec::new();
    };

    rows.filter_map(Result::ok)
        .filter_map(|(kind, expires_on, delta)| {
            Some((StockKind::from_key(&kind)?, expires_on, delta))
        })
        .collect()
}

/// The dated stock across every perishable product, earliest date first.
///
/// Only what is still on the shelf appears: a bucket a sale has already taken
/// is no longer expiring stock, and showing it would send the shop looking for
/// something it does not have.
pub fn expiring(store: &storage::Store) -> Vec<ExpiryBucket> {
    let today = today_days(store);

    let products: Vec<(i64, String, String)> = {
        let mut statement = match store.conn().prepare(
            "SELECT p.id, p.name, p.unit_label
             FROM products p
             WHERE p.tracked = 1 AND p.archived = 0
               AND (p.perishable = 1 OR EXISTS (
                   SELECT 1 FROM stock_movements m
                   WHERE m.product_id = p.id AND m.expires_on IS NOT NULL
               ))
             ORDER BY p.name COLLATE NOCASE",
        ) {
            Ok(statement) => statement,
            Err(err) => {
                tracing::warn!(error = %err, "expiry prepare failed");
                return Vec::new();
            }
        };
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        });
        match rows {
            Ok(rows) => rows.filter_map(Result::ok).collect(),
            Err(err) => {
                tracing::warn!(error = %err, "expiry query failed");
                return Vec::new();
            }
        }
    };

    let mut buckets = Vec::new();
    for (product_id, name, unit_label) in products {
        for (expires_on, on_hand) in derive_buckets(&product_movements(store, product_id)) {
            let Some(date) = expires_on else {
                // Undated stock is not what this report is about.
                continue;
            };
            let days_left = match (date_days(&date), today) {
                (Some(expiry), Some(today)) => Some(expiry - today),
                _ => None,
            };
            buckets.push(ExpiryBucket {
                product_id,
                name: name.clone(),
                unit_label: unit_label.clone(),
                expires_on: Some(date),
                on_hand,
                days_left,
            });
        }
    }

    buckets.sort_by(|a, b| {
        a.expires_on
            .cmp(&b.expires_on)
            .then_with(|| a.name.cmp(&b.name))
    });
    buckets
}

/// Pull everything left on one expired date into waste, and say how much moved.
///
/// The bucket is recomputed here rather than trusted from the screen: the list
/// on screen is a cache, and a sale between the paint and the click would
/// otherwise pull stock the shop no longer has. The waste movement carries the
/// date, so FEFO takes it off that slice and leaves a fresher delivery alone.
pub fn pull_expired(
    store: &storage::Store,
    product_id: i64,
    expires_on: &str,
    user_id: Option<&str>,
) -> anyhow::Result<i64> {
    let date = parse_expiry_date(expires_on).context("a date must be written like 31/12/2026")?;
    let remaining = derive_buckets(&product_movements(store, product_id))
        .into_iter()
        .find(|(bucket_date, _)| bucket_date.as_deref() == Some(date.as_str()))
        .map(|(_, on_hand)| on_hand)
        .unwrap_or(0);
    anyhow::ensure!(remaining > 0, "that date has no stock left to pull");

    record_stock_movement(
        store,
        product_id,
        -remaining,
        StockKind::Waste,
        "Expired",
        Some(&date),
        user_id,
    )?;
    Ok(remaining)
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
    expires_on: Option<&str>,
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

    let expires_on = normalize_expiry(expires_on)?;
    // A date says which slice of the shelf moved. Only an arrival, a return or
    // stock pulled off a dated bucket has one; a correction or a sale is a
    // change to the count as a whole.
    anyhow::ensure!(
        expires_on.is_none()
            || matches!(
                kind,
                StockKind::Receive | StockKind::Return | StockKind::Waste
            ),
        "{} cannot carry an expiry date",
        kind.label()
    );

    let (name, tracked, perishable): (String, bool, bool) = store
        .conn()
        .query_row(
            "SELECT name, tracked, perishable
             FROM products WHERE id = ?1 AND archived = 0",
            [product_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?
        .context("that product is no longer in the catalog")?;
    anyhow::ensure!(tracked, "that product's stock is not tracked");
    if kind == StockKind::Receive && perishable {
        anyhow::ensure!(
            expires_on.is_some(),
            "{name} is perishable; date the delivery from the pack"
        );
    }

    store.conn().execute(
        "INSERT INTO stock_movements
             (product_id, delta, kind, note, expires_on, user_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        (
            product_id,
            delta,
            kind.key(),
            optional_text(note),
            expires_on,
            user_id,
        ),
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
                strftime('%d/%m %H:%M', m.created_at, 'localtime'), u.name,
                m.expires_on
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
            row.get::<_, Option<String>>(6)?,
        ))
    });

    let Ok(rows) = rows else {
        tracing::warn!("stock history query failed");
        return Vec::new();
    };

    rows.filter_map(Result::ok)
        .filter_map(|(id, delta, kind, note, created_at, user, expires_on)| {
            Some(StockMovement {
                id,
                delta,
                kind: StockKind::from_key(&kind)?,
                note,
                created_at,
                user,
                expires_on,
            })
        })
        .collect()
}

/// Every purchase, newest first.
///
/// The list is small enough to read whole, like the catalog. Totals are
/// recomputed from the lines so a purchase and its list can never disagree.
pub fn purchases(store: &storage::Store) -> Vec<Purchase> {
    let mut statement = match store.conn().prepare(
        "SELECT p.id, p.supplier_id, sup.name, p.status, p.note,
                strftime('%d/%m %H:%M', p.created_at, 'localtime'),
                strftime('%d/%m %H:%M', p.received_at, 'localtime'),
                COUNT(l.id),
                COALESCE(SUM(l.qty_ordered), 0),
                COALESCE(SUM(l.qty_received), 0),
                SUM(CASE WHEN p.status = 'received' THEN l.qty_received
                         ELSE l.qty_ordered END * l.unit_cost_minor * l.pack_size)
         FROM purchases p
         LEFT JOIN suppliers sup ON sup.id = p.supplier_id
         LEFT JOIN purchase_lines l ON l.purchase_id = p.id
         GROUP BY p.id
         ORDER BY p.id DESC",
    ) {
        Ok(statement) => statement,
        Err(err) => {
            tracing::warn!(error = %err, "purchases prepare failed");
            return Vec::new();
        }
    };

    let rows = statement.query_map([], |row| {
        let status: String = row.get(3)?;
        Ok((
            Purchase {
                id: row.get(0)?,
                supplier_id: row.get(1)?,
                supplier: row.get(2)?,
                status: PurchaseStatus::from_key(&status).unwrap_or(PurchaseStatus::Planned),
                note: row.get(4)?,
                created_at: row.get(5)?,
                received_at: row.get(6)?,
                line_count: row.get(7)?,
                unit_count: row.get(8)?,
                received_count: row.get(9)?,
                cost_minor: row.get(10)?,
            },
            status,
        ))
    });

    match rows {
        Ok(rows) => rows
            .filter_map(Result::ok)
            .filter_map(|(purchase, status)| PurchaseStatus::from_key(&status).map(|_| purchase))
            .collect(),
        Err(err) => {
            tracing::warn!(error = %err, "purchases query failed");
            Vec::new()
        }
    }
}

/// One purchase and its lines, or `None` when it is gone.
pub fn purchase_detail(store: &storage::Store, id: i64) -> Option<PurchaseDetail> {
    let purchase = purchases(store)
        .into_iter()
        .find(|purchase| purchase.id == id)?;

    let mut statement = match store.conn().prepare(
        "SELECT id, product_id, name_snapshot, qty_ordered, qty_received,
                unit_cost_minor, pack_size, expires_on
         FROM purchase_lines WHERE purchase_id = ?1 ORDER BY position, id",
    ) {
        Ok(statement) => statement,
        Err(err) => {
            tracing::warn!(error = %err, "purchase lines prepare failed");
            return None;
        }
    };

    let rows = statement.query_map([id], |row| {
        Ok(PurchaseLine {
            id: row.get(0)?,
            product_id: row.get(1)?,
            name: row.get(2)?,
            qty_ordered: row.get(3)?,
            qty_received: row.get(4)?,
            unit_cost_minor: row.get(5)?,
            pack_size: row.get(6)?,
            expires_on: row.get(7)?,
        })
    });

    match rows {
        Ok(rows) => Some(PurchaseDetail {
            purchase,
            lines: rows.filter_map(Result::ok).collect(),
        }),
        Err(err) => {
            tracing::warn!(error = %err, "purchase lines query failed");
            None
        }
    }
}

/// Write a new planned purchase. Returns its id.
pub fn create_purchase(
    store: &storage::Store,
    supplier_id: Option<&str>,
    note: &str,
    lines: &[PurchaseLineInput],
    user_id: Option<&str>,
) -> anyhow::Result<i64> {
    anyhow::ensure!(!lines.is_empty(), "a purchase needs at least one line");

    let transaction = store.conn().unchecked_transaction()?;

    transaction.execute(
        "INSERT INTO purchases (uid, supplier_id, note, user_id)
         VALUES (?1, ?2, ?3, ?4)",
        (
            uuid::Uuid::now_v7().to_string(),
            supplier_id,
            optional_text(note),
            user_id,
        ),
    )?;
    let id = transaction.last_insert_rowid();

    insert_purchase_lines(&transaction, id, lines)?;
    transaction.commit()?;
    Ok(id)
}

/// Replace a planned purchase's header and lines.
///
/// Only a plan with nothing received can be edited: replacing a line that a
/// movement already points at would orphan the ledger row. A partially received
/// purchase is continued on the receiving screen instead.
pub fn update_purchase(
    store: &storage::Store,
    id: i64,
    supplier_id: Option<&str>,
    note: &str,
    lines: &[PurchaseLineInput],
) -> anyhow::Result<()> {
    anyhow::ensure!(!lines.is_empty(), "a purchase needs at least one line");

    let transaction = store.conn().unchecked_transaction()?;

    let status: String = transaction
        .query_row("SELECT status FROM purchases WHERE id = ?1", [id], |row| {
            row.get(0)
        })
        .optional()?
        .context("that purchase no longer exists")?;
    anyhow::ensure!(status == "planned", "a received purchase cannot be changed");

    let received: i64 = transaction.query_row(
        "SELECT COALESCE(SUM(qty_received), 0) FROM purchase_lines WHERE purchase_id = ?1",
        [id],
        |row| row.get(0),
    )?;
    anyhow::ensure!(
        received == 0,
        "some of this purchase has arrived; it cannot be changed"
    );

    if let Some(supplier_id) = supplier_id {
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM suppliers WHERE id = ?1)",
            [supplier_id],
            |row| row.get(0),
        )?;
        anyhow::ensure!(exists, "that supplier is no longer in the directory");
    }

    transaction.execute(
        "UPDATE purchases SET supplier_id = ?2, note = ?3 WHERE id = ?1",
        (id, supplier_id, optional_text(note)),
    )?;
    transaction.execute("DELETE FROM purchase_lines WHERE purchase_id = ?1", [id])?;
    insert_purchase_lines(&transaction, id, lines)?;

    transaction.commit()?;
    Ok(())
}

/// Delete a plan that has received nothing.
///
/// The lines go with it, which a real purchase relies on; nothing in the ledger
/// points at them because nothing was received.
pub fn delete_planned_purchase(store: &storage::Store, id: i64) -> anyhow::Result<()> {
    let transaction = store.conn().unchecked_transaction()?;

    let status: String = transaction
        .query_row("SELECT status FROM purchases WHERE id = ?1", [id], |row| {
            row.get(0)
        })
        .optional()?
        .context("that purchase no longer exists")?;
    anyhow::ensure!(status == "planned", "a received purchase cannot be deleted");

    let received: i64 = transaction.query_row(
        "SELECT COALESCE(SUM(qty_received), 0) FROM purchase_lines WHERE purchase_id = ?1",
        [id],
        |row| row.get(0),
    )?;
    anyhow::ensure!(
        received == 0,
        "some of this purchase has arrived; it cannot be deleted"
    );

    transaction.execute("DELETE FROM purchase_lines WHERE purchase_id = ?1", [id])?;
    let changed = transaction.execute("DELETE FROM purchases WHERE id = ?1", [id])?;
    anyhow::ensure!(changed == 1, "that purchase no longer exists");

    transaction.commit()?;
    Ok(())
}

/// Close a trip.
///
/// Every counted unit writes one `receive` movement and advances its line, in
/// the same transaction, and the purchase becomes `Received`. There is no
/// half-received state: coming home is one act, and a second trip is a second
/// list.
///
/// `lines` carries what was bought. A line with `line_id: None` is an impulse
/// buy: it becomes a fully received line of its own. A line counted at zero is
/// kept as not bought, which is the truth about that trip. `remove` holds lines
/// that were a planning mistake; they are deleted, and the ledger is untouched
/// because nothing was ever received against them.
///
/// `note` and `supplier_id` are the purchase's final header, editable at
/// receiving time: the counter is where the operator learns the supplier was
/// wrong, so a correction there is kept rather than silently dropped.
pub fn receive_purchase(
    store: &storage::Store,
    id: i64,
    supplier_id: Option<&str>,
    lines: &[ReceiveLine],
    remove: &[i64],
    note: &str,
    user_id: Option<&str>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        lines.iter().any(|line| line.qty > 0) || !remove.is_empty(),
        "nothing was received"
    );

    let transaction = store.conn().unchecked_transaction()?;

    if let Some(supplier_id) = supplier_id {
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM suppliers WHERE id = ?1)",
            [supplier_id],
            |row| row.get(0),
        )?;
        anyhow::ensure!(exists, "that supplier is no longer in the directory");
    }

    let status: String = transaction
        .query_row("SELECT status FROM purchases WHERE id = ?1", [id], |row| {
            row.get(0)
        })
        .optional()?
        .context("that purchase no longer exists")?;
    anyhow::ensure!(
        status == "planned",
        "this purchase has already been received"
    );

    // Migration 15 closed the trips an earlier build left half-received, and
    // nothing since writes a count without closing. The check stays because the
    // cost of being wrong is a second copy of the goods on the shelf, and the
    // database is a file a person can edit.
    let already: i64 = transaction.query_row(
        "SELECT COALESCE(SUM(qty_received), 0) FROM purchase_lines WHERE purchase_id = ?1",
        [id],
        |row| row.get(0),
    )?;
    anyhow::ensure!(
        already == 0,
        "some of this purchase has already arrived; it cannot be received twice"
    );

    for line in lines {
        anyhow::ensure!(line.qty >= 0, "a quantity cannot be negative");
        anyhow::ensure!(
            line.unit_cost_minor.is_none_or(|cost| cost >= 0),
            "a cost cannot be negative"
        );
        anyhow::ensure!(line.pack_size >= 1, "a pack holds at least one unit");
        let expires_on = normalize_expiry(line.expires_on.as_deref())?;
        if line.qty == 0 {
            continue;
        }

        let product = transaction
            .query_row(
                "SELECT name, tracked, archived FROM products WHERE id = ?1",
                [line.product_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, bool>(1)?,
                        row.get::<_, bool>(2)?,
                    ))
                },
            )
            .optional()?
            .context("that product is no longer in the catalog")?;
        anyhow::ensure!(!product.2, "{} is archived", product.0);
        anyhow::ensure!(
            product.1,
            "{} is not tracked; only tracked products can be received",
            product.0
        );

        let line_id = match line.line_id {
            Some(line_id) => {
                // `IS` rather than `=`: a line whose product was deleted has a
                // NULL product_id, and NULL = NULL is NULL, not true.
                let (belongs, same_product): (bool, bool) = transaction
                    .query_row(
                        "SELECT purchase_id = ?2, product_id IS ?3
                         FROM purchase_lines WHERE id = ?1",
                        (line_id, id, line.product_id),
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?
                    .context("that purchase line no longer exists")?;
                anyhow::ensure!(belongs, "that line belongs to another purchase");
                // The movement is written against `line.product_id`, the line it
                // points at is the one on the shelf. If the two disagree, stock
                // and ledger would split: the shelf moves for one product while
                // the purchase records another.
                anyhow::ensure!(same_product, "that line was planned for another product");

                // The count sheet is the truth about the trip. Buying more than
                // planned means the plan understated it, so the ordered count is
                // raised to match instead of refusing goods that are already in
                // the bag.
                transaction.execute(
                    "UPDATE purchase_lines
                     SET qty_ordered = MAX(qty_ordered, ?2),
                         qty_received = qty_received + ?2,
                         unit_cost_minor = COALESCE(?3, unit_cost_minor),
                         pack_size = ?4,
                         expires_on = CASE WHEN ?5 IS NULL THEN expires_on ELSE ?5 END
                     WHERE id = ?1",
                    (
                        line_id,
                        line.qty,
                        line.unit_cost_minor,
                        line.pack_size,
                        expires_on,
                    ),
                )?;
                line_id
            }
            None => {
                anyhow::ensure!(
                    line.qty > 0,
                    "an item picked up at the market needs a quantity"
                );
                let position: i64 = transaction.query_row(
                    "SELECT COALESCE(MAX(position), -1) + 1
                     FROM purchase_lines WHERE purchase_id = ?1",
                    [id],
                    |row| row.get(0),
                )?;
                transaction.execute(
                    "INSERT INTO purchase_lines
                         (uid, purchase_id, product_id, name_snapshot, qty_ordered,
                          qty_received, unit_cost_minor, position, pack_size,
                          expires_on)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?7, ?8, ?9)",
                    (
                        uuid::Uuid::now_v7().to_string(),
                        id,
                        line.product_id,
                        product.0.as_str(),
                        line.qty,
                        line.unit_cost_minor,
                        position,
                        line.pack_size,
                        expires_on,
                    ),
                )?;
                transaction.last_insert_rowid()
            }
        };

        record_receive(&transaction, line_id, line, note, user_id)?;
    }

    for line_id in remove {
        let (belongs, received): (bool, i64) = transaction
            .query_row(
                "SELECT purchase_id = ?2, qty_received
                 FROM purchase_lines WHERE id = ?1",
                (line_id, id),
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .context("that purchase line no longer exists")?;
        anyhow::ensure!(belongs, "that line belongs to another purchase");
        anyhow::ensure!(
            received == 0,
            "a line that has already arrived cannot be removed"
        );
        transaction.execute("DELETE FROM purchase_lines WHERE id = ?1", [line_id])?;
    }

    let total: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM purchase_lines WHERE purchase_id = ?1",
        [id],
        |row| row.get(0),
    )?;

    // Every line was removed: there is nothing left to plan and nothing to
    // receive. Keeping an empty purchase would leave a ghost in the list that
    // can only be edited to add lines or deleted.
    if total == 0 {
        transaction.execute("DELETE FROM purchases WHERE id = ?1", [id])?;
        transaction.commit()?;
        return Ok(());
    }

    transaction.execute(
        "UPDATE purchases
         SET supplier_id = ?2, note = ?3, status = 'received',
             received_at = datetime('now')
         WHERE id = ?1",
        (id, supplier_id, optional_text(note)),
    )?;

    transaction.commit()?;
    Ok(())
}

/// Plan and receive a trip in one write.
///
/// A list made at the market may never be saved: it is a count sheet first and
/// a purchase second. Writing it and closing it in the same transaction means a
/// failure at the counter cannot leave a saved plan behind — either the goods
/// are on the shelf and the trip is recorded, or nothing happened at all.
///
/// Only bought lines belong here: "buy zero of this" is not something a
/// recorded purchase can say, and drops a line the shop never picked up.
pub fn create_and_receive(
    store: &storage::Store,
    supplier_id: Option<&str>,
    note: &str,
    lines: &[PurchaseLineInput],
    user_id: Option<&str>,
) -> anyhow::Result<i64> {
    anyhow::ensure!(!lines.is_empty(), "a purchase needs at least one line");

    let transaction = store.conn().unchecked_transaction()?;

    if let Some(supplier_id) = supplier_id {
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM suppliers WHERE id = ?1)",
            [supplier_id],
            |row| row.get(0),
        )?;
        anyhow::ensure!(exists, "that supplier is no longer in the directory");
    }

    transaction.execute(
        "INSERT INTO purchases (uid, supplier_id, note, user_id, status, received_at)
         VALUES (?1, ?2, ?3, ?4, 'received', datetime('now'))",
        (
            uuid::Uuid::now_v7().to_string(),
            supplier_id,
            optional_text(note),
            user_id,
        ),
    )?;
    let id = transaction.last_insert_rowid();

    let ids = insert_purchase_lines(&transaction, id, lines)?;

    // Nothing was planned ahead, so what was bought is what arrived.
    transaction.execute(
        "UPDATE purchase_lines SET qty_received = qty_ordered WHERE purchase_id = ?1",
        [id],
    )?;

    for (line, line_id) in lines.iter().zip(ids) {
        let received = ReceiveLine {
            line_id: Some(line_id),
            product_id: line.product_id,
            qty: line.qty,
            unit_cost_minor: line.unit_cost_minor,
            pack_size: line.pack_size,
            expires_on: line.expires_on.clone(),
        };
        record_receive(&transaction, line_id, &received, note, user_id)?;
    }

    transaction.commit()?;
    Ok(id)
}

/// Put one received line on the shelf: its movement and the product's last cost.
///
/// Shared by a planned line, an impulse buy written at the counter, and a trip
/// created while receiving. The caller has already checked that the product
/// belongs to the line and may be restocked.
fn record_receive(
    transaction: &rusqlite::Transaction<'_>,
    line_id: i64,
    line: &ReceiveLine,
    note: &str,
    user_id: Option<&str>,
) -> anyhow::Result<()> {
    let (name, perishable): (String, bool) = transaction.query_row(
        "SELECT name, perishable FROM products WHERE id = ?1",
        [line.product_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let expires_on = normalize_expiry(line.expires_on.as_deref())?;
    if perishable {
        anyhow::ensure!(
            expires_on.is_some(),
            "{name} is perishable; date the delivery from the pack"
        );
    }

    transaction.execute(
        "INSERT INTO stock_movements
             (product_id, delta, kind, note, user_id, purchase_line_id,
              expires_on, uid)
         VALUES (?1, ?2, 'receive', ?3, ?4, ?5, ?6, ?7)",
        (
            line.product_id,
            line.qty * line.pack_size,
            optional_text(note),
            user_id,
            line_id,
            expires_on,
            uuid::Uuid::now_v7().to_string(),
        ),
    )?;

    if let Some(cost) = line.unit_cost_minor {
        // `unit_cost_minor` is the price of one ordered unit — a pack when
        // `pack_size` is above one. The product cost that margin and the
        // loose-price prefill read is per base unit, so it is divided down
        // and kept in thousandths. Rounding to nearest keeps 1,850 per 100
        // at 18.5 rather than truncating to 18.
        let cost_milli = (cost * 1000 + line.pack_size / 2) / line.pack_size;
        transaction.execute(
            "UPDATE products SET cost_milli = ?2 WHERE id = ?1",
            (line.product_id, cost_milli),
        )?;
    }

    Ok(())
}

/// Insert a plan's lines, resolving each product's name at write time.
///
/// Returns their ids in the order given, which is what receiving a trip that
/// was created in the same write needs to point its movements at.
fn insert_purchase_lines(
    transaction: &rusqlite::Transaction<'_>,
    purchase_id: i64,
    lines: &[PurchaseLineInput],
) -> anyhow::Result<Vec<i64>> {
    let mut seen = std::collections::HashSet::new();
    let mut ids = Vec::with_capacity(lines.len());

    for (position, line) in lines.iter().enumerate() {
        anyhow::ensure!(line.qty > 0, "every line needs a quantity");
        anyhow::ensure!(
            line.unit_cost_minor.is_none_or(|cost| cost >= 0),
            "a cost cannot be negative"
        );
        anyhow::ensure!(line.pack_size >= 1, "a pack holds at least one unit");
        let expires_on = normalize_expiry(line.expires_on.as_deref())?;
        anyhow::ensure!(
            seen.insert((line.product_id, expires_on.clone())),
            "the same product is on the list twice with the same date"
        );

        let (name, tracked, archived): (String, bool, bool) = transaction
            .query_row(
                "SELECT name, tracked, archived FROM products WHERE id = ?1",
                [line.product_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .context("that product is no longer in the catalog")?;
        anyhow::ensure!(!archived, "{name} is archived");
        anyhow::ensure!(
            tracked,
            "{name} is not tracked; only tracked products can be restocked"
        );

        transaction.execute(
            "INSERT INTO purchase_lines
                 (uid, purchase_id, product_id, name_snapshot, qty_ordered,
                  unit_cost_minor, position, pack_size, expires_on)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            (
                uuid::Uuid::now_v7().to_string(),
                purchase_id,
                line.product_id,
                name.as_str(),
                line.qty,
                line.unit_cost_minor,
                position as i64,
                line.pack_size,
                expires_on,
            ),
        )?;
        ids.push(transaction.last_insert_rowid());
    }

    Ok(ids)
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
/// `shift_uid` is the drawer the cash went into. `None` when the drawer could
/// not be opened — a sale is never blocked by bookkeeping, and an unattributed
/// sale is a gap in a count, not a lost sale.
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
    shift_uid: Option<&str>,
) -> anyhow::Result<RecordedSale> {
    anyhow::ensure!(!lines.is_empty(), "a sale needs at least one line");

    let total_minor = lines.iter().try_fold(0_i64, |sum, line| {
        anyhow::ensure!(line.qty > 0, "{} has no quantity", line.name);
        anyhow::ensure!(line.unit_minor >= 0, "{} has a negative price", line.name);
        anyhow::ensure!(
            line.unit_factor >= 1,
            "{} needs at least one unit in its pack",
            line.name
        );
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
    let receipt_no = next_receipt_no(&transaction, &origin_device)?;

    transaction.execute(
        "INSERT INTO sales
             (total_minor, rounding_minor, tax_minor, user_id, uid, origin_device,
              change_seq, shift_uid, receipt_no)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        (
            total_minor,
            rounding_minor,
            tax_minor,
            user_id,
            sale_uid.as_str(),
            origin_device.as_str(),
            change_seq,
            shift_uid,
            receipt_no.as_str(),
        ),
    )?;
    let id = transaction.last_insert_rowid();

    for line in lines {
        let line_uid = uuid::Uuid::now_v7().to_string();
        // The cost is the one the shop last paid, snapshotted here. Profit is a
        // historical fact: a supplier's next price must not rewrite last
        // month's margin. An untracked or never-costed product stores NULL,
        // which the margin query reads as "not known" rather than zero. The
        // product's cost is per base unit in thousandths, so a pack line's
        // cost is `qty × unit_factor × cost_milli / 1000`.
        let cost_milli: Option<i64> = match line.product_id {
            Some(product_id) => transaction
                .query_row(
                    "SELECT cost_milli FROM products WHERE id = ?1",
                    [product_id],
                    |row| row.get::<_, Option<i64>>(0),
                )
                .optional()?
                .flatten(),
            None => None,
        };
        transaction.execute(
            "INSERT INTO sale_lines
                 (sale_id, product_id, name, qty, unit_minor, total_minor, cost_minor,
                  uid, unit_factor, unit_label, cost_milli)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            (
                id,
                line.product_id,
                line.name.as_str(),
                line.qty,
                line.unit_minor,
                line.qty * line.unit_minor,
                cost_milli.map(|cost| (cost + 500) / 1000),
                line_uid.as_str(),
                line.unit_factor,
                line.unit_label.as_deref(),
                cost_milli,
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
                let units = line
                    .qty
                    .checked_mul(line.unit_factor)
                    .context("a line's stock movement does not fit in 64 bits")?;
                let movement_uid = uuid::Uuid::now_v7().to_string();
                transaction.execute(
                    "INSERT INTO stock_movements
                         (product_id, delta, kind, sale_id, user_id, uid)
                     VALUES (?1, ?2, 'sale', ?3, ?4, ?5)",
                    (product_id, -units, id, user_id, movement_uid.as_str()),
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

// ---------------------------------------------------------------------------
// The cash drawer
// ---------------------------------------------------------------------------

/// The columns every shift read selects, in the order [`shift_from_row`] wants.
const SHIFT_COLUMNS: &str = "s.uid, strftime('%d/%m %H:%M', s.opened_at, 'localtime'), u.name,
     s.float_minor, s.closed_at, s.counted_minor
     FROM shifts s LEFT JOIN users u ON u.id = s.opened_by";

fn shift_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Shift> {
    Ok(Shift {
        uid: row.get(0)?,
        opened_at: row.get(1)?,
        opened_by: row.get(2)?,
        float_minor: row.get(3)?,
        closed_at: row.get(4)?,
        counted_minor: row.get(5)?,
    })
}

/// The drawer this till is accountable for: the open shift, or one that was
/// counted today.
///
/// A shift closed yesterday is history, and history lives in the Day list. The
/// card on Today's page is about the money in the drawer right now, so it must
/// not sit there for ever saying "closed".
pub fn drawer(store: &storage::Store) -> Option<Drawer> {
    let shift = match store
        .conn()
        .query_row(
            &format!(
                "SELECT {SHIFT_COLUMNS}
                 WHERE s.closed_at IS NULL
                    OR date(s.closed_at, 'localtime') = date('now', 'localtime')
                 ORDER BY s.id DESC LIMIT 1"
            ),
            [],
            shift_from_row,
        )
        .optional()
    {
        Ok(shift) => shift?,
        Err(err) => {
            tracing::warn!(error = %err, "the drawer could not be read");
            return None;
        }
    };

    let totals = drawer_totals(store, &shift.uid);
    Some(Drawer { shift, totals })
}

/// The shift whose drawer is still open, if any.
fn open_shift(store: &storage::Store) -> Option<Shift> {
    match store
        .conn()
        .query_row(
            &format!("SELECT {SHIFT_COLUMNS} WHERE s.closed_at IS NULL"),
            [],
            shift_from_row,
        )
        .optional()
    {
        Ok(shift) => shift,
        Err(err) => {
            tracing::warn!(error = %err, "the open shift could not be read");
            None
        }
    }
}

/// Open the drawer, or hand back the one that is already open.
///
/// Called on the way into the first sale of the day, so the caller never has to
/// know whether the drawer is open. Idempotent on purpose: a retried sale must
/// not split one day's cash across two shifts, and the partial unique index in
/// migration 13 makes that a database rule rather than a convention.
pub fn ensure_open_shift(
    store: &storage::Store,
    float_minor: i64,
    user_id: Option<&str>,
) -> anyhow::Result<Shift> {
    if let Some(open) = open_shift(store) {
        return Ok(open);
    }

    let uid = uuid::Uuid::now_v7().to_string();
    store.conn().execute(
        "INSERT INTO shifts (uid, opened_by, float_minor) VALUES (?1, ?2, ?3)",
        (uid.as_str(), user_id, float_minor.max(0)),
    )?;

    open_shift(store).context("the drawer was opened but could not be read back")
}

/// Cash rung up in a shift, and how many sales that was.
///
/// `payments.amount_minor` is what the customer paid, which is exactly what the
/// drawer gained: the change was counted out of the same drawer.
pub fn drawer_totals(store: &storage::Store, shift_uid: &str) -> DrawerTotals {
    match store.conn().query_row(
        "SELECT COALESCE(SUM(p.amount_minor), 0), COUNT(DISTINCT s.id)
         FROM sales s JOIN payments p ON p.sale_id = s.id
         WHERE s.shift_uid = ?1",
        [shift_uid],
        |row| {
            Ok(DrawerTotals {
                cash_minor: row.get(0)?,
                sales_count: row.get(1)?,
            })
        },
    ) {
        Ok(totals) => totals,
        Err(err) => {
            tracing::warn!(error = %err, "the drawer totals could not be read");
            DrawerTotals::default()
        }
    }
}

/// Count the drawer and close the shift.
///
/// One update, guarded on `closed_at IS NULL`, so two taps on "Close day" close
/// it once and the second one is an error the card can report instead of
/// silently overwriting the count somebody just made.
pub fn close_shift(
    store: &storage::Store,
    shift_uid: &str,
    counted_minor: i64,
    user_id: Option<&str>,
) -> anyhow::Result<()> {
    anyhow::ensure!(counted_minor >= 0, "a cash count cannot be negative");

    let changed = store.conn().execute(
        "UPDATE shifts
            SET closed_at = datetime('now'), closed_by = ?2, counted_minor = ?3
          WHERE uid = ?1 AND closed_at IS NULL",
        (shift_uid, user_id, counted_minor),
    )?;
    anyhow::ensure!(changed == 1, "that drawer has already been counted");
    Ok(())
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

/// Revenue and cost across the lines that have a recorded cost.
///
/// The same window and the same scope as [`totals`], so the profit card and the
/// revenue card above it always talk about the same sales. Lines without a cost
/// are excluded from both sides: their revenue is not profit, and inventing a
/// cost would be worse than admitting the shop has not recorded one.
pub fn margin(store: &storage::Store, range: usize, user: Option<&str>) -> Margin {
    let clause = window_clause(range, "s.created_at");
    let sql = format!(
        "SELECT
             COALESCE(SUM(CASE WHEN l.cost_milli IS NOT NULL
                               THEN l.qty * l.unit_minor ELSE 0 END), 0),
             (COALESCE(SUM(CASE WHEN l.cost_milli IS NOT NULL
                                THEN l.qty * l.unit_factor * l.cost_milli ELSE 0 END), 0)
              + 500) / 1000,
             COALESCE(SUM(CASE WHEN l.cost_milli IS NOT NULL THEN 1 ELSE 0 END), 0),
             COUNT(l.id)
         FROM sale_lines l JOIN sales s ON s.id = l.sale_id
         WHERE {clause}"
    );

    match store.conn().query_row(&sql, [user], |row| {
        Ok(Margin {
            revenue_minor: row.get(0)?,
            cost_minor: row.get(1)?,
            costed_lines: row.get(2)?,
            total_lines: row.get(3)?,
        })
    }) {
        Ok(margin) => margin,
        Err(err) => {
            tracing::warn!(error = %err, "margin query failed");
            Margin::default()
        }
    }
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
        "SELECT s.id, s.total_minor + s.rounding_minor, strftime('%d/%m %H:%M', s.created_at, 'localtime'), u.name, s.receipt_no
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
            receipt_no: row.get(4)?,
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
                COALESCE(GROUP_CONCAT(DISTINCT p.method), ''), s.receipt_no
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
                receipt_no: row.get(7)?,
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

/// The full record of one sale, or `None` when the id points nowhere. Rows are
/// never edited — a wrong checkout is a new sale — so the head and its lines
/// always describe one recorded transaction, not a moving target.
pub fn sale_detail(store: &storage::Store, id: i64) -> Option<SaleDetail> {
    let conn = store.conn();

    let head = match conn.query_row(
        "SELECT s.total_minor, s.rounding_minor, s.tax_minor, \
         strftime('%d/%m/%Y %H:%M', s.created_at, 'localtime'), u.name, s.receipt_no
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
                row.get::<_, Option<String>>(5)?,
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
        let mut statement = match conn.prepare(
            "SELECT name, qty, unit_minor, total_minor, unit_factor, unit_label
             FROM sale_lines WHERE sale_id = ?1",
        ) {
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
                unit_factor: row.get(4)?,
                unit_label: row.get(5)?,
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
        receipt_no: head.5,
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

/// Money in, grouped by how it was paid, in the same range the page shows.
///
/// This is *taken*, not *expected in the drawer*: the drawer's expectation
/// belongs to one shift and one count, not to a range of days, so the word stays
/// out of the query and out of the screen copy.
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

/// Sales and money per cashier, ranked by takings.
///
/// Takings come from the sale (`total + rounding`), the same figure the KPI
/// above it adds up, so the rows reconcile with the card even on a till whose
/// oldest sales have no payment row. Cash comes from the payments, because only
/// a payment knows how a split sale was settled.
///
/// `user` narrows to one person, like every other report: a cashier who somehow
/// reached this query still cannot read the shop's other names.
pub fn by_person(store: &storage::Store, range: usize, user: Option<&str>) -> Vec<PersonTotal> {
    person_totals(store, &window_clause(range, "s.created_at"), [user])
}

/// The same table narrowed to one drawer, for the card that asks who handled
/// the cash that is being counted tonight.
///
/// A sale whose drawer could not be opened has no `shift_uid`; it is missing
/// here for the same reason it is missing from [`drawer_totals`] — there is no
/// shift to attribute it to.
pub fn drawer_by_person(store: &storage::Store, shift_uid: &str) -> Vec<PersonTotal> {
    person_totals(store, "s.shift_uid = ?1", [shift_uid])
}

fn person_totals<P: rusqlite::Params>(
    store: &storage::Store,
    clause: &str,
    params: P,
) -> Vec<PersonTotal> {
    // Cash is a correlated subquery rather than a join: a split sale would
    // otherwise count its whole total once per payment row.
    let sql = format!(
        "SELECT COALESCE(u.name, 'No cashier'),
                COUNT(*),
                COALESCE(SUM(s.total_minor + s.rounding_minor), 0),
                COALESCE(SUM((SELECT COALESCE(SUM(p.amount_minor), 0)
                              FROM payments p
                              WHERE p.sale_id = s.id AND p.method = 'cash')), 0)
         FROM sales s LEFT JOIN users u ON u.id = s.user_id
         WHERE {clause}
         GROUP BY s.user_id
         ORDER BY 3 DESC, 1 COLLATE NOCASE"
    );

    let mut statement = match store.conn().prepare(&sql) {
        Ok(statement) => statement,
        Err(err) => {
            tracing::warn!(error = %err, "by-person prepare failed");
            return Vec::new();
        }
    };

    let rows = statement.query_map(params, |row| {
        Ok(PersonTotal {
            name: row.get(0)?,
            sales: row.get(1)?,
            takings_minor: row.get(2)?,
            cash_minor: row.get(3)?,
        })
    });

    match rows {
        Ok(rows) => rows.filter_map(Result::ok).collect(),
        Err(err) => {
            tracing::warn!(error = %err, "by-person query failed");
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

/// A backup, unless the newest one is younger than `hours`.
///
/// The file name is the record: every backup is named by the local clock it was
/// taken with, so "when did we last copy this" needs no row of its own — and a
/// database restored from a backup answers it from the files beside it, which
/// is exactly when a fresh copy is wanted.
///
/// `None` means a recent copy is already there and nothing was written. Errors
/// are returned: a scheduled backup that did not happen has to reach a person.
pub fn backup_if_due(
    store: &storage::Store,
    dir: &Path,
    hours: i64,
) -> anyhow::Result<Option<PathBuf>> {
    let cutoff = stamp_before(store, hours)?;
    match newest_backup(dir)? {
        Some(newest) if newest >= cutoff => Ok(None),
        _ => backup(store, dir).map(Some),
    }
}

/// The local clock `hours` ago, in the sortable form [`stamp`] writes.
///
/// Asked of SQLite so the subtraction uses the same clock and the same daylight
/// rules as the formatting; two clocks disagreeing about an hour would make the
/// check wrong twice a year.
fn stamp_before(store: &storage::Store, hours: i64) -> anyhow::Result<String> {
    let modifier = format!("-{hours} hours");
    store
        .conn()
        .query_row(
            "SELECT strftime('%Y%m%d-%H%M%S', 'now', 'localtime', ?1)",
            [modifier],
            |row| row.get::<_, String>(0),
        )
        .context("reading the clock")
}

/// The stamp of the newest backup in `dir`, if the directory has any.
///
/// Read from the name, not the modification time: the name is what a person
/// sorts by, and the sequence suffix means two backups in one second still
/// order the way they were taken.
fn newest_backup(dir: &Path) -> anyhow::Result<Option<String>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        // A till that has never backed up has no folder yet. That is "none",
        // not a failure to look.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("listing {}", dir.display())),
    };

    let mut newest: Option<String> = None;
    for entry in entries.filter_map(Result::ok) {
        if !is_backup(&entry.path()) {
            continue;
        }
        let name = entry.file_name();
        let Some(rest) = name
            .to_str()
            .and_then(|name| name.strip_prefix(BACKUP_PREFIX))
        else {
            continue;
        };
        // `<YYYYMMDD>-<HHMMSS>-<NNNN>.db`: the stamp is the first two parts.
        let mut parts = rest.split('-');
        let (Some(day), Some(clock)) = (parts.next(), parts.next()) else {
            continue;
        };
        if day.len() != 8 || clock.len() != 6 {
            continue;
        }
        let stamp = format!("{day}-{clock}");
        if newest.as_ref().is_none_or(|current| stamp > *current) {
            newest = Some(stamp);
        }
    }
    Ok(newest)
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
    use storage::Migration;

    /// The pre-pack call shape, kept for the many tests that only care about a
    /// name, a price and a barcode. The production entry point takes the whole
    /// set of fields; these wrappers fill the rest with the neutral defaults.
    fn insert_product(
        store: &storage::Store,
        name: &str,
        price_minor: i64,
        category: &str,
        category_id: Option<i64>,
        barcode: Option<&str>,
    ) -> anyhow::Result<i64> {
        super::insert_product(
            store,
            &ProductFields {
                name,
                price_minor,
                category,
                category_id,
                barcode,
                unit_label: "unit",
                pack_size: 1,
                pack_price_minor: None,
                pack_barcode: None,
            },
        )
    }

    fn update_product(
        store: &storage::Store,
        id: i64,
        name: &str,
        price_minor: i64,
        category: &str,
        category_id: Option<i64>,
        barcode: Option<&str>,
    ) -> anyhow::Result<()> {
        super::update_product(
            store,
            id,
            &ProductFields {
                name,
                price_minor,
                category,
                category_id,
                barcode,
                unit_label: "unit",
                pack_size: 1,
                pack_price_minor: None,
                pack_barcode: None,
            },
        )
    }

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

        assert_eq!(sales(&store, 3, 10, Some(&alice.id)).len(), 2);
        assert!(
            sales(&store, 3, 10, Some(&alice.id))
                .iter()
                .all(|sale| sale.cashier.as_deref() == Some("Alice"))
        );

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

        // The person table is scoped like the rest: Alice's two sales, cash and
        // all; the whole shop sees both names.
        let people = by_person(&store, 3, None);
        assert_eq!(people.len(), 2);
        let alice_row = people
            .iter()
            .find(|person| person.name == "Alice")
            .expect("Alice rang two sales");
        assert_eq!(alice_row.sales, 2);
        assert_eq!(alice_row.takings_minor, 500);
        assert_eq!(alice_row.cash_minor, 200, "only the paid sale had cash");
        let mine = by_person(&store, 3, Some(&alice.id));
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].name, "Alice");

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn by_person_separates_cash_from_takings_and_names_the_unattributed() {
        let (store, path) = store("by-person");
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

        let alice_cash = insert_sale_for(&store, "-0 days", &alice.id, &[("Tea", 2, 100)]);
        let alice_mobile = insert_sale_for(&store, "-0 days", &alice.id, &[("Rice", 1, 500)]);
        let bob_cash = insert_sale_for(&store, "-0 days", &bob.id, &[("Sugar", 1, 1_000)]);
        let unattributed = insert_sale(&store, "-0 days", &[("Salt", 1, 50)]);

        for (sale, method, amount) in [
            (alice_cash, "cash", 200),
            (alice_mobile, "mobile", 500),
            (bob_cash, "cash", 1_000),
            (unattributed, "cash", 50),
        ] {
            store
                .conn()
                .execute(
                    "INSERT INTO payments (sale_id, method, amount_minor, tendered_minor, change_minor)
                     VALUES (?1, ?2, ?3, ?3, 0)",
                    (sale, method, amount),
                )
                .expect("insert payment");
        }

        let people = by_person(&store, 3, None);
        assert_eq!(people.len(), 3, "two cashiers and one sale without one");
        assert_eq!(people[0].name, "Bob", "ranked by takings, not by name");
        assert_eq!(people[0].sales, 1);
        assert_eq!(people[0].cash_minor, 1_000);
        assert_eq!(people[0].takings_minor, 1_000);

        assert_eq!(people[1].name, "Alice");
        assert_eq!(people[1].sales, 2);
        assert_eq!(
            people[1].cash_minor, 200,
            "the mobile sale is not drawer cash"
        );
        assert_eq!(people[1].takings_minor, 700, "but it is still her taking");

        assert_eq!(people[2].name, "No cashier");
        assert_eq!(people[2].takings_minor, 50);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn the_drawer_split_covers_one_shift_and_leaves_the_others_alone() {
        let (store, path) = store("drawer-by-person");
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

        let sale = |name: &str, unit_minor: i64| SaleLine {
            product_id: None,
            name: name.to_owned(),
            qty: 1,
            unit_minor,
            unit_factor: 1,
            unit_label: None,
        };
        let ring = |user: &str, shift: &str, name: &str, amount: i64| {
            record_sale(
                &store,
                &[sale(name, amount)],
                0,
                0,
                amount,
                Some(user),
                Some(shift),
            )
            .expect("sale");
        };

        ring(&alice.id, "drawer-a", "Tea", 200);
        ring(&bob.id, "drawer-a", "Rice", 500);
        ring(&alice.id, "drawer-b", "Sugar", 900);

        let people = drawer_by_person(&store, "drawer-a");
        assert_eq!(people.len(), 2);
        assert_eq!(
            people.iter().map(|person| person.cash_minor).sum::<i64>(),
            700,
            "only the drawer being counted"
        );

        let tomorrow = drawer_by_person(&store, "drawer-b");
        assert_eq!(tomorrow.len(), 1);
        assert_eq!(tomorrow[0].name, "Alice");
        assert_eq!(tomorrow[0].cash_minor, 900);

        assert!(drawer_by_person(&store, "nobody").is_empty());

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
                unit_factor: 1,
                unit_label: None,
            },
            SaleLine {
                product_id: None,
                name: "Unlisted bun".to_owned(),
                qty: 2,
                unit_minor: 500,
                unit_factor: 1,
                unit_label: None,
            },
        ];

        // 2800 + 1000 = 3800 exact; rounded down 20 to the 100 shilling coin.
        let sale = record_sale(&store, &lines, -20, 0, 4_000, None, None).expect("record");

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
    fn the_drawer_opens_once_and_counts_its_cash() {
        let (store, path) = store("drawer-counts");

        // Nothing to count before the first sale, so nothing is drawn.
        assert!(drawer(&store).is_none());

        let shift = ensure_open_shift(&store, 5_000, None).expect("open");
        assert!(shift.is_open());
        assert_eq!(shift.float_minor, 5_000, "the float it opened with");

        // A retried open hands back the same drawer rather than splitting the
        // day's cash in two.
        let again = ensure_open_shift(&store, 9_999, None).expect("reopen");
        assert_eq!(again.uid, shift.uid);
        assert_eq!(again.float_minor, 5_000);

        let line = |unit_minor: i64| SaleLine {
            product_id: None,
            name: "Bun".to_owned(),
            qty: 1,
            unit_minor,
            unit_factor: 1,
            unit_label: None,
        };
        record_sale(&store, &[line(2_000)], 0, 0, 2_000, None, Some(&shift.uid))
            .expect("first sale");
        record_sale(&store, &[line(2_000)], 0, 0, 2_000, None, Some(&shift.uid))
            .expect("second sale");
        // A sale the drawer could not be opened for still records. It is a gap
        // in a count, not a lost sale, so it must not appear in the totals.
        record_sale(&store, &[line(700)], 0, 0, 700, None, None).expect("unattributed");

        let open = drawer(&store).expect("drawer");
        assert_eq!(open.totals.cash_minor, 4_000);
        assert_eq!(open.totals.sales_count, 2);
        assert_eq!(open.expected_minor(), 9_000, "float plus cash taken");
        assert_eq!(open.difference_minor(), None, "nothing counted yet");

        close_shift(&store, &shift.uid, 8_900, None).expect("close");

        let closed = drawer(&store).expect("a count from today is still today's");
        assert!(!closed.shift.is_open());
        assert_eq!(closed.shift.counted_minor, Some(8_900));
        assert_eq!(closed.difference_minor(), Some(-100), "short by 100");
        assert!(
            close_shift(&store, &shift.uid, 8_900, None).is_err(),
            "a count is made once"
        );

        // The next sale opens a new drawer; the count starts again.
        let next = ensure_open_shift(&store, 5_000, None).expect("reopen after close");
        assert_ne!(next.uid, shift.uid);
        assert!(next.is_open());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn yesterdays_drawer_is_history_not_todays_card() {
        let (store, path) = store("drawer-ages-out");

        let shift = ensure_open_shift(&store, 0, None).expect("open");
        close_shift(&store, &shift.uid, 0, None).expect("close");
        assert!(drawer(&store).is_some(), "closed today, so still the day's");

        store
            .conn()
            .execute(
                "UPDATE shifts SET closed_at = datetime('now', '-2 days') WHERE uid = ?1",
                [&shift.uid],
            )
            .expect("age the count");

        assert!(
            drawer(&store).is_none(),
            "an old count belongs in history, not at the top of Today"
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn tracking_writes_an_opening_movement_and_manual_kinds_keep_their_signs() {
        let (store, path) = store("stock-track");
        let soap = insert_product(&store, "Soap", 1_200, "Household", None, None).expect("product");

        track_product(&store, soap, 12, 3, None, None).expect("track");
        let tracked = stock(&store);
        assert_eq!(tracked.len(), 1);
        assert_eq!(tracked[0].on_hand, 12);
        assert_eq!(tracked[0].low_stock, 3);
        assert!(!tracked[0].is_out());
        assert!(!tracked[0].is_low());

        record_stock_movement(&store, soap, 5, StockKind::Receive, "Delivery", None, None)
            .expect("receive");
        record_stock_movement(&store, soap, 2, StockKind::Return, "", None, None).expect("return");
        record_stock_movement(&store, soap, -4, StockKind::Waste, "Broken", None, None)
            .expect("waste");
        record_stock_movement(&store, soap, -2, StockKind::Adjust, "Counted", None, None)
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
        track_product(&store, soap, 10, 5, None, None).expect("track soap");

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
                    unit_factor: 1,
                    unit_label: None,
                },
                SaleLine {
                    product_id: Some(bun),
                    name: "Bun".to_owned(),
                    qty: 2,
                    unit_minor: 500,
                    unit_factor: 1,
                    unit_label: None,
                },
            ],
            0,
            0,
            4_000,
            Some(&cashier.id),
            None,
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

        track_product(&store, soap, 0, 5, None, None).expect("track");
        let tracked = stock(&store);
        assert_eq!(tracked.len(), 1);
        assert_eq!(tracked[0].on_hand, 0);
        assert!(tracked[0].is_out());
        assert!(stock_movements(&store, soap).is_empty());

        // Tracking is a decision, not a button: starting twice is refused.
        assert!(track_product(&store, soap, 4, 5, None, None).is_err());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_movement_whose_sign_contradicts_its_kind_is_refused() {
        let (store, path) = store("stock-signs");
        let soap = insert_product(&store, "Soap", 1_200, "Household", None, None).expect("product");
        track_product(&store, soap, 5, 2, None, None).expect("track");

        for (delta, kind) in [
            (-1, StockKind::Receive),
            (-1, StockKind::Return),
            (1, StockKind::Waste),
            (1, StockKind::Opening),
            (-1, StockKind::Sale),
            (0, StockKind::Adjust),
        ] {
            assert!(
                record_stock_movement(&store, soap, delta, kind, "", None, None).is_err(),
                "{kind:?} with a delta of {delta} should be refused"
            );
        }
        assert_eq!(stock(&store)[0].on_hand, 5, "nothing was written");

        let untracked = insert_product(&store, "Bun", 500, "Bakery", None, None).expect("bun");
        assert!(
            record_stock_movement(&store, untracked, 1, StockKind::Receive, "", None, None)
                .is_err(),
            "an untracked product has no ledger to append to"
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn stock_may_go_negative_when_the_shelf_and_the_ledger_disagree() {
        let (store, path) = store("stock-negative");
        let soap = insert_product(&store, "Soap", 1_000, "Household", None, None).expect("product");
        track_product(&store, soap, 1, 2, None, None).expect("track");

        record_sale(
            &store,
            &[SaleLine {
                product_id: Some(soap),
                name: "Soap".to_owned(),
                qty: 4,
                unit_minor: 1_000,
                unit_factor: 1,
                unit_label: None,
            }],
            0,
            0,
            4_000,
            None,
            None,
        )
        .expect("the till sells even when the count says it cannot");

        let tracked = stock(&store);
        assert_eq!(tracked[0].on_hand, -3);
        assert!(tracked[0].is_out());

        // Receiving does not hide the debt: the count stays negative until the
        // shelf has caught up with what was sold.
        record_stock_movement(&store, soap, 2, StockKind::Receive, "", None, None)
            .expect("receive");
        assert_eq!(stock(&store)[0].on_hand, -1);
        assert!(stock(&store)[0].is_out());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn history_is_newest_first_and_capped() {
        let (store, path) = store("stock-history");
        let soap = insert_product(&store, "Soap", 1_200, "Household", None, None).expect("product");
        track_product(&store, soap, 0, 5, None, None).expect("track");

        for _ in 0..(MOVEMENT_HISTORY + 5) {
            record_stock_movement(&store, soap, 1, StockKind::Receive, "", None, None)
                .expect("receive");
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
            unit_factor: 1,
            unit_label: None,
        }];

        assert!(
            record_sale(&store, &lines, 0, 0, 2_000, None, None).is_err(),
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

    /// A receipt number says which till, which day and which sale. The row id
    /// alone is the same 42 on two tills that sync, and the slip in the
    /// customer's hand has to be one number.
    #[test]
    fn a_sale_is_numbered_by_its_device_and_its_day() {
        let (store, path) = store("receipt-number");
        let line = || SaleLine {
            product_id: None,
            name: "Milk".to_owned(),
            qty: 1,
            unit_minor: 2_000,
            unit_factor: 1,
            unit_label: None,
        };

        let first = record_sale(&store, &[line()], 0, 0, 2_000, None, None).expect("first");
        let second = record_sale(&store, &[line()], 0, 0, 2_000, None, None).expect("second");

        let device = ensure_device_id(&store).expect("device");
        let short = device_short(&device);
        let day: String = store
            .conn()
            .query_row("SELECT strftime('%Y%m%d', 'now', 'localtime')", [], |row| {
                row.get(0)
            })
            .expect("the clock");

        let numbers: Vec<String> = [first.id, second.id]
            .into_iter()
            .map(|id| {
                sale_detail(&store, id)
                    .expect("detail")
                    .receipt_no
                    .expect("a new sale is numbered")
            })
            .collect();

        assert_eq!(numbers[0], format!("{short}-{day}-001"));
        assert_eq!(numbers[1], format!("{short}-{day}-002"));
        assert_eq!(short.len(), 4, "four characters of the device, no more");
        assert!(
            short
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()),
            "the prefix is safe to read aloud: {short}"
        );

        let _ = std::fs::remove_file(path);
    }

    /// A new morning starts the count at one. The day is stored beside the
    /// count, so there is no reset job that can be forgotten.
    #[test]
    fn the_count_starts_over_on_a_new_day() {
        let (store, path) = store("receipt-new-day");
        set_meta(&store, "receipt_day", "20200101").expect("yesterday");
        set_meta(&store, "receipt_seq", "37").expect("a busy day");

        let sale = record_sale(
            &store,
            &[SaleLine {
                product_id: None,
                name: "Milk".to_owned(),
                qty: 1,
                unit_minor: 2_000,
                unit_factor: 1,
                unit_label: None,
            }],
            0,
            0,
            2_000,
            None,
            None,
        )
        .expect("sale");
        let number = sale_detail(&store, sale.id)
            .expect("detail")
            .receipt_no
            .expect("numbered");

        assert!(number.ends_with("-001"), "{number}");

        let _ = std::fs::remove_file(path);
    }

    /// A sale written before numbers carried a device keeps the row id it was
    /// printed with, and the detail says so rather than inventing a number.
    #[test]
    fn an_old_sale_has_no_device_number() {
        let (store, path) = store("receipt-old");
        store
            .conn()
            .execute("INSERT INTO sales (total_minor) VALUES (1000)", [])
            .expect("plant");
        let id = store.conn().last_insert_rowid();

        let detail = sale_detail(&store, id).expect("detail");
        assert_eq!(detail.receipt_no, None);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_empty_cart_is_not_a_sale() {
        let (store, path) = store("empty-cart");

        assert!(record_sale(&store, &[], 0, 0, 0, None, None).is_err());
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
                unit_factor: 1,
                unit_label: None,
            }],
            0,
            0,
            3_400,
            None,
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
        track_product(&store, soap, 10, 3, None, None).expect("track");

        let line = SaleLine {
            product_id: Some(soap),
            name: "Soap".to_owned(),
            qty: 1,
            unit_minor: 1_000,
            unit_factor: 1,
            unit_label: None,
        };
        let first = record_sale(&store, std::slice::from_ref(&line), 0, 0, 1_000, None, None)
            .expect("first");
        let second = record_sale(&store, &[line], 0, 0, 1_000, None, None).expect("second");

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
                unit_factor: 1,
                unit_label: None,
            }],
            0,
            0,
            500,
            Some(&brian.id),
            None,
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
                    unit_factor: 1,
                    unit_label: None,
                },
                SaleLine {
                    product_id: None,
                    name: "Tea".to_owned(),
                    qty: 1,
                    unit_minor: 500,
                    unit_factor: 1,
                    unit_label: None,
                },
            ],
            // Total is 16 500; UGX coins end in 100, so the till rounds up 100.
            100,
            0,
            16_600,
            Some(&brian.id),
            None,
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
                unit_factor: 1,
                unit_label: None,
            }],
            0,
            0,
            1_000,
            None,
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
                unit_factor: 1,
                unit_label: None,
            }],
            60,
            0,
            1_000,
            None,
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
                unit_factor: 1,
                unit_label: None,
            }],
            0,
            0,
            500,
            None,
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

    /// A till that has never been copied gets a backup on the first check, and
    /// the check creates the folder it needs.
    #[test]
    fn a_till_that_was_never_backed_up_gets_one() {
        let (store, path) = store("backup-first");
        let dir = std::env::temp_dir().join(format!("pos-backup-first-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let written = backup_if_due(&store, &dir, 24)
            .expect("backup")
            .expect("a copy");
        assert!(written.exists(), "the copy is on disk");
        assert!(newest_backup(&dir).expect("newest").is_some());

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(path);
    }

    /// The second check of the same day writes nothing: a backup is a copy of
    /// everything, and one every ten minutes would keep ten copies of the last
    /// hour and none of last week.
    #[test]
    fn a_recent_backup_is_not_taken_twice() {
        let (store, path) = store("backup-twice");
        let dir = std::env::temp_dir().join(format!("pos-backup-twice-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        backup_if_due(&store, &dir, 24).expect("first");
        assert!(
            backup_if_due(&store, &dir, 24).expect("second").is_none(),
            "a copy from a moment ago is recent enough"
        );

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(path);
    }

    /// A copy older than the window is history: the check writes a fresh one.
    /// The planted name comes from the same clock helper the check uses, so the
    /// test does not depend on how long it takes to run.
    #[test]
    fn a_stale_backup_is_replaced() {
        let (store, path) = store("backup-stale");
        let dir = std::env::temp_dir().join(format!("pos-backup-stale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("make directory");

        let old = stamp_before(&store, 48).expect("clock");
        std::fs::write(dir.join(format!("{BACKUP_PREFIX}{old}-0001.db")), b"").expect("plant");
        // A file that is not a backup must not answer the question either way.
        std::fs::write(dir.join("notes.txt"), b"a note").expect("plant");

        assert!(
            backup_if_due(&store, &dir, 24).expect("backup").is_some(),
            "two days is older than a day"
        );

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

    /// A tracked product with an opening count, ready for a purchase test.
    fn stocked(
        store: &storage::Store,
        name: &str,
        opening: i64,
        low: i64,
        target: Option<i64>,
    ) -> i64 {
        let id = insert_product(store, name, 500, "Shop", None, None).expect("insert");
        track_product(store, id, opening, low, target, None).expect("track");
        id
    }

    #[test]
    fn a_plan_counts_nothing_until_it_is_received() {
        let (store, path) = store("purchase-plan");
        let soap = stocked(&store, "Soap", 0, 2, Some(10));

        let id = create_purchase(
            &store,
            None,
            "Saturday market",
            &[PurchaseLineInput {
                product_id: soap,
                qty: 5,
                unit_cost_minor: Some(100),
                pack_size: 1,
                expires_on: None,
            }],
            None,
        )
        .expect("plan");

        let item = stock(&store)
            .into_iter()
            .find(|item| item.product_id == soap)
            .expect("stock");
        assert_eq!(item.on_hand, 0, "a plan is not stock");
        assert_eq!(item.incoming, 5, "but it is on its way");
        assert_eq!(item.suggested(), 5, "target 10, nothing here, five coming");
        assert!(
            !item.needs_restock(),
            "the plan already covers the warning line"
        );

        let list = purchases(&store);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].status, PurchaseStatus::Planned);
        assert_eq!(list[0].unit_count, 5);
        assert_eq!(list[0].cost_minor, Some(500));
        assert_eq!(list[0].count_label(), "5 items");
        assert_eq!(
            purchase_detail(&store, id).expect("detail").lines[0].name,
            "Soap"
        );

        // Three arrive, and the trip is over: there is no half-received state
        // to come back to.
        receive_purchase(
            &store,
            id,
            None,
            &[ReceiveLine {
                line_id: Some(purchase_detail(&store, id).expect("detail").lines[0].id),
                product_id: soap,
                qty: 3,
                unit_cost_minor: Some(120),
                pack_size: 1,
                expires_on: None,
            }],
            &[],
            "",
            None,
        )
        .expect("receive the trip");

        let item = stock(&store)
            .into_iter()
            .find(|item| item.product_id == soap)
            .expect("stock");
        assert_eq!(item.on_hand, 3);
        assert_eq!(item.incoming, 0, "the list is closed, nothing is coming");
        assert_eq!(
            item.cost_milli,
            Some(120_000),
            "last cost came from receiving"
        );

        let list = purchases(&store);
        assert_eq!(list[0].status, PurchaseStatus::Received);
        assert!(list[0].received_at.is_some());
        assert_eq!(
            list[0].count_label(),
            "3 of 5 bought",
            "coming home short is worth saying"
        );
        assert_eq!(
            stock_movements(&store, soap).len(),
            1,
            "one movement for one trip"
        );

        let line = purchase_detail(&store, id).expect("detail").lines[0].id;
        assert!(
            receive_purchase(
                &store,
                id,
                None,
                &[ReceiveLine {
                    line_id: Some(line),
                    product_id: soap,
                    qty: 2,
                    unit_cost_minor: None,
                    pack_size: 1,
                    expires_on: None,
                }],
                &[],
                "",
                None,
            )
            .is_err(),
            "a closed trip cannot be received twice"
        );
        assert_eq!(stock(&store)[0].on_hand, 3, "and the shelf did not move");

        let _ = std::fs::remove_file(path);
    }

    /// The sheet is the truth about the trip. Buying more than the list said
    /// means the list understated it, so the plan rises to match instead of the
    /// goods being refused at the counter.
    #[test]
    fn buying_more_than_planned_raises_the_plan() {
        let (store, path) = store("purchase-overbuy");
        let soap = stocked(&store, "Soap", 0, 2, None);
        let id = create_purchase(
            &store,
            None,
            "",
            &[PurchaseLineInput {
                product_id: soap,
                qty: 2,
                unit_cost_minor: Some(100),
                pack_size: 1,
                expires_on: None,
            }],
            None,
        )
        .expect("plan");
        let line = purchase_detail(&store, id).expect("detail").lines[0].id;

        receive_purchase(
            &store,
            id,
            None,
            &[ReceiveLine {
                line_id: Some(line),
                product_id: soap,
                qty: 7,
                unit_cost_minor: None,
                pack_size: 1,
                expires_on: None,
            }],
            &[],
            "",
            None,
        )
        .expect("the market had more than the list said");

        let list = purchases(&store);
        assert_eq!(list[0].count_label(), "7 items", "not a shortfall");
        assert_eq!(list[0].cost_minor, Some(700), "seven at one hundred");
        assert_eq!(stock(&store)[0].on_hand, 7);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_received_trip_cannot_be_changed_or_deleted() {
        let (store, path) = store("purchase-partial");
        let soap = stocked(&store, "Soap", 0, 2, None);
        let id = create_purchase(
            &store,
            None,
            "",
            &[PurchaseLineInput {
                product_id: soap,
                qty: 4,
                unit_cost_minor: None,
                pack_size: 1,
                expires_on: None,
            }],
            None,
        )
        .expect("plan");
        assert!(
            update_purchase(
                &store,
                id,
                None,
                "",
                &[PurchaseLineInput {
                    product_id: soap,
                    qty: 9,
                    unit_cost_minor: None,
                    pack_size: 1,
                    expires_on: None,
                }]
            )
            .is_ok(),
            "an untouched list is editable"
        );
        assert!(delete_planned_purchase(&store, id).is_ok(), "and deletable");

        let id = create_purchase(
            &store,
            None,
            "",
            &[PurchaseLineInput {
                product_id: soap,
                qty: 4,
                unit_cost_minor: None,
                pack_size: 1,
                expires_on: None,
            }],
            None,
        )
        .expect("plan again");
        let line = purchase_detail(&store, id).expect("detail").lines[0].id;
        receive_purchase(
            &store,
            id,
            None,
            &[ReceiveLine {
                line_id: Some(line),
                product_id: soap,
                qty: 1,
                unit_cost_minor: None,
                pack_size: 1,
                expires_on: None,
            }],
            &[],
            "",
            None,
        )
        .expect("receive the trip");

        assert!(
            update_purchase(
                &store,
                id,
                None,
                "",
                &[PurchaseLineInput {
                    product_id: soap,
                    qty: 4,
                    unit_cost_minor: None,
                    pack_size: 1,
                    expires_on: None,
                }]
            )
            .is_err(),
            "a line a movement points at cannot be replaced"
        );
        assert!(
            delete_planned_purchase(&store, id).is_err(),
            "nor can the trip vanish"
        );
        assert_eq!(purchases(&store).len(), 1, "the record stays");

        let _ = std::fs::remove_file(path);
    }

    /// Migration 12 shipped a `planned` purchase that could already have stock
    /// counted against it. Such a row cannot be finished — receiving it again
    /// would put the same goods on the shelf twice — so migration 15 closes it
    /// as what it always was: a trip that came home short.
    #[test]
    fn a_half_received_trip_from_an_older_build_is_closed() {
        // Build the file the way the build before this one left it.
        let path =
            std::env::temp_dir().join(format!("pos-purchase-legacy-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut store = storage::Store::open(&path).expect("open database");
        let older: Vec<Migration> = MIGRATIONS
            .iter()
            .filter(|migration| migration.version <= 14)
            .map(|migration| Migration {
                version: migration.version,
                name: migration.name,
                sql: migration.sql,
            })
            .collect();
        store.migrate(&older).expect("migrate to 14");

        // The old schema has no pack columns, so seed the row the old way.
        store
            .conn()
            .execute(
                "INSERT INTO products (name, price_minor, category) VALUES ('Soap', 500, 'Shop')",
                [],
            )
            .expect("legacy product");
        let soap = store.conn().last_insert_rowid();
        track_product(&store, soap, 0, 2, None, None).expect("track");
        store
            .conn()
            .execute(
                "INSERT INTO purchases (uid, status, created_at)
                 VALUES (?1, 'planned', '2024-01-01 09:00:00')",
                [uuid::Uuid::now_v7().to_string()],
            )
            .expect("legacy purchase");
        let id = store.conn().last_insert_rowid();
        store
            .conn()
            .execute(
                "INSERT INTO purchase_lines
                     (uid, purchase_id, product_id, name_snapshot, qty_ordered,
                      qty_received, position)
                 VALUES (?1, ?2, ?3, 'Soap', 5, 3, 0)",
                (uuid::Uuid::now_v7().to_string(), id, soap),
            )
            .expect("legacy line");
        let line = store.conn().last_insert_rowid();
        store
            .conn()
            .execute(
                "INSERT INTO stock_movements
                     (product_id, delta, kind, created_at, purchase_line_id, uid)
                 VALUES (?1, 3, 'receive', '2024-01-02 10:30:00', ?2, ?3)",
                (soap, line, uuid::Uuid::now_v7().to_string()),
            )
            .expect("legacy movement");

        store.migrate(MIGRATIONS).expect("migrate to 15");

        let trip = purchases(&store).into_iter().next().expect("the trip");
        assert_eq!(trip.status, PurchaseStatus::Received, "it was finished");
        assert_eq!(
            trip.count_label(),
            "3 of 5 bought",
            "and it came home short"
        );
        let received_at: Option<String> = store
            .conn()
            .query_row(
                "SELECT received_at FROM purchases WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .expect("received_at");
        assert_eq!(
            received_at.as_deref(),
            Some("2024-01-02 10:30:00"),
            "dated when the goods actually arrived, not when the update ran"
        );
        assert_eq!(stock(&store)[0].on_hand, 3, "the shelf already knew");

        assert!(
            receive_purchase(
                &store,
                id,
                None,
                &[ReceiveLine {
                    line_id: Some(line),
                    product_id: soap,
                    qty: 3,
                    unit_cost_minor: None,
                    pack_size: 1,
                    expires_on: None,
                }],
                &[],
                "",
                None,
            )
            .is_err(),
            "and it cannot be received a second time"
        );
        assert_eq!(stock(&store)[0].on_hand, 3, "so the shelf did not move");

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn receiving_can_add_an_impulse_buy_and_remove_an_unbought_line() {
        let (store, path) = store("purchase-impulse");
        let soap = stocked(&store, "Soap", 0, 2, None);
        let rice = stocked(&store, "Rice", 0, 2, None);
        let salt = stocked(&store, "Salt", 0, 2, None);
        let id = create_purchase(
            &store,
            None,
            "",
            &[
                PurchaseLineInput {
                    product_id: soap,
                    qty: 2,
                    unit_cost_minor: None,
                    pack_size: 1,
                    expires_on: None,
                },
                PurchaseLineInput {
                    product_id: rice,
                    qty: 5,
                    unit_cost_minor: None,
                    pack_size: 1,
                    expires_on: None,
                },
            ],
            None,
        )
        .expect("plan");
        let lines = purchase_detail(&store, id).expect("detail").lines;
        let soap_line = lines
            .iter()
            .find(|line| line.product_id == Some(soap))
            .expect("soap")
            .id;
        let rice_line = lines
            .iter()
            .find(|line| line.product_id == Some(rice))
            .expect("rice")
            .id;

        receive_purchase(
            &store,
            id,
            None,
            &[
                ReceiveLine {
                    line_id: Some(soap_line),
                    product_id: soap,
                    qty: 2,
                    unit_cost_minor: Some(50),
                    pack_size: 1,
                    expires_on: None,
                },
                ReceiveLine {
                    line_id: None,
                    product_id: salt,
                    qty: 3,
                    unit_cost_minor: Some(80),
                    pack_size: 1,
                    expires_on: None,
                },
            ],
            &[rice_line],
            "Rice was on offer",
            None,
        )
        .expect("receive");

        let detail = purchase_detail(&store, id).expect("detail");
        assert_eq!(detail.lines.len(), 2, "the unbought rice line is gone");
        let soap_line = detail
            .lines
            .iter()
            .find(|line| line.product_id == Some(soap))
            .expect("soap");
        assert_eq!((soap_line.qty_ordered, soap_line.qty_received), (2, 2));
        let impulse = detail
            .lines
            .iter()
            .find(|line| line.product_id == Some(salt))
            .expect("salt");
        assert_eq!(
            (impulse.qty_ordered, impulse.qty_received),
            (3, 3),
            "an impulse buy is fully received as its own line"
        );
        assert_eq!(detail.purchase.status, PurchaseStatus::Received);
        assert_eq!(detail.purchase.note.as_deref(), Some("Rice was on offer"));

        let second = create_purchase(
            &store,
            None,
            "",
            &[PurchaseLineInput {
                product_id: soap,
                qty: 2,
                unit_cost_minor: None,
                pack_size: 1,
                expires_on: None,
            }],
            None,
        )
        .expect("second plan");
        let line = purchase_detail(&store, second).expect("detail").lines[0].id;
        receive_purchase(&store, second, None, &[], &[line], "", None).expect("remove unbought");

        assert!(
            purchase_detail(&store, second).is_none(),
            "a plan with every line marked Not bought is cleared, not left empty"
        );
        assert!(
            purchases(&store).iter().all(|p| p.id != second),
            "nothing to buy means no purchase to show"
        );

        let _ = std::fs::remove_file(path);
    }

    /// The counter is where the operator learns the supplier was wrong. A
    /// correction made while counting in must survive the receive, not be
    /// silently dropped because the plan was written days earlier.
    #[test]
    fn receiving_keeps_the_supplier_chosen_at_the_counter() {
        let (store, path) = store("receive-supplier");
        let soap = stocked(&store, "Soap", 0, 2, None);
        let supplier = insert_supplier(
            &store,
            &SupplierFields {
                name: "Kampala Wholesale",
                contact_name: "",
                phone: "",
                email: "",
                address: "",
                notes: "",
            },
        )
        .expect("supplier");

        let id = create_purchase(
            &store,
            None,
            "",
            &[PurchaseLineInput {
                product_id: soap,
                qty: 3,
                unit_cost_minor: None,
                pack_size: 1,
                expires_on: None,
            }],
            None,
        )
        .expect("plan");
        assert!(
            purchase_detail(&store, id)
                .expect("detail")
                .purchase
                .supplier_id
                .is_none(),
            "the plan named no supplier"
        );

        let line = purchase_detail(&store, id).expect("detail").lines[0].id;
        receive_purchase(
            &store,
            id,
            Some(&supplier.id),
            &[ReceiveLine {
                line_id: Some(line),
                product_id: soap,
                qty: 3,
                unit_cost_minor: None,
                pack_size: 1,
                expires_on: None,
            }],
            &[],
            "",
            None,
        )
        .expect("receive");

        let detail = purchase_detail(&store, id).expect("detail");
        assert_eq!(
            detail.purchase.supplier_id.as_deref(),
            Some(supplier.id.as_str()),
            "the supplier typed at receiving is the one recorded"
        );
        assert_eq!(
            detail.purchase.supplier.as_deref(),
            Some("Kampala Wholesale")
        );

        // A supplier id that is not in the directory is refused, and refusing
        // must not close the trip.
        let second = create_purchase(
            &store,
            None,
            "",
            &[PurchaseLineInput {
                product_id: soap,
                qty: 1,
                unit_cost_minor: None,
                pack_size: 1,
                expires_on: None,
            }],
            None,
        )
        .expect("second plan");
        let line = purchase_detail(&store, second).expect("detail").lines[0].id;
        let error = receive_purchase(
            &store,
            second,
            Some("no-such-supplier"),
            &[ReceiveLine {
                line_id: Some(line),
                product_id: soap,
                qty: 1,
                unit_cost_minor: None,
                pack_size: 1,
                expires_on: None,
            }],
            &[],
            "",
            None,
        )
        .expect_err("an unknown supplier is refused");
        assert!(
            error.to_string().contains("no longer in the directory"),
            "the operator is told why: {error}"
        );
        assert_eq!(
            purchase_detail(&store, second)
                .expect("still there")
                .purchase
                .status,
            PurchaseStatus::Planned,
            "a refused receive leaves the trip open"
        );

        let _ = std::fs::remove_file(path);
    }

    /// The movement is written for `product_id`, the line it points at is the
    /// one on the shelf. If the two disagree, the shelf would move for one
    /// product while the purchase recorded another, so the write is refused.
    #[test]
    fn a_line_cannot_be_received_as_a_different_product() {
        let (store, path) = store("receive-mismatch");
        let soap = stocked(&store, "Soap", 0, 1, None);
        let tea = stocked(&store, "Tea", 0, 1, None);

        let id = create_purchase(
            &store,
            None,
            "",
            &[
                PurchaseLineInput {
                    product_id: soap,
                    qty: 2,
                    unit_cost_minor: None,
                    pack_size: 1,
                    expires_on: None,
                },
                PurchaseLineInput {
                    product_id: tea,
                    qty: 1,
                    unit_cost_minor: None,
                    pack_size: 1,
                    expires_on: None,
                },
            ],
            None,
        )
        .expect("plan");
        let soap_line = purchase_detail(&store, id)
            .expect("detail")
            .lines
            .iter()
            .find(|line| line.product_id == Some(soap))
            .expect("soap line")
            .id;

        let error = receive_purchase(
            &store,
            id,
            None,
            &[ReceiveLine {
                line_id: Some(soap_line),
                product_id: tea,
                qty: 2,
                unit_cost_minor: None,
                pack_size: 1,
                expires_on: None,
            }],
            &[],
            "",
            None,
        )
        .expect_err("a line is received as the product it was planned for");
        assert!(
            error.to_string().contains("another product"),
            "the operator is told why: {error}"
        );

        assert!(
            stock_movements(&store, soap).is_empty(),
            "soap did not move"
        );
        assert!(stock_movements(&store, tea).is_empty(), "tea did not move");
        assert_eq!(
            purchase_detail(&store, id).expect("detail").purchase.status,
            PurchaseStatus::Planned,
            "the trip is still open"
        );

        let _ = std::fs::remove_file(path);
    }

    /// A list made at the market is a count sheet and a purchase at once. The
    /// trip is written and closed in one transaction, so the shelf and the
    /// record cannot disagree about whether the shop went.
    #[test]
    fn a_trip_created_at_the_counter_arrives_in_one_write() {
        let (store, path) = store("receive-created");
        let soap = stocked(&store, "Soap", 0, 1, None);
        let tea = stocked(&store, "Tea", 0, 1, None);

        let id = create_and_receive(
            &store,
            None,
            "Market run",
            &[
                PurchaseLineInput {
                    product_id: soap,
                    qty: 2,
                    unit_cost_minor: Some(250),
                    pack_size: 1,
                    expires_on: None,
                },
                PurchaseLineInput {
                    product_id: tea,
                    qty: 1,
                    unit_cost_minor: Some(900),
                    pack_size: 3,
                    expires_on: None,
                },
            ],
            None,
        )
        .expect("created and received");

        let detail = purchase_detail(&store, id).expect("detail");
        assert_eq!(detail.purchase.status, PurchaseStatus::Received);
        assert_eq!(detail.purchase.note.as_deref(), Some("Market run"));
        for line in &detail.lines {
            assert_eq!(
                line.qty_ordered, line.qty_received,
                "what was bought is what arrived"
            );
        }

        let shelf = |product_id: i64| {
            stock(&store)
                .into_iter()
                .find(|item| item.product_id == product_id)
                .expect("stock")
        };
        assert_eq!(shelf(soap).on_hand, 2);
        assert_eq!(shelf(soap).incoming, 0, "a received trip is not on its way");
        assert_eq!(shelf(tea).on_hand, 3, "one box of three moves three units");

        let tea_line = detail
            .lines
            .iter()
            .find(|line| line.product_id == Some(tea))
            .expect("tea line");
        let linked: i64 = store
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM stock_movements WHERE purchase_line_id = ?1",
                [tea_line.id],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(linked, 1, "the movement points back at the trip");

        // Cost is kept per base unit in thousandths: 900 for a box of three.
        let product = products(&store)
            .into_iter()
            .find(|product| product.id == tea)
            .expect("tea");
        assert_eq!(product.cost_milli, Some(300_000));

        let _ = std::fs::remove_file(path);
    }

    /// The point of one write: if any line cannot be received, nothing is
    /// written. A saved plan left behind would be a trip the shop never made.
    #[test]
    fn a_refused_trip_leaves_no_plan_behind() {
        let (store, path) = store("receive-created-refused");
        let soap = stocked(&store, "Soap", 0, 1, None);
        let retired = stocked(&store, "Retired", 0, 1, None);
        delete_product(&store, retired).expect("archive");

        let error = create_and_receive(
            &store,
            None,
            "",
            &[
                PurchaseLineInput {
                    product_id: soap,
                    qty: 2,
                    unit_cost_minor: None,
                    pack_size: 1,
                    expires_on: None,
                },
                PurchaseLineInput {
                    product_id: retired,
                    qty: 1,
                    unit_cost_minor: None,
                    pack_size: 1,
                    expires_on: None,
                },
            ],
            None,
        )
        .expect_err("an archived product cannot be received");
        assert!(
            error.to_string().contains("archived"),
            "the operator is told why: {error}"
        );

        assert!(purchases(&store).is_empty(), "no plan is left behind");
        assert!(stock_movements(&store, soap).is_empty(), "nothing moved");
        assert_eq!(
            stock(&store)
                .into_iter()
                .find(|item| item.product_id == soap)
                .expect("stock")
                .on_hand,
            0,
            "and the shelf did not change"
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_finished_purchase_keeps_its_names_after_the_catalog_changes() {
        let (store, path) = store("purchase-snapshot");
        let soap = stocked(&store, "Soap", 0, 2, None);
        let id = create_purchase(
            &store,
            None,
            "",
            &[PurchaseLineInput {
                product_id: soap,
                qty: 2,
                unit_cost_minor: Some(70),
                pack_size: 1,
                expires_on: None,
            }],
            None,
        )
        .expect("plan");
        let line = purchase_detail(&store, id).expect("detail").lines[0].id;
        receive_purchase(
            &store,
            id,
            None,
            &[ReceiveLine {
                line_id: Some(line),
                product_id: soap,
                qty: 2,
                unit_cost_minor: None,
                pack_size: 1,
                expires_on: None,
            }],
            &[],
            "",
            None,
        )
        .expect("receive");

        store
            .conn()
            .execute(
                "UPDATE products SET name = 'Soap (large)', archived = 1 WHERE id = ?1",
                [soap],
            )
            .expect("archive");

        let detail = purchase_detail(&store, id).expect("detail");
        assert_eq!(
            detail.lines[0].name, "Soap",
            "the line remembers what was bought, not what the catalog says now"
        );
        assert!(
            update_purchase(
                &store,
                id,
                None,
                "",
                &[PurchaseLineInput {
                    product_id: soap,
                    qty: 1,
                    unit_cost_minor: None,
                    pack_size: 1,
                    expires_on: None,
                }]
            )
            .is_err(),
            "a received trip is locked"
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn only_tracked_products_can_be_restocked() {
        let (store, path) = store("purchase-tracked");
        let loose =
            insert_product(&store, "Loose sweets", 100, "Shop", None, None).expect("insert");

        assert!(
            create_purchase(
                &store,
                None,
                "",
                &[PurchaseLineInput {
                    product_id: loose,
                    qty: 5,
                    unit_cost_minor: None,
                    pack_size: 1,
                    expires_on: None,
                }],
                None
            )
            .is_err(),
            "an untracked product has no shelf to count"
        );

        let soap = stocked(&store, "Soap", 0, 2, None);
        let id = create_purchase(
            &store,
            None,
            "",
            &[PurchaseLineInput {
                product_id: soap,
                qty: 5,
                unit_cost_minor: None,
                pack_size: 1,
                expires_on: None,
            }],
            None,
        )
        .expect("plan");
        let line = purchase_detail(&store, id).expect("detail").lines[0].id;
        store
            .conn()
            .execute("UPDATE products SET tracked = 0 WHERE id = ?1", [soap])
            .expect("untrack");
        assert!(
            receive_purchase(
                &store,
                id,
                None,
                &[ReceiveLine {
                    line_id: Some(line),
                    product_id: soap,
                    qty: 1,
                    unit_cost_minor: None,
                    pack_size: 1,
                    expires_on: None,
                }],
                &[],
                "",
                None
            )
            .is_err(),
            "receiving into an untracked product would write a movement nothing reads"
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn profit_only_counts_lines_with_a_cost() {
        let (store, path) = store("margin");
        let id = insert_sale(&store, "-0 days", &[("Tea", 2, 300), ("Sugar", 1, 200)]);
        store
            .conn()
            .execute(
                "UPDATE sale_lines SET cost_milli = 100000 WHERE sale_id = ?1 AND name = 'Tea'",
                [id],
            )
            .expect("cost one line");

        let reported = margin(&store, 0, None);
        assert_eq!(
            reported.revenue_minor, 600,
            "only the costed line's revenue"
        );
        assert_eq!(reported.cost_minor, 200);
        assert_eq!(reported.profit_minor(), 400);
        assert_eq!((reported.costed_lines, reported.total_lines), (1, 2));
        assert!(!reported.is_complete(), "the sugar line had no cost");
        assert!(reported.has_cost());

        assert_eq!(margin(&store, 3, None).profit_minor(), 400);

        let _ = std::fs::remove_file(path);
    }

    /// A pack is a conversion on one catalog row, not a second product: both
    /// codes point at the row, and the second code is never free for another
    /// product to take.
    #[test]
    fn a_pack_lives_on_the_same_row_as_its_loose_units() {
        let (store, path) = store("pack-fields");
        let id = super::insert_product(
            &store,
            &ProductFields {
                name: "Amoxicillin",
                price_minor: 30,
                category: "Medicine",
                category_id: None,
                barcode: Some("100100"),
                unit_label: "tablet",
                pack_size: 100,
                pack_price_minor: Some(2_800),
                pack_barcode: Some("100200"),
            },
        )
        .expect("insert");

        let list = products(&store);
        assert_eq!(list.len(), 1, "a pack is not a second product");
        let product = &list[0];
        assert_eq!(product.id, id);
        assert_eq!(product.price_minor, 30, "the tablet price");
        assert_eq!(product.unit_label, "tablet");
        assert_eq!(product.pack_size, 100);
        assert_eq!(product.pack_price_minor, Some(2_800));
        assert_eq!(product.pack_barcode.as_deref(), Some("100200"));
        assert!(barcode_taken(&store, "100100", None), "the loose code");
        assert!(barcode_taken(&store, "100200", None), "the box code");

        // The box code cannot be reused as a loose code on another product.
        let error = super::insert_product(
            &store,
            &ProductFields {
                name: "Other",
                price_minor: 10,
                category: "Medicine",
                category_id: None,
                barcode: Some("100200"),
                unit_label: "unit",
                pack_size: 1,
                pack_price_minor: None,
                pack_barcode: None,
            },
        )
        .expect_err("a code belongs to one product");
        assert!(error.to_string().contains("barcode"), "{error}");

        let _ = std::fs::remove_file(path);
    }

    /// A pack of one is just the base unit. Keeping its second price and code
    /// would offer two names for the same thing.
    #[test]
    fn a_pack_of_one_keeps_no_pack_price_or_code() {
        let (store, path) = store("pack-of-one");
        super::insert_product(
            &store,
            &ProductFields {
                name: "Single",
                price_minor: 500,
                category: "Shop",
                category_id: None,
                barcode: None,
                unit_label: "box",
                pack_size: 1,
                pack_price_minor: Some(9_000),
                pack_barcode: Some("999"),
            },
        )
        .expect("insert");

        let product = products(&store).into_iter().next().expect("product");
        assert_eq!(product.pack_size, 1);
        assert_eq!(product.pack_price_minor, None);
        assert_eq!(product.pack_barcode, None);
        assert!(!barcode_taken(&store, "999", None), "the code was dropped");

        let _ = std::fs::remove_file(path);
    }

    /// Buying a pack stores the cost per tablet, not the cost of the box, so a
    /// later loose sale can be costed at the exact half-unit it cost.
    #[test]
    fn receiving_a_pack_stores_the_cost_per_tablet() {
        let (store, path) = store("pack-receive");
        let id = super::insert_product(
            &store,
            &ProductFields {
                name: "Amoxicillin",
                price_minor: 30,
                category: "Medicine",
                category_id: None,
                barcode: None,
                unit_label: "tablet",
                pack_size: 100,
                pack_price_minor: Some(2_800),
                pack_barcode: None,
            },
        )
        .expect("insert");
        track_product(&store, id, 0, 0, None, None).expect("track");

        let purchase = create_purchase(
            &store,
            None,
            "",
            &[PurchaseLineInput {
                product_id: id,
                qty: 2,
                unit_cost_minor: Some(1_850),
                pack_size: 100,
                expires_on: None,
            }],
            None,
        )
        .expect("plan");
        let line = purchase_detail(&store, purchase).expect("detail").lines[0].id;
        receive_purchase(
            &store,
            purchase,
            None,
            &[ReceiveLine {
                line_id: Some(line),
                product_id: id,
                qty: 2,
                unit_cost_minor: Some(1_850),
                pack_size: 100,
                expires_on: None,
            }],
            &[],
            "",
            None,
        )
        .expect("receive");

        let item = stock(&store).into_iter().next().expect("stock");
        assert_eq!(item.on_hand, 200, "two packs of 100");
        assert_eq!(item.cost_milli, Some(18_500), "1,850 / 100, exact");
        assert_eq!(item.unit_cost_minor(), Some(19), "rounded for a loose line");
        assert_eq!(item.pack_cost_minor(), Some(1_850), "back to the invoice");

        let _ = std::fs::remove_file(path);
    }

    /// The pack leaves the shelf as its size, and the profit uses the same
    /// factor so a box sale is not costed as one tablet.
    #[test]
    fn a_pack_sale_moves_a_pack_of_stock_and_costs_a_pack() {
        let (store, path) = store("pack-sale");
        let id = super::insert_product(
            &store,
            &ProductFields {
                name: "Amoxicillin",
                price_minor: 30,
                category: "Medicine",
                category_id: None,
                barcode: None,
                unit_label: "tablet",
                pack_size: 100,
                pack_price_minor: Some(2_800),
                pack_barcode: None,
            },
        )
        .expect("insert");
        track_product(&store, id, 200, 0, None, None).expect("track");
        store
            .conn()
            .execute("UPDATE products SET cost_milli = 18500 WHERE id = ?1", [id])
            .expect("cost");

        record_sale(
            &store,
            &[SaleLine {
                product_id: Some(id),
                name: "Amoxicillin".to_owned(),
                qty: 1,
                unit_minor: 2_800,
                unit_factor: 100,
                unit_label: Some("pack of 100".to_owned()),
            }],
            0,
            0,
            2_800,
            None,
            None,
        )
        .expect("sell a pack");

        let item = stock(&store).into_iter().next().expect("stock");
        assert_eq!(item.on_hand, 100, "one pack left the shelf");

        let reported = margin(&store, 0, None);
        assert_eq!(reported.revenue_minor, 2_800);
        assert_eq!(reported.cost_minor, 1_850, "100 tablets, not one");
        assert_eq!(reported.profit_minor(), 950);
        assert!(reported.is_complete());

        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn a_printed_date_is_read_in_the_orders_a_pack_uses() {
        assert_eq!(
            parse_expiry_date("2026-12-31").as_deref(),
            Some("2026-12-31")
        );
        assert_eq!(
            parse_expiry_date("31/12/2026").as_deref(),
            Some("2026-12-31")
        );
        assert_eq!(
            parse_expiry_date("  1/2/2027 ").as_deref(),
            Some("2027-02-01")
        );
        assert_eq!(
            parse_expiry_date("29/02/2028").as_deref(),
            Some("2028-02-29"),
            "2028 is a leap year"
        );
        assert_eq!(parse_expiry_date("29/02/2026"), None, "2026 is not");
        assert_eq!(
            parse_expiry_date("2026-02-30"),
            None,
            "a date SQLite would quietly move to March is a typo"
        );
        assert_eq!(
            parse_expiry_date("2026-13-01"),
            None,
            "there is no month 13"
        );
        assert_eq!(parse_expiry_date("31/12"), None, "a date needs a year");
        assert_eq!(parse_expiry_date(""), None, "blank is no date");
    }

    #[test]
    fn sales_take_the_earliest_date_first_and_undated_stock_last() {
        let movements = vec![
            (StockKind::Receive, Some("2026-03-01".to_owned()), 5),
            (StockKind::Receive, Some("2026-01-01".to_owned()), 4),
            (StockKind::Receive, None, 6),
            (StockKind::Sale, None, -6),
        ];
        assert_eq!(
            derive_buckets(&movements),
            vec![
                // January is gone, March lost the other two, undated untouched.
                (Some("2026-03-01".to_owned()), 3),
                (None, 6),
            ]
        );

        // A tagged waste movement comes off its own date: pulling expired stock
        // must not eat a fresher delivery.
        let movements = vec![
            (StockKind::Receive, Some("2026-01-01".to_owned()), 5),
            (StockKind::Receive, Some("2026-06-01".to_owned()), 5),
            (StockKind::Waste, Some("2026-06-01".to_owned()), -2),
        ];
        assert_eq!(
            derive_buckets(&movements),
            vec![
                (Some("2026-01-01".to_owned()), 5),
                (Some("2026-06-01".to_owned()), 3),
            ]
        );
    }

    #[test]
    fn a_shortfall_is_reported_against_undated_stock_not_invented_for_a_date() {
        let movements = vec![
            (StockKind::Receive, Some("2026-01-01".to_owned()), 2),
            (StockKind::Sale, None, -5),
        ];
        let buckets = derive_buckets(&movements);
        assert_eq!(buckets, vec![(None, -3)], "the shop is three short");

        let total: i64 = buckets.iter().map(|(_, on_hand)| *on_hand).sum();
        assert_eq!(total, -3, "buckets still add up to the ledger");
    }

    #[test]
    fn a_dated_waste_cannot_make_a_date_hold_less_than_nothing() {
        let movements = vec![
            (StockKind::Receive, Some("2026-06-01".to_owned()), 3),
            (StockKind::Waste, Some("2026-06-01".to_owned()), -5),
        ];
        let buckets = derive_buckets(&movements);
        assert_eq!(
            buckets,
            vec![(None, -2)],
            "the two that date never held came off the rest of the shelf"
        );
        let total: i64 = buckets.iter().map(|(_, on_hand)| *on_hand).sum();
        assert_eq!(total, -2, "buckets still add up to the ledger");
    }

    #[test]
    fn only_a_tracked_product_can_be_perishable_and_a_delivery_needs_its_date() {
        let (store, path) = store("expiry-receive");
        let milk = insert_product(&store, "Milk", 2_000, "Dairy", None, None).expect("product");

        assert!(
            set_perishability(&store, milk, true, Some(7)).is_err(),
            "an untracked product has no stock for a date to sit on"
        );

        track_product(&store, milk, 0, 0, None, None).expect("track");
        set_perishability(&store, milk, true, Some(7)).expect("perishable");
        assert!(stock(&store)[0].perishable);
        assert_eq!(stock(&store)[0].shelf_life_days, Some(7));

        assert!(
            record_stock_movement(&store, milk, 6, StockKind::Receive, "Delivery", None, None)
                .is_err(),
            "a perishable delivery has to carry the printed date"
        );
        assert!(
            record_stock_movement(
                &store,
                milk,
                6,
                StockKind::Receive,
                "Delivery",
                Some("next Tuesday"),
                None,
            )
            .is_err(),
            "a date that is not a date is refused"
        );

        record_stock_movement(
            &store,
            milk,
            6,
            StockKind::Receive,
            "Delivery",
            Some("31/07/2026"),
            None,
        )
        .expect("dated delivery");
        assert_eq!(
            stock_movements(&store, milk)[0].expires_on.as_deref(),
            Some("2026-07-31"),
            "stored the way dates are compared"
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn expiring_stock_is_what_is_left_on_each_date_and_pulling_it_leaves_the_rest() {
        let (store, path) = store("expiry-pull");
        let milk = insert_product(&store, "Milk", 2_000, "Dairy", None, None).expect("product");
        track_product(&store, milk, 0, 0, None, None).expect("track");
        set_perishability(&store, milk, true, None).expect("perishable");

        let delivery = |date: &str, qty: i64| PurchaseLineInput {
            product_id: milk,
            qty,
            unit_cost_minor: Some(1_000),
            pack_size: 1,
            expires_on: Some(date.to_owned()),
        };
        create_and_receive(&store, None, "", &[delivery("2020-01-01", 2)], None)
            .expect("past delivery");
        create_and_receive(&store, None, "", &[delivery("2099-01-01", 3)], None)
            .expect("later delivery");

        let buckets = expiring(&store);
        assert_eq!(buckets.len(), 2);
        assert_eq!(buckets[0].expires_on.as_deref(), Some("2020-01-01"));
        assert!(buckets[0].is_expired(), "the date has been and gone");
        assert_eq!(buckets[0].on_hand, 2);
        assert!(!buckets[1].is_expired());

        assert_eq!(
            pull_expired(&store, milk, "2020-01-01", None).expect("pull"),
            2
        );
        assert_eq!(stock(&store)[0].on_hand, 3, "the later delivery stays");
        assert!(
            pull_expired(&store, milk, "2020-01-01", None).is_err(),
            "that date has nothing left"
        );

        let left = expiring(&store);
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].expires_on.as_deref(), Some("2099-01-01"));

        let waste = &stock_movements(&store, milk)[0];
        assert_eq!(waste.kind, StockKind::Waste);
        assert_eq!(waste.delta, -2);
        assert_eq!(waste.expires_on.as_deref(), Some("2020-01-01"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn one_product_can_arrive_on_two_dates_but_not_the_same_date_twice() {
        let (store, path) = store("expiry-lines");
        let milk = insert_product(&store, "Milk", 2_000, "Dairy", None, None).expect("product");
        track_product(&store, milk, 0, 0, None, None).expect("track");
        set_perishability(&store, milk, true, None).expect("perishable");

        let line = |date: &str| PurchaseLineInput {
            product_id: milk,
            qty: 1,
            unit_cost_minor: None,
            pack_size: 1,
            expires_on: Some(date.to_owned()),
        };
        create_and_receive(
            &store,
            None,
            "",
            &[line("2026-06-01"), line("2026-07-01")],
            None,
        )
        .expect("two printed dates on one trip");
        assert_eq!(stock(&store)[0].on_hand, 2);
        assert_eq!(expiring(&store).len(), 2);

        assert!(
            create_and_receive(
                &store,
                None,
                "",
                &[line("2026-08-01"), line("2026-08-01")],
                None
            )
            .is_err(),
            "the same product on the same date is one line"
        );

        let _ = std::fs::remove_file(path);
    }
}
