//! Wire shapes for the desktop till's LAN endpoints. Mirrors the relevant
//! structs in apps/desktop/src/sync/protocol.rs — same names, same defaults.
//!
//! Several fields are part of the wire contract but not consumed yet by the
//! companion; they are kept so the shapes stay a faithful mirror.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

pub const PROTOCOL: i64 = 2;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Ping {
    pub ok: bool,
    pub service: String,
    pub protocol: i64,
    pub device_id: String,
    pub device_name: String,
    pub server_time: String,
    pub catalog_owner: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PriceBreakDto {
    pub min_qty: i64,
    pub price_minor: i64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProductDto {
    pub id: i64,
    pub name: String,
    pub price_minor: i64,
    #[serde(default)]
    pub barcode: Option<String>,
    #[serde(default)]
    pub sku: Option<String>,
    pub category: String,
    #[serde(default = "one")]
    pub pack_size: i64,
    #[serde(default)]
    pub pack_price_minor: Option<i64>,
    #[serde(default)]
    pub pack_barcode: Option<String>,
    #[serde(default)]
    pub price_breaks: Vec<PriceBreakDto>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CategoryDto {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub sort_order: i64,
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StaffDto {
    pub id: String,
    pub name: String,
    pub role: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CustomerDto {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub discount_bp: i64,
    #[serde(default)]
    pub archived: i64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CatalogSnapshot {
    #[serde(default)]
    pub protocol: i64,
    pub version: i64,
    pub currency: String,
    #[serde(default)]
    pub currency_exponent: i64,
    pub cash_step_minor: i64,
    pub tax_rate_percent: i64,
    pub products: Vec<ProductDto>,
    #[serde(default)]
    pub categories: Vec<CategoryDto>,
    pub staff: Vec<StaffDto>,
    #[serde(default)]
    pub customers: Vec<CustomerDto>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct LineDto {
    pub uid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product_id: Option<i64>,
    pub name: String,
    pub qty: i64,
    pub unit_minor: i64,
    pub total_minor: i64,
    pub unit_factor: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct PaymentDto {
    pub uid: String,
    pub method: String,
    pub amount_minor: i64,
    pub tendered_minor: i64,
    pub change_minor: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct SaleDto {
    pub uid: String,
    pub origin_device: String,
    pub is_offline: i64,
    pub occurred_at: String,
    pub total_minor: i64,
    pub rounding_minor: i64,
    pub discount_minor: i64,
    pub discount_bp: i64,
    pub price_mode: String,
    pub is_on_credit: i64,
    pub lines: Vec<LineDto>,
    pub payments: Vec<PaymentDto>,
    pub movements: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct PushRequest {
    pub protocol: i64,
    pub sales: Vec<SaleDto>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Reject {
    pub uid: String,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PushResponse {
    pub accepted: Vec<String>,
    pub rejected: Vec<Reject>,
    #[serde(default)]
    pub server_time: String,
}

fn one() -> i64 {
    1
}
