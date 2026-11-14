//! End-to-end tests for the headless sync core.
//!
//! These use real SQLite files rather than sockets: the interesting failures
//! are in what is allowed to be written, not in the HTTP framing (which
//! `lan-http` tests separately). Two or three stores joined by direct calls are
//! the same protocol with the wire removed.

use std::path::PathBuf;

use super::ingest::ingest_sales;
use super::protocol::{CatalogSnapshot, PROTOCOL, PullResponse, PushRequest, SaleDto};
use super::worker::{PeerCursor, PeerLink, sync_once};
use crate::db::{self, SaleLine, StockKind};

/// A migrated database that deletes itself when the test ends.
struct Fixture {
    store: storage::Store,
    path: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("pos-sync-{name}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut store = storage::Store::open(&path).expect("open database");
        store.migrate(db::MIGRATIONS).expect("migrate");
        Self { store, path }
    }

    fn db(&self) -> &storage::Store {
        &self.store
    }

    fn db_mut(&mut self) -> &mut storage::Store {
        &mut self.store
    }

    fn scalar<T: rusqlite::types::FromSql>(&self, sql: &str) -> T {
        self.store
            .conn()
            .query_row(sql, [], |row| row.get(0))
            .expect("scalar")
    }

    fn execute(&self, sql: &str) {
        self.store.conn().execute(sql, []).expect("execute");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A catalog with one tracked product that a sale can name.
fn seed_catalog(fixture: &Fixture) {
    fixture.execute("INSERT INTO categories (id, name, sort_order) VALUES (1, 'Drinks', 0)");
    fixture.execute(
        "INSERT INTO products
             (id, name, price_minor, category, category_id, barcode, tracked)
         VALUES (10, 'Tea', 500, 'Drinks', 1, '5901234123457', 1)",
    );
    db::set_currency(
        fixture.db(),
        money::tender_from_code("UGX").expect("UGX is a tender"),
    )
    .expect("set currency");
}

fn record_tea(store: &storage::Store) -> db::RecordedSale {
    db::record_sale(
        store,
        &[SaleLine {
            product_id: Some(10),
            name: "Tea".to_owned(),
            qty: 2,
            unit_minor: 500,
        }],
        0,
        0,
        1_000,
        None,
        None,
    )
    .expect("record sale")
}

fn pulled(store: &storage::Store) -> SaleDto {
    let (sales, _, _) = db::pull_sales(store, 0, 100).expect("pull");
    assert_eq!(sales.len(), 1, "the fixture records exactly one sale");
    sales.into_iter().next().expect("one sale")
}

fn offer(sales: Vec<SaleDto>) -> PushRequest {
    PushRequest {
        protocol: PROTOCOL,
        sales,
    }
}

#[test]
fn a_sale_moves_to_a_peer_and_a_replay_does_nothing() {
    let a = Fixture::new("idempotent-a");
    let b = Fixture::new("idempotent-b");
    seed_catalog(&a);
    seed_catalog(&b);
    record_tea(a.db());

    let sale = pulled(a.db());
    let request = offer(vec![sale.clone()]);

    let first = ingest_sales(b.db(), &request).expect("first ingest");
    assert_eq!(first.accepted, vec![sale.uid.clone()]);
    assert!(first.rejected.is_empty());

    let second = ingest_sales(b.db(), &request).expect("second ingest");
    assert_eq!(
        second.accepted,
        vec![sale.uid.clone()],
        "a replayed uid is acknowledged, not refused"
    );
    assert!(second.rejected.is_empty());

    assert_eq!(b.scalar::<i64>("SELECT COUNT(*) FROM sales"), 1);
    assert_eq!(b.scalar::<i64>("SELECT COUNT(*) FROM sale_lines"), 1);
    assert_eq!(b.scalar::<i64>("SELECT COUNT(*) FROM payments"), 1);
    assert_eq!(
        b.scalar::<i64>("SELECT COUNT(*) FROM stock_movements"),
        1,
        "the ledger is written once, from the lines"
    );
}

#[test]
fn a_relayed_sale_keeps_its_origin_device() {
    let a = Fixture::new("relay-a");
    let b = Fixture::new("relay-b");
    let c = Fixture::new("relay-c");
    seed_catalog(&a);
    seed_catalog(&b);
    seed_catalog(&c);
    record_tea(a.db());

    let origin = db::ensure_device_id(a.db()).expect("origin device");
    let sale = pulled(a.db());
    ingest_sales(b.db(), &offer(vec![sale])).expect("a to b");

    // B offers the sale on its own change feed. It is B's row, but not B's sale.
    let relayed = pulled(b.db());
    ingest_sales(c.db(), &offer(vec![relayed])).expect("b to c");

    let stamped: String = c
        .db()
        .conn()
        .query_row("SELECT origin_device FROM sales", [], |row| row.get(0))
        .expect("origin");
    assert_eq!(stamped, origin, "C must still say where the sale happened");
    assert_ne!(stamped, db::ensure_device_id(c.db()).expect("c's device"));
}

#[test]
fn a_synced_sale_keeps_the_number_it_was_printed_with() {
    let a = Fixture::new("receipt-a");
    let b = Fixture::new("receipt-b");
    seed_catalog(&a);
    seed_catalog(&b);
    record_tea(a.db());

    let sale = pulled(a.db());
    let printed = sale.receipt_no.clone().expect("A numbers its sales");
    ingest_sales(b.db(), &offer(vec![sale])).expect("a to b");

    // B's own counter must not be touched: it did not sell anything, so its
    // next sale is still number one. The relayed slip keeps A's number.
    let stored: Option<String> = b
        .db()
        .conn()
        .query_row("SELECT receipt_no FROM sales", [], |row| row.get(0))
        .expect("stored number");
    assert_eq!(stored.as_deref(), Some(printed.as_str()));

    let relayed = pulled(b.db());
    assert_eq!(relayed.receipt_no.as_deref(), Some(printed.as_str()));

    record_tea(b.db());
    let own: String = b
        .db()
        .conn()
        .query_row(
            "SELECT receipt_no FROM sales ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("B's own sale");
    assert_ne!(own, printed, "B does not reuse A's number");
    assert!(own.ends_with("-001"), "B's counter starts at one: {own}");
    assert_eq!(
        own.split('-').next().map(str::len),
        Some(4),
        "four characters of B's device: {own}"
    );
}

#[test]
fn a_tampered_total_is_refused_and_kept_for_inspection() {
    let a = Fixture::new("bad-total-a");
    let b = Fixture::new("bad-total-b");
    seed_catalog(&a);
    seed_catalog(&b);
    record_tea(a.db());

    let mut sale = pulled(a.db());
    sale.total_minor += 1;

    let response = ingest_sales(b.db(), &offer(vec![sale.clone()])).expect("ingest");
    assert!(response.accepted.is_empty());
    assert_eq!(response.rejected.len(), 1);
    assert_eq!(response.rejected[0].uid, sale.uid);
    assert_eq!(b.scalar::<i64>("SELECT COUNT(*) FROM sales"), 0);
    assert_eq!(
        b.scalar::<i64>("SELECT COUNT(*) FROM sync_rejects"),
        1,
        "the operator can see what was refused"
    );
}

#[test]
fn a_tampered_payment_is_refused() {
    let a = Fixture::new("bad-payment-a");
    let b = Fixture::new("bad-payment-b");
    seed_catalog(&a);
    seed_catalog(&b);
    record_tea(a.db());

    let mut sale = pulled(a.db());
    let payment = &mut sale.payments[0];
    payment.amount_minor -= 100;
    payment.tendered_minor -= 100;

    let response = ingest_sales(b.db(), &offer(vec![sale])).expect("ingest");
    assert_eq!(response.rejected.len(), 1);
    assert!(response.rejected[0].reason.contains("sum"));
    assert_eq!(b.scalar::<i64>("SELECT COUNT(*) FROM sales"), 0);
}

#[test]
fn a_tampered_stock_delta_is_refused() {
    let a = Fixture::new("bad-delta-a");
    let b = Fixture::new("bad-delta-b");
    seed_catalog(&a);
    seed_catalog(&b);
    record_tea(a.db());

    let mut sale = pulled(a.db());
    sale.movements[0].delta += 1;

    let response = ingest_sales(b.db(), &offer(vec![sale])).expect("ingest");
    assert_eq!(response.rejected.len(), 1);
    assert!(response.rejected[0].reason.contains("movement"));
    assert_eq!(b.scalar::<i64>("SELECT COUNT(*) FROM sales"), 0);
}

#[test]
fn the_cursor_pages_through_the_feed_and_stops() {
    let fixture = Fixture::new("paging");
    seed_catalog(&fixture);
    for _ in 0..3 {
        record_tea(fixture.db());
    }

    let (first, next, more) = db::pull_sales(fixture.db(), 0, 2).expect("page one");
    assert_eq!(first.len(), 2);
    assert!(more, "the seller stopped at the limit");
    assert!(next > 0);

    let (second, second_next, more) = db::pull_sales(fixture.db(), next, 2).expect("page two");
    assert_eq!(second.len(), 1);
    assert!(!more);
    assert!(second_next > next);

    let (third, third_next, more) =
        db::pull_sales(fixture.db(), second_next, 2).expect("page three");
    assert!(third.is_empty());
    assert!(!more);
    assert_eq!(third_next, second_next, "an empty page does not move");
}

#[test]
fn an_unknown_product_keeps_the_receipt_but_loses_the_link() {
    let a = Fixture::new("unknown-product-a");
    let b = Fixture::new("unknown-product-b");
    seed_catalog(&a);
    seed_catalog(&b);
    record_tea(a.db());

    let mut sale = pulled(a.db());
    sale.lines[0].product_id = Some(999_999);
    sale.movements.clear();

    let response = ingest_sales(b.db(), &offer(vec![sale])).expect("ingest");
    assert_eq!(response.accepted.len(), 1);
    assert!(response.rejected.is_empty());
    assert_eq!(b.scalar::<i64>("SELECT COUNT(*) FROM sales"), 1);
    let linked: Option<i64> = b
        .db()
        .conn()
        .query_row("SELECT product_id FROM sale_lines", [], |row| row.get(0))
        .expect("line");
    assert_eq!(
        linked, None,
        "the sale is true; the shelf is not its authority"
    );
    assert_eq!(b.scalar::<i64>("SELECT COUNT(*) FROM stock_movements"), 0);
}

#[test]
fn a_phone_clock_in_the_past_is_repaired() {
    let a = Fixture::new("clock-past-a");
    let b = Fixture::new("clock-past-b");
    seed_catalog(&a);
    seed_catalog(&b);
    record_tea(a.db());

    let mut sale = pulled(a.db());
    sale.occurred_at = "2019-01-01T00:00:00.000Z".to_owned();

    let response = ingest_sales(b.db(), &offer(vec![sale])).expect("ingest");
    assert_eq!(response.accepted.len(), 1);

    let (created_at, skew): (String, i64) = b
        .db()
        .conn()
        .query_row("SELECT created_at, clock_skew FROM sales", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .expect("sale");
    let today: String = b
        .db()
        .conn()
        .query_row("SELECT date('now')", [], |row| row.get(0))
        .expect("today");
    assert!(created_at.starts_with(&today), "dated today, not 2019");
    assert_eq!(skew, 1, "the origin's clock is recorded as untrusted");
}

#[test]
fn a_reasonable_origin_time_is_kept() {
    let a = Fixture::new("clock-ok-a");
    let b = Fixture::new("clock-ok-b");
    seed_catalog(&a);
    seed_catalog(&b);
    record_tea(a.db());

    let mut sale = pulled(a.db());
    let now: String = a
        .db()
        .conn()
        .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
            row.get(0)
        })
        .expect("now");
    sale.occurred_at = now.clone();

    ingest_sales(b.db(), &offer(vec![sale])).expect("ingest");

    let (created_at, skew): (String, i64) = b
        .db()
        .conn()
        .query_row("SELECT created_at, clock_skew FROM sales", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .expect("sale");
    let canonical: String = b
        .db()
        .conn()
        .query_row("SELECT datetime(?1)", [&now], |row| row.get(0))
        .expect("canonical");
    assert_eq!(created_at, canonical);
    assert_eq!(skew, 0);
}

#[test]
fn a_catalog_snapshot_upserts_and_archives_without_losing_history() {
    let a = Fixture::new("catalog-a");
    let b = Fixture::new("catalog-b");
    seed_catalog(&a);
    a.execute(
        "INSERT INTO products
             (id, name, price_minor, category, category_id, tracked)
         VALUES (20, 'Coffee', 700, 'Drinks', 1, 1)",
    );

    let first = db::catalog_snapshot(a.db(), 0).expect("snapshot");
    assert_eq!(first.currency, "UGX");
    assert_eq!(first.products.len(), 2);
    db::apply_catalog_snapshot(b.db(), &first).expect("apply");

    assert_eq!(b.scalar::<i64>("SELECT COUNT(*) FROM products"), 2);
    assert_eq!(
        b.scalar::<String>("SELECT value FROM meta WHERE key = 'currency'"),
        "UGX"
    );

    db::record_stock_movement(b.db(), 20, 10, StockKind::Receive, "delivery", None)
        .expect("receive coffee");

    db::delete_product(a.db(), 20).expect("archive coffee");
    let second = db::catalog_snapshot(a.db(), 0).expect("second snapshot");
    assert!(
        second.version > first.version,
        "archiving a product is a catalog change"
    );
    db::apply_catalog_snapshot(b.db(), &second).expect("re-apply");

    assert_eq!(
        b.scalar::<i64>("SELECT archived FROM products WHERE id = 20"),
        1,
        "the owner's archive reached the replica"
    );
    assert_eq!(
        b.scalar::<i64>("SELECT COUNT(*) FROM stock_movements WHERE product_id = 20"),
        1,
        "an archive is not a delete"
    );
    assert_eq!(
        b.scalar::<i64>("SELECT COUNT(*) FROM products"),
        2,
        "the row survives for history"
    );
    let live: Vec<i64> = db::products(b.db()).into_iter().map(|p| p.id).collect();
    assert_eq!(live, vec![10], "the shop no longer lists coffee");
}

#[test]
fn a_catalog_snapshot_cannot_renumber_an_existing_currency() {
    let a = Fixture::new("catalog-currency-a");
    let b = Fixture::new("catalog-currency-b");
    seed_catalog(&a);
    db::set_currency(
        b.db(),
        money::tender_from_code("KES").expect("KES is a tender"),
    )
    .expect("set currency");

    let snapshot = db::catalog_snapshot(a.db(), 0).expect("snapshot");
    let error = db::apply_catalog_snapshot(b.db(), &snapshot).expect_err("must refuse");
    assert!(error.to_string().contains("prices in"), "{error}");
    assert_eq!(
        b.scalar::<i64>("SELECT COUNT(*) FROM products"),
        0,
        "nothing was applied"
    );
}

#[test]
fn the_worker_applies_the_catalog_before_the_sales() {
    let a = Fixture::new("worker-a");
    let mut b = Fixture::new("worker-b");
    seed_catalog(&a);
    record_tea(a.db());

    let snapshot = db::catalog_snapshot(a.db(), 0).expect("snapshot");
    let sale = pulled(a.db());
    let peer = FakePeer {
        page: PullResponse {
            protocol: PROTOCOL,
            device_id: db::ensure_device_id(a.db()).expect("device"),
            device_name: "A".to_owned(),
            catalog_owner: true,
            catalog_version: snapshot.version,
            next_seq: 1,
            has_more: false,
            sales: vec![sale],
            server_time: "2026-01-01T00:00:00.000Z".to_owned(),
        },
        catalog: snapshot.clone(),
    };

    let mut cursor = PeerCursor::default();
    sync_once(b.db_mut(), &peer, &mut cursor).expect("sync");

    assert_eq!(cursor.since_seq, 1);
    assert_eq!(cursor.catalog_version, snapshot.version);
    let linked: Option<i64> = b
        .db()
        .conn()
        .query_row("SELECT product_id FROM sale_lines", [], |row| row.get(0))
        .expect("line");
    assert_eq!(
        linked,
        Some(10),
        "the catalog was in place before the sale was applied"
    );
    assert_eq!(b.scalar::<i64>("SELECT COUNT(*) FROM stock_movements"), 1);
}

#[test]
fn the_worker_refuses_a_peer_that_never_advances_the_cursor() {
    let a = Fixture::new("stuck-a");
    let mut b = Fixture::new("stuck-b");
    seed_catalog(&a);
    record_tea(a.db());

    let snapshot = db::catalog_snapshot(a.db(), 0).expect("snapshot");
    let page = PullResponse {
        protocol: PROTOCOL,
        device_id: "peer".to_owned(),
        device_name: "A".to_owned(),
        catalog_owner: true,
        catalog_version: snapshot.version,
        next_seq: 0,
        has_more: true,
        sales: Vec::new(),
        server_time: "2026-01-01T00:00:00.000Z".to_owned(),
    };
    let peer = FakePeer {
        page,
        catalog: snapshot,
    };

    let error = sync_once(b.db_mut(), &peer, &mut PeerCursor::default())
        .expect_err("must not spin forever");
    assert!(error.to_string().contains("advance"), "{error}");
}

struct FakePeer {
    page: PullResponse,
    catalog: CatalogSnapshot,
}

impl PeerLink for FakePeer {
    fn pull(
        &self,
        _since_seq: i64,
        _limit: usize,
        _catalog_version: i64,
    ) -> anyhow::Result<PullResponse> {
        Ok(self.page.clone())
    }

    fn catalog(&self) -> anyhow::Result<CatalogSnapshot> {
        Ok(self.catalog.clone())
    }
}
