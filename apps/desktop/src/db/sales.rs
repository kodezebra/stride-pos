//! Sale rows and the reports built from them.
//!
//! A sale, the shift it belongs to, the drawer totals that shift is closed with,
//! and the summaries the dashboard and reports read. The window helpers are here
//! because every report compares the same range against the one before it.

use super::*;

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
    terms: &SaleTerms<'_>,
) -> anyhow::Result<RecordedSale> {
    anyhow::ensure!(!lines.is_empty(), "a sale needs at least one line");

    let gross_minor = lines.iter().try_fold(0_i64, |sum, line| {
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

    anyhow::ensure!(terms.discount_minor >= 0, "a discount cannot be negative");
    anyhow::ensure!(
        terms.discount_minor <= gross_minor,
        "a discount cannot exceed the basket"
    );
    anyhow::ensure!(
        (0..=10_000).contains(&terms.discount_bp),
        "a discount rate cannot exceed the whole basket"
    );
    let total_minor = gross_minor - terms.discount_minor;

    anyhow::ensure!(
        terms.tax_minor <= total_minor,
        "tax cannot exceed the sale total"
    );

    let paid_minor = total_minor
        .checked_add(terms.rounding_minor)
        .context("the amount due does not fit in 64 bits")?;
    anyhow::ensure!(paid_minor >= 0, "the amount due cannot be negative");

    // When the sale is on credit, zero tenders are allowed and the payment
    // rows are optional. The outstanding balance is tracked as
    // `total_minor - paid_minor` and the customer is billed later.
    let is_credit = terms.is_on_credit;
    anyhow::ensure!(
        !is_credit || terms.customer_id.is_some(),
        "a credit sale needs a customer to bill later"
    );

    let single_cash = PaymentLine {
        method: "cash",
        amount_minor: paid_minor,
        tendered_minor: terms.tendered_minor,
        reference: None,
    };
    let tenders: &[PaymentLine<'_>] = if terms.payments.is_empty() && is_credit {
        // No tenders on a credit sale: allow zero rows; the loop below will
        // be skipped and no payment rows will be written.
        &[]
    } else if terms.payments.is_empty() {
        std::slice::from_ref(&single_cash)
    } else {
        terms.payments
    };

    let mut taken_minor = 0_i64;
    let mut tendered_minor = 0_i64;
    let mut change_minor = 0_i64;
    for tender in tenders {
        let method = tender.method.trim();
        anyhow::ensure!(!method.is_empty(), "a payment needs a method");
        anyhow::ensure!(tender.amount_minor >= 0, "a payment cannot be negative");
        anyhow::ensure!(
            tender.tendered_minor >= 0,
            "cash received cannot be negative"
        );

        if method.eq_ignore_ascii_case("cash") {
            // Cash may overpay and produce change; it may never settle less
            // than it claims, because that would mean a tender contributed
            // money the drawer never saw.
            anyhow::ensure!(
                tender.tendered_minor >= tender.amount_minor,
                "cash received is less than the amount it settles"
            );
            change_minor = change_minor
                .checked_add(tender.tendered_minor - tender.amount_minor)
                .context("the change does not fit in 64 bits")?;
        } else {
            // A mobile-money row is exact: the customer approved a figure on
            // their phone, and the till hands nothing back against it.
            anyhow::ensure!(
                tender.tendered_minor == tender.amount_minor,
                "a non-cash payment must equal its amount"
            );
            if let Some(reference) = tender.reference {
                anyhow::ensure!(
                    reference.chars().count() <= 64,
                    "a payment reference is too long"
                );
            }
        }

        taken_minor = taken_minor
            .checked_add(tender.amount_minor)
            .context("the payments do not fit in 64 bits")?;
        tendered_minor = tendered_minor
            .checked_add(tender.tendered_minor)
            .context("the payments do not fit in 64 bits")?;
    }
    // On a credit sale, zero tenders are valid; only require the sum when
    // we actually have tenders (legacy single-cash path).
    if !is_credit && taken_minor != paid_minor {
        anyhow::ensure!(
            false,
            "payments sum to {taken_minor}, not the amount due {paid_minor}"
        );
    }

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
             (total_minor, rounding_minor, tax_minor, discount_minor, discount_bp,
              price_mode, customer_id, customer_name, user_id, uid, origin_device,
              change_seq, shift_uid, receipt_no, is_on_credit)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        (
            total_minor,
            terms.rounding_minor,
            terms.tax_minor,
            terms.discount_minor,
            terms.discount_bp,
            terms.price_mode,
            terms.customer_id,
            terms.customer_name,
            terms.user_id,
            sale_uid.as_str(),
            origin_device.as_str(),
            change_seq,
            terms.shift_uid,
            receipt_no.as_str(),
            i64::from(is_credit),
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
                    (product_id, -units, id, terms.user_id, movement_uid.as_str()),
                )?;
            }
        }
    }

    // One row per tender. The rows are append-only and carry a `uid` so sync
    // replays them exactly once; the order they are written in is the order
    // the cashier took them.
    for tender in tenders {
        let payment_uid = uuid::Uuid::now_v7().to_string();
        let method = tender.method.trim();
        let change = if method.eq_ignore_ascii_case("cash") {
            tender.tendered_minor - tender.amount_minor
        } else {
            0
        };
        transaction.execute(
            "INSERT INTO payments
                 (sale_id, method, amount_minor, tendered_minor, change_minor, uid, reference,
                  shift_uid, user_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'))",
            (
                id,
                method,
                tender.amount_minor,
                tender.tendered_minor,
                change,
                payment_uid.as_str(),
                tender.reference,
                terms.shift_uid,
                terms.user_id,
            ),
        )?;
    }

    transaction.commit()?;

    Ok(RecordedSale {
        id,
        uid: sale_uid,
        total_minor,
        rounding_minor: terms.rounding_minor,
        tax_minor: terms.tax_minor,
        discount_minor: terms.discount_minor,
        discount_bp: terms.discount_bp,
        price_mode: terms.price_mode.to_owned(),
        customer_name: terms.customer_name.map(str::to_owned),
        tendered_minor,
        change_minor,
    })
}

// ---------------------------------------------------------------------------
// The cash drawer
// ---------------------------------------------------------------------------

/// The columns every shift read selects, in the order [`shift_from_row`] wants.
pub(super) const SHIFT_COLUMNS: &str =
    "s.uid, strftime('%d/%m %H:%M', s.opened_at, 'localtime'), u.name,
     s.float_minor, s.closed_at, s.counted_minor
     FROM shifts s LEFT JOIN users u ON u.id = s.opened_by";

pub(super) fn shift_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Shift> {
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
pub(super) fn open_shift(store: &storage::Store) -> Option<Shift> {
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
        // Attribute by the payment's own shift. A payment taken today against
        // an older credit sale belongs in the drawer being counted now; before
        // payments carried attribution it was silently filed with the old sale.
        "SELECT COALESCE(SUM(p.amount_minor), 0), COUNT(DISTINCT p.sale_id)
         FROM payments p JOIN sales s ON s.id = p.sale_id
         WHERE COALESCE(p.shift_uid, s.shift_uid) = ?1",
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
pub(super) fn range_clause(range: usize, column: &str) -> Option<String> {
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
pub(super) fn previous_clause(range: usize, column: &str) -> Option<String> {
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
pub(super) fn window_clause(range: usize, column: &str) -> String {
    match range_clause(range, column) {
        Some(clause) => format!("({clause}) AND (?1 IS NULL OR s.user_id = ?1)"),
        None => "(?1 IS NULL OR s.user_id = ?1)".to_owned(),
    }
}

/// [`window_clause`] for the period before the range, or `None` for all time.
pub(super) fn previous_window_clause(range: usize, column: &str) -> Option<String> {
    previous_clause(range, column)
        .map(|clause| format!("({clause}) AND (?1 IS NULL OR s.user_id = ?1)"))
}

/// How many days back the zero-filled calendar opens, as a SQLite modifier.
///
/// Ranges with a fixed length get a calendar so a day that sold nothing is a
/// zero bar, not a missing one. All time has no fixed calendar — it is every
/// day that sold, capped for display — and returns `None`.
pub(super) fn calendar_start(range: usize) -> Option<&'static str> {
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

pub(super) fn totals_in(store: &storage::Store, clause: &str, user: Option<&str>) -> (i64, i64) {
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
    // The discount belongs to the sale, not to any one line, so it is spread
    // across the sale's lines in proportion to their gross. Subtracting the
    // whole discount on every joined line would count it once per item; not
    // subtracting it at all would report profit the shop did not keep. The
    // correlated sum is indexed by `sale_id` and only touches sales in the
    // range.
    let sql = format!(
        "SELECT
             COALESCE(SUM(CASE WHEN l.cost_milli IS NOT NULL THEN
                 l.qty * l.unit_minor
                 - s.discount_minor * (l.qty * l.unit_minor)
                   / NULLIF((SELECT SUM(l2.qty * l2.unit_minor)
                             FROM sale_lines l2 WHERE l2.sale_id = s.id), 0)
             ELSE 0 END), 0),
             (COALESCE(SUM(CASE WHEN l.cost_milli IS NOT NULL
                                THEN l.qty * l.unit_factor * l.cost_milli ELSE 0 END), 0)
              + 500) / 1000,
             COALESCE(SUM(CASE WHEN l.cost_milli IS NOT NULL THEN 1 ELSE 0 END), 0),
             COUNT(l.id)
         FROM sale_lines l JOIN sales s ON s.id = l.sale_id
         WHERE {clause}"
    );

    let discount_minor = store
        .conn()
        .query_row(
            &format!("SELECT COALESCE(SUM(s.discount_minor), 0) FROM sales s WHERE {clause}"),
            [user],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0);

    match store.conn().query_row(&sql, [user], |row| {
        Ok(Margin {
            revenue_minor: row.get(0)?,
            discount_minor,
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
                COALESCE(GROUP_CONCAT(DISTINCT p.method), ''), s.receipt_no,
                s.discount_minor, s.price_mode, s.customer_name
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
                discount_minor: row.get(8)?,
                price_mode: row.get(9)?,
                customer: row.get(10)?,
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
         strftime('%d/%m/%Y %H:%M', s.created_at, 'localtime'), u.name, s.receipt_no, \
         s.discount_minor, s.discount_bp, s.price_mode, s.customer_name, \
         s.customer_id, s.is_on_credit
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
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<i64>>(10)?,
                row.get::<_, i64>(11)? != 0,
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
    // settled across two methods must show both, not one pile. Each row is a
    // tender, in the order the cashier took it.
    let mut method_parts: Vec<String> = Vec::new();
    let mut payments = Vec::new();
    let mut tendered_minor = 0;
    let mut change_minor = 0;
    let mut settled_minor = 0;
    if let Ok(mut statement) = conn.prepare(
        "SELECT method, amount_minor, tendered_minor, change_minor, reference
           FROM payments WHERE sale_id = ?1 ORDER BY id",
    ) {
        if let Ok(rows) = statement.query_map([id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        }) {
            for payment in rows.filter_map(Result::ok) {
                if !method_parts
                    .iter()
                    .any(|method| method.eq_ignore_ascii_case(&payment.0))
                {
                    method_parts.push(payment.0.clone());
                }
                payments.push(PaidBy {
                    method: payment.0,
                    amount_minor: payment.1,
                    tendered_minor: payment.2,
                    change_minor: payment.3,
                    reference: payment.4,
                });
                tendered_minor += payment.2;
                change_minor += payment.3;
                settled_minor += payment.1;
            }
        }
    } else {
        tracing::warn!("payments prepare failed");
    };

    let due_minor = head.0 + head.1;

    Some(SaleDetail {
        id,
        receipt_no: head.5,
        cashier: head.4,
        created_at: head.3,
        total_minor: head.0,
        discount_minor: head.6,
        discount_bp: head.7,
        price_mode: head.8,
        customer_name: head.9,
        customer_id: head.10,
        is_on_credit: head.11,
        rounding_minor: head.1,
        tax_minor: head.2,
        paid_minor: due_minor,
        method: if method_parts.is_empty() {
            "-".to_owned()
        } else {
            method_parts.join(" + ")
        },
        payments,
        tendered_minor,
        change_minor,
        settled_minor,
        balance_minor: (due_minor - settled_minor).max(0),
        lines,
    })
}

/// Credit sales a customer still owes on, newest first.
///
/// The balance is derived from the payment rows rather than stored: a payment
/// taken later is just another append-only row against the same sale, so the
/// figure cannot drift from what was actually received. The existing
/// `sales_customer` index serves the lookup.
pub fn credit_sales(store: &storage::Store, customer_id: i64) -> Vec<CreditSale> {
    let conn = store.conn();
    let mut statement = match conn.prepare(
        "SELECT s.id, s.receipt_no,
                strftime('%d/%m/%Y %H:%M', s.created_at, 'localtime'),
                s.total_minor + s.rounding_minor,
                COALESCE(SUM(p.amount_minor), 0)
           FROM sales s LEFT JOIN payments p ON p.sale_id = s.id
          WHERE s.customer_id = ?1 AND s.is_on_credit = 1
          GROUP BY s.id
         HAVING (s.total_minor + s.rounding_minor) > COALESCE(SUM(p.amount_minor), 0)
          ORDER BY s.created_at DESC, s.id DESC",
    ) {
        Ok(statement) => statement,
        Err(err) => {
            tracing::warn!(error = %err, "credit sales prepare failed");
            return Vec::new();
        }
    };
    let rows = match statement.query_map([customer_id], |row| {
        let due = row.get::<_, i64>(3)?;
        let settled = row.get::<_, i64>(4)?;
        Ok(CreditSale {
            id: row.get(0)?,
            receipt_no: row.get(1)?,
            created_at: row.get(2)?,
            total_minor: due,
            settled_minor: settled,
            balance_minor: (due - settled).max(0),
        })
    }) {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(error = %err, "credit sales query failed");
            return Vec::new();
        }
    };
    rows.filter_map(Result::ok).collect()
}

/// Record a payment taken later against a credit sale.
///
/// This is what happens when a customer comes back with the money: another row
/// in the append-only `payments` table, tied to the original sale by `sale_id`.
/// Nothing is written unless the amount is positive, fits the outstanding
/// balance, and obeys the same cash/non-cash rules as a tender at checkout.
pub fn record_credit_payment(
    store: &storage::Store,
    payment: &CreditPayment<'_>,
) -> anyhow::Result<()> {
    let CreditPayment {
        sale_id,
        method,
        amount_minor,
        tendered_minor,
        reference,
        shift_uid,
        user_id,
    } = *payment;
    let method = method.trim();
    anyhow::ensure!(!method.is_empty(), "a payment needs a method");
    anyhow::ensure!(amount_minor > 0, "a payment must be more than zero");
    anyhow::ensure!(tendered_minor >= 0, "cash received cannot be negative");
    if let Some(reference) = reference {
        anyhow::ensure!(
            reference.chars().count() <= 64,
            "a payment reference is too long"
        );
    }
    if method.eq_ignore_ascii_case("cash") {
        anyhow::ensure!(
            tendered_minor >= amount_minor,
            "cash received is less than the amount it settles"
        );
    } else {
        anyhow::ensure!(
            tendered_minor == amount_minor,
            "a non-cash payment must equal its amount"
        );
    }

    let transaction = store.conn().unchecked_transaction()?;
    // Later payments are append-only rows, so there is no sale row to carry a
    // new change sequence; the cursor still moves so a peer knows to pull.
    let _change_seq = bump_change_seq(&transaction)?;

    let due: i64 = transaction.query_row(
        "SELECT total_minor + rounding_minor FROM sales WHERE id = ?1 AND is_on_credit = 1",
        [sale_id],
        |row| row.get(0),
    )?;
    let settled: i64 = transaction.query_row(
        "SELECT COALESCE(SUM(amount_minor), 0) FROM payments WHERE sale_id = ?1",
        [sale_id],
        |row| row.get(0),
    )?;
    let balance = due - settled;
    anyhow::ensure!(
        amount_minor <= balance,
        "payment of {amount_minor} exceeds the outstanding balance of {balance}"
    );

    let payment_uid = uuid::Uuid::now_v7().to_string();
    let change = if method.eq_ignore_ascii_case("cash") {
        tendered_minor - amount_minor
    } else {
        0
    };
    transaction.execute(
        "INSERT INTO payments
             (sale_id, method, amount_minor, tendered_minor, change_minor, uid, reference,
              shift_uid, user_id, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'))",
        (
            sale_id,
            method,
            amount_minor,
            tendered_minor,
            change,
            payment_uid.as_str(),
            reference,
            shift_uid,
            user_id,
        ),
    )?;
    transaction.commit()?;
    Ok(())
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
    let sql = "SELECT COALESCE(u.name, 'No cashier'),
                      COUNT(DISTINCT p.sale_id),
                      COALESCE(SUM(CASE WHEN p.method = 'cash' THEN p.amount_minor ELSE 0 END), 0),
                      COALESCE(SUM(p.amount_minor), 0)
                 FROM payments p
                 JOIN sales s ON s.id = p.sale_id
                 LEFT JOIN users u ON u.id = COALESCE(p.user_id, s.user_id)
                WHERE COALESCE(p.shift_uid, s.shift_uid) = ?1
                GROUP BY COALESCE(p.user_id, s.user_id)
                ORDER BY 3 DESC, 1 COLLATE NOCASE";
    let mut statement = match store.conn().prepare(sql) {
        Ok(statement) => statement,
        Err(err) => {
            tracing::warn!(error = %err, "drawer by person prepare failed");
            return Vec::new();
        }
    };
    let rows = match statement.query_map([shift_uid], |row| {
        Ok(PersonTotal {
            name: row.get(0)?,
            sales: row.get(1)?,
            cash_minor: row.get(2)?,
            takings_minor: row.get(3)?,
        })
    }) {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(error = %err, "drawer by person query failed");
            return Vec::new();
        }
    };
    rows.filter_map(Result::ok).collect()
}

pub(super) fn person_totals<P: rusqlite::Params>(
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
