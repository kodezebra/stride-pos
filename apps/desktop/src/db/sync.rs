//! Sync rows: this device's identity, the change counter, the peer list, and the
//! two snapshot directions.
//!
//! Everything here exists so a till can say "this is what I have" and another
//! till can answer with what it is missing. The `meta` table is the device's own
//! memory, which is why the counters live beside the peers.

use super::*;

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

/// The receipt number a sale is printed with: `<device>-<day>-<seq>`.
///
/// The row id is a local counter — two tills syncing to each other both have a
/// sale 42 — so the customer's copy needs a number that is theirs alone. The
/// device prefix makes it unique between tills, the day makes it findable in a
/// box of paper, and the sequence restarts every morning so it stays short.
///
/// Read and bumped inside the sale's own transaction: two sales can never share
/// a number, and a number is never reused. A deleted sale does not give its
/// number back, which is why the count lives in `meta` rather than being a
/// `COUNT(*)` of the rows that remain.
pub fn next_receipt_no(
    transaction: &rusqlite::Transaction<'_>,
    device: &str,
) -> anyhow::Result<String> {
    let day: String =
        transaction.query_row("SELECT strftime('%Y%m%d', 'now', 'localtime')", [], |row| {
            row.get(0)
        })?;

    let stored: Option<String> = transaction
        .query_row(
            "SELECT value FROM meta WHERE key = 'receipt_day'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let last: i64 = if stored.as_deref() == Some(day.as_str()) {
        transaction
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM meta WHERE key = 'receipt_seq'",
                [],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0)
    } else {
        0
    };

    let seq = last + 1;
    put_meta(transaction, "receipt_day", &day)?;
    put_meta(transaction, "receipt_seq", &seq.to_string())?;
    Ok(format!("{}-{day}-{seq:03}", device_short(device)))
}

/// Four characters of a device id, for the front of a receipt number.
///
/// The tail, not the head: a UUIDv7 leads with a timestamp, so two tills
/// commissioned the same day would share a prefix and the number would not say
/// which one sold. Uniqueness does not rest on these four characters — the
/// sequence is per device — so a short collision costs nothing but a shrug.
pub(super) fn device_short(device: &str) -> String {
    let compact: String = device.chars().filter(char::is_ascii_hexdigit).collect();
    let start = compact.len().saturating_sub(4);
    compact[start..].to_ascii_uppercase()
}

/// One `meta` value, written inside a transaction.
pub(super) fn put_meta(
    transaction: &rusqlite::Transaction<'_>,
    key: &str,
    value: &str,
) -> anyhow::Result<()> {
    transaction.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        (key, value),
    )?;
    Ok(())
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
                    COALESCE(strftime('%Y-%m-%dT%H:%M:%fZ', created_at), ''),
                    receipt_no, discount_minor, discount_bp, price_mode,
                    customer_id, customer_name, is_on_credit
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
                    receipt_no: row.get(10)?,
                    discount_minor: row.get(11)?,
                    discount_bp: row.get(12)?,
                    price_mode: row.get(13)?,
                    customer_id: row.get(14)?,
                    customer_name: row.get(15)?,
                    is_on_credit: row.get(16)?,
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
                "SELECT uid, product_id, name, qty, unit_minor, total_minor,
                        unit_factor, unit_label
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
                    unit_factor: row.get(6)?,
                    unit_label: row.get(7)?,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        pulled.sale.payments = {
            let mut statement = conn.prepare(
                "SELECT uid, method, amount_minor, tendered_minor, change_minor,
                        shift_uid, user_id,
                        strftime('%Y-%m-%dT%H:%M:%fZ', created_at)
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
                    shift_uid: row.get(5)?,
                    user_id: row.get(6)?,
                    occurred_at: row.get(7)?,
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
            "SELECT id, name, price_minor, barcode, sku, category, category_id,
                    tracked, archived, unit_label, pack_size,
                    pack_price_minor, pack_barcode
             FROM products
             ORDER BY id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(sync_proto::ProductDto {
                id: row.get(0)?,
                name: row.get(1)?,
                price_minor: row.get(2)?,
                barcode: row.get(3)?,
                sku: row.get(4)?,
                category: row.get(5)?,
                category_id: row.get(6)?,
                tracked: row.get(7)?,
                archived: row.get(8)?,
                unit_label: row.get(9)?,
                pack_size: row.get(10)?,
                pack_price_minor: row.get(11)?,
                pack_barcode: row.get(12)?,
                price_breaks: Vec::new(),
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    // Breaks in a second pass, grouped by product: one statement for the whole
    // catalog rather than one per row.
    let mut products = products;
    {
        let mut statement = store.conn().prepare(
            "SELECT product_id, min_qty, price_minor FROM price_breaks
             ORDER BY product_id, min_qty",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                sync_proto::PriceBreakDto {
                    min_qty: row.get(1)?,
                    price_minor: row.get(2)?,
                },
            ))
        })?;
        let mut grouped: std::collections::HashMap<i64, Vec<sync_proto::PriceBreakDto>> =
            std::collections::HashMap::new();
        for row in rows {
            let (product_id, tier) = row?;
            grouped.entry(product_id).or_default().push(tier);
        }
        for product in &mut products {
            if let Some(tiers) = grouped.remove(&product.id) {
                product.price_breaks = tiers;
            }
        }
    }

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

    // Archived customers travel too: a replica learns a customer is gone the
    // same way it learns about a product, and the local sale history keeps
    // pointing at the same id.
    let customers = {
        let mut statement = store.conn().prepare(
            "SELECT id, name, phone, discount_bp, archived
             FROM customers ORDER BY id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(sync_proto::CustomerDto {
                id: row.get(0)?,
                name: row.get(1)?,
                phone: row.get(2)?,
                discount_bp: row.get(3)?,
                archived: row.get(4)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

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
        customers,
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

    let mut customer_ids = std::collections::HashSet::new();
    let mut customer_names = std::collections::HashSet::new();
    for customer in &snapshot.customers {
        anyhow::ensure!(customer.id > 0, "a customer needs a positive id");
        anyhow::ensure!(
            customer_ids.insert(customer.id),
            "the catalog lists customer {} twice",
            customer.id
        );
        let name = customer.name.trim();
        anyhow::ensure!(!name.is_empty(), "a customer needs a name");
        anyhow::ensure!(
            (0..=10_000).contains(&customer.discount_bp),
            "customer {name} has a discount outside 0..100%"
        );
        anyhow::ensure!(
            customer.archived == 0 || customer.archived == 1,
            "archived must be 0 or 1"
        );
        if customer.archived == 0 {
            anyhow::ensure!(
                customer_names.insert(name.to_lowercase()),
                "the catalog lists the customer name {name} twice"
            );
        }
    }

    let mut product_ids = std::collections::HashSet::new();
    let mut barcodes: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    let mut skus: std::collections::HashSet<String> = std::collections::HashSet::new();
    // The ladder is cleaned once, here, and the cleaned copy is what gets
    // written. A peer that sends rungs out of order is sorted, not rejected;
    // a peer that sends a climbing ladder is rejected, because honouring it
    // would charge a wholesale count more than a single unit.
    let mut ladders: std::collections::HashMap<i64, Vec<PriceTier>> =
        std::collections::HashMap::new();
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
        anyhow::ensure!(
            !product.unit_label.trim().is_empty(),
            "a product unit needs a name"
        );
        anyhow::ensure!(
            product.pack_size >= 1,
            "product {} says a pack holds {} units",
            product.id,
            product.pack_size
        );
        anyhow::ensure!(
            product.pack_price_minor.is_none_or(|price| price >= 0),
            "a pack price cannot be negative"
        );
        if let Some(category_id) = product.category_id {
            anyhow::ensure!(
                category_ids.contains(&category_id),
                "product {} names category {category_id}, which is not in the snapshot",
                product.id
            );
        }
        if product.archived == 0 {
            if let Some(sku) = product
                .sku
                .as_deref()
                .map(str::trim)
                .filter(|sku| !sku.is_empty())
            {
                anyhow::ensure!(
                    skus.insert(sku.to_lowercase()),
                    "the catalog puts shop code {sku} on two active products"
                );
            }
            for barcode in [product.barcode.as_deref(), product.pack_barcode.as_deref()]
                .into_iter()
                .flatten()
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
        if product.pack_size <= 1 {
            // A blank code is no code: an older or hand-built peer may send
            // `Some("")` where this binary would store `None`.
            let has_pack_code = product
                .pack_barcode
                .as_deref()
                .is_some_and(|code| !code.trim().is_empty());
            anyhow::ensure!(
                product.pack_price_minor.is_none() && !has_pack_code,
                "product {} prices a pack of one",
                product.id
            );
        }
        let breaks: Vec<PriceTier> = product
            .price_breaks
            .iter()
            .map(|tier| PriceTier {
                min_qty: tier.min_qty,
                price_minor: tier.price_minor,
            })
            .collect();
        let breaks = super::catalog::clean_tiers(product.price_minor, &breaks)
            .with_context(|| format!("product {} has an unusable price ladder", product.id))?;
        if !breaks.is_empty() {
            ladders.insert(product.id, breaks);
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

    // Two products may swap barcodes or shop codes in one snapshot. Clearing
    // every code in the snapshot first avoids a transient unique-index conflict
    // that has nothing to do with the final state. Both barcode columns are
    // cleared because a pack code may take a loose code's place.
    for product in &snapshot.products {
        transaction.execute(
            "UPDATE products SET barcode = NULL, pack_barcode = NULL, sku = NULL WHERE id = ?1",
            [product.id],
        )?;
    }

    for product in &snapshot.products {
        let barcode = product
            .barcode
            .as_deref()
            .map(str::trim)
            .filter(|barcode| !barcode.is_empty());
        let pack_barcode = product
            .pack_barcode
            .as_deref()
            .map(str::trim)
            .filter(|barcode| !barcode.is_empty());
        let sku = product
            .sku
            .as_deref()
            .map(str::trim)
            .filter(|sku| !sku.is_empty());
        transaction.execute(
            "INSERT INTO products
                 (id, name, price_minor, category, category_id, barcode, sku,
                  tracked, archived, unit_label, pack_size, pack_price_minor,
                  pack_barcode)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(id) DO UPDATE SET
                 name = excluded.name,
                 price_minor = excluded.price_minor,
                 category = excluded.category,
                 category_id = excluded.category_id,
                 barcode = excluded.barcode,
                 sku = excluded.sku,
                 tracked = excluded.tracked,
                 archived = excluded.archived,
                 unit_label = excluded.unit_label,
                 pack_size = excluded.pack_size,
                 pack_price_minor = excluded.pack_price_minor,
                 pack_barcode = excluded.pack_barcode",
            rusqlite::params![
                product.id,
                product.name.trim(),
                product.price_minor,
                product.category.trim(),
                product.category_id,
                barcode,
                sku,
                product.tracked,
                product.archived,
                product.unit_label.trim(),
                product.pack_size,
                product.pack_price_minor,
                pack_barcode,
            ],
        )?;
    }

    // Breaks are replaced with the product, not merged: the owner's ladder is
    // the whole ladder, and a rung removed there has to disappear here. Done
    // after every product row so the foreign key holds.
    for product in &snapshot.products {
        transaction.execute(
            "DELETE FROM price_breaks WHERE product_id = ?1",
            [product.id],
        )?;
        for tier in ladders.get(&product.id).into_iter().flatten() {
            transaction.execute(
                "INSERT INTO price_breaks (product_id, min_qty, price_minor)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![product.id, tier.min_qty, tier.price_minor],
            )?;
        }
    }

    // Customers are archived before they are upserted. A replica may still
    // have a live customer whose name the owner has given to someone else, and
    // archiving the row the snapshot no longer lists first leaves the unique
    // live-name index clear for the rename.
    let local_customers = {
        let mut statement = transaction.prepare("SELECT id FROM customers WHERE archived = 0")?;
        let rows = statement.query_map([], |row| row.get::<_, i64>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for id in local_customers {
        if !customer_ids.contains(&id) {
            transaction.execute("UPDATE customers SET archived = 1 WHERE id = ?1", [id])?;
        }
    }

    // Archived rows first, live ones second: a snapshot that renames A to B's
    // old name while archiving B must not hit the live-name index on the way.
    for archived in [1_i64, 0] {
        for customer in snapshot
            .customers
            .iter()
            .filter(|customer| customer.archived == archived)
        {
            let phone = customer
                .phone
                .as_deref()
                .map(str::trim)
                .filter(|phone| !phone.is_empty());
            transaction.execute(
                "INSERT INTO customers (id, name, phone, discount_bp, archived)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET
                     name = excluded.name,
                     phone = excluded.phone,
                     discount_bp = excluded.discount_bp,
                     archived = excluded.archived",
                rusqlite::params![
                    customer.id,
                    customer.name.trim(),
                    phone,
                    customer.discount_bp,
                    customer.archived,
                ],
            )?;
        }
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

/// One row of the change feed while its children are being gathered.
pub(super) struct PulledSale {
    id: i64,
    change_seq: i64,
    sale: sync_proto::SaleDto,
}
