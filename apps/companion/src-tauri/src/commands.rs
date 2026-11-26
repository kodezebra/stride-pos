//! Tauri commands. The Svelte UI only ever calls these; it never opens SQLite
//! or a socket itself.

use std::sync::Mutex;

use lan_http::{Client, Endpoint, Request};
use tauri::State;

use crate::db::{self, Db};
use crate::protocol::{
    CatalogSnapshot, LineDto, PROTOCOL, PaymentDto, Ping, PushRequest, PushResponse, SaleDto,
};
use crate::types::{Catalog, NewSale, Sale, Status, SyncResult};

pub struct AppState {
    pub db: Mutex<Db>,
}

fn err<E: std::fmt::Display>(error: E) -> String {
    error.to_string()
}

fn device_id(conn: &rusqlite::Connection) -> Result<String, String> {
    if let Some(id) = db::meta(conn, "device_id").map_err(err)? {
        return Ok(id);
    }
    let id = uuid::Uuid::now_v7().to_string();
    db::set_meta(conn, "device_id", &id).map_err(err)?;
    Ok(id)
}

fn endpoint(conn: &rusqlite::Connection) -> Result<(Endpoint, String), String> {
    let host = db::meta(conn, "host")
        .map_err(err)?
        .ok_or_else(|| "not paired with a till yet".to_owned())?;
    let port = db::meta(conn, "port")
        .map_err(err)?
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| "stored port is invalid".to_owned())?;
    let token = db::meta(conn, "token")
        .map_err(err)?
        .ok_or_else(|| "no pairing token".to_owned())?;
    Ok((Endpoint::new(host, port), token))
}

#[tauri::command]
pub fn get_status(state: State<'_, AppState>) -> Result<Status, String> {
    let db = state.db.lock().map_err(err)?;
    db::status(db.conn()).map_err(err)
}

#[tauri::command]
pub fn get_catalog(state: State<'_, AppState>) -> Result<Catalog, String> {
    let db = state.db.lock().map_err(err)?;
    db::catalog(db.conn()).map_err(err)
}

#[tauri::command]
pub fn get_sales(state: State<'_, AppState>) -> Result<Vec<Sale>, String> {
    let db = state.db.lock().map_err(err)?;
    db::sales(db.conn()).map_err(err)
}

#[tauri::command]
pub fn record_sale(state: State<'_, AppState>, sale: NewSale) -> Result<Sale, String> {
    let db = state.db.lock().map_err(err)?;
    let conn = db.conn();
    let id = device_id(conn)?;
    let uid = db::insert_sale(conn, &sale, &id).map_err(err)?;
    db::sales(conn)
        .map_err(err)?
        .into_iter()
        .find(|entry| entry.uid == uid)
        .ok_or_else(|| "sale was not saved".to_owned())
}

/// Probe the till and pull its catalog. Only writes local state after both
/// requests succeed, so a failed pairing cannot half-configure the device.
#[tauri::command]
pub fn pair(
    state: State<'_, AppState>,
    host: String,
    port: u16,
    token: String,
) -> Result<i64, String> {
    let token = lan_http::pairing::normalize(&token)
        .ok_or_else(|| "that is not a pairing code".to_owned())?;
    let endpoint = Endpoint::new(host, port);
    let client = Client::new();

    let ping_response = client
        .request(
            &endpoint,
            &Request::post("/v1/ping", b"{}".to_vec()).with_token(token.clone()),
        )
        .map_err(err)?;
    let ping: Ping = serde_json::from_slice(&ping_response.body).map_err(err)?;
    if !ping.ok || ping.protocol != PROTOCOL {
        return Err(format!(
            "till speaks protocol {}, this companion speaks {PROTOCOL}",
            ping.protocol
        ));
    }

    let catalog_response = client
        .request(
            &endpoint,
            &Request::post("/v1/sync/catalog", b"{}".to_vec()).with_token(token.clone()),
        )
        .map_err(err)?;
    let snapshot: CatalogSnapshot = serde_json::from_slice(&catalog_response.body).map_err(err)?;

    let db = state.db.lock().map_err(err)?;
    let conn = db.conn();
    let tx = conn.unchecked_transaction().map_err(err)?;
    db::replace_catalog(&tx, &snapshot).map_err(err)?;
    db::set_meta(&tx, "host", &endpoint.host).map_err(err)?;
    db::set_meta(&tx, "port", &port.to_string()).map_err(err)?;
    db::set_meta(&tx, "token", &token).map_err(err)?;
    db::set_meta(&tx, "offline", "0").map_err(err)?;
    let version = snapshot.version;
    tx.commit().map_err(err)?;
    Ok(version)
}

fn sale_dto(sale: &Sale, device_id: &str) -> SaleDto {
    let line_sum: i64 = sale
        .lines
        .iter()
        .map(|line| line.qty * line.unit_minor)
        .sum();
    let discount = line_sum.saturating_sub(sale.total_minor).max(0);
    let rounding = sale.total_minor.saturating_sub(line_sum).max(0);
    let lines = sale
        .lines
        .iter()
        .map(|line| LineDto {
            uid: uuid::Uuid::now_v7().to_string(),
            product_id: line
                .product_id
                .as_deref()
                .and_then(|value| value.parse::<i64>().ok()),
            name: line.name.clone(),
            qty: line.qty,
            unit_minor: line.unit_minor,
            total_minor: line.qty * line.unit_minor,
            unit_factor: 1,
        })
        .collect();

    let payments = if sale.tender == "cash" {
        let amount = sale.total_minor + rounding;
        vec![PaymentDto {
            uid: uuid::Uuid::now_v7().to_string(),
            method: "cash".to_owned(),
            amount_minor: amount,
            tendered_minor: amount,
            change_minor: 0,
        }]
    } else {
        Vec::new()
    };

    SaleDto {
        uid: sale.uid.clone(),
        origin_device: device_id.to_owned(),
        is_offline: 1,
        occurred_at: sale.occurred_at.clone(),
        total_minor: sale.total_minor,
        rounding_minor: rounding,
        discount_minor: discount,
        discount_bp: 0,
        price_mode: "retail".to_owned(),
        is_on_credit: i64::from(sale.tender == "credit"),
        lines,
        payments,
        movements: Vec::new(),
    }
}

/// Push what the till will accept. Offline mobile-money and credit sales are
/// held locally: the till's ingest only accepts a single cash tender offline,
/// and a held sale is never silently discarded.
#[tauri::command]
pub fn sync_now(state: State<'_, AppState>) -> Result<SyncResult, String> {
    let db = state.db.lock().map_err(err)?;
    let conn = db.conn();
    let (endpoint, token) = endpoint(conn)?;
    let id = device_id(conn)?;

    let mut held = 0_i64;
    let mut pushable = Vec::new();
    for sale in db::pending_sales(conn).map_err(err)? {
        if sale.tender != "cash" {
            let reason = format!(
                "{} cannot be pushed offline; confirm it on the main till",
                sale.tender.replace('_', " ")
            );
            db::set_sale_state(conn, &sale.uid, "held", Some(&reason)).map_err(err)?;
            held += 1;
        } else {
            pushable.push(sale_dto(&sale, &id));
        }
    }

    let mut accepted = 0_i64;
    let mut rejected = 0_i64;
    if !pushable.is_empty() {
        let body = serde_json::to_vec(&PushRequest {
            protocol: PROTOCOL,
            sales: pushable.clone(),
        })
        .map_err(err)?;
        let response = Client::new()
            .request(
                &endpoint,
                &Request::post("/v1/sync/sales", body).with_token(token),
            )
            .map_err(|error| {
                let _ = db::set_meta(conn, "offline", "1");
                err(error)
            })?;
        let result: PushResponse = serde_json::from_slice(&response.body).map_err(err)?;

        for uid in &result.accepted {
            db::set_sale_state(conn, uid, "synced", None).map_err(err)?;
            accepted += 1;
        }
        for reject in &result.rejected {
            db::set_sale_state(conn, &reject.uid, "held", Some(&reject.reason)).map_err(err)?;
            rejected += 1;
        }
    }

    db::set_meta(conn, "offline", "0").map_err(err)?;
    db::set_meta(conn, "last_sync", &db::now_string()).map_err(err)?;
    Ok(SyncResult {
        accepted,
        rejected,
        held,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SaleLine;

    #[test]
    fn sale_dto_carries_discount_and_cash_rounding() {
        let sale = Sale {
            uid: "sale-1".to_owned(),
            occurred_at: "2026-11-26T12:00:00.000Z".to_owned(),
            total_minor: 13_050,
            tender: "cash".to_owned(),
            reference: None,
            customer_id: None,
            cashier: "Amina".to_owned(),
            status: "pending".to_owned(),
            unverified: false,
            held_reason: None,
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

        let dto = sale_dto(&sale, "device-1");
        assert_eq!(dto.discount_minor, 450);
        assert_eq!(dto.rounding_minor, 0);
        assert_eq!(dto.total_minor, 13_050);
        assert_eq!(dto.payments.len(), 1);
        assert_eq!(dto.payments[0].amount_minor, 13_050);
        assert_eq!(dto.is_offline, 1);
    }
}
