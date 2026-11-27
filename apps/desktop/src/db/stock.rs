//! Stock rows: movements, the expiry buckets they roll into, and the date
//! arithmetic those buckets need.
//!
//! A date is stored as `YYYY-MM-DD` text, so the comparisons are string
//! comparisons and the only real work is turning "today" and a shelf life into
//! the same shape. That is what the civil-date helpers below do.

use super::*;

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

/// Record the last price paid for one unit of a product.
///
/// A delivery recorded straight from a product's stock action never passes
/// through a restock list, so without this it would leave no cost behind and
/// margin reports would read it as free goods. The cost lives on the product
/// as thousandths of a minor unit, the same precision purchase lines use.
pub fn set_last_cost(
    store: &storage::Store,
    product_id: i64,
    cost_minor: i64,
) -> anyhow::Result<()> {
    anyhow::ensure!(cost_minor >= 0, "a unit cost cannot be negative");
    let cost_milli = cost_minor
        .checked_mul(1000)
        .context("that unit cost is out of range")?;
    let changed = store.conn().execute(
        "UPDATE products SET cost_milli = ?2 WHERE id = ?1 AND archived = 0",
        (product_id, cost_milli),
    )?;
    anyhow::ensure!(changed == 1, "that product is no longer in the catalog");
    Ok(())
}

/// Set the low-stock warning line, and optionally the target, for several
/// products at once.
///
/// The line used to be a one-time choice at "Track stock" with no way back.
/// Square edits it from the stock list, because a shop's sense of "low" changes
/// with demand; a line that cannot be corrected makes the Low stock filter a
/// guess the owner has learned to ignore.
pub fn set_warning_lines(
    store: &storage::Store,
    product_ids: &[i64],
    low_stock: i64,
    target_stock: Option<i64>,
) -> anyhow::Result<()> {
    anyhow::ensure!(!product_ids.is_empty(), "select a product first");
    anyhow::ensure!(low_stock >= 0, "the low-stock level cannot be negative");
    anyhow::ensure!(
        target_stock.is_none_or(|target| target >= low_stock),
        "the target cannot be below the low-stock level"
    );

    let transaction = store.conn().unchecked_transaction()?;
    for &product_id in product_ids {
        let changed = transaction.execute(
            "UPDATE products SET low_stock = ?2, target_stock = ?3
              WHERE id = ?1 AND tracked = 1 AND archived = 0",
            (product_id, low_stock, target_stock),
        )?;
        anyhow::ensure!(
            changed == 1,
            "one of the selected products is no longer tracked"
        );
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
pub(super) fn normalize_expiry(expires_on: Option<&str>) -> anyhow::Result<Option<String>> {
    match expires_on {
        None => Ok(None),
        Some(text) if text.trim().is_empty() => Ok(None),
        Some(text) => parse_expiry_date(text)
            .map(Some)
            .context("a date must be written like 31/12/2026"),
    }
}

pub(super) fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

pub(super) fn days_in_month(year: i64, month: i64) -> u32 {
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
pub(super) fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = (month + 9) % 12;
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Days since 1970-01-01 for a canonical `YYYY-MM-DD`, or `None` if malformed.
pub(super) fn date_days(date: &str) -> Option<i64> {
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
pub(super) fn today_days(store: &storage::Store) -> Option<i64> {
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
pub(super) fn derive_buckets(
    movements: &[(StockKind, Option<String>, i64)],
) -> Vec<(Option<String>, i64)> {
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
pub(super) fn product_movements(
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

    record_stock_movement_cause(
        store,
        product_id,
        -remaining,
        StockKind::Waste,
        Some(WasteCause::Spoilage),
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
#[cfg(test)]
pub fn record_stock_movement(
    store: &storage::Store,
    product_id: i64,
    delta: i64,
    kind: StockKind,
    note: &str,
    expires_on: Option<&str>,
    user_id: Option<&str>,
) -> anyhow::Result<()> {
    record_stock_movement_cause(
        store, product_id, delta, kind, None, note, expires_on, user_id,
    )
}

/// [`record_stock_movement`] with an optional write-off cause recorded
/// alongside the ledger row. Normal movements pass `None` and read as before.
// Each input is a separate fact about the movement, and the existing callers
// already name them in order; a bundle would add a type for no extra safety.
#[allow(clippy::too_many_arguments)]
pub fn record_stock_movement_cause(
    store: &storage::Store,
    product_id: i64,
    delta: i64,
    kind: StockKind,
    cause: Option<WasteCause>,
    note: &str,
    expires_on: Option<&str>,
    user_id: Option<&str>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        cause.is_none() || kind == StockKind::Waste,
        "only a write-off can name a cause"
    );
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
             (product_id, delta, kind, cause, note, expires_on, user_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        (
            product_id,
            delta,
            kind.key(),
            cause.map(WasteCause::key),
            optional_text(note),
            expires_on,
            user_id,
        ),
    )?;
    Ok(())
}

/// Apply a batch of corrections from one physical count.
///
/// All of them land in one transaction: a count is a single act in the shop,
/// and a half-applied count is worse than one that fails outright.
pub fn apply_counts(
    store: &storage::Store,
    counts: &[(i64, i64)],
    user_id: Option<&str>,
) -> anyhow::Result<usize> {
    let transaction = store.conn().unchecked_transaction()?;
    for &(product_id, delta) in counts {
        anyhow::ensure!(
            delta != 0,
            "a count that matches the ledger is not a movement"
        );
        let tracked: Option<bool> = transaction
            .query_row(
                "SELECT tracked FROM products WHERE id = ?1 AND archived = 0",
                [product_id],
                |row| row.get(0),
            )
            .optional()?;
        anyhow::ensure!(
            tracked == Some(true),
            "a counted product is no longer tracked"
        );
        transaction.execute(
            "INSERT INTO stock_movements (product_id, delta, kind, note, user_id)
             VALUES (?1, ?2, 'adjust', 'Counted', ?3)",
            (product_id, delta, user_id),
        )?;
    }
    transaction.commit()?;
    Ok(counts.len())
}

/// The shop-wide stock ledger, newest first, with the product name attached.
///
/// Filters are built from a bounded choice of days and the fixed `StockKind`
/// keys, never from typed text. Capped because this is an answer to "what
/// happened lately?", not an infinite report.
pub fn all_stock_movements(
    store: &storage::Store,
    days: Option<i64>,
    reason: Option<StockKind>,
    cause: Option<WasteCause>,
) -> Vec<GlobalStockMovement> {
    let since = days
        .map(|days| format!(" AND m.created_at >= datetime('now','localtime','-{days} days')"))
        .unwrap_or_default();
    let kind = reason
        .map(|kind| format!(" AND m.kind = '{}'", kind.key()))
        .unwrap_or_default();
    let cause_clause = cause
        .map(|cause| format!(" AND m.cause = '{}'", cause.key()))
        .unwrap_or_default();
    let sql = format!(
        "SELECT m.id, p.name, m.delta, m.kind, m.cause, m.note,
                strftime('%d/%m %H:%M', m.created_at, 'localtime'), u.name
           FROM stock_movements m
           JOIN products p ON p.id = m.product_id
           LEFT JOIN users u ON u.id = m.user_id
          WHERE 1 = 1 {since} {kind} {cause_clause}
          ORDER BY m.id DESC LIMIT 500"
    );
    let mut statement = match store.conn().prepare(&sql) {
        Ok(statement) => statement,
        Err(err) => {
            tracing::warn!(error = %err, "stock history prepare failed");
            return Vec::new();
        }
    };
    let rows = statement.query_map([], |row| {
        let kind: String = row.get(3)?;
        let cause: Option<String> = row.get(4)?;
        Ok(GlobalStockMovement {
            id: row.get(0)?,
            product_name: row.get(1)?,
            delta: row.get(2)?,
            kind: StockKind::from_key(&kind).unwrap_or(StockKind::Adjust),
            cause: cause.and_then(|key| WasteCause::from_key(&key)),
            note: row.get(5)?,
            created_at: row.get(6)?,
            user: row.get(7)?,
        })
    });
    match rows {
        Ok(rows) => rows.filter_map(Result::ok).collect(),
        Err(err) => {
            tracing::warn!(error = %err, "stock history query failed");
            Vec::new()
        }
    }
}

/// A product's movements, newest first.
///
/// Capped because the history is a question about recent stock, not an archive:
/// a modal that has to paint ten thousand deliveries is slower than the answer
/// is useful. The cap is stated on screen when it cuts the list.
pub const MOVEMENT_HISTORY: i64 = 200;

pub fn stock_movements(store: &storage::Store, product_id: i64) -> Vec<StockMovement> {
    let mut statement = match store.conn().prepare(
        "SELECT m.id, m.delta, m.kind, m.cause, m.note,
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
            row.get::<_, Option<String>>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, Option<String>>(7)?,
        ))
    });

    let Ok(rows) = rows else {
        tracing::warn!("stock history query failed");
        return Vec::new();
    };

    rows.filter_map(Result::ok)
        .filter_map(
            |(id, delta, kind, cause, note, created_at, user, expires_on)| {
                Some(StockMovement {
                    id,
                    delta,
                    kind: StockKind::from_key(&kind)?,
                    cause: cause.and_then(|key| WasteCause::from_key(&key)),
                    note,
                    created_at,
                    user,
                    expires_on,
                })
            },
        )
        .collect()
}
