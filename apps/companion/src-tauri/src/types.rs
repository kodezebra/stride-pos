//! Wire types shared with the Svelte UI. Field names match `src/lib/types.ts`.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Product {
    pub id: String,
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
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Customer {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub discount_bp: i64,
    #[serde(default = "default_credit")]
    pub credit_limit_minor: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Staff {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Caps {
    pub max_sale_minor: i64,
    pub max_mobile_money_minor: i64,
    pub total_mobile_money_minor: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Catalog {
    pub version: i64,
    pub currency: String,
    pub cash_step: i64,
    pub tax_bp: i64,
    pub products: Vec<Product>,
    pub customers: Vec<Customer>,
    pub staff: Vec<Staff>,
    pub caps: Caps,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Status {
    pub paired: bool,
    pub offline: bool,
    pub catalog_version: i64,
    pub last_sync: String,
    pub pending: i64,
    #[serde(default)]
    pub battery: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SaleLine {
    pub name: String,
    pub qty: i64,
    pub unit_minor: i64,
    #[serde(default)]
    pub product_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Sale {
    pub uid: String,
    pub occurred_at: String,
    pub total_minor: i64,
    pub tender: String,
    #[serde(default)]
    pub reference: Option<String>,
    #[serde(default)]
    pub customer_id: Option<String>,
    pub cashier: String,
    pub status: String,
    pub unverified: bool,
    #[serde(default)]
    pub held_reason: Option<String>,
    pub lines: Vec<SaleLine>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct NewSale {
    pub total_minor: i64,
    pub tender: String,
    #[serde(default)]
    pub reference: Option<String>,
    #[serde(default)]
    pub customer_id: Option<String>,
    pub cashier: String,
    pub lines: Vec<SaleLine>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SyncResult {
    pub accepted: i64,
    pub rejected: i64,
    pub held: i64,
}

fn one() -> i64 {
    1
}

fn default_credit() -> i64 {
    120_000
}
