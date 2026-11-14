//! Turning a peer's sales into local rows.
//!
//! This is the meaning of a sale as far as this product is concerned: which
//! fields must agree before a row is allowed to exist, what an unknown product
//! means, and how a phone's clock is repaired. The transport and the cursor
//! loop do not live here; this module takes a validated shape and writes it.
//!
//! Everything in one sale is one transaction. A replay of the same sale is a
//! no-op because `sales.uid` is unique and the early check answers "accepted"
//! for an id we already hold. A page that fails halfway can therefore be
//! retried from its first sale without creating a second copy of anything.

use std::collections::{HashMap, HashSet};

use anyhow::{Context as _, Result};

use crate::db;
use crate::sync::protocol::{MovementDto, PROTOCOL, PushRequest, PushResponse, Reject, SaleDto};
use storage::Store;

/// Ingest a batch of offered sales.
///
/// Returns one verdict per sale. A database failure aborts the request instead
/// of pretending a sale was rejected: the caller should retry, and per-sale
/// transactions make that safe.
pub fn ingest_sales(store: &Store, request: &PushRequest) -> Result<PushResponse> {
    anyhow::ensure!(
        request.protocol == 0 || request.protocol == PROTOCOL,
        "unsupported sync protocol {}",
        request.protocol
    );

    let mut accepted = Vec::with_capacity(request.sales.len());
    let mut rejected = Vec::new();

    for sale in &request.sales {
        if sale.uid.trim().is_empty() {
            rejected.push(Reject {
                uid: sale.uid.clone(),
                reason: "a sale needs a uid".to_owned(),
            });
            continue;
        }

        // The common replay path: a lost acknowledgement, or a peer that saw
        // the sale through another till. Do not re-validate or touch children.
        if sale_known(store, &sale.uid)? {
            accepted.push(sale.uid.clone());
            continue;
        }

        let expected = match validate(sale) {
            Ok(expected) => expected,
            Err(error) => {
                let reason = error.to_string();
                record_reject(store, sale, &reason)?;
                rejected.push(Reject {
                    uid: sale.uid.clone(),
                    reason,
                });
                continue;
            }
        };

        apply_sale(store, sale, &expected)?;
        accepted.push(sale.uid.clone());
    }

    Ok(PushResponse {
        protocol: PROTOCOL,
        accepted,
        rejected,
        server_time: now_iso(store)?,
    })
}

fn sale_known(store: &Store, uid: &str) -> Result<bool> {
    Ok(store.conn().query_row(
        "SELECT EXISTS(SELECT 1 FROM sales WHERE uid = ?1)",
        [uid],
        |row| row.get(0),
    )?)
}

/// Check everything that can be checked without a database handle.
///
/// Returns the quantity sold per product, which is the number the stock
/// movement is rebuilt from. The payload's `delta` is only evidence that the
/// sender and this till agree; the value written is ours.
fn validate(sale: &SaleDto) -> Result<HashMap<i64, i64>> {
    anyhow::ensure!(is_uuid(&sale.uid), "the sale uid is not a UUID");
    anyhow::ensure!(
        !sale.origin_device.trim().is_empty(),
        "a sale needs its origin device"
    );
    anyhow::ensure!(
        sale.is_offline == 0 || sale.is_offline == 1,
        "is_offline must be 0 or 1"
    );
    anyhow::ensure!(!sale.lines.is_empty(), "a sale needs at least one line");
    anyhow::ensure!(
        !sale.payments.is_empty(),
        "a sale needs at least one payment"
    );
    anyhow::ensure!(sale.total_minor >= 0, "the total cannot be negative");
    anyhow::ensure!(
        sale.tax_minor >= 0 && sale.tax_minor <= sale.total_minor,
        "tax must be between zero and the total"
    );

    let mut line_uids = HashSet::new();
    let mut expected: HashMap<i64, i64> = HashMap::new();
    let mut line_total = 0_i64;
    for line in &sale.lines {
        anyhow::ensure!(is_uuid(&line.uid), "a line uid is not a UUID");
        anyhow::ensure!(
            line_uids.insert(line.uid.as_str()),
            "the same line uid appears twice"
        );
        anyhow::ensure!(!line.name.trim().is_empty(), "a line needs a name");
        anyhow::ensure!(line.qty > 0, "a line needs a positive quantity");
        anyhow::ensure!(line.unit_minor >= 0, "a unit price cannot be negative");
        anyhow::ensure!(line.total_minor >= 0, "a line total cannot be negative");

        let computed = line
            .qty
            .checked_mul(line.unit_minor)
            .context("a line total is too large")?;
        anyhow::ensure!(
            line.total_minor == computed,
            "a line total does not match quantity times price"
        );
        line_total = line_total
            .checked_add(computed)
            .context("the sale total is too large")?;

        if let Some(product_id) = line.product_id
            && product_id > 0
        {
            let entry = expected.entry(product_id).or_insert(0);
            *entry = entry
                .checked_add(line.qty)
                .context("a product quantity is too large")?;
        }
    }
    anyhow::ensure!(
        sale.total_minor == line_total,
        "the total does not match the lines"
    );

    let due = sale
        .total_minor
        .checked_add(sale.rounding_minor)
        .context("the amount due is too large")?;
    anyhow::ensure!(due >= 0, "the amount due cannot be negative");

    let mut payment_uids = HashSet::new();
    let mut paid = 0_i64;
    for payment in &sale.payments {
        anyhow::ensure!(is_uuid(&payment.uid), "a payment uid is not a UUID");
        anyhow::ensure!(
            payment_uids.insert(payment.uid.as_str()),
            "the same payment uid appears twice"
        );
        let method = payment.method.trim();
        anyhow::ensure!(!method.is_empty(), "a payment needs a method");
        anyhow::ensure!(payment.amount_minor >= 0, "a payment cannot be negative");
        anyhow::ensure!(payment.tendered_minor >= 0, "a tender cannot be negative");
        anyhow::ensure!(payment.change_minor >= 0, "change cannot be negative");
        paid = paid
            .checked_add(payment.amount_minor)
            .context("the payments are too large")?;

        if method.eq_ignore_ascii_case("cash") {
            anyhow::ensure!(
                payment.tendered_minor >= payment.amount_minor,
                "a cash tender is less than its amount"
            );
            anyhow::ensure!(
                payment.change_minor == payment.tendered_minor - payment.amount_minor,
                "cash change does not match the tender"
            );
        } else {
            anyhow::ensure!(
                payment.tendered_minor == payment.amount_minor,
                "a non-cash tender must equal its amount"
            );
            anyhow::ensure!(
                payment.change_minor == 0,
                "a non-cash payment cannot give change"
            );
        }
    }
    anyhow::ensure!(paid == due, "payments do not sum to total");

    if sale.is_offline == 1 {
        anyhow::ensure!(
            sale.payments.len() == 1,
            "an offline sale must have exactly one payment"
        );
        anyhow::ensure!(
            sale.payments[0].method.trim().eq_ignore_ascii_case("cash"),
            "an offline sale can only be cash"
        );
    }

    let mut movement_products = HashSet::new();
    for movement in &sale.movements {
        anyhow::ensure!(is_uuid(&movement.uid), "a movement uid is not a UUID");
        anyhow::ensure!(
            movement_products.insert(movement.product_id),
            "the same product has two movements in one sale"
        );
        anyhow::ensure!(
            movement.kind == db::StockKind::Sale.key(),
            "a sale can only carry sale movements"
        );
        let qty = expected.get(&movement.product_id).copied().unwrap_or(0);
        anyhow::ensure!(
            qty > 0,
            "a movement names a product that is not on the sale"
        );
        anyhow::ensure!(
            movement.delta == -qty,
            "a movement does not match the lines"
        );
    }

    Ok(expected)
}

fn apply_sale(store: &Store, sale: &SaleDto, expected: &HashMap<i64, i64>) -> Result<()> {
    let transaction = store.conn().unchecked_transaction()?;

    // Take the write lock before reading anything else. SQLite upgrades a
    // deferred transaction on its first write, and doing it first keeps a
    // concurrent checkout from making this transaction's snapshot stale.
    let change_seq = db::bump_change_seq(&transaction)?;
    let (created_at, clock_skew) = clamp_time(&transaction, &sale.occurred_at)?;

    let user_id = match sale.user_id.as_deref().map(str::trim) {
        Some(id) if !id.is_empty() => {
            let exists: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM users WHERE id = ?1)",
                [id],
                |row| row.get(0),
            )?;
            exists.then(|| id.to_owned())
        }
        _ => None,
    };

    let inserted = transaction.execute(
        "INSERT INTO sales
             (uid, origin_device, is_offline, clock_skew, created_at,
              received_at, change_seq, user_id, total_minor, rounding_minor,
              tax_minor, receipt_no)
         VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'), ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT DO NOTHING",
        rusqlite::params![
            sale.uid.trim(),
            sale.origin_device.trim(),
            sale.is_offline,
            clock_skew,
            created_at,
            change_seq,
            user_id.as_deref(),
            sale.total_minor,
            sale.rounding_minor,
            sale.tax_minor,
            // The origin's number, kept as it was printed. A peer too old to
            // send one leaves it null and the slip falls back to the row id.
            sale.receipt_no.as_deref(),
        ],
    )?;

    if inserted == 0 {
        // Another writer inserted the same uid between the early check and the
        // write lock. It is the same sale; our cursor bump is harmless.
        transaction.commit()?;
        return Ok(());
    }

    let sale_id = transaction.last_insert_rowid();
    apply_lines(&transaction, sale_id, sale)?;
    apply_payments(&transaction, sale_id, sale)?;
    apply_movements(
        &transaction,
        sale_id,
        sale,
        expected,
        user_id.as_deref(),
        &created_at,
    )?;

    transaction.commit()?;
    Ok(())
}

fn apply_lines(
    transaction: &rusqlite::Transaction<'_>,
    sale_id: i64,
    sale: &SaleDto,
) -> Result<()> {
    for line in &sale.lines {
        // A line whose product this till has never seen keeps its receipt text
        // and price, and loses only the link to the catalog. The sale is still
        // true; the shelf is not the authority on it.
        let product_id = match line.product_id {
            Some(id) if id > 0 => {
                let exists: bool = transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM products WHERE id = ?1)",
                    [id],
                    |row| row.get(0),
                )?;
                exists.then_some(id)
            }
            _ => None,
        };

        transaction.execute(
            "INSERT INTO sale_lines
                 (sale_id, product_id, name, qty, unit_minor, total_minor, uid)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT DO NOTHING",
            rusqlite::params![
                sale_id,
                product_id,
                line.name.trim(),
                line.qty,
                line.unit_minor,
                line.total_minor,
                line.uid.trim(),
            ],
        )?;
    }
    Ok(())
}

fn apply_payments(
    transaction: &rusqlite::Transaction<'_>,
    sale_id: i64,
    sale: &SaleDto,
) -> Result<()> {
    for payment in &sale.payments {
        transaction.execute(
            "INSERT INTO payments
                 (sale_id, method, amount_minor, tendered_minor, change_minor, uid)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT DO NOTHING",
            rusqlite::params![
                sale_id,
                payment.method.trim(),
                payment.amount_minor,
                payment.tendered_minor,
                payment.change_minor,
                payment.uid.trim(),
            ],
        )?;
    }
    Ok(())
}

fn apply_movements(
    transaction: &rusqlite::Transaction<'_>,
    sale_id: i64,
    sale: &SaleDto,
    expected: &HashMap<i64, i64>,
    user_id: Option<&str>,
    created_at: &str,
) -> Result<()> {
    for movement in &sale.movements {
        if !product_on_shelf(transaction, movement.product_id)? {
            continue;
        }

        // `validate` proved this entry exists and equals `-delta`; rebuilding it
        // here means the stored ledger is derived from the lines, never from a
        // number the sender could have edited.
        let qty = expected
            .get(&movement.product_id)
            .copied()
            .context("a validated movement lost its product")?;

        insert_movement(transaction, sale_id, movement, -qty, user_id, created_at)?;
    }
    Ok(())
}

fn insert_movement(
    transaction: &rusqlite::Transaction<'_>,
    sale_id: i64,
    movement: &MovementDto,
    delta: i64,
    user_id: Option<&str>,
    created_at: &str,
) -> Result<()> {
    transaction.execute(
        "INSERT INTO stock_movements
             (product_id, delta, kind, note, sale_id, user_id, created_at, uid)
         VALUES (?1, ?2, 'sale', NULL, ?3, ?4, ?5, ?6)
         ON CONFLICT DO NOTHING",
        rusqlite::params![
            movement.product_id,
            delta,
            sale_id,
            user_id,
            created_at,
            movement.uid.trim(),
        ],
    )?;
    Ok(())
}

fn product_on_shelf(transaction: &rusqlite::Transaction<'_>, product_id: i64) -> Result<bool> {
    Ok(transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM products WHERE id = ?1)",
        [product_id],
        |row| row.get(0),
    )?)
}

/// Repair a peer's clock.
///
/// The origin's time is used when it is plausible: not more than a week old and
/// not more than ten minutes in the future. Otherwise the sale is stamped with
/// this till's time and `clock_skew` records that the origin's clock was not
/// trusted. A phone in a shop with no network regularly drifts; refusing the
/// sale would lose money, and silently accepting a 2019 timestamp would hide it.
fn clamp_time(
    transaction: &rusqlite::Transaction<'_>,
    occurred_at: &str,
) -> Result<(String, bool)> {
    let (created_at, skewed): (String, bool) = transaction.query_row(
        "WITH t AS (SELECT datetime('now') AS now)
         SELECT CASE
                  WHEN datetime(?1) IS NULL THEN now
                  WHEN datetime(?1) < datetime(now, '-7 days') THEN now
                  WHEN datetime(?1) > datetime(now, '+10 minutes') THEN now
                  ELSE datetime(?1)
                END,
                CASE
                  WHEN datetime(?1) IS NULL THEN 1
                  WHEN datetime(?1) < datetime(now, '-7 days') THEN 1
                  WHEN datetime(?1) > datetime(now, '+10 minutes') THEN 1
                  ELSE 0
                END
         FROM t",
        [occurred_at],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok((created_at, skewed))
}

fn record_reject(store: &Store, sale: &SaleDto, reason: &str) -> Result<()> {
    let payload = serde_json::to_string(sale).unwrap_or_else(|_| "{}".to_owned());
    let origin = (!sale.origin_device.trim().is_empty()).then_some(sale.origin_device.as_str());

    store.conn().execute(
        "INSERT INTO sync_rejects (uid, origin_device, reason, payload)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(uid) DO UPDATE SET
             origin_device = excluded.origin_device,
             reason = excluded.reason,
             payload = excluded.payload,
             rejected_at = datetime('now')",
        rusqlite::params![sale.uid.trim(), origin, reason, payload],
    )?;
    Ok(())
}

fn now_iso(store: &Store) -> Result<String> {
    Ok(store
        .conn()
        .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
            row.get(0)
        })?)
}

fn is_uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok()
}
