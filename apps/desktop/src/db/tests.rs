//! Tests for the db layer: schema, queries, and the rules each one holds.

use super::*;
use crate::auth::{CredentialKind, Role};
use std::path::PathBuf;
use storage::Migration;

/// The pre-pack call shape, kept for the many tests that only care about a
/// name, a price and a barcode. The production entry point takes the whole
/// set of fields; these wrappers fill the rest with the neutral defaults.
fn insert_product(
    store: &storage::Store,
    name: &str,
    price_minor: i64,
    category: &str,
    category_id: Option<i64>,
    barcode: Option<&str>,
) -> anyhow::Result<i64> {
    super::insert_product(
        store,
        &ProductFields {
            name,
            price_minor,
            category,
            category_id,
            barcode,
            sku: None,
            unit_label: "unit",
            pack_size: 1,
            pack_price_minor: None,
            pack_barcode: None,
            tiers: &[],
        },
    )
}

fn update_product(
    store: &storage::Store,
    id: i64,
    name: &str,
    price_minor: i64,
    category: &str,
    category_id: Option<i64>,
    barcode: Option<&str>,
) -> anyhow::Result<()> {
    super::update_product(
        store,
        id,
        &ProductFields {
            name,
            price_minor,
            category,
            category_id,
            barcode,
            sku: None,
            unit_label: "unit",
            pack_size: 1,
            pack_price_minor: None,
            pack_barcode: None,
            tiers: &[],
        },
    )
}

/// A real file, not `:memory:`: the till runs with pragmas that only exist
/// on disk, and a test that skips them is testing a different database.
fn store(name: &str) -> (storage::Store, PathBuf) {
    let path = std::env::temp_dir().join(format!("pos-{name}-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut store = storage::Store::open(&path).expect("open database");
    store.migrate(MIGRATIONS).expect("migrate");
    (store, path)
}

/// Four sales, each dated relative to now so the test never rots: today,
/// three days ago, ten days ago, forty days ago.
fn seed(store: &storage::Store) {
    for (offset, total_minor) in [
        ("-0 days", 500_i64),
        ("-3 days", 700),
        ("-10 days", 1100),
        ("-40 days", 2000),
    ] {
        store
            .conn()
            .execute(
                "INSERT INTO sales (total_minor, created_at) VALUES (?1, datetime('now', ?2))",
                (total_minor, offset),
            )
            .expect("insert sale");
    }
}

#[test]
fn ranges_select_the_window_they_advertise() {
    let (store, path) = store("ranges");
    seed(&store);

    assert_eq!(totals(&store, 0, None), (1, 500), "Today");
    assert_eq!(totals(&store, 1, None), (2, 1200), "7 days");
    assert_eq!(totals(&store, 2, None), (3, 2300), "30 days");
    assert_eq!(totals(&store, 3, None), (4, 4300), "All time");

    assert_eq!(sales(&store, 1, 10, None).len(), 2);

    let two = sales(&store, 3, 2, None);
    assert_eq!(two.len(), 2, "the limit applies after the filter");
    assert!(two[0].id > two[1].id, "newest first");

    let _ = std::fs::remove_file(path);
}

#[test]
fn the_previous_window_is_whole_and_sits_before_the_range() {
    let (store, path) = store("previous-window");
    insert_sale(&store, "-0 days", &[("Tea", 1, 500)]);
    insert_sale(&store, "-3 days", &[("Tea", 1, 700)]);
    insert_sale(&store, "-10 days", &[("Tea", 2, 550)]);
    insert_sale(&store, "-40 days", &[("Tea", 4, 500)]);

    assert_eq!(
        previous_totals(&store, 0, None),
        Some((0, 0)),
        "yesterday was quiet, not missing"
    );
    assert_eq!(
        previous_totals(&store, 1, None),
        Some((1, 1_100)),
        "the whole week before this one"
    );
    assert_eq!(
        previous_totals(&store, 2, None),
        Some((1, 2_000)),
        "the whole month before this one"
    );
    assert_eq!(
        previous_totals(&store, 3, None),
        None,
        "nothing came before all time"
    );

    let _ = std::fs::remove_file(path);
}

/// A sale that knows whose it was — the scope tests need a cashier to
/// narrow to, and `insert_sale` deliberately leaves that column empty.
fn insert_sale_for(
    store: &storage::Store,
    offset: &str,
    user: &str,
    lines: &[(&str, i64, i64)],
) -> i64 {
    let conn = store.conn();
    let total: i64 = lines.iter().map(|(_, qty, unit)| qty * unit).sum();
    conn.execute(
        "INSERT INTO sales (total_minor, created_at, user_id)
         VALUES (?1, datetime('now', ?2), ?3)",
        (total, offset, user),
    )
    .expect("insert sale");
    let id = conn.last_insert_rowid();
    for (name, qty, unit) in lines {
        conn.execute(
            "INSERT INTO sale_lines (sale_id, product_id, name, qty, unit_minor, total_minor)
             VALUES (?1, NULL, ?2, ?3, ?4, ?5)",
            (id, name, *qty, *unit, qty * unit),
        )
        .expect("insert line");
    }
    id
}

#[test]
fn a_scope_narrows_every_report_to_one_person() {
    let (store, path) = store("report-scope");
    let alice = insert_user(
        &store,
        "Alice",
        Role::Cashier,
        CredentialKind::Pin,
        "$argon2id$placeholder",
    )
    .expect("insert alice");
    let bob = insert_user(
        &store,
        "Bob",
        Role::Cashier,
        CredentialKind::Pin,
        "$argon2id$placeholder",
    )
    .expect("insert bob");

    let alice_today = insert_sale_for(&store, "-0 days", &alice.id, &[("Tea", 2, 100)]);
    let bob_today = insert_sale_for(&store, "-0 days", &bob.id, &[("Rice", 1, 500)]);
    insert_sale_for(&store, "-3 days", &alice.id, &[("Tea", 3, 100)]);

    // One payment each, so the method card has figures to scope.
    for (sale, amount) in [(alice_today, 200), (bob_today, 500)] {
        store
            .conn()
            .execute(
                "INSERT INTO payments (sale_id, method, amount_minor, tendered_minor, change_minor)
                 VALUES (?1, 'cash', ?2, ?2, 0)",
                (sale, amount),
            )
            .expect("insert payment");
    }

    assert_eq!(totals(&store, 3, None), (3, 1_000));
    assert_eq!(totals(&store, 3, Some(&alice.id)), (2, 500));
    assert_eq!(totals(&store, 3, Some(&bob.id)), (1, 500));

    assert_eq!(sales(&store, 3, 10, Some(&alice.id)).len(), 2);
    assert!(
        sales(&store, 3, 10, Some(&alice.id))
            .iter()
            .all(|sale| sale.cashier.as_deref() == Some("Alice"))
    );

    let items = top_items(&store, 3, 10, Some(&alice.id));
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].qty, 5);

    let methods = by_method(&store, 3, Some(&alice.id));
    assert_eq!(methods.len(), 1);
    assert_eq!(methods[0].amount_minor, 200, "only Alice's payment");
    assert_eq!(
        by_method(&store, 3, None)
            .iter()
            .map(|method| method.amount_minor)
            .sum::<i64>(),
        700
    );

    let rows = export_sales(&store, 3, Some(&alice.id)).expect("export");
    assert_eq!(rows.len(), 2, "an export cannot outrun the scope");

    // The person table is scoped like the rest: Alice's two sales, cash and
    // all; the whole shop sees both names.
    let people = by_person(&store, 3, None);
    assert_eq!(people.len(), 2);
    let alice_row = people
        .iter()
        .find(|person| person.name == "Alice")
        .expect("Alice rang two sales");
    assert_eq!(alice_row.sales, 2);
    assert_eq!(alice_row.takings_minor, 500);
    assert_eq!(alice_row.cash_minor, 200, "only the paid sale had cash");
    let mine = by_person(&store, 3, Some(&alice.id));
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].name, "Alice");

    let _ = std::fs::remove_file(path);
}

#[test]
fn by_person_separates_cash_from_takings_and_names_the_unattributed() {
    let (store, path) = store("by-person");
    let alice = insert_user(
        &store,
        "Alice",
        Role::Cashier,
        CredentialKind::Pin,
        "$argon2id$placeholder",
    )
    .expect("insert alice");
    let bob = insert_user(
        &store,
        "Bob",
        Role::Cashier,
        CredentialKind::Pin,
        "$argon2id$placeholder",
    )
    .expect("insert bob");

    let alice_cash = insert_sale_for(&store, "-0 days", &alice.id, &[("Tea", 2, 100)]);
    let alice_mobile = insert_sale_for(&store, "-0 days", &alice.id, &[("Rice", 1, 500)]);
    let bob_cash = insert_sale_for(&store, "-0 days", &bob.id, &[("Sugar", 1, 1_000)]);
    let unattributed = insert_sale(&store, "-0 days", &[("Salt", 1, 50)]);

    for (sale, method, amount) in [
        (alice_cash, "cash", 200),
        (alice_mobile, "mobile", 500),
        (bob_cash, "cash", 1_000),
        (unattributed, "cash", 50),
    ] {
        store
            .conn()
            .execute(
                "INSERT INTO payments (sale_id, method, amount_minor, tendered_minor, change_minor)
                 VALUES (?1, ?2, ?3, ?3, 0)",
                (sale, method, amount),
            )
            .expect("insert payment");
    }

    let people = by_person(&store, 3, None);
    assert_eq!(people.len(), 3, "two cashiers and one sale without one");
    assert_eq!(people[0].name, "Bob", "ranked by takings, not by name");
    assert_eq!(people[0].sales, 1);
    assert_eq!(people[0].cash_minor, 1_000);
    assert_eq!(people[0].takings_minor, 1_000);

    assert_eq!(people[1].name, "Alice");
    assert_eq!(people[1].sales, 2);
    assert_eq!(
        people[1].cash_minor, 200,
        "the mobile sale is not drawer cash"
    );
    assert_eq!(people[1].takings_minor, 700, "but it is still her taking");

    assert_eq!(people[2].name, "No cashier");
    assert_eq!(people[2].takings_minor, 50);

    let _ = std::fs::remove_file(path);
}

#[test]
fn the_drawer_split_covers_one_shift_and_leaves_the_others_alone() {
    let (store, path) = store("drawer-by-person");
    let alice = insert_user(
        &store,
        "Alice",
        Role::Cashier,
        CredentialKind::Pin,
        "$argon2id$placeholder",
    )
    .expect("insert alice");
    let bob = insert_user(
        &store,
        "Bob",
        Role::Cashier,
        CredentialKind::Pin,
        "$argon2id$placeholder",
    )
    .expect("insert bob");

    let sale = |name: &str, unit_minor: i64| SaleLine {
        product_id: None,
        name: name.to_owned(),
        qty: 1,
        unit_minor,
        unit_factor: 1,
        unit_label: None,
    };
    let ring = |user: &str, shift: &str, name: &str, amount: i64| {
        record_sale(
            &store,
            &[sale(name, amount)],
            &SaleTerms {
                rounding_minor: 0,
                tax_minor: 0,
                tendered_minor: amount,
                user_id: Some(user),
                shift_uid: Some(shift),
                ..SaleTerms::default()
            },
        )
        .expect("sale");
    };

    ring(&alice.id, "drawer-a", "Tea", 200);
    ring(&bob.id, "drawer-a", "Rice", 500);
    ring(&alice.id, "drawer-b", "Sugar", 900);

    let people = drawer_by_person(&store, "drawer-a");
    assert_eq!(people.len(), 2);
    assert_eq!(
        people.iter().map(|person| person.cash_minor).sum::<i64>(),
        700,
        "only the drawer being counted"
    );

    let tomorrow = drawer_by_person(&store, "drawer-b");
    assert_eq!(tomorrow.len(), 1);
    assert_eq!(tomorrow[0].name, "Alice");
    assert_eq!(tomorrow[0].cash_minor, 900);

    assert!(drawer_by_person(&store, "nobody").is_empty());

    let _ = std::fs::remove_file(path);
}

#[test]
fn the_busiest_hour_is_the_one_with_the_most_sales() {
    let (store, path) = store("peak-hour");
    insert_sale(&store, "-0 days", &[("Tea", 1, 100)]);
    insert_sale(&store, "-0 days", &[("Tea", 1, 100)]);
    // A third sale, well away from this hour so it cannot tie the peak.
    store
        .conn()
        .execute(
            "INSERT INTO sales (total_minor, created_at)
             VALUES (100, datetime('now', '-3 days', '-5 hours'))",
            [],
        )
        .expect("insert sale");

    let peak = peak_hour(&store, 1, None).expect("two sales share an hour");
    assert_eq!(peak.sales, 2);
    assert!(peak.hour <= 23);

    // Narrowed to a window with no sales, there is no busiest hour.
    assert!(peak_hour(&store, 0, Some("nobody")).is_none());

    let _ = std::fs::remove_file(path);
}

#[test]
fn the_currency_is_stored_once_and_read_back() {
    let (store, path) = store("currency");

    assert_eq!(currency(&store), None, "a fresh install has not chosen yet");
    assert!(!has_sales(&store));

    let ugx = money::tender_from_code("UGX").expect("UGX is a tender");
    let kes = money::tender_from_code("KES").expect("KES is a tender");

    set_currency(&store, ugx).expect("store currency");
    assert_eq!(currency(&store), Some(ugx));

    // A second choice replaces the row rather than adding one.
    set_currency(&store, kes).expect("replace currency");
    assert_eq!(currency(&store), Some(kes));

    seed(&store);
    assert!(has_sales(&store));

    let _ = std::fs::remove_file(path);
}

#[test]
fn an_unknown_stored_currency_reads_as_not_chosen() {
    let (store, path) = store("bad-currency");

    store
        .conn()
        .execute(
            "INSERT INTO meta (key, value) VALUES ('currency', 'NOPE')",
            [],
        )
        .expect("insert meta");

    assert_eq!(currency(&store), None);

    let _ = std::fs::remove_file(path);
}

#[test]
fn a_fresh_database_reports_zero_rather_than_failing() {
    let (store, path) = store("empty");

    assert_eq!(totals(&store, 0, None), (0, 0));
    assert_eq!(previous_totals(&store, 0, None), Some((0, 0)));
    assert!(sales(&store, 3, 5, None).is_empty());
    assert!(by_day(&store, 3, None).is_empty());
    assert_eq!(
        by_day(&store, 1, None).len(),
        7,
        "a quiet week is still seven bars"
    );
    assert_eq!(by_day(&store, 1, None)[0].revenue, 0);
    assert!(peak_hour(&store, 0, None).is_none());
    assert!(top_items(&store, 3, 5, None).is_empty());
    assert!(by_method(&store, 3, None).is_empty());

    let _ = std::fs::remove_file(path);
}

/// A sale with its lines, dated relative to now — `sale_lines` carries the
/// name, so nothing has to exist in the catalog for this to be real.
fn insert_sale(store: &storage::Store, offset: &str, lines: &[(&str, i64, i64)]) -> i64 {
    let conn = store.conn();
    let total: i64 = lines.iter().map(|(_, qty, unit)| qty * unit).sum();
    conn.execute(
        "INSERT INTO sales (total_minor, created_at) VALUES (?1, datetime('now', ?2))",
        (total, offset),
    )
    .expect("insert sale");
    let id = conn.last_insert_rowid();
    for (name, qty, unit) in lines {
        conn.execute(
            "INSERT INTO sale_lines (sale_id, product_id, name, qty, unit_minor, total_minor)
             VALUES (?1, NULL, ?2, ?3, ?4, ?5)",
            (id, name, *qty, *unit, qty * unit),
        )
        .expect("insert line");
    }
    id
}

#[test]
fn a_week_is_seven_calendar_days_not_168_hours() {
    let (store, path) = store("week-window");

    insert_sale(&store, "-0 days", &[("Tea", 1, 100)]);
    insert_sale(&store, "-6 days", &[("Tea", 1, 100)]);
    insert_sale(&store, "-7 days", &[("Tea", 1, 100)]);

    let week = by_day(&store, 1, None);
    assert_eq!(week.len(), 7, "a week is seven bars, zero days included");
    assert_eq!(
        week.iter().filter(|day| day.sales > 0).count(),
        2,
        "today and six days back"
    );
    assert_eq!(totals(&store, 1, None), (2, 200));
    let month = by_day(&store, 2, None);
    assert_eq!(month.len(), 30, "a month is its calendar, not its sales");
    assert_eq!(
        month.iter().filter(|day| day.sales > 0).count(),
        3,
        "the seventh day is still inside the month"
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn the_trend_and_the_top_items_follow_their_sales() {
    let (store, path) = store("analytics");

    // 3500 today, 1000 three days ago, 5000 ten days ago, 500 forty days ago.
    insert_sale(&store, "-0 days", &[("Tea", 3, 500), ("Sugar", 1, 2_000)]);
    insert_sale(&store, "-3 days", &[("Tea", 2, 500)]);
    insert_sale(&store, "-10 days", &[("Rice", 1, 5_000)]);
    insert_sale(&store, "-40 days", &[("Tea", 1, 500)]);

    let today = by_day(&store, 0, None);
    assert_eq!(
        today.len(),
        1,
        "today is one bar, however many sales it had"
    );
    assert_eq!(today[0].sales, 1, "two sales on one day share its row");
    let month = by_day(&store, 2, None);
    assert_eq!(month.len(), 30, "a month is its calendar");
    assert_eq!(
        month.iter().filter(|day| day.sales > 0).count(),
        3,
        "the forty-day-old sale is outside a 30-day range"
    );

    let days = by_day(&store, 3, None);
    assert_eq!(days.len(), 4);
    assert_eq!(
        days[0].revenue, 3_500,
        "newest first, like every other list"
    );
    assert_eq!(days[0].sales, 1, "two sales on one day share its row");
    assert_eq!(days[1].revenue, 1_000);
    assert_eq!(days[3].revenue, 500);

    let items = top_items(&store, 3, 5, None);
    assert_eq!(
        items.len(),
        3,
        "the same product on different days is one item, not three"
    );
    assert_eq!(items[0].name, "Rice", "revenue ranks first, not units");
    assert_eq!(items[0].revenue, 5_000);
    assert_eq!(items[1].name, "Tea");
    assert_eq!(items[1].qty, 6, "units sum across every sale in the range");
    assert_eq!(items[1].revenue, 3_000);
    assert_eq!(items[2].name, "Sugar");
    assert_eq!(items[2].qty, 1);

    assert_eq!(
        top_items(&store, 0, 5, None)[0].name,
        "Sugar",
        "the ranking follows the chosen range, not all history"
    );
    assert_eq!(
        top_items(&store, 3, 2, None).len(),
        2,
        "the limit ranks, not hides"
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn products_round_trip_and_stay_sorted_by_name() {
    let (store, path) = store("products");

    assert!(products(&store).is_empty(), "a fresh till has no catalog");
    assert!(categories(&store).is_empty());

    let flat_white =
        insert_product(&store, "Flat white", 3_400, "Coffee", None, None).expect("insert");
    insert_product(&store, "Croissant", 3_200, "Bakery", None, Some("4001")).expect("insert");
    insert_product(&store, "Espresso", 2_800, "Coffee", None, None).expect("insert");

    let names: Vec<String> = products(&store)
        .into_iter()
        .map(|product| product.name)
        .collect();
    assert_eq!(
        names,
        ["Croissant", "Espresso", "Flat white"],
        "the list is by name, not by insertion order"
    );
    let listed = categories(&store);
    let shelves: Vec<(&str, usize)> = listed
        .iter()
        .map(|shelf| (shelf.name.as_str(), shelf.count))
        .collect();
    assert_eq!(shelves, [("Bakery", 1), ("Coffee", 2)]);

    update_product(
        &store,
        flat_white,
        "Flat white",
        3_600,
        "Coffee",
        None,
        Some("4002"),
    )
    .expect("update");
    let updated = products(&store)
        .into_iter()
        .find(|product| product.id == flat_white)
        .expect("the product survived the update");
    assert_eq!(updated.price_minor, 3_600);
    assert_eq!(updated.barcode.as_deref(), Some("4002"));

    delete_product(&store, flat_white).expect("delete");
    assert_eq!(products(&store).len(), 2);

    let _ = std::fs::remove_file(path);
}

#[test]
fn a_barcode_belongs_to_one_product() {
    let (store, path) = store("barcodes");
    let espresso =
        insert_product(&store, "Espresso", 2_800, "Coffee", None, Some("4001")).expect("insert");

    assert!(barcode_taken(&store, "4001", None));
    assert!(
        !barcode_taken(&store, "4001", Some(espresso)),
        "a product keeps its own barcode when it is edited"
    );
    assert!(!barcode_taken(&store, "4002", None));

    assert!(
        insert_product(&store, "Croissant", 3_200, "Bakery", None, Some("4001")).is_err(),
        "the unique index is the real guard, not the pre-check"
    );

    // Products without a barcode are not competing for one.
    insert_product(&store, "Filter", 2_600, "Coffee", None, None).expect("insert");
    insert_product(&store, "Lemonade", 3_000, "Cold", None, None).expect("insert");

    // Archiving frees the barcode: the old product is no longer sellable,
    // and refusing a live one because of a hidden one is a dead end.
    delete_product(&store, espresso).expect("archive");
    assert!(!barcode_taken(&store, "4001", None));
    insert_product(&store, "Espresso", 2_800, "Coffee", None, Some("4001"))
        .expect("the archived product's barcode is reusable");

    let _ = std::fs::remove_file(path);
}

#[test]
fn the_database_refuses_a_negative_price() {
    let (store, path) = store("negative-price");

    assert!(
        insert_product(&store, "Refund", -100, "Coffee", None, None).is_err(),
        "a price is a non-negative amount; the CHECK is the guard"
    );

    let _ = std::fs::remove_file(path);
}

fn supplier_fields(name: &str) -> SupplierFields<'_> {
    SupplierFields {
        name,
        contact_name: "",
        phone: "",
        email: "",
        address: "",
        notes: "",
    }
}

#[test]
fn suppliers_round_trip_and_archive() {
    let (store, path) = store("suppliers");
    assert!(
        suppliers(&store).is_empty(),
        "a fresh till has no suppliers"
    );

    let mut acme_fields = supplier_fields("  Acme  ");
    acme_fields.contact_name = "Amina";
    acme_fields.phone = "+256 700 000";
    acme_fields.email = "orders@acme.test";
    acme_fields.address = "Kampala";
    acme_fields.notes = "Delivers on Tuesdays";
    let acme = insert_supplier(&store, &acme_fields).expect("insert");
    assert_eq!(acme.name, "Acme", "the name is trimmed before it is stored");
    assert_eq!(acme.phone.as_deref(), Some("+256 700 000"));
    assert!(!acme.archived);

    insert_supplier(&store, &supplier_fields("Zebra")).expect("insert");
    let names: Vec<String> = suppliers(&store)
        .into_iter()
        .map(|supplier| supplier.name)
        .collect();
    assert_eq!(
        names,
        ["Acme", "Zebra"],
        "the list is by name, not by insertion order"
    );
    let zebra = suppliers(&store)
        .into_iter()
        .find(|supplier| supplier.name == "Zebra")
        .expect("the supplier was written");
    assert_eq!(
        zebra.phone, None,
        "a blank optional field is stored as NULL"
    );
    assert_eq!(zebra.notes, None);

    let mut updated_fields = supplier_fields("Acme Ltd");
    updated_fields.contact_name = "Amina";
    updated_fields.email = "orders@acme.test";
    update_supplier(&store, &acme.id, &updated_fields).expect("update");
    let updated = suppliers(&store)
        .into_iter()
        .find(|supplier| supplier.id == acme.id)
        .expect("the supplier survived the update");
    assert_eq!(updated.name, "Acme Ltd");
    assert_eq!(updated.phone, None, "clearing a field clears it");

    set_supplier_archived(&store, &acme.id, true).expect("archive");
    assert!(
        suppliers(&store)
            .iter()
            .find(|supplier| supplier.id == acme.id)
            .expect("archived rows stay readable")
            .archived
    );
    set_supplier_archived(&store, &acme.id, false).expect("restore");
    assert!(!suppliers(&store)[0].archived);

    let _ = std::fs::remove_file(path);
}

#[test]
fn a_supplier_name_belongs_to_one_supplier() {
    let (store, path) = store("supplier-names");
    let acme = insert_supplier(&store, &supplier_fields("Acme")).expect("insert");

    assert!(supplier_name_taken(&store, "acme", None), "case is ignored");
    assert!(
        !supplier_name_taken(&store, "ACME", Some(&acme.id)),
        "a supplier keeps its own name when it is edited"
    );
    assert!(!supplier_name_taken(&store, "Other", None));
    assert!(
        insert_supplier(&store, &supplier_fields("ACME")).is_err(),
        "the unique index is the real guard, not the pre-check"
    );

    assert!(
        insert_supplier(&store, &supplier_fields("   ")).is_err(),
        "a supplier with no name is not a supplier"
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn editing_a_missing_supplier_fails() {
    let (store, path) = store("missing-supplier");

    assert!(
        update_supplier(&store, "missing", &supplier_fields("Name")).is_err(),
        "'Saved' over a row that is gone is a lie"
    );
    assert!(set_supplier_archived(&store, "missing", true).is_err());

    let _ = std::fs::remove_file(path);
}

/// The bug that sent us here: migration 6 was recorded as applied while a
/// previous build's `suppliers` table was still in place, so every write
/// failed on the missing `contact_name`. The repair must keep the rows and
/// leave a table the new code can use.
#[test]
fn an_intermediate_supplier_table_is_repaired_without_losing_rows() {
    let (store, path) = store("supplier-repair");

    // The shape the unfinished build left behind: integer ids, one
    // `contact` column, and no unique index.
    store
        .conn()
        .execute_batch(
            "DROP TABLE suppliers;
             CREATE TABLE suppliers (
                 id      INTEGER PRIMARY KEY,
                 name    TEXT NOT NULL CHECK (length(name) > 0),
                 contact TEXT,
                 phone   TEXT,
                 email   TEXT
             );
             INSERT INTO suppliers (id, name, contact, phone, email)
                 VALUES (7, 'Acme', 'Jane', '0700', 'jane@acme.test'),
                        (8, 'acme', 'John', '0800', NULL);",
        )
        .expect("intermediate schema");

    repair_suppliers(&store).expect("repair");

    let repaired = suppliers(&store);
    assert_eq!(repaired.len(), 2, "rows are kept");
    assert_eq!(
        repaired[0].id, "7",
        "an integer id becomes the string the schema uses"
    );
    assert_eq!(repaired[0].contact_name.as_deref(), Some("Jane"));
    assert_eq!(repaired[0].phone.as_deref(), Some("0700"));
    assert_eq!(repaired[0].email.as_deref(), Some("jane@acme.test"));
    assert_eq!(
        repaired[1].name, "acme (2)",
        "a duplicate name is made unique"
    );
    assert!(supplier_name_taken(&store, "ACME", None));

    // Running it again changes nothing: boot calls it every time.
    repair_suppliers(&store).expect("second repair");
    assert_eq!(suppliers(&store).len(), 2);
    insert_supplier(&store, &supplier_fields("Beta")).expect("the repaired table accepts writes");

    let _ = std::fs::remove_file(path);
}

#[test]
fn a_sale_writes_its_lines_and_its_payment() {
    let (store, path) = store("record-sale");
    let espresso =
        insert_product(&store, "Espresso", 2_800, "Coffee", None, None).expect("insert product");

    let lines = [
        SaleLine {
            product_id: Some(espresso),
            name: "Espresso".to_owned(),
            qty: 1,
            unit_minor: 2_800,
            unit_factor: 1,
            unit_label: None,
        },
        SaleLine {
            product_id: None,
            name: "Unlisted bun".to_owned(),
            qty: 2,
            unit_minor: 500,
            unit_factor: 1,
            unit_label: None,
        },
    ];

    // 2800 + 1000 = 3800 exact; rounded down 20 to the 100 shilling coin.
    let sale = record_sale(
        &store,
        &lines,
        &SaleTerms {
            rounding_minor: -20,
            tax_minor: 0,
            tendered_minor: 4_000,
            user_id: None,
            shift_uid: None,
            ..SaleTerms::default()
        },
    )
    .expect("record");

    assert_eq!(sale.total_minor, 3_800, "the exact total is kept exact");
    assert_eq!(sale.paid_minor(), 3_780);
    assert_eq!(sale.change_minor, 220, "4000 tendered against 3780 due");

    let (count, revenue) = totals(&store, 3, None);
    assert_eq!(count, 1);
    assert_eq!(revenue, 3_780, "revenue is what the drawer took");

    let history = sales(&store, 3, 10, None);
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].paid_minor, 3_780);
    assert_eq!(history[0].id, sale.id);

    let line_count: i64 = store
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM sale_lines WHERE sale_id = ?1",
            [sale.id],
            |row| row.get(0),
        )
        .expect("count lines");
    assert_eq!(line_count, 2);

    let (method, amount, tendered): (String, i64, i64) = store
        .conn()
        .query_row(
            "SELECT method, amount_minor, tendered_minor FROM payments WHERE sale_id = ?1",
            [sale.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read payment");
    assert_eq!(method, "cash");
    assert_eq!(amount, 3_780);
    assert_eq!(tendered, 4_000);

    let _ = std::fs::remove_file(path);
}

#[test]
fn the_drawer_opens_once_and_counts_its_cash() {
    let (store, path) = store("drawer-counts");

    // Nothing to count before the first sale, so nothing is drawn.
    assert!(drawer(&store).is_none());

    let shift = ensure_open_shift(&store, 5_000, None).expect("open");
    assert!(shift.is_open());
    assert_eq!(shift.float_minor, 5_000, "the float it opened with");

    // A retried open hands back the same drawer rather than splitting the
    // day's cash in two.
    let again = ensure_open_shift(&store, 9_999, None).expect("reopen");
    assert_eq!(again.uid, shift.uid);
    assert_eq!(again.float_minor, 5_000);

    let line = |unit_minor: i64| SaleLine {
        product_id: None,
        name: "Bun".to_owned(),
        qty: 1,
        unit_minor,
        unit_factor: 1,
        unit_label: None,
    };
    record_sale(
        &store,
        &[line(2_000)],
        &SaleTerms {
            rounding_minor: 0,
            tax_minor: 0,
            tendered_minor: 2_000,
            user_id: None,
            shift_uid: Some(&shift.uid),
            ..SaleTerms::default()
        },
    )
    .expect("first sale");
    record_sale(
        &store,
        &[line(2_000)],
        &SaleTerms {
            rounding_minor: 0,
            tax_minor: 0,
            tendered_minor: 2_000,
            user_id: None,
            shift_uid: Some(&shift.uid),
            ..SaleTerms::default()
        },
    )
    .expect("second sale");
    // A sale the drawer could not be opened for still records. It is a gap
    // in a count, not a lost sale, so it must not appear in the totals.
    record_sale(
        &store,
        &[line(700)],
        &SaleTerms {
            rounding_minor: 0,
            tax_minor: 0,
            tendered_minor: 700,
            user_id: None,
            shift_uid: None,
            ..SaleTerms::default()
        },
    )
    .expect("unattributed");

    let open = drawer(&store).expect("drawer");
    assert_eq!(open.totals.cash_minor, 4_000);
    assert_eq!(open.totals.sales_count, 2);
    assert_eq!(open.expected_minor(), 9_000, "float plus cash taken");
    assert_eq!(open.difference_minor(), None, "nothing counted yet");

    close_shift(&store, &shift.uid, 8_900, None).expect("close");

    let closed = drawer(&store).expect("a count from today is still today's");
    assert!(!closed.shift.is_open());
    assert_eq!(closed.shift.counted_minor, Some(8_900));
    assert_eq!(closed.difference_minor(), Some(-100), "short by 100");
    assert!(
        close_shift(&store, &shift.uid, 8_900, None).is_err(),
        "a count is made once"
    );

    // The next sale opens a new drawer; the count starts again.
    let next = ensure_open_shift(&store, 5_000, None).expect("reopen after close");
    assert_ne!(next.uid, shift.uid);
    assert!(next.is_open());

    let _ = std::fs::remove_file(path);
}

#[test]
fn yesterdays_drawer_is_history_not_todays_card() {
    let (store, path) = store("drawer-ages-out");

    let shift = ensure_open_shift(&store, 0, None).expect("open");
    close_shift(&store, &shift.uid, 0, None).expect("close");
    assert!(drawer(&store).is_some(), "closed today, so still the day's");

    store
        .conn()
        .execute(
            "UPDATE shifts SET closed_at = datetime('now', '-2 days') WHERE uid = ?1",
            [&shift.uid],
        )
        .expect("age the count");

    assert!(
        drawer(&store).is_none(),
        "an old count belongs in history, not at the top of Today"
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn tracking_writes_an_opening_movement_and_manual_kinds_keep_their_signs() {
    let (store, path) = store("stock-track");
    let soap = insert_product(&store, "Soap", 1_200, "Household", None, None).expect("product");

    track_product(&store, soap, 12, 3, None, None).expect("track");
    let tracked = stock(&store);
    assert_eq!(tracked.len(), 1);
    assert_eq!(tracked[0].on_hand, 12);
    assert_eq!(tracked[0].low_stock, 3);
    assert!(!tracked[0].is_out());
    assert!(!tracked[0].is_low());

    record_stock_movement(&store, soap, 5, StockKind::Receive, "Delivery", None, None)
        .expect("receive");
    record_stock_movement(&store, soap, 2, StockKind::Return, "", None, None).expect("return");
    record_stock_movement(&store, soap, -4, StockKind::Waste, "Broken", None, None).expect("waste");
    record_stock_movement(&store, soap, -2, StockKind::Adjust, "Counted", None, None)
        .expect("adjust");

    assert_eq!(stock(&store)[0].on_hand, 13);
    assert!(!stock(&store)[0].is_out());
    assert!(!stock(&store)[0].is_low());

    let history = stock_movements(&store, soap);
    assert_eq!(history.len(), 5, "opening plus four moves");
    assert_eq!(history[0].delta, -2, "newest first");
    assert_eq!(history[0].kind, StockKind::Adjust);
    assert_eq!(history[0].note.as_deref(), Some("Counted"));
    assert_eq!(history[4].kind, StockKind::Opening);

    // Archiving hides the product but keeps its ledger: the movements are
    // the history of stock that was really on the shelf, and removing them
    // would erase the only record of where the count went.
    delete_product(&store, soap).expect("archive");
    assert!(stock(&store).is_empty());
    assert_eq!(stock_movements(&store, soap).len(), 5);

    let _ = std::fs::remove_file(path);
}

#[test]
fn a_sale_subtracts_only_the_tracked_lines() {
    let (store, path) = store("stock-sale");
    let soap = insert_product(&store, "Soap", 1_000, "Household", None, None).expect("soap");
    let bun = insert_product(&store, "Bun", 500, "Bakery", None, None).expect("bun");
    track_product(&store, soap, 10, 5, None, None).expect("track soap");

    let cashier = insert_user(
        &store,
        "Amina",
        Role::Owner,
        CredentialKind::Password,
        "$argon2id$stock",
    )
    .expect("cashier");

    let sale = record_sale(
        &store,
        &[
            SaleLine {
                product_id: Some(soap),
                name: "Soap".to_owned(),
                qty: 3,
                unit_minor: 1_000,
                unit_factor: 1,
                unit_label: None,
            },
            SaleLine {
                product_id: Some(bun),
                name: "Bun".to_owned(),
                qty: 2,
                unit_minor: 500,
                unit_factor: 1,
                unit_label: None,
            },
        ],
        &SaleTerms {
            rounding_minor: 0,
            tax_minor: 0,
            tendered_minor: 4_000,
            user_id: Some(&cashier.id),
            shift_uid: None,
            ..SaleTerms::default()
        },
    )
    .expect("sale");

    let tracked = stock(&store);
    assert_eq!(tracked.len(), 1, "untracked products stay out of inventory");
    assert_eq!(tracked[0].on_hand, 7);

    let movements = stock_movements(&store, soap);
    assert_eq!(movements[0].kind, StockKind::Sale);
    assert_eq!(movements[0].delta, -3);
    assert_eq!(
        movements[0].user.as_deref(),
        Some("Amina"),
        "the ledger names who took the money"
    );
    assert!(
        stock_movements(&store, bun).is_empty(),
        "a product with no ledger never gets a movement"
    );

    let sale_id: Option<i64> = store
        .conn()
        .query_row(
            "SELECT sale_id FROM stock_movements WHERE product_id = ?1 AND kind = 'sale'",
            [soap],
            |row| row.get(0),
        )
        .expect("sale id");
    assert_eq!(sale_id, Some(sale.id));

    let _ = std::fs::remove_file(path);
}

#[test]
fn a_zero_opening_count_tracks_without_pretending_anything_moved() {
    let (store, path) = store("stock-zero");
    let soap = insert_product(&store, "Soap", 1_200, "Household", None, None).expect("product");

    track_product(&store, soap, 0, 5, None, None).expect("track");
    let tracked = stock(&store);
    assert_eq!(tracked.len(), 1);
    assert_eq!(tracked[0].on_hand, 0);
    assert!(tracked[0].is_out());
    assert!(stock_movements(&store, soap).is_empty());

    // Tracking is a decision, not a button: starting twice is refused.
    assert!(track_product(&store, soap, 4, 5, None, None).is_err());

    let _ = std::fs::remove_file(path);
}

#[test]
fn a_movement_whose_sign_contradicts_its_kind_is_refused() {
    let (store, path) = store("stock-signs");
    let soap = insert_product(&store, "Soap", 1_200, "Household", None, None).expect("product");
    track_product(&store, soap, 5, 2, None, None).expect("track");

    for (delta, kind) in [
        (-1, StockKind::Receive),
        (-1, StockKind::Return),
        (1, StockKind::Waste),
        (1, StockKind::Opening),
        (-1, StockKind::Sale),
        (0, StockKind::Adjust),
    ] {
        assert!(
            record_stock_movement(&store, soap, delta, kind, "", None, None).is_err(),
            "{kind:?} with a delta of {delta} should be refused"
        );
    }
    assert_eq!(stock(&store)[0].on_hand, 5, "nothing was written");

    let untracked = insert_product(&store, "Bun", 500, "Bakery", None, None).expect("bun");
    assert!(
        record_stock_movement(&store, untracked, 1, StockKind::Receive, "", None, None).is_err(),
        "an untracked product has no ledger to append to"
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn stock_may_go_negative_when_the_shelf_and_the_ledger_disagree() {
    let (store, path) = store("stock-negative");
    let soap = insert_product(&store, "Soap", 1_000, "Household", None, None).expect("product");
    track_product(&store, soap, 1, 2, None, None).expect("track");

    record_sale(
        &store,
        &[SaleLine {
            product_id: Some(soap),
            name: "Soap".to_owned(),
            qty: 4,
            unit_minor: 1_000,
            unit_factor: 1,
            unit_label: None,
        }],
        &SaleTerms {
            rounding_minor: 0,
            tax_minor: 0,
            tendered_minor: 4_000,
            user_id: None,
            shift_uid: None,
            ..SaleTerms::default()
        },
    )
    .expect("the till sells even when the count says it cannot");

    let tracked = stock(&store);
    assert_eq!(tracked[0].on_hand, -3);
    assert!(tracked[0].is_out());

    // Receiving does not hide the debt: the count stays negative until the
    // shelf has caught up with what was sold.
    record_stock_movement(&store, soap, 2, StockKind::Receive, "", None, None).expect("receive");
    assert_eq!(stock(&store)[0].on_hand, -1);
    assert!(stock(&store)[0].is_out());

    let _ = std::fs::remove_file(path);
}

#[test]
fn history_is_newest_first_and_capped() {
    let (store, path) = store("stock-history");
    let soap = insert_product(&store, "Soap", 1_200, "Household", None, None).expect("product");
    track_product(&store, soap, 0, 5, None, None).expect("track");

    for _ in 0..(MOVEMENT_HISTORY + 5) {
        record_stock_movement(&store, soap, 1, StockKind::Receive, "", None, None)
            .expect("receive");
    }

    let history = stock_movements(&store, soap);
    assert_eq!(history.len() as i64, MOVEMENT_HISTORY);
    assert!(history[0].id > history[1].id, "newest first");
    assert_eq!(stock(&store)[0].on_hand, MOVEMENT_HISTORY + 5);

    let _ = std::fs::remove_file(path);
}

#[test]
fn a_short_tender_writes_nothing() {
    let (store, path) = store("short-tender");
    let lines = [SaleLine {
        product_id: None,
        name: "Espresso".to_owned(),
        qty: 1,
        unit_minor: 2_800,
        unit_factor: 1,
        unit_label: None,
    }];

    assert!(
        record_sale(
            &store,
            &lines,
            &SaleTerms {
                rounding_minor: 0,
                tax_minor: 0,
                tendered_minor: 2_000,
                user_id: None,
                shift_uid: None,
                ..SaleTerms::default()
            }
        )
        .is_err(),
        "2000 does not cover 2800"
    );

    // The sale row is inserted first, so this proves the transaction rolled
    // back rather than leaving a sale with no payment behind.
    assert_eq!(totals(&store, 3, None), (0, 0));
    let line_count: i64 = store
        .conn()
        .query_row("SELECT COUNT(*) FROM sale_lines", [], |row| row.get(0))
        .expect("count lines");
    assert_eq!(line_count, 0);

    let _ = std::fs::remove_file(path);
}

/// A receipt number says which till, which day and which sale. The row id
/// alone is the same 42 on two tills that sync, and the slip in the
/// customer's hand has to be one number.
#[test]
fn a_sale_is_numbered_by_its_device_and_its_day() {
    let (store, path) = store("receipt-number");
    let line = || SaleLine {
        product_id: None,
        name: "Milk".to_owned(),
        qty: 1,
        unit_minor: 2_000,
        unit_factor: 1,
        unit_label: None,
    };

    let first = record_sale(
        &store,
        &[line()],
        &SaleTerms {
            rounding_minor: 0,
            tax_minor: 0,
            tendered_minor: 2_000,
            user_id: None,
            shift_uid: None,
            ..SaleTerms::default()
        },
    )
    .expect("first");
    let second = record_sale(
        &store,
        &[line()],
        &SaleTerms {
            rounding_minor: 0,
            tax_minor: 0,
            tendered_minor: 2_000,
            user_id: None,
            shift_uid: None,
            ..SaleTerms::default()
        },
    )
    .expect("second");

    let device = ensure_device_id(&store).expect("device");
    let short = device_short(&device);
    let day: String = store
        .conn()
        .query_row("SELECT strftime('%Y%m%d', 'now', 'localtime')", [], |row| {
            row.get(0)
        })
        .expect("the clock");

    let numbers: Vec<String> = [first.id, second.id]
        .into_iter()
        .map(|id| {
            sale_detail(&store, id)
                .expect("detail")
                .receipt_no
                .expect("a new sale is numbered")
        })
        .collect();

    assert_eq!(numbers[0], format!("{short}-{day}-001"));
    assert_eq!(numbers[1], format!("{short}-{day}-002"));
    assert_eq!(short.len(), 4, "four characters of the device, no more");
    assert!(
        short
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()),
        "the prefix is safe to read aloud: {short}"
    );

    let _ = std::fs::remove_file(path);
}

/// A new morning starts the count at one. The day is stored beside the
/// count, so there is no reset job that can be forgotten.
#[test]
fn the_count_starts_over_on_a_new_day() {
    let (store, path) = store("receipt-new-day");
    set_meta(&store, "receipt_day", "20200101").expect("yesterday");
    set_meta(&store, "receipt_seq", "37").expect("a busy day");

    let sale = record_sale(
        &store,
        &[SaleLine {
            product_id: None,
            name: "Milk".to_owned(),
            qty: 1,
            unit_minor: 2_000,
            unit_factor: 1,
            unit_label: None,
        }],
        &SaleTerms {
            rounding_minor: 0,
            tax_minor: 0,
            tendered_minor: 2_000,
            user_id: None,
            shift_uid: None,
            ..SaleTerms::default()
        },
    )
    .expect("sale");
    let number = sale_detail(&store, sale.id)
        .expect("detail")
        .receipt_no
        .expect("numbered");

    assert!(number.ends_with("-001"), "{number}");

    let _ = std::fs::remove_file(path);
}

/// A sale written before numbers carried a device keeps the row id it was
/// printed with, and the detail says so rather than inventing a number.
#[test]
fn an_old_sale_has_no_device_number() {
    let (store, path) = store("receipt-old");
    store
        .conn()
        .execute("INSERT INTO sales (total_minor) VALUES (1000)", [])
        .expect("plant");
    let id = store.conn().last_insert_rowid();

    let detail = sale_detail(&store, id).expect("detail");
    assert_eq!(detail.receipt_no, None);

    let _ = std::fs::remove_file(path);
}

#[test]
fn an_empty_cart_is_not_a_sale() {
    let (store, path) = store("empty-cart");

    assert!(
        record_sale(
            &store,
            &[],
            &SaleTerms {
                rounding_minor: 0,
                tax_minor: 0,
                tendered_minor: 0,
                user_id: None,
                shift_uid: None,
                ..SaleTerms::default()
            }
        )
        .is_err()
    );
    assert_eq!(totals(&store, 3, None), (0, 0));

    let _ = std::fs::remove_file(path);
}

#[test]
fn archiving_a_product_does_not_rewrite_its_receipt() {
    let (store, path) = store("sale-then-archive");
    let flat_white =
        insert_product(&store, "Flat white", 3_400, "Coffee", None, None).expect("insert product");

    let sale = record_sale(
        &store,
        &[SaleLine {
            product_id: Some(flat_white),
            name: "Flat white".to_owned(),
            qty: 1,
            unit_minor: 3_400,
            unit_factor: 1,
            unit_label: None,
        }],
        &SaleTerms {
            rounding_minor: 0,
            tax_minor: 0,
            tendered_minor: 3_400,
            user_id: None,
            shift_uid: None,
            ..SaleTerms::default()
        },
    )
    .expect("record");

    delete_product(&store, flat_white).expect("archive");

    let (product_id, name, total_minor): (Option<i64>, String, i64) = store
        .conn()
        .query_row(
            "SELECT product_id, name, total_minor FROM sale_lines WHERE sale_id = ?1",
            [sale.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("the line survives the product");
    assert_eq!(
        product_id,
        Some(flat_white),
        "the link survives, so the report still knows which product it was"
    );
    assert_eq!(name, "Flat white");
    assert_eq!(total_minor, 3_400);
    assert!(products(&store).is_empty(), "the catalog hides it");

    let _ = std::fs::remove_file(path);
}

#[test]
fn a_sale_is_stamped_with_an_identity_and_moves_the_cursor() {
    let (store, path) = store("sale-identity");
    let soap = insert_product(&store, "Soap", 1_000, "Household", None, None).expect("product");
    track_product(&store, soap, 10, 3, None, None).expect("track");

    let line = SaleLine {
        product_id: Some(soap),
        name: "Soap".to_owned(),
        qty: 1,
        unit_minor: 1_000,
        unit_factor: 1,
        unit_label: None,
    };
    let first = record_sale(
        &store,
        std::slice::from_ref(&line),
        &SaleTerms {
            rounding_minor: 0,
            tax_minor: 0,
            tendered_minor: 1_000,
            user_id: None,
            shift_uid: None,
            ..SaleTerms::default()
        },
    )
    .expect("first");
    let second = record_sale(
        &store,
        &[line],
        &SaleTerms {
            rounding_minor: 0,
            tax_minor: 0,
            tendered_minor: 1_000,
            user_id: None,
            shift_uid: None,
            ..SaleTerms::default()
        },
    )
    .expect("second");

    assert_ne!(first.uid, second.uid);
    assert!(uuid::Uuid::parse_str(&first.uid).is_ok(), "{}", first.uid);
    assert!(uuid::Uuid::parse_str(&second.uid).is_ok());

    let device = ensure_device_id(&store).expect("device id");
    assert_eq!(
        ensure_device_id(&store).expect("stable"),
        device,
        "the install keeps one name"
    );

    let (origin, seq, received_at, offline): (String, i64, Option<String>, i64) = store
        .conn()
        .query_row(
            "SELECT origin_device, change_seq, received_at, is_offline
             FROM sales WHERE id = ?1",
            [first.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("the sale is stamped");
    assert_eq!(origin, device);
    assert_eq!(seq, 1);
    assert_eq!(
        received_at, None,
        "a local sale was not received from a peer"
    );
    assert_eq!(offline, 0);

    let second_seq: i64 = store
        .conn()
        .query_row(
            "SELECT change_seq FROM sales WHERE id = ?1",
            [second.id],
            |row| row.get(0),
        )
        .expect("second cursor");
    assert_eq!(second_seq, 2, "the cursor counts sales, not seconds");

    let line_uid: String = store
        .conn()
        .query_row(
            "SELECT uid FROM sale_lines WHERE sale_id = ?1",
            [first.id],
            |row| row.get(0),
        )
        .expect("line uid");
    assert!(uuid::Uuid::parse_str(&line_uid).is_ok());

    let payment_uid: String = store
        .conn()
        .query_row(
            "SELECT uid FROM payments WHERE sale_id = ?1",
            [first.id],
            |row| row.get(0),
        )
        .expect("payment uid");
    assert!(uuid::Uuid::parse_str(&payment_uid).is_ok());

    let movement_uid: String = store
        .conn()
        .query_row(
            "SELECT uid FROM stock_movements WHERE sale_id = ?1",
            [first.id],
            |row| row.get(0),
        )
        .expect("movement uid");
    assert!(uuid::Uuid::parse_str(&movement_uid).is_ok());

    let _ = std::fs::remove_file(path);
}

#[test]
fn catalog_writes_advance_the_catalog_version() {
    let (store, path) = store("catalog-version");
    let version = || {
        meta(&store, "catalog_version")
            .and_then(|value| value.parse::<i64>().ok())
            .expect("catalog_version is an integer")
    };

    let start = version();
    let product = insert_product(&store, "Soap", 1_000, "Household", None, None).expect("product");
    let after_insert = version();
    assert!(after_insert > start, "a peer must see the catalog move");

    update_product(&store, product, "Soap", 1_200, "Household", None, None).expect("rename");
    assert!(version() > after_insert, "a rename is a catalog change too");

    let _ = std::fs::remove_file(path);
}

#[test]
fn a_sale_names_the_person_who_took_it() {
    let (store, path) = store("sale-cashier");

    let owner = insert_user(
        &store,
        "Amina",
        Role::Owner,
        CredentialKind::Password,
        "$argon2id$owner",
    )
    .expect("owner");
    let brian = insert_user(
        &store,
        "Brian",
        Role::Cashier,
        CredentialKind::Pin,
        "$argon2id$brian",
    )
    .expect("cashier");

    // One owner, and one person per name whatever the capitals: the lock
    // screen is a list of names, and two Amimas would be a way to steal a
    // shift.
    assert!(
        insert_user(
            &store,
            "Amina",
            Role::Owner,
            CredentialKind::Password,
            "$argon2id$other"
        )
        .is_err()
    );
    assert!(
        insert_user(
            &store,
            "brian",
            Role::Cashier,
            CredentialKind::Pin,
            "$argon2id$x"
        )
        .is_err()
    );

    let sale = record_sale(
        &store,
        &[SaleLine {
            product_id: None,
            name: "Tea".to_owned(),
            qty: 1,
            unit_minor: 500,
            unit_factor: 1,
            unit_label: None,
        }],
        &SaleTerms {
            rounding_minor: 0,
            tax_minor: 0,
            tendered_minor: 500,
            user_id: Some(&brian.id),
            shift_uid: None,
            ..SaleTerms::default()
        },
    )
    .expect("record");

    assert_eq!(sale.paid_minor(), 500);
    let history = sales(&store, 3, 10, None);
    assert_eq!(history[0].cashier.as_deref(), Some("Brian"));

    // Switching somebody off keeps their name on their history: the money
    // was still taken by that person.
    set_user_active(&store, &brian.id, false).expect("switch off");
    assert_eq!(
        users(&store)
            .iter()
            .find(|user| user.id == brian.id)
            .map(|user| user.active),
        Some(false)
    );
    assert_eq!(
        sales(&store, 3, 10, None)[0].cashier.as_deref(),
        Some("Brian")
    );

    // Owner first, then managers, then cashiers.
    assert_eq!(users(&store).first().map(|user| &user.id), Some(&owner.id));

    let _ = std::fs::remove_file(path);
}

#[test]
fn a_lockout_survives_the_app_closing() {
    let (store, path) = store("lockout");
    let cashier = insert_user(
        &store,
        "Brian",
        Role::Cashier,
        CredentialKind::Pin,
        "$argon2id$brian",
    )
    .expect("cashier");

    assert_eq!(attempts(&store, &cashier.id), (0, None));

    record_attempt(&store, &cashier.id, 5, Some(1_000));
    assert_eq!(attempts(&store, &cashier.id), (5, Some(1_000)));

    // A further failure overwrites rather than stacking a second row.
    record_attempt(&store, &cashier.id, 6, Some(2_000));
    assert_eq!(attempts(&store, &cashier.id), (6, Some(2_000)));

    clear_attempts(&store, &cashier.id);
    assert_eq!(attempts(&store, &cashier.id), (0, None));

    let _ = std::fs::remove_file(path);
}

#[test]
fn a_sale_detail_reads_back_its_receipt() {
    let (store, path) = store("sale-detail");
    let brian = insert_user(
        &store,
        "Brian",
        Role::Cashier,
        CredentialKind::Pin,
        "$argon2id$brian",
    )
    .expect("cashier");

    let sale = record_sale(
        &store,
        &[
            SaleLine {
                product_id: None,
                name: "Sugar 1kg".to_owned(),
                qty: 2,
                unit_minor: 8000,
                unit_factor: 1,
                unit_label: None,
            },
            SaleLine {
                product_id: None,
                name: "Tea".to_owned(),
                qty: 1,
                unit_minor: 500,
                unit_factor: 1,
                unit_label: None,
            },
        ],
        // Total is 16 500; UGX coins end in 100, so the till rounds up 100.
        &SaleTerms {
            rounding_minor: 100,
            tendered_minor: 16_600,
            user_id: Some(&brian.id),
            ..SaleTerms::default()
        },
    )
    .expect("record");

    let detail = sale_detail(&store, sale.id).expect("detail");
    assert_eq!(detail.total_minor, 16_500);
    assert_eq!(detail.rounding_minor, 100);
    assert_eq!(detail.paid_minor, 16_600);
    assert_eq!(detail.cashier.as_deref(), Some("Brian"));
    assert_eq!(detail.tendered_minor, 16_600);
    assert_eq!(detail.change_minor, 0);
    assert_eq!(detail.method, "cash");

    assert_eq!(detail.lines.len(), 2);
    assert_eq!(detail.lines[0].name, "Sugar 1kg");
    assert_eq!(detail.lines[0].qty, 2);
    assert_eq!(detail.lines[0].unit_minor, 8000);
    assert_eq!(detail.lines[0].total_minor, 16_000);
    assert_eq!(detail.lines[1].name, "Tea");
    assert_eq!(detail.lines[1].total_minor, 500);

    // An id that points nowhere reads as nothing, not as an empty record
    // pretending to be sale 99.
    assert!(sale_detail(&store, 999_999).is_none());

    let _ = std::fs::remove_file(path);
}

#[test]
fn a_change_returns_in_the_sale_detail() {
    let (store, path) = store("sale-change");

    let sale = record_sale(
        &store,
        &[SaleLine {
            product_id: None,
            name: "Tea".to_owned(),
            qty: 1,
            unit_minor: 500,
            unit_factor: 1,
            unit_label: None,
        }],
        &SaleTerms {
            rounding_minor: 0,
            tax_minor: 0,
            tendered_minor: 1_000,
            user_id: None,
            shift_uid: None,
            ..SaleTerms::default()
        },
    )
    .expect("record");

    let detail = sale_detail(&store, sale.id).expect("detail");
    assert_eq!(detail.tendered_minor, 1_000);
    assert_eq!(detail.change_minor, 500);
    // A sale written before the till had staff still opens: the receipt is
    // the record, and "someone" is honest enough for its header.
    assert_eq!(detail.cashier, None);

    let _ = std::fs::remove_file(path);
}

/// A split sale is one sale with several tenders: each is its own `payments`
/// row, in the order the cashier took it, and the detail reads them back as a
/// list rather than a combined string.
#[test]
fn a_split_sale_writes_one_payment_row_per_tender() {
    let (store, path) = store("sale-split");

    let sale = record_sale(
        &store,
        &[SaleLine {
            product_id: None,
            name: "Sugar".to_owned(),
            qty: 1,
            unit_minor: 10_000,
            unit_factor: 1,
            unit_label: None,
        }],
        &SaleTerms {
            payments: &[
                PaymentLine {
                    method: "cash",
                    amount_minor: 4_000,
                    tendered_minor: 5_000,
                    reference: None,
                },
                PaymentLine {
                    method: "mtn_momo",
                    amount_minor: 6_000,
                    tendered_minor: 6_000,
                    reference: Some("MP240101.1234.A12345"),
                },
            ],
            ..SaleTerms::default()
        },
    )
    .expect("record");

    let detail = sale_detail(&store, sale.id).expect("detail");
    assert_eq!(detail.paid_minor, 10_000);
    assert_eq!(detail.payments.len(), 2);

    // The cash part overpaid and gave change; the mobile part was exact.
    assert_eq!(detail.payments[0].method, "cash");
    assert_eq!(detail.payments[0].amount_minor, 4_000);
    assert_eq!(detail.payments[0].tendered_minor, 5_000);
    assert_eq!(detail.payments[0].change_minor, 1_000);
    assert_eq!(detail.payments[0].reference, None);

    assert_eq!(detail.payments[1].method, "mtn_momo");
    assert_eq!(detail.payments[1].amount_minor, 6_000);
    assert_eq!(detail.payments[1].change_minor, 0);
    assert_eq!(
        detail.payments[1].reference.as_deref(),
        Some("MP240101.1234.A12345")
    );

    assert_eq!(detail.tendered_minor, 11_000);
    assert_eq!(detail.change_minor, 1_000);
    assert_eq!(detail.method, "cash + mtn_momo");

    // The report keeps each method on its own line rather than lumping the
    // sale under the first one.
    let methods = by_method(&store, 0, None);
    let cash = methods
        .iter()
        .find(|total| total.method == "cash")
        .expect("cash total");
    assert_eq!(cash.amount_minor, 4_000);
    let momo = methods
        .iter()
        .find(|total| total.method == "mtn_momo")
        .expect("mobile total");
    assert_eq!(momo.amount_minor, 6_000);

    let _ = std::fs::remove_file(path);
}

/// The per-tender rules are the ones the sync validator already enforces: cash
/// may overpay, a mobile tender is exact, and the tenders must settle the whole
/// sale. A split that breaks any of them writes nothing.
#[test]
fn a_split_sale_refuses_tenders_that_do_not_settle_exactly() {
    let (store, path) = store("sale-split-refused");
    let line = [SaleLine {
        product_id: None,
        name: "Sugar".to_owned(),
        qty: 1,
        unit_minor: 10_000,
        unit_factor: 1,
        unit_label: None,
    }];

    // The tenders add up to 9,000 against a 10,000 bill.
    let error = record_sale(
        &store,
        &line,
        &SaleTerms {
            payments: &[PaymentLine {
                method: "cash",
                amount_minor: 9_000,
                tendered_minor: 9_000,
                reference: None,
            }],
            ..SaleTerms::default()
        },
    )
    .expect_err("a sale that does not settle cannot be written");
    assert!(
        error.to_string().contains("payments sum to 9000"),
        "{error}"
    );

    // A mobile tender cannot hand change back, so it cannot overshoot.
    let error = record_sale(
        &store,
        &line,
        &SaleTerms {
            payments: &[PaymentLine {
                method: "mtn_momo",
                amount_minor: 10_000,
                tendered_minor: 10_500,
                reference: None,
            }],
            ..SaleTerms::default()
        },
    )
    .expect_err("a mobile tender is exact");
    assert!(
        error
            .to_string()
            .contains("a non-cash payment must equal its amount"),
        "{error}"
    );

    assert_eq!(totals(&store, 0, None), (0, 0));

    let _ = std::fs::remove_file(path);
}

/// Cash, two sales, one of them rounded. The method total is what the drawer
/// kept, so the rounded amount counts — not the pre-rounding total the
/// customer's items added up to, and not the note that was handed over.
#[test]
fn payments_taken_are_the_amounts_that_stayed() {
    let (store, path) = store("by-method");

    record_sale(
        &store,
        &[SaleLine {
            product_id: None,
            name: "Bread".to_owned(),
            qty: 1,
            unit_minor: 340,
            unit_factor: 1,
            unit_label: None,
        }],
        &SaleTerms {
            rounding_minor: 60,
            tax_minor: 0,
            tendered_minor: 1_000,
            user_id: None,
            shift_uid: None,
            ..SaleTerms::default()
        },
    )
    .expect("record a rounded sale");
    record_sale(
        &store,
        &[SaleLine {
            product_id: None,
            name: "Milk".to_owned(),
            qty: 1,
            unit_minor: 500,
            unit_factor: 1,
            unit_label: None,
        }],
        &SaleTerms {
            rounding_minor: 0,
            tax_minor: 0,
            tendered_minor: 500,
            user_id: None,
            shift_uid: None,
            ..SaleTerms::default()
        },
    )
    .expect("record an exact sale");

    let methods = by_method(&store, 3, None);
    assert_eq!(methods.len(), 1);
    assert_eq!(methods[0].method, "cash");
    assert_eq!(methods[0].sales, 2);
    assert_eq!(methods[0].amount_minor, 900, "400 rounded + 500 exact");

    let _ = std::fs::remove_file(path);
}

/// A range nobody sold in is an empty list, not a row of zeros: the card
/// draws "no money taken" from emptiness, and a zero row would draw a table.
#[test]
fn a_range_with_no_sales_reports_no_methods() {
    let (store, path) = store("by-method-empty");

    assert!(by_method(&store, 0, None).is_empty());

    let _ = std::fs::remove_file(path);
}

/// A sale paid in two parts is one line in the books, not two: an ungrouped
/// join would double every split sale and nobody would notice until a
/// reconciliation came up short.
#[test]
fn a_split_sale_exports_as_one_row_with_both_methods() {
    let (store, path) = store("export-split");

    store
        .conn()
        .execute(
            "INSERT INTO sales (total_minor, rounding_minor) VALUES (1000, 0)",
            [],
        )
        .expect("insert sale");
    let id = store.conn().last_insert_rowid();
    for (method, amount) in [("mobile", 400_i64), ("cash", 600)] {
        store
            .conn()
            .execute(
                "INSERT INTO payments
                    (sale_id, method, amount_minor, tendered_minor, change_minor)
                 VALUES (?1, ?2, ?3, ?3, 0)",
                (id, method, amount),
            )
            .expect("insert payment");
    }

    let rows = export_sales(&store, 3, None).expect("export");
    assert_eq!(rows.len(), 1, "a split sale is one sale");
    assert_eq!(
        rows[0].method, "cash+mobile",
        "sorted, so the file does not depend on insert order"
    );
    assert_eq!(rows[0].total_minor, 1_000);
    assert_eq!(rows[0].rounding_minor, 0);
    assert_eq!(rows[0].paid_minor, 1_000);
    assert!(rows[0].cashier.is_none());

    let _ = std::fs::remove_file(path);
}

/// A backup is only a backup if it opens: the file has to be a complete
/// database with this binary's schema, not a copy of whatever pages
/// happened to be flushed.
#[test]
fn a_backup_is_a_database_that_opens() {
    let (store, path) = store("backup");
    let dir = std::env::temp_dir().join(format!("pos-backup-open-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let file = backup(&store, &dir).expect("backup");

    assert!(file.starts_with(&dir), "{}", file.display());
    assert!(
        file.metadata().expect("metadata").len() > 0,
        "an empty file is not a backup"
    );
    let copy = storage::Store::open(&file).expect("a backup opens");
    assert_eq!(
        copy.user_version().expect("a version"),
        store.user_version().expect("a version")
    );

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(path);
}

/// Twelve presses do not mean twelve files: exactly the newest ten remain,
/// and the newest — the one somebody would actually restore — stays.
///
/// The expected set is derived from the same names the backup chose rather
/// than from "the first two went", because the run can cross a second
/// boundary — and then the eleventh file is older than the first of the new
/// second, which is the ordering the prune has to get right anyway.
#[test]
fn only_the_newest_ten_backups_are_kept() {
    let (store, path) = store("backup-prune");
    let dir = std::env::temp_dir().join(format!("pos-backup-prune-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let written: Vec<PathBuf> = (0..12)
        .map(|_| backup(&store, &dir).expect("backup"))
        .collect();

    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("list backups")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();

    let mut expected = written.clone();
    expected.sort();
    expected.drain(..written.len() - BACKUP_KEEP);

    files.sort();
    assert_eq!(files, expected);
    assert!(files.contains(&written[11]), "the newest survives");

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(path);
}

/// A till that has never been copied gets a backup on the first check, and
/// the check creates the folder it needs.
#[test]
fn a_till_that_was_never_backed_up_gets_one() {
    let (store, path) = store("backup-first");
    let dir = std::env::temp_dir().join(format!("pos-backup-first-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let written = backup_if_due(&store, &dir, 24)
        .expect("backup")
        .expect("a copy");
    assert!(written.exists(), "the copy is on disk");
    assert!(newest_backup(&dir).expect("newest").is_some());

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(path);
}

/// The second check of the same day writes nothing: a backup is a copy of
/// everything, and one every ten minutes would keep ten copies of the last
/// hour and none of last week.
#[test]
fn a_recent_backup_is_not_taken_twice() {
    let (store, path) = store("backup-twice");
    let dir = std::env::temp_dir().join(format!("pos-backup-twice-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    backup_if_due(&store, &dir, 24).expect("first");
    assert!(
        backup_if_due(&store, &dir, 24).expect("second").is_none(),
        "a copy from a moment ago is recent enough"
    );

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(path);
}

/// A copy older than the window is history: the check writes a fresh one.
/// The planted name comes from the same clock helper the check uses, so the
/// test does not depend on how long it takes to run.
#[test]
fn a_stale_backup_is_replaced() {
    let (store, path) = store("backup-stale");
    let dir = std::env::temp_dir().join(format!("pos-backup-stale-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("make directory");

    let old = stamp_before(&store, 48).expect("clock");
    std::fs::write(dir.join(format!("{BACKUP_PREFIX}{old}-0001.db")), b"").expect("plant");
    // A file that is not a backup must not answer the question either way.
    std::fs::write(dir.join("notes.txt"), b"a note").expect("plant");

    assert!(
        backup_if_due(&store, &dir, 24).expect("backup").is_some(),
        "two days is older than a day"
    );

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(path);
}

/// A sequence that has been pruned is still used up: filling the gap would
/// hand the newest backup a name the sort reads as the oldest, and the next
/// prune would delete it. Planted files make this independent of how fast
/// twelve real backups run.
#[test]
fn a_pruned_sequence_is_not_reused() {
    let (store, path) = store("backup-gap");
    let dir = std::env::temp_dir().join(format!("pos-backup-gap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("make directory");

    let stem = format!("{BACKUP_PREFIX}{}", stamp(&store).expect("clock"));
    for sequence in [3, 12] {
        std::fs::write(dir.join(format!("{stem}-{sequence:04}.db")), b"").expect("planted backup");
    }

    let next = next_backup_path(&store, &dir).expect("next name");
    assert_eq!(
        next.file_name().and_then(|name| name.to_str()),
        Some(format!("{stem}-0013.db").as_str()),
        "the first free number is a gap that must stay empty"
    );

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(path);
}

/// A tracked product with an opening count, ready for a purchase test.
fn stocked(store: &storage::Store, name: &str, opening: i64, low: i64, target: Option<i64>) -> i64 {
    let id = insert_product(store, name, 500, "Shop", None, None).expect("insert");
    track_product(store, id, opening, low, target, None).expect("track");
    id
}

#[test]
fn a_plan_counts_nothing_until_it_is_received() {
    let (store, path) = store("purchase-plan");
    let soap = stocked(&store, "Soap", 0, 2, Some(10));

    let id = create_purchase(
        &store,
        None,
        "Saturday market",
        &[PurchaseLineInput {
            product_id: soap,
            qty: 5,
            unit_cost_minor: Some(100),
            pack_size: 1,
            expires_on: None,
        }],
        None,
    )
    .expect("plan");

    let item = stock(&store)
        .into_iter()
        .find(|item| item.product_id == soap)
        .expect("stock");
    assert_eq!(item.on_hand, 0, "a plan is not stock");
    assert_eq!(item.incoming, 5, "but it is on its way");
    assert_eq!(item.suggested(), 5, "target 10, nothing here, five coming");
    assert!(
        !item.needs_restock(),
        "the plan already covers the warning line"
    );

    let list = purchases(&store);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].status, PurchaseStatus::Planned);
    assert_eq!(list[0].unit_count, 5);
    assert_eq!(list[0].cost_minor, Some(500));
    assert_eq!(list[0].count_label(), "5 items");
    assert_eq!(
        purchase_detail(&store, id).expect("detail").lines[0].name,
        "Soap"
    );

    // Three arrive, and the trip is over: there is no half-received state
    // to come back to.
    receive_purchase(
        &store,
        id,
        None,
        &[ReceiveLine {
            line_id: Some(purchase_detail(&store, id).expect("detail").lines[0].id),
            product_id: soap,
            qty: 3,
            unit_cost_minor: Some(120),
            pack_size: 1,
            expires_on: None,
        }],
        &[],
        "",
        None,
    )
    .expect("receive the trip");

    let item = stock(&store)
        .into_iter()
        .find(|item| item.product_id == soap)
        .expect("stock");
    assert_eq!(item.on_hand, 3);
    assert_eq!(item.incoming, 0, "the list is closed, nothing is coming");
    assert_eq!(
        item.cost_milli,
        Some(120_000),
        "last cost came from receiving"
    );

    let list = purchases(&store);
    assert_eq!(list[0].status, PurchaseStatus::Received);
    assert!(list[0].received_at.is_some());
    assert_eq!(
        list[0].count_label(),
        "3 of 5 bought",
        "coming home short is worth saying"
    );
    assert_eq!(
        stock_movements(&store, soap).len(),
        1,
        "one movement for one trip"
    );

    let line = purchase_detail(&store, id).expect("detail").lines[0].id;
    assert!(
        receive_purchase(
            &store,
            id,
            None,
            &[ReceiveLine {
                line_id: Some(line),
                product_id: soap,
                qty: 2,
                unit_cost_minor: None,
                pack_size: 1,
                expires_on: None,
            }],
            &[],
            "",
            None,
        )
        .is_err(),
        "a closed trip cannot be received twice"
    );
    assert_eq!(stock(&store)[0].on_hand, 3, "and the shelf did not move");

    let _ = std::fs::remove_file(path);
}

/// The sheet is the truth about the trip. Buying more than the list said
/// means the list understated it, so the plan rises to match instead of the
/// goods being refused at the counter.
#[test]
fn buying_more_than_planned_raises_the_plan() {
    let (store, path) = store("purchase-overbuy");
    let soap = stocked(&store, "Soap", 0, 2, None);
    let id = create_purchase(
        &store,
        None,
        "",
        &[PurchaseLineInput {
            product_id: soap,
            qty: 2,
            unit_cost_minor: Some(100),
            pack_size: 1,
            expires_on: None,
        }],
        None,
    )
    .expect("plan");
    let line = purchase_detail(&store, id).expect("detail").lines[0].id;

    receive_purchase(
        &store,
        id,
        None,
        &[ReceiveLine {
            line_id: Some(line),
            product_id: soap,
            qty: 7,
            unit_cost_minor: None,
            pack_size: 1,
            expires_on: None,
        }],
        &[],
        "",
        None,
    )
    .expect("the market had more than the list said");

    let list = purchases(&store);
    assert_eq!(list[0].count_label(), "7 items", "not a shortfall");
    assert_eq!(list[0].cost_minor, Some(700), "seven at one hundred");
    assert_eq!(stock(&store)[0].on_hand, 7);

    let _ = std::fs::remove_file(path);
}

#[test]
fn a_received_trip_cannot_be_changed_or_deleted() {
    let (store, path) = store("purchase-partial");
    let soap = stocked(&store, "Soap", 0, 2, None);
    let id = create_purchase(
        &store,
        None,
        "",
        &[PurchaseLineInput {
            product_id: soap,
            qty: 4,
            unit_cost_minor: None,
            pack_size: 1,
            expires_on: None,
        }],
        None,
    )
    .expect("plan");
    assert!(
        update_purchase(
            &store,
            id,
            None,
            "",
            &[PurchaseLineInput {
                product_id: soap,
                qty: 9,
                unit_cost_minor: None,
                pack_size: 1,
                expires_on: None,
            }]
        )
        .is_ok(),
        "an untouched list is editable"
    );
    assert!(delete_planned_purchase(&store, id).is_ok(), "and deletable");

    let id = create_purchase(
        &store,
        None,
        "",
        &[PurchaseLineInput {
            product_id: soap,
            qty: 4,
            unit_cost_minor: None,
            pack_size: 1,
            expires_on: None,
        }],
        None,
    )
    .expect("plan again");
    let line = purchase_detail(&store, id).expect("detail").lines[0].id;
    receive_purchase(
        &store,
        id,
        None,
        &[ReceiveLine {
            line_id: Some(line),
            product_id: soap,
            qty: 1,
            unit_cost_minor: None,
            pack_size: 1,
            expires_on: None,
        }],
        &[],
        "",
        None,
    )
    .expect("receive the trip");

    assert!(
        update_purchase(
            &store,
            id,
            None,
            "",
            &[PurchaseLineInput {
                product_id: soap,
                qty: 4,
                unit_cost_minor: None,
                pack_size: 1,
                expires_on: None,
            }]
        )
        .is_err(),
        "a line a movement points at cannot be replaced"
    );
    assert!(
        delete_planned_purchase(&store, id).is_err(),
        "nor can the trip vanish"
    );
    assert_eq!(purchases(&store).len(), 1, "the record stays");

    let _ = std::fs::remove_file(path);
}

/// Migration 12 shipped a `planned` purchase that could already have stock
/// counted against it. Such a row cannot be finished — receiving it again
/// would put the same goods on the shelf twice — so migration 15 closes it
/// as what it always was: a trip that came home short.
#[test]
fn a_half_received_trip_from_an_older_build_is_closed() {
    // Build the file the way the build before this one left it.
    let path = std::env::temp_dir().join(format!("pos-purchase-legacy-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut store = storage::Store::open(&path).expect("open database");
    let older: Vec<Migration> = MIGRATIONS
        .iter()
        .filter(|migration| migration.version <= 14)
        .map(|migration| Migration {
            version: migration.version,
            name: migration.name,
            sql: migration.sql,
        })
        .collect();
    store.migrate(&older).expect("migrate to 14");

    // The old schema has no pack columns, so seed the row the old way.
    store
        .conn()
        .execute(
            "INSERT INTO products (name, price_minor, category) VALUES ('Soap', 500, 'Shop')",
            [],
        )
        .expect("legacy product");
    let soap = store.conn().last_insert_rowid();
    track_product(&store, soap, 0, 2, None, None).expect("track");
    store
        .conn()
        .execute(
            "INSERT INTO purchases (uid, status, created_at)
             VALUES (?1, 'planned', '2024-01-01 09:00:00')",
            [uuid::Uuid::now_v7().to_string()],
        )
        .expect("legacy purchase");
    let id = store.conn().last_insert_rowid();
    store
        .conn()
        .execute(
            "INSERT INTO purchase_lines
                 (uid, purchase_id, product_id, name_snapshot, qty_ordered,
                  qty_received, position)
             VALUES (?1, ?2, ?3, 'Soap', 5, 3, 0)",
            (uuid::Uuid::now_v7().to_string(), id, soap),
        )
        .expect("legacy line");
    let line = store.conn().last_insert_rowid();
    store
        .conn()
        .execute(
            "INSERT INTO stock_movements
                 (product_id, delta, kind, created_at, purchase_line_id, uid)
             VALUES (?1, 3, 'receive', '2024-01-02 10:30:00', ?2, ?3)",
            (soap, line, uuid::Uuid::now_v7().to_string()),
        )
        .expect("legacy movement");

    store.migrate(MIGRATIONS).expect("migrate to 15");

    let trip = purchases(&store).into_iter().next().expect("the trip");
    assert_eq!(trip.status, PurchaseStatus::Received, "it was finished");
    assert_eq!(
        trip.count_label(),
        "3 of 5 bought",
        "and it came home short"
    );
    let received_at: Option<String> = store
        .conn()
        .query_row(
            "SELECT received_at FROM purchases WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .expect("received_at");
    assert_eq!(
        received_at.as_deref(),
        Some("2024-01-02 10:30:00"),
        "dated when the goods actually arrived, not when the update ran"
    );
    assert_eq!(stock(&store)[0].on_hand, 3, "the shelf already knew");

    assert!(
        receive_purchase(
            &store,
            id,
            None,
            &[ReceiveLine {
                line_id: Some(line),
                product_id: soap,
                qty: 3,
                unit_cost_minor: None,
                pack_size: 1,
                expires_on: None,
            }],
            &[],
            "",
            None,
        )
        .is_err(),
        "and it cannot be received a second time"
    );
    assert_eq!(stock(&store)[0].on_hand, 3, "so the shelf did not move");

    let _ = std::fs::remove_file(path);
}

#[test]
fn receiving_can_add_an_impulse_buy_and_remove_an_unbought_line() {
    let (store, path) = store("purchase-impulse");
    let soap = stocked(&store, "Soap", 0, 2, None);
    let rice = stocked(&store, "Rice", 0, 2, None);
    let salt = stocked(&store, "Salt", 0, 2, None);
    let id = create_purchase(
        &store,
        None,
        "",
        &[
            PurchaseLineInput {
                product_id: soap,
                qty: 2,
                unit_cost_minor: None,
                pack_size: 1,
                expires_on: None,
            },
            PurchaseLineInput {
                product_id: rice,
                qty: 5,
                unit_cost_minor: None,
                pack_size: 1,
                expires_on: None,
            },
        ],
        None,
    )
    .expect("plan");
    let lines = purchase_detail(&store, id).expect("detail").lines;
    let soap_line = lines
        .iter()
        .find(|line| line.product_id == Some(soap))
        .expect("soap")
        .id;
    let rice_line = lines
        .iter()
        .find(|line| line.product_id == Some(rice))
        .expect("rice")
        .id;

    receive_purchase(
        &store,
        id,
        None,
        &[
            ReceiveLine {
                line_id: Some(soap_line),
                product_id: soap,
                qty: 2,
                unit_cost_minor: Some(50),
                pack_size: 1,
                expires_on: None,
            },
            ReceiveLine {
                line_id: None,
                product_id: salt,
                qty: 3,
                unit_cost_minor: Some(80),
                pack_size: 1,
                expires_on: None,
            },
        ],
        &[rice_line],
        "Rice was on offer",
        None,
    )
    .expect("receive");

    let detail = purchase_detail(&store, id).expect("detail");
    assert_eq!(detail.lines.len(), 2, "the unbought rice line is gone");
    let soap_line = detail
        .lines
        .iter()
        .find(|line| line.product_id == Some(soap))
        .expect("soap");
    assert_eq!((soap_line.qty_ordered, soap_line.qty_received), (2, 2));
    let impulse = detail
        .lines
        .iter()
        .find(|line| line.product_id == Some(salt))
        .expect("salt");
    assert_eq!(
        (impulse.qty_ordered, impulse.qty_received),
        (3, 3),
        "an impulse buy is fully received as its own line"
    );
    assert_eq!(detail.purchase.status, PurchaseStatus::Received);
    assert_eq!(detail.purchase.note.as_deref(), Some("Rice was on offer"));

    let second = create_purchase(
        &store,
        None,
        "",
        &[PurchaseLineInput {
            product_id: soap,
            qty: 2,
            unit_cost_minor: None,
            pack_size: 1,
            expires_on: None,
        }],
        None,
    )
    .expect("second plan");
    let line = purchase_detail(&store, second).expect("detail").lines[0].id;
    receive_purchase(&store, second, None, &[], &[line], "", None).expect("remove unbought");

    assert!(
        purchase_detail(&store, second).is_none(),
        "a plan with every line marked Not bought is cleared, not left empty"
    );
    assert!(
        purchases(&store).iter().all(|p| p.id != second),
        "nothing to buy means no purchase to show"
    );

    let _ = std::fs::remove_file(path);
}

/// The counter is where the operator learns the supplier was wrong. A
/// correction made while counting in must survive the receive, not be
/// silently dropped because the plan was written days earlier.
#[test]
fn receiving_keeps_the_supplier_chosen_at_the_counter() {
    let (store, path) = store("receive-supplier");
    let soap = stocked(&store, "Soap", 0, 2, None);
    let supplier = insert_supplier(
        &store,
        &SupplierFields {
            name: "Kampala Wholesale",
            contact_name: "",
            phone: "",
            email: "",
            address: "",
            notes: "",
        },
    )
    .expect("supplier");

    let id = create_purchase(
        &store,
        None,
        "",
        &[PurchaseLineInput {
            product_id: soap,
            qty: 3,
            unit_cost_minor: None,
            pack_size: 1,
            expires_on: None,
        }],
        None,
    )
    .expect("plan");
    assert!(
        purchase_detail(&store, id)
            .expect("detail")
            .purchase
            .supplier_id
            .is_none(),
        "the plan named no supplier"
    );

    let line = purchase_detail(&store, id).expect("detail").lines[0].id;
    receive_purchase(
        &store,
        id,
        Some(&supplier.id),
        &[ReceiveLine {
            line_id: Some(line),
            product_id: soap,
            qty: 3,
            unit_cost_minor: None,
            pack_size: 1,
            expires_on: None,
        }],
        &[],
        "",
        None,
    )
    .expect("receive");

    let detail = purchase_detail(&store, id).expect("detail");
    assert_eq!(
        detail.purchase.supplier_id.as_deref(),
        Some(supplier.id.as_str()),
        "the supplier typed at receiving is the one recorded"
    );
    assert_eq!(
        detail.purchase.supplier.as_deref(),
        Some("Kampala Wholesale")
    );

    // A supplier id that is not in the directory is refused, and refusing
    // must not close the trip.
    let second = create_purchase(
        &store,
        None,
        "",
        &[PurchaseLineInput {
            product_id: soap,
            qty: 1,
            unit_cost_minor: None,
            pack_size: 1,
            expires_on: None,
        }],
        None,
    )
    .expect("second plan");
    let line = purchase_detail(&store, second).expect("detail").lines[0].id;
    let error = receive_purchase(
        &store,
        second,
        Some("no-such-supplier"),
        &[ReceiveLine {
            line_id: Some(line),
            product_id: soap,
            qty: 1,
            unit_cost_minor: None,
            pack_size: 1,
            expires_on: None,
        }],
        &[],
        "",
        None,
    )
    .expect_err("an unknown supplier is refused");
    assert!(
        error.to_string().contains("no longer in the directory"),
        "the operator is told why: {error}"
    );
    assert_eq!(
        purchase_detail(&store, second)
            .expect("still there")
            .purchase
            .status,
        PurchaseStatus::Planned,
        "a refused receive leaves the trip open"
    );

    let _ = std::fs::remove_file(path);
}

/// The movement is written for `product_id`, the line it points at is the
/// one on the shelf. If the two disagree, the shelf would move for one
/// product while the purchase recorded another, so the write is refused.
#[test]
fn a_line_cannot_be_received_as_a_different_product() {
    let (store, path) = store("receive-mismatch");
    let soap = stocked(&store, "Soap", 0, 1, None);
    let tea = stocked(&store, "Tea", 0, 1, None);

    let id = create_purchase(
        &store,
        None,
        "",
        &[
            PurchaseLineInput {
                product_id: soap,
                qty: 2,
                unit_cost_minor: None,
                pack_size: 1,
                expires_on: None,
            },
            PurchaseLineInput {
                product_id: tea,
                qty: 1,
                unit_cost_minor: None,
                pack_size: 1,
                expires_on: None,
            },
        ],
        None,
    )
    .expect("plan");
    let soap_line = purchase_detail(&store, id)
        .expect("detail")
        .lines
        .iter()
        .find(|line| line.product_id == Some(soap))
        .expect("soap line")
        .id;

    let error = receive_purchase(
        &store,
        id,
        None,
        &[ReceiveLine {
            line_id: Some(soap_line),
            product_id: tea,
            qty: 2,
            unit_cost_minor: None,
            pack_size: 1,
            expires_on: None,
        }],
        &[],
        "",
        None,
    )
    .expect_err("a line is received as the product it was planned for");
    assert!(
        error.to_string().contains("another product"),
        "the operator is told why: {error}"
    );

    assert!(
        stock_movements(&store, soap).is_empty(),
        "soap did not move"
    );
    assert!(stock_movements(&store, tea).is_empty(), "tea did not move");
    assert_eq!(
        purchase_detail(&store, id).expect("detail").purchase.status,
        PurchaseStatus::Planned,
        "the trip is still open"
    );

    let _ = std::fs::remove_file(path);
}

/// A list made at the market is a count sheet and a purchase at once. The
/// trip is written and closed in one transaction, so the shelf and the
/// record cannot disagree about whether the shop went.
#[test]
fn a_trip_created_at_the_counter_arrives_in_one_write() {
    let (store, path) = store("receive-created");
    let soap = stocked(&store, "Soap", 0, 1, None);
    let tea = stocked(&store, "Tea", 0, 1, None);

    let id = create_and_receive(
        &store,
        None,
        "Market run",
        &[
            PurchaseLineInput {
                product_id: soap,
                qty: 2,
                unit_cost_minor: Some(250),
                pack_size: 1,
                expires_on: None,
            },
            PurchaseLineInput {
                product_id: tea,
                qty: 1,
                unit_cost_minor: Some(900),
                pack_size: 3,
                expires_on: None,
            },
        ],
        None,
    )
    .expect("created and received");

    let detail = purchase_detail(&store, id).expect("detail");
    assert_eq!(detail.purchase.status, PurchaseStatus::Received);
    assert_eq!(detail.purchase.note.as_deref(), Some("Market run"));
    for line in &detail.lines {
        assert_eq!(
            line.qty_ordered, line.qty_received,
            "what was bought is what arrived"
        );
    }

    let shelf = |product_id: i64| {
        stock(&store)
            .into_iter()
            .find(|item| item.product_id == product_id)
            .expect("stock")
    };
    assert_eq!(shelf(soap).on_hand, 2);
    assert_eq!(shelf(soap).incoming, 0, "a received trip is not on its way");
    assert_eq!(shelf(tea).on_hand, 3, "one box of three moves three units");

    let tea_line = detail
        .lines
        .iter()
        .find(|line| line.product_id == Some(tea))
        .expect("tea line");
    let linked: i64 = store
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM stock_movements WHERE purchase_line_id = ?1",
            [tea_line.id],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(linked, 1, "the movement points back at the trip");

    // Cost is kept per base unit in thousandths: 900 for a box of three.
    let product = products(&store)
        .into_iter()
        .find(|product| product.id == tea)
        .expect("tea");
    assert_eq!(product.cost_milli, Some(300_000));

    let _ = std::fs::remove_file(path);
}

/// The point of one write: if any line cannot be received, nothing is
/// written. A saved plan left behind would be a trip the shop never made.
#[test]
fn a_refused_trip_leaves_no_plan_behind() {
    let (store, path) = store("receive-created-refused");
    let soap = stocked(&store, "Soap", 0, 1, None);
    let retired = stocked(&store, "Retired", 0, 1, None);
    delete_product(&store, retired).expect("archive");

    let error = create_and_receive(
        &store,
        None,
        "",
        &[
            PurchaseLineInput {
                product_id: soap,
                qty: 2,
                unit_cost_minor: None,
                pack_size: 1,
                expires_on: None,
            },
            PurchaseLineInput {
                product_id: retired,
                qty: 1,
                unit_cost_minor: None,
                pack_size: 1,
                expires_on: None,
            },
        ],
        None,
    )
    .expect_err("an archived product cannot be received");
    assert!(
        error.to_string().contains("archived"),
        "the operator is told why: {error}"
    );

    assert!(purchases(&store).is_empty(), "no plan is left behind");
    assert!(stock_movements(&store, soap).is_empty(), "nothing moved");
    assert_eq!(
        stock(&store)
            .into_iter()
            .find(|item| item.product_id == soap)
            .expect("stock")
            .on_hand,
        0,
        "and the shelf did not change"
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn a_finished_purchase_keeps_its_names_after_the_catalog_changes() {
    let (store, path) = store("purchase-snapshot");
    let soap = stocked(&store, "Soap", 0, 2, None);
    let id = create_purchase(
        &store,
        None,
        "",
        &[PurchaseLineInput {
            product_id: soap,
            qty: 2,
            unit_cost_minor: Some(70),
            pack_size: 1,
            expires_on: None,
        }],
        None,
    )
    .expect("plan");
    let line = purchase_detail(&store, id).expect("detail").lines[0].id;
    receive_purchase(
        &store,
        id,
        None,
        &[ReceiveLine {
            line_id: Some(line),
            product_id: soap,
            qty: 2,
            unit_cost_minor: None,
            pack_size: 1,
            expires_on: None,
        }],
        &[],
        "",
        None,
    )
    .expect("receive");

    store
        .conn()
        .execute(
            "UPDATE products SET name = 'Soap (large)', archived = 1 WHERE id = ?1",
            [soap],
        )
        .expect("archive");

    let detail = purchase_detail(&store, id).expect("detail");
    assert_eq!(
        detail.lines[0].name, "Soap",
        "the line remembers what was bought, not what the catalog says now"
    );
    assert!(
        update_purchase(
            &store,
            id,
            None,
            "",
            &[PurchaseLineInput {
                product_id: soap,
                qty: 1,
                unit_cost_minor: None,
                pack_size: 1,
                expires_on: None,
            }]
        )
        .is_err(),
        "a received trip is locked"
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn only_tracked_products_can_be_restocked() {
    let (store, path) = store("purchase-tracked");
    let loose = insert_product(&store, "Loose sweets", 100, "Shop", None, None).expect("insert");

    assert!(
        create_purchase(
            &store,
            None,
            "",
            &[PurchaseLineInput {
                product_id: loose,
                qty: 5,
                unit_cost_minor: None,
                pack_size: 1,
                expires_on: None,
            }],
            None
        )
        .is_err(),
        "an untracked product has no shelf to count"
    );

    let soap = stocked(&store, "Soap", 0, 2, None);
    let id = create_purchase(
        &store,
        None,
        "",
        &[PurchaseLineInput {
            product_id: soap,
            qty: 5,
            unit_cost_minor: None,
            pack_size: 1,
            expires_on: None,
        }],
        None,
    )
    .expect("plan");
    let line = purchase_detail(&store, id).expect("detail").lines[0].id;
    store
        .conn()
        .execute("UPDATE products SET tracked = 0 WHERE id = ?1", [soap])
        .expect("untrack");
    assert!(
        receive_purchase(
            &store,
            id,
            None,
            &[ReceiveLine {
                line_id: Some(line),
                product_id: soap,
                qty: 1,
                unit_cost_minor: None,
                pack_size: 1,
                expires_on: None,
            }],
            &[],
            "",
            None
        )
        .is_err(),
        "receiving into an untracked product would write a movement nothing reads"
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn profit_only_counts_lines_with_a_cost() {
    let (store, path) = store("margin");
    let id = insert_sale(&store, "-0 days", &[("Tea", 2, 300), ("Sugar", 1, 200)]);
    store
        .conn()
        .execute(
            "UPDATE sale_lines SET cost_milli = 100000 WHERE sale_id = ?1 AND name = 'Tea'",
            [id],
        )
        .expect("cost one line");

    let reported = margin(&store, 0, None);
    assert_eq!(
        reported.revenue_minor, 600,
        "only the costed line's revenue"
    );
    assert_eq!(reported.cost_minor, 200);
    assert_eq!(reported.profit_minor(), 400);
    assert_eq!((reported.costed_lines, reported.total_lines), (1, 2));
    assert!(!reported.is_complete(), "the sugar line had no cost");
    assert!(reported.has_cost());

    assert_eq!(margin(&store, 3, None).profit_minor(), 400);

    let _ = std::fs::remove_file(path);
}

/// A pack is a conversion on one catalog row, not a second product: both
/// codes point at the row, and the second code is never free for another
/// product to take.
#[test]
fn a_pack_lives_on_the_same_row_as_its_loose_units() {
    let (store, path) = store("pack-fields");
    let id = super::insert_product(
        &store,
        &ProductFields {
            name: "Amoxicillin",
            price_minor: 30,
            category: "Medicine",
            category_id: None,
            barcode: Some("100100"),
            sku: None,
            unit_label: "tablet",
            pack_size: 100,
            pack_price_minor: Some(2_800),
            pack_barcode: Some("100200"),
            tiers: &[],
        },
    )
    .expect("insert");

    let list = products(&store);
    assert_eq!(list.len(), 1, "a pack is not a second product");
    let product = &list[0];
    assert_eq!(product.id, id);
    assert_eq!(product.price_minor, 30, "the tablet price");
    assert_eq!(product.unit_label, "tablet");
    assert_eq!(product.pack_size, 100);
    assert_eq!(product.pack_price_minor, Some(2_800));
    assert_eq!(product.pack_barcode.as_deref(), Some("100200"));
    assert!(barcode_taken(&store, "100100", None), "the loose code");
    assert!(barcode_taken(&store, "100200", None), "the box code");

    // The box code cannot be reused as a loose code on another product.
    let error = super::insert_product(
        &store,
        &ProductFields {
            name: "Other",
            price_minor: 10,
            category: "Medicine",
            category_id: None,
            barcode: Some("100200"),
            sku: None,
            unit_label: "unit",
            pack_size: 1,
            pack_price_minor: None,
            pack_barcode: None,
            tiers: &[],
        },
    )
    .expect_err("a code belongs to one product");
    assert!(error.to_string().contains("barcode"), "{error}");

    let _ = std::fs::remove_file(path);
}

/// A pack of one is just the base unit. Keeping its second price and code
/// would offer two names for the same thing.
#[test]
fn a_pack_of_one_keeps_no_pack_price_or_code() {
    let (store, path) = store("pack-of-one");
    super::insert_product(
        &store,
        &ProductFields {
            name: "Single",
            price_minor: 500,
            category: "Shop",
            category_id: None,
            barcode: None,
            sku: None,
            unit_label: "box",
            pack_size: 1,
            pack_price_minor: Some(9_000),
            pack_barcode: Some("999"),
            tiers: &[],
        },
    )
    .expect("insert");

    let product = products(&store).into_iter().next().expect("product");
    assert_eq!(product.pack_size, 1);
    assert_eq!(product.pack_price_minor, None);
    assert_eq!(product.pack_barcode, None);
    assert!(!barcode_taken(&store, "999", None), "the code was dropped");

    let _ = std::fs::remove_file(path);
}

/// A price break is a rung on a ladder that starts at the shelf price. It is
/// stored against the product, read back in quantity order, and absent when the
/// shop prices one way.
#[test]
fn price_breaks_read_back_with_the_product_in_quantity_order() {
    let (store, path) = store("price-breaks");
    let id = super::insert_product(
        &store,
        &ProductFields {
            name: "Panadol",
            price_minor: 500,
            category: "Medicine",
            category_id: None,
            barcode: None,
            sku: None,
            unit_label: "tablet",
            pack_size: 100,
            pack_price_minor: Some(40_000),
            pack_barcode: None,
            // Deliberately out of order: the form can hold them any way and
            // the ladder is the same ladder.
            tiers: &[
                PriceTier {
                    min_qty: 50,
                    price_minor: 450,
                },
                PriceTier {
                    min_qty: 10,
                    price_minor: 475,
                },
            ],
        },
    )
    .expect("insert");

    let product = products(&store).into_iter().next().expect("product");
    assert_eq!(product.id, id);
    assert_eq!(product.price_minor, 500, "the shelf price is not a rung");
    assert_eq!(
        product.tiers,
        vec![
            PriceTier {
                min_qty: 10,
                price_minor: 475
            },
            PriceTier {
                min_qty: 50,
                price_minor: 450
            },
        ]
    );

    let _ = std::fs::remove_file(path);
}

/// Buying more cannot cost more. A rung above the rung below it is a typing
/// error, not a price, and saving it would overcharge the shop's best customer.
#[test]
fn a_price_break_above_the_shelf_price_is_refused() {
    let (store, path) = store("price-break-too-high");
    fn fields(tiers: &[PriceTier]) -> ProductFields<'_> {
        ProductFields {
            name: "Panadol",
            price_minor: 500,
            category: "Medicine",
            category_id: None,
            barcode: None,
            sku: None,
            unit_label: "tablet",
            pack_size: 1,
            pack_price_minor: None,
            pack_barcode: None,
            tiers,
        }
    }

    let error = super::insert_product(
        &store,
        &fields(&[PriceTier {
            min_qty: 10,
            price_minor: 600,
        }]),
    )
    .expect_err("a break cannot cost more than the shelf");
    assert!(error.to_string().contains("cannot be above"), "{error}");

    // A rung at the shelf price is allowed: a shop sometimes wants the till to
    // say "10+ price" without changing the amount. A ladder that climbs from
    // there is not.
    super::insert_product(
        &store,
        &fields(&[PriceTier {
            min_qty: 10,
            price_minor: 500,
        }]),
    )
    .expect("a rung at the shelf price is allowed");

    let error = super::insert_product(
        &store,
        &fields(&[
            PriceTier {
                min_qty: 10,
                price_minor: 400,
            },
            PriceTier {
                min_qty: 50,
                price_minor: 450,
            },
        ]),
    )
    .expect_err("the ladder has to keep going down");
    assert!(error.to_string().contains("cannot be above"), "{error}");

    let _ = std::fs::remove_file(path);
}

/// The form saves the whole ladder every time. Appending would leave the old
/// rungs behind and quietly keep a price the shop removed.
#[test]
fn saving_a_product_replaces_its_breaks() {
    let (store, path) = store("price-break-update");
    let id = super::insert_product(
        &store,
        &ProductFields {
            name: "Panadol",
            price_minor: 500,
            category: "Medicine",
            category_id: None,
            barcode: None,
            sku: None,
            unit_label: "tablet",
            pack_size: 1,
            pack_price_minor: None,
            pack_barcode: None,
            tiers: &[
                PriceTier {
                    min_qty: 10,
                    price_minor: 475,
                },
                PriceTier {
                    min_qty: 50,
                    price_minor: 450,
                },
            ],
        },
    )
    .expect("insert");

    super::update_product(
        &store,
        id,
        &ProductFields {
            name: "Panadol",
            price_minor: 500,
            category: "Medicine",
            category_id: None,
            barcode: None,
            sku: None,
            unit_label: "tablet",
            pack_size: 1,
            pack_price_minor: None,
            pack_barcode: None,
            tiers: &[PriceTier {
                min_qty: 20,
                price_minor: 460,
            }],
        },
    )
    .expect("update");

    let product = products(&store).into_iter().next().expect("product");
    assert_eq!(
        product.tiers,
        vec![PriceTier {
            min_qty: 20,
            price_minor: 460
        }]
    );

    let _ = std::fs::remove_file(path);
}

/// Buying a pack stores the cost per tablet, not the cost of the box, so a
/// later loose sale can be costed at the exact half-unit it cost.
#[test]
fn receiving_a_pack_stores_the_cost_per_tablet() {
    let (store, path) = store("pack-receive");
    let id = super::insert_product(
        &store,
        &ProductFields {
            name: "Amoxicillin",
            price_minor: 30,
            category: "Medicine",
            category_id: None,
            barcode: None,
            sku: None,
            unit_label: "tablet",
            pack_size: 100,
            pack_price_minor: Some(2_800),
            pack_barcode: None,
            tiers: &[],
        },
    )
    .expect("insert");
    track_product(&store, id, 0, 0, None, None).expect("track");

    let purchase = create_purchase(
        &store,
        None,
        "",
        &[PurchaseLineInput {
            product_id: id,
            qty: 2,
            unit_cost_minor: Some(1_850),
            pack_size: 100,
            expires_on: None,
        }],
        None,
    )
    .expect("plan");
    let line = purchase_detail(&store, purchase).expect("detail").lines[0].id;
    receive_purchase(
        &store,
        purchase,
        None,
        &[ReceiveLine {
            line_id: Some(line),
            product_id: id,
            qty: 2,
            unit_cost_minor: Some(1_850),
            pack_size: 100,
            expires_on: None,
        }],
        &[],
        "",
        None,
    )
    .expect("receive");

    let item = stock(&store).into_iter().next().expect("stock");
    assert_eq!(item.on_hand, 200, "two packs of 100");
    assert_eq!(item.cost_milli, Some(18_500), "1,850 / 100, exact");
    assert_eq!(item.unit_cost_minor(), Some(19), "rounded for a loose line");
    assert_eq!(item.pack_cost_minor(), Some(1_850), "back to the invoice");

    let _ = std::fs::remove_file(path);
}

/// The pack leaves the shelf as its size, and the profit uses the same
/// factor so a box sale is not costed as one tablet.
#[test]
fn a_pack_sale_moves_a_pack_of_stock_and_costs_a_pack() {
    let (store, path) = store("pack-sale");
    let id = super::insert_product(
        &store,
        &ProductFields {
            name: "Amoxicillin",
            price_minor: 30,
            category: "Medicine",
            category_id: None,
            barcode: None,
            sku: None,
            unit_label: "tablet",
            pack_size: 100,
            pack_price_minor: Some(2_800),
            pack_barcode: None,
            tiers: &[],
        },
    )
    .expect("insert");
    track_product(&store, id, 200, 0, None, None).expect("track");
    store
        .conn()
        .execute("UPDATE products SET cost_milli = 18500 WHERE id = ?1", [id])
        .expect("cost");

    record_sale(
        &store,
        &[SaleLine {
            product_id: Some(id),
            name: "Amoxicillin".to_owned(),
            qty: 1,
            unit_minor: 2_800,
            unit_factor: 100,
            unit_label: Some("pack of 100".to_owned()),
        }],
        &SaleTerms {
            rounding_minor: 0,
            tax_minor: 0,
            tendered_minor: 2_800,
            user_id: None,
            shift_uid: None,
            ..SaleTerms::default()
        },
    )
    .expect("sell a pack");

    let item = stock(&store).into_iter().next().expect("stock");
    assert_eq!(item.on_hand, 100, "one pack left the shelf");

    let reported = margin(&store, 0, None);
    assert_eq!(reported.revenue_minor, 2_800);
    assert_eq!(reported.cost_minor, 1_850, "100 tablets, not one");
    assert_eq!(reported.profit_minor(), 950);
    assert!(reported.is_complete());

    let _ = std::fs::remove_file(path);
}
#[test]
fn a_printed_date_is_read_in_the_orders_a_pack_uses() {
    assert_eq!(
        parse_expiry_date("2026-12-31").as_deref(),
        Some("2026-12-31")
    );
    assert_eq!(
        parse_expiry_date("31/12/2026").as_deref(),
        Some("2026-12-31")
    );
    assert_eq!(
        parse_expiry_date("  1/2/2027 ").as_deref(),
        Some("2027-02-01")
    );
    assert_eq!(
        parse_expiry_date("29/02/2028").as_deref(),
        Some("2028-02-29"),
        "2028 is a leap year"
    );
    assert_eq!(parse_expiry_date("29/02/2026"), None, "2026 is not");
    assert_eq!(
        parse_expiry_date("2026-02-30"),
        None,
        "a date SQLite would quietly move to March is a typo"
    );
    assert_eq!(
        parse_expiry_date("2026-13-01"),
        None,
        "there is no month 13"
    );
    assert_eq!(parse_expiry_date("31/12"), None, "a date needs a year");
    assert_eq!(parse_expiry_date(""), None, "blank is no date");
}

#[test]
fn sales_take_the_earliest_date_first_and_undated_stock_last() {
    let movements = vec![
        (StockKind::Receive, Some("2026-03-01".to_owned()), 5),
        (StockKind::Receive, Some("2026-01-01".to_owned()), 4),
        (StockKind::Receive, None, 6),
        (StockKind::Sale, None, -6),
    ];
    assert_eq!(
        derive_buckets(&movements),
        vec![
            // January is gone, March lost the other two, undated untouched.
            (Some("2026-03-01".to_owned()), 3),
            (None, 6),
        ]
    );

    // A tagged waste movement comes off its own date: pulling expired stock
    // must not eat a fresher delivery.
    let movements = vec![
        (StockKind::Receive, Some("2026-01-01".to_owned()), 5),
        (StockKind::Receive, Some("2026-06-01".to_owned()), 5),
        (StockKind::Waste, Some("2026-06-01".to_owned()), -2),
    ];
    assert_eq!(
        derive_buckets(&movements),
        vec![
            (Some("2026-01-01".to_owned()), 5),
            (Some("2026-06-01".to_owned()), 3),
        ]
    );
}

#[test]
fn a_shortfall_is_reported_against_undated_stock_not_invented_for_a_date() {
    let movements = vec![
        (StockKind::Receive, Some("2026-01-01".to_owned()), 2),
        (StockKind::Sale, None, -5),
    ];
    let buckets = derive_buckets(&movements);
    assert_eq!(buckets, vec![(None, -3)], "the shop is three short");

    let total: i64 = buckets.iter().map(|(_, on_hand)| *on_hand).sum();
    assert_eq!(total, -3, "buckets still add up to the ledger");
}

#[test]
fn a_dated_waste_cannot_make_a_date_hold_less_than_nothing() {
    let movements = vec![
        (StockKind::Receive, Some("2026-06-01".to_owned()), 3),
        (StockKind::Waste, Some("2026-06-01".to_owned()), -5),
    ];
    let buckets = derive_buckets(&movements);
    assert_eq!(
        buckets,
        vec![(None, -2)],
        "the two that date never held came off the rest of the shelf"
    );
    let total: i64 = buckets.iter().map(|(_, on_hand)| *on_hand).sum();
    assert_eq!(total, -2, "buckets still add up to the ledger");
}

#[test]
fn only_a_tracked_product_can_be_perishable_and_a_delivery_needs_its_date() {
    let (store, path) = store("expiry-receive");
    let milk = insert_product(&store, "Milk", 2_000, "Dairy", None, None).expect("product");

    assert!(
        set_perishability(&store, milk, true, Some(7)).is_err(),
        "an untracked product has no stock for a date to sit on"
    );

    track_product(&store, milk, 0, 0, None, None).expect("track");
    set_perishability(&store, milk, true, Some(7)).expect("perishable");
    assert!(stock(&store)[0].perishable);
    assert_eq!(stock(&store)[0].shelf_life_days, Some(7));

    assert!(
        record_stock_movement(&store, milk, 6, StockKind::Receive, "Delivery", None, None).is_err(),
        "a perishable delivery has to carry the printed date"
    );
    assert!(
        record_stock_movement(
            &store,
            milk,
            6,
            StockKind::Receive,
            "Delivery",
            Some("next Tuesday"),
            None,
        )
        .is_err(),
        "a date that is not a date is refused"
    );

    record_stock_movement(
        &store,
        milk,
        6,
        StockKind::Receive,
        "Delivery",
        Some("31/07/2026"),
        None,
    )
    .expect("dated delivery");
    assert_eq!(
        stock_movements(&store, milk)[0].expires_on.as_deref(),
        Some("2026-07-31"),
        "stored the way dates are compared"
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn expiring_stock_is_what_is_left_on_each_date_and_pulling_it_leaves_the_rest() {
    let (store, path) = store("expiry-pull");
    let milk = insert_product(&store, "Milk", 2_000, "Dairy", None, None).expect("product");
    track_product(&store, milk, 0, 0, None, None).expect("track");
    set_perishability(&store, milk, true, None).expect("perishable");

    let delivery = |date: &str, qty: i64| PurchaseLineInput {
        product_id: milk,
        qty,
        unit_cost_minor: Some(1_000),
        pack_size: 1,
        expires_on: Some(date.to_owned()),
    };
    create_and_receive(&store, None, "", &[delivery("2020-01-01", 2)], None)
        .expect("past delivery");
    create_and_receive(&store, None, "", &[delivery("2099-01-01", 3)], None)
        .expect("later delivery");

    let buckets = expiring(&store);
    assert_eq!(buckets.len(), 2);
    assert_eq!(buckets[0].expires_on.as_deref(), Some("2020-01-01"));
    assert!(buckets[0].is_expired(), "the date has been and gone");
    assert_eq!(buckets[0].on_hand, 2);
    assert!(!buckets[1].is_expired());

    assert_eq!(
        pull_expired(&store, milk, "2020-01-01", None).expect("pull"),
        2
    );
    assert_eq!(stock(&store)[0].on_hand, 3, "the later delivery stays");
    assert!(
        pull_expired(&store, milk, "2020-01-01", None).is_err(),
        "that date has nothing left"
    );

    let left = expiring(&store);
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].expires_on.as_deref(), Some("2099-01-01"));

    let waste = &stock_movements(&store, milk)[0];
    assert_eq!(waste.kind, StockKind::Waste);
    assert_eq!(waste.delta, -2);
    assert_eq!(waste.expires_on.as_deref(), Some("2020-01-01"));

    let _ = std::fs::remove_file(path);
}

#[test]
fn one_product_can_arrive_on_two_dates_but_not_the_same_date_twice() {
    let (store, path) = store("expiry-lines");
    let milk = insert_product(&store, "Milk", 2_000, "Dairy", None, None).expect("product");
    track_product(&store, milk, 0, 0, None, None).expect("track");
    set_perishability(&store, milk, true, None).expect("perishable");

    let line = |date: &str| PurchaseLineInput {
        product_id: milk,
        qty: 1,
        unit_cost_minor: None,
        pack_size: 1,
        expires_on: Some(date.to_owned()),
    };
    create_and_receive(
        &store,
        None,
        "",
        &[line("2026-06-01"), line("2026-07-01")],
        None,
    )
    .expect("two printed dates on one trip");
    assert_eq!(stock(&store)[0].on_hand, 2);
    assert_eq!(expiring(&store).len(), 2);

    assert!(
        create_and_receive(
            &store,
            None,
            "",
            &[line("2026-08-01"), line("2026-08-01")],
            None
        )
        .is_err(),
        "the same product on the same date is one line"
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn a_basket_discount_is_written_and_read_back() {
    let (store, path) = store("sale-discount");
    let lines = vec![SaleLine {
        product_id: None,
        name: "Sugar".to_owned(),
        qty: 4,
        unit_minor: 2_500,
        unit_factor: 1,
        unit_label: None,
    }];

    let sale = record_sale(
        &store,
        &lines,
        &SaleTerms {
            tendered_minor: 10_000,
            discount_minor: 1_000,
            discount_bp: 1_000,
            customer_name: Some("Mama Njeri"),
            ..SaleTerms::default()
        },
    )
    .expect("record");

    assert_eq!(sale.total_minor, 9_000, "the net is what the customer owes");
    assert_eq!(sale.change_minor, 1_000);

    let detail = sale_detail(&store, sale.id).expect("the sale just written");
    assert_eq!(detail.total_minor, 9_000);
    assert_eq!(detail.discount_minor, 1_000);
    assert_eq!(detail.discount_bp, 1_000);
    assert_eq!(detail.price_mode, "retail");
    assert_eq!(detail.customer_name.as_deref(), Some("Mama Njeri"));
    assert_eq!(detail.paid_minor, 9_000);

    let _ = std::fs::remove_file(path);
}

#[test]
fn a_discount_larger_than_the_basket_is_refused() {
    let (store, path) = store("sale-discount-over");
    let lines = vec![SaleLine {
        product_id: None,
        name: "Tea".to_owned(),
        qty: 1,
        unit_minor: 1_000,
        unit_factor: 1,
        unit_label: None,
    }];

    assert!(
        record_sale(
            &store,
            &lines,
            &SaleTerms {
                tendered_minor: 0,
                discount_minor: 1_001,
                ..SaleTerms::default()
            },
        )
        .is_err(),
        "a discount cannot exceed the lines it comes off"
    );
    assert!(
        record_sale(
            &store,
            &lines,
            &SaleTerms {
                tendered_minor: 1_000,
                discount_bp: 10_001,
                ..SaleTerms::default()
            },
        )
        .is_err(),
        "a rate above the whole basket is not a rate"
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn a_basket_discount_comes_out_of_margin_by_its_share() {
    let (store, path) = store("margin-discount");
    // Gross 600 + 200 = 800; a tenth off leaves 720 to the customer.
    let id = insert_sale(&store, "-0 days", &[("Tea", 2, 300), ("Sugar", 1, 200)]);
    store
        .conn()
        .execute(
            "UPDATE sales SET discount_minor = 80, discount_bp = 1000, total_minor = 720
             WHERE id = ?1",
            [id],
        )
        .expect("write the discount");
    store
        .conn()
        .execute(
            "UPDATE sale_lines SET cost_milli = 100000 WHERE sale_id = ?1 AND name = 'Tea'",
            [id],
        )
        .expect("cost one line");

    let reported = margin(&store, 0, None);
    assert_eq!(
        reported.revenue_minor, 540,
        "600 gross less the tea line's 60 share of the discount"
    );
    assert_eq!(reported.discount_minor, 80, "the shop gave 80 away");
    assert_eq!(reported.cost_minor, 200);
    assert_eq!(reported.profit_minor(), 340);

    let _ = std::fs::remove_file(path);
}

#[test]
fn a_customer_name_is_unique_while_live_and_freed_by_archiving() {
    let (store, path) = store("customers");
    let first = save_customer(&store, None, "Mama Njeri", Some("0700 000 000"), 500).expect("add");
    assert!(
        customer_name_taken(&store, "mama njeri", None),
        "the index is case-insensitive"
    );
    assert!(
        save_customer(&store, None, "Mama Njeri", None, 0).is_err(),
        "two live customers cannot share a name"
    );

    // Saving the same row keeps it, and does not count itself as a clash.
    save_customer(&store, Some(first), "Mama Njeri", None, 750).expect("re-save");
    let listed = customers(&store);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].discount_bp, 750);
    assert_eq!(listed[0].phone, None, "a cleared phone is stored as none");

    archive_customer(&store, first).expect("archive");
    assert!(
        customers(&store).is_empty(),
        "the counter no longer offers an archived customer"
    );
    assert!(
        !customer_name_taken(&store, "Mama Njeri", None),
        "the name is free once the row is archived"
    );

    let second = save_customer(&store, None, "Mama Njeri", None, 0).expect("reuse the name");
    assert_ne!(second, first, "a new row, not the archived one");
    assert_eq!(customers(&store).len(), 1);

    let _ = std::fs::remove_file(path);
}

/// A sale sold on credit is written with no tenders and keeps the whole amount
/// as its balance. The flag must survive the write: it is what makes the sale
/// findable when the customer comes back with the money.
#[test]
fn a_credit_sale_is_marked_and_owes_the_whole_amount() {
    let (store, path) = store("sale-credit-record");
    let customer = save_customer(&store, None, "Mama Njeri", None, 0).expect("customer");

    let sale = record_sale(
        &store,
        &[SaleLine {
            product_id: None,
            name: "Sugar".to_owned(),
            qty: 1,
            unit_minor: 10_000,
            unit_factor: 1,
            unit_label: None,
        }],
        &SaleTerms {
            payments: &[],
            is_on_credit: true,
            customer_id: Some(customer),
            customer_name: Some("Mama Njeri"),
            ..SaleTerms::default()
        },
    )
    .expect("credit sale");

    let detail = sale_detail(&store, sale.id).expect("detail");
    assert!(
        detail.is_on_credit,
        "the flag is stored, not just in memory"
    );
    assert_eq!(detail.customer_id, Some(customer));
    assert_eq!(detail.settled_minor, 0);
    assert_eq!(detail.balance_minor, 10_000);

    let owed = credit_sales(&store, customer);
    assert_eq!(owed.len(), 1);
    assert_eq!(owed[0].id, sale.id);
    assert_eq!(owed[0].balance_minor, 10_000);

    let _ = std::fs::remove_file(path);
}

/// The customer comes back twice. Each payment is another row against the
/// same sale, and the balance shrinks until there is nothing left to show.
#[test]
fn a_customer_pays_off_a_credit_sale_over_several_visits() {
    let (store, path) = store("sale-credit-settle");
    let customer = save_customer(&store, None, "Mama Njeri", None, 0).expect("customer");

    let sale = record_sale(
        &store,
        &[SaleLine {
            product_id: None,
            name: "Sugar".to_owned(),
            qty: 1,
            unit_minor: 10_000,
            unit_factor: 1,
            unit_label: None,
        }],
        &SaleTerms {
            payments: &[],
            is_on_credit: true,
            customer_id: Some(customer),
            customer_name: Some("Mama Njeri"),
            ..SaleTerms::default()
        },
    )
    .expect("credit sale");

    record_credit_payment(
        &store,
        &CreditPayment {
            sale_id: sale.id,
            method: "cash",
            amount_minor: 4_000,
            tendered_minor: 5_000,
            reference: None,
            shift_uid: Some("today"),
            user_id: Some("cashier"),
        },
    )
    .expect("first payment");
    let detail = sale_detail(&store, sale.id).expect("detail");
    assert_eq!(detail.settled_minor, 4_000);
    assert_eq!(detail.balance_minor, 6_000, "cash change is not a payment");
    assert_eq!(detail.change_minor, 1_000);
    assert_eq!(credit_sales(&store, customer)[0].balance_minor, 6_000);

    record_credit_payment(
        &store,
        &CreditPayment {
            sale_id: sale.id,
            method: "mtn_momo",
            amount_minor: 6_000,
            tendered_minor: 6_000,
            reference: Some("TX123"),
            shift_uid: Some("today"),
            user_id: Some("cashier"),
        },
    )
    .expect("second payment");
    let detail = sale_detail(&store, sale.id).expect("detail");
    assert_eq!(detail.settled_minor, 10_000);
    assert_eq!(detail.balance_minor, 0);
    assert!(
        credit_sales(&store, customer).is_empty(),
        "nothing left to settle"
    );
    assert_eq!(detail.payments.len(), 2, "both visits are on the receipt");

    let _ = std::fs::remove_file(path);
}

/// A later payment cannot take more than the customer owes, and a failed
/// payment leaves no row behind.
#[test]
fn a_credit_payment_cannot_exceed_the_balance() {
    let (store, path) = store("sale-credit-overpay");
    let customer = save_customer(&store, None, "Mama Njeri", None, 0).expect("customer");
    let sale = record_sale(
        &store,
        &[SaleLine {
            product_id: None,
            name: "Sugar".to_owned(),
            qty: 1,
            unit_minor: 10_000,
            unit_factor: 1,
            unit_label: None,
        }],
        &SaleTerms {
            payments: &[],
            is_on_credit: true,
            customer_id: Some(customer),
            customer_name: Some("Mama Njeri"),
            ..SaleTerms::default()
        },
    )
    .expect("credit sale");

    assert!(
        record_credit_payment(
            &store,
            &CreditPayment {
                sale_id: sale.id,
                method: "cash",
                amount_minor: 11_000,
                tendered_minor: 11_000,
                reference: None,
                shift_uid: None,
                user_id: None,
            },
        )
        .is_err(),
        "cannot collect more than the balance"
    );
    let detail = sale_detail(&store, sale.id).expect("detail");
    assert_eq!(detail.payments.len(), 0, "a refused payment writes nothing");

    let _ = std::fs::remove_file(path);
}

/// Credit needs somebody to come back and pay. A walk-in cannot be found
/// again, so the write is refused even if a caller sets the flag.
#[test]
fn a_walk_in_cannot_be_sold_on_credit() {
    let (store, path) = store("sale-credit-walkin");
    let result = record_sale(
        &store,
        &[SaleLine {
            product_id: None,
            name: "Sugar".to_owned(),
            qty: 1,
            unit_minor: 1_000,
            unit_factor: 1,
            unit_label: None,
        }],
        &SaleTerms {
            payments: &[],
            is_on_credit: true,
            customer_id: None,
            ..SaleTerms::default()
        },
    );
    assert!(result.is_err(), "there is nobody to bill later");
    assert_eq!(crate::db::totals(&store, 3, None), (0, 0));

    let _ = std::fs::remove_file(path);
}

/// A payment taken today against yesterday's sale must land in today's drawer.
/// Before payments carried their own shift, it was filed with the closed one.
#[test]
fn a_later_payment_counts_in_the_shift_that_took_it() {
    let (store, path) = store("sale-credit-attribution");
    let customer = save_customer(&store, None, "Mama Njeri", None, 0).expect("customer");
    let sale = record_sale(
        &store,
        &[SaleLine {
            product_id: None,
            name: "Sugar".to_owned(),
            qty: 1,
            unit_minor: 10_000,
            unit_factor: 1,
            unit_label: None,
        }],
        &SaleTerms {
            payments: &[],
            is_on_credit: true,
            customer_id: Some(customer),
            shift_uid: Some("yesterday"),
            ..SaleTerms::default()
        },
    )
    .expect("credit sale");

    assert_eq!(drawer_totals(&store, "yesterday").cash_minor, 0);
    record_credit_payment(
        &store,
        &CreditPayment {
            sale_id: sale.id,
            method: "cash",
            amount_minor: 4_000,
            tendered_minor: 4_000,
            reference: None,
            shift_uid: Some("today"),
            user_id: Some("bob"),
        },
    )
    .expect("later payment");

    assert_eq!(
        drawer_totals(&store, "today").cash_minor,
        4_000,
        "the money is in the drawer being counted"
    );
    assert_eq!(drawer_totals(&store, "yesterday").cash_minor, 0);

    let people = drawer_by_person(&store, "today");
    assert_eq!(people.len(), 1);
    assert_eq!(people[0].cash_minor, 4_000);

    let _ = std::fs::remove_file(path);
}

/// A till that already holds sales and payments must upgrade cleanly: the
/// attribution columns are backfilled from the rows that were there before.
#[test]
fn migration_23_upgrades_an_existing_database() {
    let path = std::env::temp_dir().join(format!("pos-migrate-23-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    {
        let mut store = storage::Store::open(&path).expect("open");
        store.migrate(&MIGRATIONS[..22]).expect("migrate to 22");
        store
            .conn()
            .execute(
                "INSERT INTO sales (id, total_minor, uid, shift_uid, created_at)
                 VALUES (1, 1_000, 'sale-uid', 'old-shift', '2026-01-01 10:00:00')",
                [],
            )
            .expect("old sale");
        store
            .conn()
            .execute(
                "INSERT INTO payments
                    (sale_id, method, amount_minor, tendered_minor, change_minor, uid)
                 VALUES (1, 'cash', 1_000, 1_000, 0, 'pay-uid')",
                [],
            )
            .expect("old payment");
    }
    {
        let mut store = storage::Store::open(&path).expect("reopen");
        store.migrate(MIGRATIONS).expect("migrate to latest");
        let (shift, created): (Option<String>, Option<String>) = store
            .conn()
            .query_row(
                "SELECT shift_uid, created_at FROM payments WHERE uid = 'pay-uid'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read backfilled payment");
        assert_eq!(shift.as_deref(), Some("old-shift"));
        assert!(created.is_some(), "the timestamp is taken from the sale");
    }
    let _ = std::fs::remove_file(&path);
}

/// A normal sale still refuses a short tender: the credit path must not leak
/// into the legacy one.
#[test]
fn a_normal_sale_still_requires_the_full_tender() {
    let (store, path) = store("sale-credit-legacy");
    let result = record_sale(
        &store,
        &[SaleLine {
            product_id: None,
            name: "Sugar".to_owned(),
            qty: 1,
            unit_minor: 10_000,
            unit_factor: 1,
            unit_label: None,
        }],
        &SaleTerms {
            payments: &[PaymentLine {
                method: "cash",
                amount_minor: 4_000,
                tendered_minor: 4_000,
                reference: None,
            }],
            ..SaleTerms::default()
        },
    );
    assert!(result.is_err(), "a short tender is still a failed sale");

    let _ = std::fs::remove_file(path);
}

/// Shelves are the table's order, counts come from live products, and a shelf
/// that only exists on a product is still offered to the sidebars.
#[test]
fn shelves_follow_saved_order_and_count_live_products() {
    let (store, path) = store("shelf-order");

    insert_category(&store, "Bakery", 5).expect("bakery");
    insert_category(&store, "Drinks", 2).expect("drinks");
    let same = insert_category(&store, "bakery", 99).expect("case-insensitive lookup");
    let bakery: i64 = store
        .conn()
        .query_row(
            "SELECT id FROM categories WHERE name = 'Bakery'",
            [],
            |row| row.get(0),
        )
        .expect("the original row");
    assert_eq!(same, bakery, "a second spelling cannot add a shelf");

    insert_product(&store, "Croissant", 1_000, "Bakery", Some(bakery), None).expect("croissant");
    insert_product(&store, "Cola", 1_000, "Drinks", None, None).expect("cola");
    insert_product(&store, "Mystery", 1_000, "Other", None, None).expect("product-only shelf");

    let listed = categories(&store);
    let shelves: Vec<(&str, usize)> = listed
        .iter()
        .map(|shelf| (shelf.name.as_str(), shelf.count))
        .collect();
    assert_eq!(
        shelves,
        [("Drinks", 1), ("Bakery", 1), ("Other", 1)],
        "saved order first, then names the table never saw"
    );

    let _ = std::fs::remove_file(path);
}

/// The shop code is a key, not a label: two live rows cannot claim one, and a
/// search by code or by barcode resolves to the same row.
#[test]
fn shop_codes_are_unique_and_find_their_product() {
    let (store, path) = store("shop-codes");
    fn fields<'a>(name: &'a str, sku: Option<&'a str>) -> ProductFields<'a> {
        ProductFields {
            name,
            price_minor: 1_000,
            category: "Drinks",
            category_id: None,
            barcode: Some("6001"),
            sku,
            unit_label: "unit",
            pack_size: 1,
            pack_price_minor: None,
            pack_barcode: None,
            tiers: &[],
        }
    }
    let id = super::insert_product(&store, &fields("Tea", Some("TEA-100"))).expect("tea");

    assert_eq!(find_by_code(&store, Some("tea-100"), None), Some(id));
    assert_eq!(find_by_code(&store, None, Some("6001")), Some(id));
    assert!(
        !sku_taken(&store, "TEA-100", Some(id)),
        "itself is not a clash"
    );
    assert!(
        super::insert_product(&store, &fields("Tea again", Some("TEA-100"))).is_err(),
        "a second live product cannot claim the same code"
    );

    let _ = std::fs::remove_file(path);
}
