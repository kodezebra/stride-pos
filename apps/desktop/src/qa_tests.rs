//! Fast, persona-driven QA of a fresh pharmacy.
//!
//! These drive the same `State` methods the screens call. No rendering, no OCR,
//! no sleeps: each workflow finishes in milliseconds, so a whole shop can be
//! exercised on every `cargo test` run.

use std::path::PathBuf;

use crate::{
    auth::{CredentialKind, Permission, Role},
    db,
    settings::Settings,
    state::{CustomerMode, Paths, Session, State},
};

/// A newly opened pharmacy with a cashier already on the till.
fn fresh(name: &str) -> (State, PathBuf) {
    let path = std::env::temp_dir().join(format!("pos-qa-{name}-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);

    let mut store = storage::Store::open(&path).expect("open a fresh database");
    store.migrate(db::MIGRATIONS).expect("migrate");
    db::set_currency(&store, money::Currency::UGX).expect("set currency");

    let cashier = db::insert_user(
        &store,
        "Cashier A",
        Role::Cashier,
        CredentialKind::Pin,
        "$argon2id$placeholder",
    )
    .expect("seed cashier");

    let settings = Settings {
        shop_name: "Bright Pharmacy".to_owned(),
        business_type: Some("pharmacy".to_owned()),
        ..Default::default()
    };

    let mut state = State::new(
        store,
        0,
        Vec::new(),
        settings,
        Paths {
            data: std::env::temp_dir(),
            settings: path.with_extension("qa-settings.json"),
        },
        money::Currency::UGX,
        false,
    );
    state.session = Some(Session {
        user_id: cashier.id,
        name: cashier.name,
        role: cashier.role,
    });
    state.reload_products();
    state.reload_customers();
    state.reload_stock();
    (state, path)
}

/// Put a real medicine on the shelf, with a real opening count.
fn medicine(state: &mut State, name: &str, price_minor: i64, barcode: &str, opening: i64) -> i64 {
    let id = db::insert_product(
        &state.store,
        &db::ProductFields {
            name,
            price_minor,
            category: "Pain Relief",
            category_id: None,
            barcode: Some(barcode),
            sku: None,
            unit_label: "tablet",
            pack_size: 20,
            pack_price_minor: None,
            pack_barcode: None,
            tiers: &[],
        },
    )
    .expect("seed medicine");
    db::track_product(&state.store, id, opening, 10, Some(60), None).expect("track stock");
    state.reload_products();
    state.reload_stock();
    id
}

fn scan(state: &mut State, product_id: i64, times: i64) {
    let product = state
        .products
        .iter()
        .find(|product| product.id == product_id)
        .expect("product is on the catalog")
        .clone();
    for _ in 0..times {
        state.add_to_cart(&product, 1);
    }
}

fn on_hand(state: &State, product_id: i64) -> i64 {
    state
        .stock
        .iter()
        .find(|item| item.product_id == product_id)
        .expect("product is tracked")
        .on_hand
}

/// A normal cashier: scan two common items, take cash, hand over a receipt.
#[test]
fn normal_cashier_completes_a_pharmacy_sale() {
    let (mut state, _db) = fresh("normal-sale");
    let paracetamol = medicine(&mut state, "Paracetamol 500mg", 500, "SCR-PARA-500", 60);
    let ors = medicine(&mut state, "ORS sachet", 200, "SCR-ORS", 40);

    scan(&mut state, paracetamol, 2);
    scan(&mut state, ors, 1);
    assert_eq!(state.cart_count(), 3);
    assert_eq!(state.cart_total_minor(), 1200);

    state.start_payment();
    assert!(state.payment.is_some());
    state.payment.as_mut().unwrap().tendered = "2000".to_owned();
    state.confirm_payment();

    assert!(state.cart.is_empty(), "the basket must clear after a sale");
    assert!(state.payment.is_none(), "the tender panel must close");
    assert_eq!(
        state.customer_mode,
        CustomerMode::Retail,
        "terms reset for the next customer"
    );
    assert_eq!(
        on_hand(&state, paracetamol),
        58,
        "sale decrements tracked stock"
    );
    assert_eq!(on_hand(&state, ors), 39);

    let sales = db::sales(&state.store, 3, 100, None);
    assert_eq!(sales.len(), 1, "exactly one sale was written");
    assert_eq!(sales[0].paid_minor, 1200);
    assert_eq!(sales[0].cashier.as_deref(), Some("Cashier A"));
    assert!(
        sales[0].receipt_no.is_some(),
        "the customer gets a receipt number"
    );
}

/// A careless cashier: a short tender is a part-payment, not a lost basket.
#[test]
fn careless_cashier_short_tender_leaves_basket_intact() {
    let (mut state, _db) = fresh("short-tender");
    let product = medicine(&mut state, "Ibuprofen 200mg", 800, "SCR-IBU-200", 25);
    scan(&mut state, product, 2);
    assert_eq!(state.cart_total_minor(), 1600);

    state.start_payment();
    state.payment.as_mut().unwrap().tendered = "500".to_owned();
    state.confirm_payment();

    let payment = state.payment.as_ref().expect("the balance is still open");
    assert_eq!(
        payment.error, None,
        "cash below the total is a part-payment"
    );
    assert_eq!(payment.tenders.len(), 1);
    assert_eq!(state.payment_remaining_minor(), 1100);
    assert_eq!(
        state.cart.len(),
        1,
        "a part-payment must not clear the basket"
    );
    assert_eq!(state.cart_count(), 2);
    assert_eq!(
        on_hand(&state, product),
        25,
        "an unpaid balance must not move stock"
    );
    assert!(db::sales(&state.store, 3, 100, None).is_empty());

    state.payment.as_mut().unwrap().tendered = "2000".to_owned();
    state.confirm_payment();
    assert_eq!(db::sales(&state.store, 3, 100, None).len(), 1);
    assert_eq!(
        on_hand(&state, product),
        23,
        "only the settled sale decrements stock"
    );
}

/// A careless cashier: broken forms are rejected before anything is written.
#[test]
fn careless_cashier_broken_forms_do_not_write_partial_records() {
    let (mut state, _db) = fresh("broken-forms");
    let products_before = state.products.len();

    state.start_product();
    let draft = state.product_editor.as_mut().unwrap();
    draft.name = "Amoxicillin 500mg".to_owned();
    draft.category = "Prescription".to_owned();
    draft.price = "not-a-price".to_owned();
    state.save_product();
    assert!(
        state.product_editor.is_some(),
        "an invalid price keeps the form open"
    );
    assert_eq!(
        state.products.len(),
        products_before,
        "no half-written product"
    );

    state.product_editor.as_mut().unwrap().price = "1250".to_owned();
    state.save_product();
    assert!(state.product_editor.is_none(), "a corrected price saves");
    assert_eq!(state.products.len(), products_before + 1);

    state.start_customer();
    let draft = state.customer_editor.as_mut().unwrap();
    draft.name = "Kiswa Clinic".to_owned();
    draft.discount = "101".to_owned();
    state.save_customer();
    assert!(
        state.customer_editor.is_some(),
        "a discount over 100% is refused"
    );
    assert!(state.customers.is_empty(), "no half-written customer");

    state.customer_editor.as_mut().unwrap().discount = "5".to_owned();
    state.save_customer();
    assert!(
        state.customer_editor.is_none(),
        "a corrected discount saves"
    );
    assert_eq!(state.customers.len(), 1);
    assert_eq!(state.customers[0].discount_bp, 500);
}

/// A shop owner: pharmacy identity, staff, and the books.
#[test]
fn owner_configures_the_pharmacy_and_adds_staff() {
    let (mut state, _db) = fresh("owner-setup");

    state.shop_draft = "   Green Cross Pharmacy   ".to_owned();
    state.save_shop_name();
    assert_eq!(state.settings.shop_name, "Green Cross Pharmacy");
    assert!(
        state.settings_dirty,
        "a letterhead change is marked unsaved"
    );

    state.business_draft = "  Pharmacy  ".to_owned();
    state.save_business_type();
    assert_eq!(state.settings.business_type.as_deref(), Some("pharmacy"));
    assert_eq!(
        crate::category_seeds::seed_names("pharmacy"),
        crate::category_seeds::seed_names(state.settings.business_type.as_deref().unwrap_or("")),
        "the seeded pharmacy shelves are the ones the owner configured",
    );

    let staff_before = state.users.len();
    state.staff.name = "  Amara  ".to_owned();
    state.staff.role = Role::Manager;
    state.add_staff();
    assert_eq!(state.users.len(), staff_before + 1, "a manager was added");
    assert!(
        state
            .users
            .iter()
            .any(|user| user.name == "Amara" && user.role == Role::Manager)
    );

    state.staff.name = "Amara".to_owned();
    state.add_staff();
    assert_eq!(
        state.users.len(),
        staff_before + 1,
        "a duplicate name is refused"
    );
    assert!(state.staff.error.is_some());

    // Reports are widened by rank, not by being signed in.
    assert!(Role::Owner.permits(Permission::ViewReports));
    assert!(Role::Manager.permits(Permission::ViewReports));
    assert!(!Role::Cashier.permits(Permission::ViewReports));
    assert!(!Role::Manager.permits(Permission::ManageUsers));
    assert!(!Role::Manager.permits(Permission::ChangeCurrency));
}

/// An impatient user: repeated taps must be idempotent at every boundary.
#[test]
fn impatient_user_repeated_clicks_never_double_charge() {
    let (mut state, _db) = fresh("double-clicks");
    let product = medicine(&mut state, "Cetirizine 10mg", 600, "SCR-CET", 30);

    scan(&mut state, product, 1);
    scan(&mut state, product, 1);
    assert_eq!(state.cart.len(), 1, "the same item stays one line");
    assert_eq!(state.cart_count(), 2);

    state.start_payment();
    state.start_payment();
    assert!(
        state.payment.is_some(),
        "Pay twice still opens one tender panel"
    );

    state.confirm_payment();
    state.confirm_payment();
    assert_eq!(
        db::sales(&state.store, 3, 100, None).len(),
        1,
        "one sale, not two"
    );
    assert_eq!(on_hand(&state, product), 28, "stock moved once");

    // A second count on a closed shift cannot overwrite the first.
    state.drawer_count = "0".to_owned();
    state.close_day();
    let first = state.drawer.clone();
    assert!(first.is_some(), "the first sale opened a shift");
    let counted = first.as_ref().and_then(|d| d.shift.counted_minor);
    assert_eq!(counted, Some(0), "the typed drawer count was recorded");
    assert!(
        first.as_ref().is_some_and(|d| d.shift.closed_at.is_some()),
        "the shift is closed"
    );

    state.drawer_count = "999999".to_owned();
    state.close_day();
    assert_eq!(
        state.drawer.as_ref().and_then(|d| d.shift.counted_minor),
        counted,
        "a second close must not overwrite the count",
    );
}

/// A malicious cashier: guardrails hold without a manager in the room.
#[test]
fn malicious_cashier_cannot_bypass_restrictions() {
    let owner_pin = "481516";
    let path = std::env::temp_dir().join(format!("pos-qa-bypass-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut store = storage::Store::open(&path).expect("reopen bypass database");
    store.migrate(db::MIGRATIONS).expect("migrate");
    let owner_hash = crate::auth::hash(owner_pin).expect("hash owner PIN");
    let owner = db::insert_user(
        &store,
        "Owner",
        Role::Owner,
        CredentialKind::Password,
        &owner_hash,
    )
    .expect("seed owner");
    let cashier = db::insert_user(
        &store,
        "Cashier A",
        Role::Cashier,
        CredentialKind::Pin,
        "$argon2id$placeholder",
    )
    .expect("seed cashier");
    let settings = Settings {
        business_type: Some("pharmacy".to_owned()),
        ..Default::default()
    };
    let mut state = State::new(
        store,
        0,
        Vec::new(),
        settings,
        Paths {
            data: std::env::temp_dir(),
            settings: path.with_extension("qa-settings.json"),
        },
        money::Currency::UGX,
        false,
    );
    state.session = Some(Session {
        user_id: cashier.id,
        name: cashier.name,
        role: cashier.role,
    });
    let product = medicine(&mut state, "Morphine 10mg", 2500, "SCR-MOR", 10);
    state.reload_products();
    state.reload_stock();

    // Back-office powers are absent from the cashier's role and from their session.
    assert!(!state.permits(Permission::EditSettings));
    assert!(!state.permits(Permission::ManageUsers));
    assert!(!state.permits(Permission::ChangeCurrency));
    assert!(!state.permits(Permission::ViewReports));

    // A negative quantity typed into the quantity modal is ignored, not a refund.
    scan(&mut state, product, 1);
    state.set_cart_quantity(product, 1, -5);
    assert_eq!(state.cart_count(), 1, "negative quantity is ignored");

    // A discount customer needs an approval the cashier cannot fake.
    let customer_id = db::save_customer(&state.store, None, "Insider", Some(""), 500)
        .expect("seed discount customer");
    state.reload_customers();
    let customer = state
        .customers
        .iter()
        .find(|customer| customer.id == customer_id)
        .expect("discount customer is in the list")
        .clone();
    state.set_customer(Some(customer.clone()));
    assert!(
        state.customer.is_none(),
        "discount is not attached without approval"
    );
    let request = state
        .override_request
        .as_ref()
        .expect("an approval was requested");
    assert_eq!(request.permission, Permission::Discount);

    // No approver chosen: refused. Wrong PIN: refused. No elevation either way.
    state.approve_override();
    assert!(
        state.override_request.is_some(),
        "an empty approver is refused"
    );
    assert!(
        !state.permits(Permission::Discount),
        "no elevation without an approver"
    );

    let request = state.override_request.as_mut().unwrap();
    request.user_id = Some(owner.id.clone());
    request.secret = "wrong".to_owned();
    state.approve_override();
    assert!(state.override_request.is_some(), "a wrong PIN is refused");
    assert!(
        !state.permits(Permission::Discount),
        "a wrong PIN grants nothing"
    );

    let request = state.override_request.as_mut().unwrap();
    request.secret = owner_pin.to_owned();
    state.approve_override();
    assert!(
        state.override_request.is_none(),
        "a real owner PIN approves"
    );
    assert!(
        state.permits(Permission::Discount),
        "approval opens the escalated action"
    );

    state.set_customer(Some(customer.clone()));
    assert_eq!(
        state.customer.as_ref().map(|c| c.id),
        Some(customer.id),
        "approved discount attaches"
    );

    // Wholesale does not stack the regular rate on top of ladder pricing.
    state.set_customer_mode(CustomerMode::Wholesale);
    assert_eq!(
        state.effective_discount_bp(),
        0,
        "discounts do not stack with wholesale"
    );

    // Negative cash is not a way to walk out with the drawer.
    state.start_payment();
    state.payment.as_mut().unwrap().tendered = "-100".to_owned();
    state.confirm_payment();
    assert!(
        state
            .payment
            .as_ref()
            .and_then(|p| p.error.clone())
            .is_some()
    );
    assert!(
        db::sales(&state.store, 3, 100, None).is_empty(),
        "no negative-tender sale"
    );
}

/// A shop running on tired hardware: a sale survives a restart, and a failed
/// action can be retried without leaving debris behind.
#[test]
fn poor_hardware_workflow_survives_a_restart() {
    let (mut state, path) = fresh("restart");
    let product = medicine(&mut state, "Vitamin C 500mg", 350, "SCR-VITC", 15);
    state.reload_products();
    state.reload_stock();

    scan(&mut state, product, 3);
    state.start_payment();
    state.payment.as_mut().unwrap().tendered = "".to_owned(); // exact cash
    state.confirm_payment();
    assert_eq!(db::sales(&state.store, 3, 100, None).len(), 1);

    // The till is closed and opened again against the same file.
    drop(state);
    let mut store = storage::Store::open(&path).expect("reopen the database");
    store
        .migrate(db::MIGRATIONS)
        .expect("migration is idempotent");
    let mut reopened = State::new(
        store,
        0,
        Vec::new(),
        Settings::default(),
        Paths {
            data: std::env::temp_dir(),
            settings: path.with_extension("qa-settings.json"),
        },
        money::Currency::UGX,
        false,
    );
    reopened.reload_products();
    reopened.reload_customers();
    reopened.reload_stock();
    let cashier = db::users(&reopened.store)
        .into_iter()
        .next()
        .expect("the cashier survived the restart");
    reopened.session = Some(Session {
        user_id: cashier.id,
        name: cashier.name,
        role: cashier.role,
    });

    assert!(
        reopened.products.iter().any(|p| p.id == product),
        "catalog persisted"
    );
    assert_eq!(on_hand(&reopened, product), 12, "stock persisted");
    let sales = db::sales(&reopened.store, 3, 100, None);
    assert_eq!(sales.len(), 1, "the sale persisted");
    assert_eq!(
        sales[0].paid_minor, 1100,
        "UGX rounds 1050 up to the next 100 note"
    );

    // A retry after a rejected action still works on the reopened till.
    scan(&mut reopened, product, 1);
    reopened.start_payment();
    reopened.payment.as_mut().unwrap().tendered = "not money".to_owned();
    reopened.confirm_payment();
    assert_eq!(reopened.cart_count(), 1, "a parse failure keeps the basket");
    reopened.payment.as_mut().unwrap().tendered = "500".to_owned();
    reopened.confirm_payment();
    assert_eq!(
        db::sales(&reopened.store, 3, 100, None).len(),
        2,
        "the retry sold"
    );
    assert_eq!(on_hand(&reopened, product), 11);
}

/// A pharmacist: a perishable medicine cannot be received without its pack date.
#[test]
fn pharmacist_must_date_a_perishable_delivery() {
    let (mut state, _db) = fresh("expiry");
    let product = medicine(&mut state, "Insulin 100IU", 4500, "SCR-INS", 10);
    db::set_perishability(&state.store, product, true, Some(60)).expect("mark perishable");
    state.reload_stock();

    state.start_purchase();
    state.purchase_add_product(product);
    let line = &mut state.purchase_editor.as_mut().unwrap().lines[0];
    line.qty = "5".to_owned();
    line.cost = "2000".to_owned();
    line.pack_size = "1".to_owned(); // five loose units, not five 20-tablet boxes
    line.expires_on = String::new();

    state.receive_purchase();
    let error = state
        .purchase_editor
        .as_ref()
        .and_then(|d| d.error.clone())
        .unwrap_or_default();
    assert!(
        error.contains("perishable"),
        "an undated delivery is refused: {error}"
    );
    assert!(
        error.contains("date"),
        "the cashier is told what to fix: {error}"
    );
    assert_eq!(on_hand(&state, product), 10, "no stock moved");
    assert!(state.expiring.is_empty(), "no dated bucket was invented");

    // A typed pack date passes validation; the first press arms the write.
    state.purchase_editor.as_mut().unwrap().lines[0].expires_on = "31/12/2099".to_owned();
    state.receive_purchase();
    assert_eq!(
        on_hand(&state, product),
        10,
        "the first press only arms receipt"
    );
    state.receive_purchase();

    assert_eq!(
        on_hand(&state, product),
        15,
        "the dated delivery reached the shelf"
    );
    let bucket = state
        .expiring
        .iter()
        .find(|bucket| bucket.product_id == product)
        .expect("the dated batch is tracked");
    assert_eq!(bucket.expires_on.as_deref(), Some("2099-12-31"));
    assert_eq!(bucket.on_hand, 5);
    assert!(!bucket.is_expired(), "a 2099 batch is not out of date");

    // FEFO takes the dated batch first when that medicine is sold.
    scan(&mut state, product, 1);
    state.start_payment();
    state.confirm_payment();
    assert_eq!(on_hand(&state, product), 14);
    let bucket = state
        .expiring
        .iter()
        .find(|bucket| bucket.product_id == product)
        .expect("the dated batch is still tracked");
    assert_eq!(bucket.on_hand, 4, "the sale came off the dated batch");
}
