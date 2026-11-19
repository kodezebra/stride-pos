//! Purchase rows: the list, the detail view, and the two ways a purchase becomes
//! stock — receiving a saved order, or creating and receiving in one go.
//!
//! Receiving is the only path that writes stock, so the line-by-line price
//! difference between plan and delivery is worked out here rather than twice.

use super::*;

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
pub(super) fn record_receive(
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
pub(super) fn insert_purchase_lines(
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
