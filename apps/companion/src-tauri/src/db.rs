//! The companion's own SQLite database. Local source of truth; the till is a
//! peer that receives sales, never the other way round for daily operation.

use anyhow::{Context, Result};
use directories::ProjectDirs;
use rusqlite::{Connection, OptionalExtension, params};
use storage::{Migration, Store};

use crate::protocol::CatalogSnapshot;
use crate::types::{Caps, Catalog, Customer, NewSale, Product, Sale, SaleLine, Staff, Status};

/// Arbitrary, permanent, never derived from the app name.
pub const APPLICATION_ID: i64 = 1_204_118_937;

const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "init",
    sql: include_str!("../../migrations/001_init.sql"),
}];

pub struct Db {
    store: Store,
}

impl Db {
    pub fn open() -> Result<Self> {
        let dirs = ProjectDirs::from("com", "stride", "companion")
            .context("no data directory for this platform")?;
        let path = dirs.data_dir().join("companion.db");
        let mut store = Store::open(path)?;
        store.claim_application_id(APPLICATION_ID)?;
        store.migrate(MIGRATIONS)?;
        Ok(Self { store })
    }

    pub fn conn(&self) -> &Connection {
        self.store.conn()
    }
}

pub fn meta(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()?)
}

pub fn set_meta(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

fn meta_i64(conn: &Connection, key: &str, default: i64) -> i64 {
    meta(conn, key)
        .ok()
        .flatten()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(default)
}

/// Write a catalog into a connection that is already inside a transaction.
pub(crate) fn write_catalog(tx: &Connection, snapshot: &CatalogSnapshot) -> Result<()> {
    tx.execute_batch(
        "DELETE FROM products; DELETE FROM categories; DELETE FROM staff; DELETE FROM customers;",
    )?;

    {
        let mut insert = tx.prepare(
            "INSERT INTO products (id, name, price_minor, barcode, sku, category, tracked,
                                   archived, unit_label, pack_size, pack_price_minor, pack_barcode)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 0, 'unit', ?7, ?8, ?9)",
        )?;
        for product in &snapshot.products {
            insert.execute(params![
                product.id,
                product.name,
                product.price_minor,
                product.barcode,
                product.sku,
                product.category,
                product.pack_size,
                product.pack_price_minor,
                product.pack_barcode
            ])?;
        }
    }

    {
        let mut insert = tx.prepare(
            "INSERT INTO categories (id, name, sort_order, color) VALUES (?1, ?2, ?3, ?4)",
        )?;
        for category in &snapshot.categories {
            insert.execute(params![
                category.id,
                category.name,
                category.sort_order,
                category.color
            ])?;
        }
    }

    {
        let mut insert = tx.prepare("INSERT INTO staff (id, name, role) VALUES (?1, ?2, ?3)")?;
        for person in &snapshot.staff {
            insert.execute(params![person.id, person.name, person.role])?;
        }
    }

    {
        let mut insert = tx.prepare(
            "INSERT INTO customers (id, name, phone, discount_bp, archived)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for customer in &snapshot.customers {
            insert.execute(params![
                customer.id,
                customer.name,
                customer.phone,
                customer.discount_bp,
                customer.archived
            ])?;
        }
    }

    set_meta(tx, "catalog_version", &snapshot.version.to_string())?;
    set_meta(tx, "currency", &snapshot.currency)?;
    set_meta(
        tx,
        "cash_step",
        &snapshot.cash_step_minor.max(1).to_string(),
    )?;
    set_meta(tx, "tax_bp", &(snapshot.tax_rate_percent * 100).to_string())?;
    set_meta(tx, "last_sync", &now_string())?;
    Ok(())
}

pub fn catalog(conn: &Connection) -> Result<Catalog> {
    let mut products = Vec::new();
    {
        let mut statement = conn.prepare(
            "SELECT id, name, price_minor, barcode, sku, category, pack_size, pack_price_minor
             FROM products WHERE archived = 0 ORDER BY name",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(Product {
                id: row.get::<_, i64>(0)?.to_string(),
                name: row.get(1)?,
                price_minor: row.get(2)?,
                barcode: row.get(3)?,
                sku: row.get(4)?,
                category: row.get(5)?,
                pack_size: row.get::<_, i64>(6)?,
                pack_price_minor: row.get(7)?,
            })
        })?;
        for row in rows {
            products.push(row?);
        }
    }

    let mut customers = Vec::new();
    {
        let mut statement = conn.prepare(
            "SELECT id, name, discount_bp FROM customers WHERE archived = 0 ORDER BY name",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(Customer {
                id: row.get::<_, i64>(0)?.to_string(),
                name: row.get(1)?,
                discount_bp: row.get(2)?,
                credit_limit_minor: default_credit(),
            })
        })?;
        for row in rows {
            customers.push(row?);
        }
    }

    let mut staff = Vec::new();
    {
        let mut statement = conn.prepare("SELECT id, name FROM staff ORDER BY name")?;
        let rows = statement.query_map([], |row| {
            Ok(Staff {
                id: row.get(0)?,
                name: row.get(1)?,
            })
        })?;
        for row in rows {
            staff.push(row?);
        }
    }

    Ok(Catalog {
        version: meta_i64(conn, "catalog_version", 0),
        currency: meta(conn, "currency")?.unwrap_or_else(|| "UGX".to_owned()),
        cash_step: meta_i64(conn, "cash_step", 100),
        tax_bp: meta_i64(conn, "tax_bp", 0),
        products,
        customers,
        staff,
        caps: Caps {
            max_sale_minor: meta_i64(conn, "max_sale_minor", 200_000),
            max_mobile_money_minor: meta_i64(conn, "max_mobile_money_minor", 50_000),
            total_mobile_money_minor: meta_i64(conn, "total_mobile_money_minor", 200_000),
        },
    })
}

pub fn status(conn: &Connection) -> Result<Status> {
    let paired = meta(conn, "token")?.is_some();
    let pending: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sales WHERE status = 'pending'",
        [],
        |row| row.get(0),
    )?;
    Ok(Status {
        paired,
        offline: !paired || meta(conn, "offline")?.as_deref() == Some("1"),
        catalog_version: meta_i64(conn, "catalog_version", 0),
        last_sync: meta(conn, "last_sync")?.unwrap_or_default(),
        pending,
        battery: None,
    })
}

pub fn sales(conn: &Connection) -> Result<Vec<Sale>> {
    let mut result = Vec::new();
    let headers: Vec<_> = {
        let mut statement = conn.prepare(
            "SELECT uid, occurred_at, total_minor, tender, reference, customer_id, cashier,
                    status, unverified, held_reason
             FROM sales ORDER BY occurred_at DESC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, i64>(8)? != 0,
                row.get::<_, Option<String>>(9)?,
            ))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    for header in headers {
        let mut lines = Vec::new();
        {
            let mut statement = conn.prepare(
                "SELECT name, qty, unit_minor, product_id FROM sale_lines
                 WHERE sale_uid = ?1 ORDER BY rowid",
            )?;
            let rows = statement.query_map(params![header.0], |row| {
                Ok(SaleLine {
                    name: row.get(0)?,
                    qty: row.get(1)?,
                    unit_minor: row.get(2)?,
                    product_id: row.get::<_, Option<i64>>(3)?.map(|id| id.to_string()),
                })
            })?;
            for row in rows {
                lines.push(row?);
            }
        }
        result.push(Sale {
            uid: header.0,
            occurred_at: header.1,
            total_minor: header.2,
            tender: header.3,
            reference: header.4,
            customer_id: header.5.map(|id| id.to_string()),
            cashier: header.6,
            status: header.7,
            unverified: header.8,
            held_reason: header.9,
            lines,
        });
    }
    Ok(result)
}

pub fn insert_sale(conn: &Connection, input: &NewSale, device_id: &str) -> Result<String> {
    let uid = uuid::Uuid::now_v7().to_string();
    let unverified = matches!(input.tender.as_str(), "mtn_momo" | "airtel_money");
    let is_on_credit = i64::from(input.tender == "credit");
    let customer_id = input
        .customer_id
        .as_deref()
        .and_then(|value| value.parse::<i64>().ok());

    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO sales (uid, origin_device, total_minor, tender, reference, customer_id,
                            cashier, is_on_credit, unverified)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            uid,
            device_id,
            input.total_minor,
            input.tender,
            input.reference,
            customer_id,
            input.cashier,
            is_on_credit,
            i64::from(unverified),
        ],
    )?;

    {
        let mut insert = tx.prepare(
            "INSERT INTO sale_lines (uid, sale_uid, product_id, name, qty, unit_minor, total_minor)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        for line in &input.lines {
            insert.execute(params![
                uuid::Uuid::now_v7().to_string(),
                uid,
                line.product_id
                    .as_deref()
                    .and_then(|value| value.parse::<i64>().ok()),
                line.name,
                line.qty,
                line.unit_minor,
                line.qty * line.unit_minor,
            ])?;
        }
    }
    tx.commit()?;
    Ok(uid)
}

pub fn set_sale_state(
    conn: &Connection,
    uid: &str,
    status: &str,
    reason: Option<&str>,
) -> Result<()> {
    conn.execute(
        "UPDATE sales SET status = ?2, held_reason = ?3,
            synced_at = CASE WHEN ?2 = 'synced' THEN strftime('%Y-%m-%dT%H:%M:%fZ','now') ELSE synced_at END,
            attempts = attempts + 1, last_error = ?3
         WHERE uid = ?1",
        params![uid, status, reason],
    )?;
    Ok(())
}

pub fn pending_sales(conn: &Connection) -> Result<Vec<Sale>> {
    Ok(sales(conn)?
        .into_iter()
        .filter(|sale| sale.status == "pending")
        .collect())
}

/// UTC timestamp in the same shape the desktop writes.
pub fn now_string() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // SQLite's strftime is used for stored times; this is the wire fallback.
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.000Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    (y + i64::from(m <= 2), m, d)
}

fn default_credit() -> i64 {
    120_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sale_and_its_lines_survive_a_round_trip() {
        let mut store = Store::open_in_memory().expect("memory store");
        store.claim_application_id(APPLICATION_ID).expect("claim");
        store.migrate(MIGRATIONS).expect("migrate");

        let input = NewSale {
            total_minor: 13_050,
            tender: "cash".to_owned(),
            reference: None,
            customer_id: None,
            cashier: "Amina".to_owned(),
            lines: vec![
                SaleLine {
                    name: "Sugar 2kg".to_owned(),
                    qty: 2,
                    unit_minor: 4_500,
                    product_id: Some("1".to_owned()),
                },
                SaleLine {
                    name: "Bread".to_owned(),
                    qty: 1,
                    unit_minor: 4_500,
                    product_id: Some("2".to_owned()),
                },
            ],
        };

        let uid = insert_sale(store.conn(), &input, "device-1").expect("insert");
        let stored = sales(store.conn()).expect("read");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].uid, uid);
        assert_eq!(stored[0].total_minor, 13_050);
        assert_eq!(stored[0].lines.len(), 2);
        assert_eq!(status(store.conn()).expect("status").pending, 1);
    }
}
