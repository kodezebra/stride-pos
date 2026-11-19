//! Supplier rows: the list, the editor, archiving, and the repair pass that
//! fills in supplier ids on stock recorded before suppliers had ids.

use super::*;

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
pub(super) fn optional_text(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}
