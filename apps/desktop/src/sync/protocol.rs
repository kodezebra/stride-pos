//! The wire format between two tills, and between a phone and a till.
//!
//! This module is deliberately just data: no SQL, no database handle, no policy
//! beyond the protocol number. The same shapes are read by the seller and the
//! buyer, so a field can never mean one thing in a pull response and another in
//! an ingest transaction.
//!
//! Amounts are minor units and timestamps are strings in the database's UTC
//! form. A peer never sends a database rowid as identity: `sales.uid`,
//! `sale_lines.uid`, `payments.uid` and `stock_movements.uid` are the portable
//! ids, and the receiver's integers stay local.

use serde::{Deserialize, Serialize};

/// Bumped when a shape changes in a way an older peer cannot read.
pub const PROTOCOL: i64 = 1;

/// One sale, with the lines, payments and stock movements that belong to it.
///
/// A relayed sale keeps the original `origin_device`: B may pass A's sale on to
/// C, and C must still be able to say where it happened. The receiving till
/// stamps its own local `change_seq`; that cursor never travels.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SaleDto {
    pub uid: String,
    pub origin_device: String,
    /// `1` for a sale rung up without the desktop, otherwise `0`.
    pub is_offline: i64,
    /// When the sale happened at its origin, as the origin's clock saw it.
    pub occurred_at: String,
    #[serde(default)]
    pub user_id: Option<String>,
    pub total_minor: i64,
    #[serde(default)]
    pub rounding_minor: i64,
    #[serde(default)]
    pub tax_minor: i64,
    pub lines: Vec<LineDto>,
    pub payments: Vec<PaymentDto>,
    #[serde(default)]
    pub movements: Vec<MovementDto>,
}

/// One line of a sale. The name and unit price are the receipt's, not a
/// catalog lookup: the catalog may have moved on since the sale.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LineDto {
    pub uid: String,
    #[serde(default)]
    pub product_id: Option<i64>,
    pub name: String,
    pub qty: i64,
    pub unit_minor: i64,
    pub total_minor: i64,
}

/// One tender. A sale may be split, so the method is a field rather than three
/// columns on the sale.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PaymentDto {
    pub uid: String,
    pub method: String,
    pub amount_minor: i64,
    pub tendered_minor: i64,
    pub change_minor: i64,
}

/// One stock movement written by a sale. `delta` is validated against the
/// lines and then recomputed, so the receiver does not have to trust it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MovementDto {
    pub uid: String,
    pub product_id: i64,
    pub delta: i64,
    pub kind: String,
    #[serde(default)]
    pub note: Option<String>,
}

/// A request for a page of the change feed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PullRequest {
    /// `0` is tolerated for clients built before the envelope carried a
    /// version, but new clients always send [`PROTOCOL`].
    #[serde(default)]
    pub protocol: i64,
    pub since_seq: i64,
    pub limit: usize,
    /// The last catalog version the caller applied. A hint only; the response
    /// carries the seller's current version so the caller can decide.
    #[serde(default)]
    pub catalog_version: i64,
}

/// A page of sales plus the seller's identity and catalog version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PullResponse {
    pub protocol: i64,
    pub device_id: String,
    pub device_name: String,
    pub catalog_owner: bool,
    pub catalog_version: i64,
    pub sales: Vec<SaleDto>,
    /// The cursor to send next. Equals `since_seq` when the page is empty.
    pub next_seq: i64,
    /// True when the seller stopped at the page limit and may have more.
    pub has_more: bool,
    pub server_time: String,
}

/// Sales offered to a till. A phone sends its outbox here; a till sends the
/// page it just pulled when relaying to another till.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PushRequest {
    #[serde(default)]
    pub protocol: i64,
    pub sales: Vec<SaleDto>,
}

/// What happened to each offered sale.
///
/// `accepted` includes uids the receiver already knew: that is what lets a
/// phone clear an outbox whose acknowledgement was lost in transit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PushResponse {
    pub protocol: i64,
    pub accepted: Vec<String>,
    pub rejected: Vec<Reject>,
    pub server_time: String,
}

/// A sale the receiver refused, with the reason shown in a log or admin view.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Reject {
    pub uid: String,
    pub reason: String,
}

/// The catalog, as the authoring till publishes it.
///
/// A replica applies products and categories by id and archives what the owner
/// no longer lists. `staff` is a read-only directory for pairing and the phone:
/// it carries no credential or PIN, and a desktop replica does not write it
/// into `users` because a login is local policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CatalogSnapshot {
    #[serde(default)]
    pub protocol: i64,
    pub version: i64,
    pub currency: String,
    pub cash_step_minor: i64,
    pub tax_rate_percent: i64,
    pub products: Vec<ProductDto>,
    pub categories: Vec<CategoryDto>,
    pub staff: Vec<StaffDto>,
}

/// One catalog product. `id` is the authoring till's rowid and is stable for
/// the life of that catalog; a replica uses it as its own id so the same
/// product is the same product on every till.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProductDto {
    pub id: i64,
    pub name: String,
    pub price_minor: i64,
    #[serde(default)]
    pub barcode: Option<String>,
    pub category: String,
    #[serde(default)]
    pub category_id: Option<i64>,
    #[serde(default)]
    pub tracked: i64,
    #[serde(default)]
    pub archived: i64,
}

/// One category row. Categories are upserted by id and never deleted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CategoryDto {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub sort_order: i64,
    #[serde(default)]
    pub color: Option<String>,
}

/// A person the phone may attribute a sale to. No credential, no PIN.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StaffDto {
    pub id: String,
    pub name: String,
    /// The stable stored spelling: `cashier`, `manager`, `owner`.
    pub role: String,
}

/// The generic frame the worker will carry when the replication engine is
/// extracted for a second product.
///
/// It exists now so the boundary is written down rather than discovered later:
/// a change is an opaque payload with a portable id, an origin and a sequence,
/// not a `SaleDto`. POS's worker still deals in sales in Phase 2; the frame is
/// the seam, not a second code path.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ChangeRecord {
    pub uid: String,
    pub origin_device: String,
    pub seq: i64,
    pub occurred_at: String,
    pub payload: serde_json::Value,
}
