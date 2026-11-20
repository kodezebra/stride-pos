//! The state tests: one file per behaviour the screens rely on.
//!
//! They drive `State` the way a screen does — through the same methods,
//! against a temporary store — so a change in behaviour shows up here first.

use super::*;

/// A till with a real database, because the payment path writes.
fn state(name: &str, currency: Currency) -> (State, PathBuf) {
    let path = std::env::temp_dir().join(format!("pos-state-{name}-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut store = storage::Store::open(&path).expect("open database");
    store.migrate(crate::db::MIGRATIONS).expect("migrate");
    // A cart only ever holds real products — `sale_lines.product_id` is a
    // foreign key — so the first row is the one `line()` points at.
    seed_product(&store, "Test item", 100, None);
    let mut state = State::new(
        store,
        0,
        Vec::new(),
        Settings::default(),
        Paths {
            data: std::env::temp_dir(),
            settings: PathBuf::from("settings.json"),
        },
        currency,
        false,
    );
    // Paying needs somebody signed in, because the sale records who took the
    // money. The secret is never checked on this path, so a placeholder hash
    // is enough to have a real row for the foreign key.
    let cashier = crate::db::insert_user(
        &state.store,
        "Test cashier",
        crate::auth::Role::Cashier,
        crate::auth::CredentialKind::Pin,
        "$argon2id$placeholder",
    )
    .expect("seed cashier");
    state.session = Some(Session {
        user_id: cashier.id,
        name: cashier.name,
        role: cashier.role,
    });
    (state, path)
}

fn line(name: &str, qty: i64, unit_minor: i64) -> CartLine {
    CartLine {
        product_id: 1,
        name: name.to_owned(),
        qty,
        unit_minor,
        unit_factor: 1,
        unit_label: None,
        tiers: vec![PriceTier {
            min_qty: 1,
            price_minor: unit_minor,
        }],
        tier_qty: None,
        wholesale: false,
    }
}

/// Seed a plain, loose product row, bypassing the form draft.
fn seed_product(store: &storage::Store, name: &str, price: i64, barcode: Option<&str>) {
    crate::db::insert_product(
        store,
        &crate::db::ProductFields {
            name,
            price_minor: price,
            category: "Test",
            category_id: None,
            barcode,
            unit_label: "unit",
            pack_size: 1,
            pack_price_minor: None,
            pack_barcode: None,
            tiers: &[],
        },
    )
    .expect("seed product");
}

/// Seed a pack product: one tablet price, one box price, no codes.
fn seed_pack(
    store: &storage::Store,
    name: &str,
    price: i64,
    pack_size: i64,
    pack_price: Option<i64>,
) -> i64 {
    crate::db::insert_product(
        store,
        &crate::db::ProductFields {
            name,
            price_minor: price,
            category: "Test",
            category_id: None,
            barcode: None,
            unit_label: "tablet",
            pack_size,
            pack_price_minor: pack_price,
            pack_barcode: None,
            tiers: &[],
        },
    )
    .expect("seed pack")
}

/// Seed a loose product with a price ladder, bypassing the form draft.
fn seed_ladder(store: &storage::Store, name: &str, price: i64, tiers: &[PriceTier]) -> i64 {
    crate::db::insert_product(
        store,
        &crate::db::ProductFields {
            name,
            price_minor: price,
            category: "Test",
            category_id: None,
            barcode: None,
            unit_label: "tablet",
            pack_size: 1,
            pack_price_minor: None,
            pack_barcode: None,
            tiers,
        },
    )
    .expect("seed ladder")
}

/// A trip that was never saved is still a trip: planning at the market and
/// counting in should not need a save in between, and nothing reaches the
/// shelf except through receiving.
#[test]
fn a_trip_planned_at_the_market_is_received_in_one_go() {
    let (mut state, path) = state("receive-unsaved", Currency::UGX);
    state
        .store
        .conn()
        .execute("UPDATE products SET tracked = 1 WHERE id = 1", [])
        .expect("track the seeded item");
    state.reload_stock();

    state.start_purchase();
    state
        .purchase_editor
        .as_mut()
        .expect("open")
        .lines
        .push(PurchaseLineDraft {
            product_id: 1,
            name: "Test item".to_owned(),
            qty: "3".to_owned(),
            cost: "500".to_owned(),
            pack_size: "1".to_owned(),
            expires_on: String::new(),
        });

    assert_eq!(state.stock[0].on_hand, 0, "a plan moves no stock");
    state.receive_purchase();
    state.receive_purchase();

    assert!(state.purchase_editor.is_none(), "the trip is closed");
    let trip = state.purchases.first().expect("one trip");
    assert_eq!(trip.status, crate::db::PurchaseStatus::Received);
    assert_eq!(trip.count_label(), "3 items");
    assert_eq!(trip.cost_minor, Some(1_500), "three at five hundred");
    assert_eq!(state.stock[0].on_hand, 3, "the shelf moved on receiving");

    let _ = std::fs::remove_file(path);
}

/// Stock is written once and there is no undo, so the Received button asks
/// twice: the first press arms it, the second records the trip.
#[test]
fn receiving_asks_for_a_second_press() {
    let (mut state, path) = state("receive-confirm", Currency::UGX);
    state
        .store
        .conn()
        .execute("UPDATE products SET tracked = 1 WHERE id = 1", [])
        .expect("track the seeded item");
    state.reload_stock();

    state.start_purchase();
    state
        .purchase_editor
        .as_mut()
        .expect("open")
        .lines
        .push(PurchaseLineDraft {
            product_id: 1,
            name: "Test item".to_owned(),
            qty: "2".to_owned(),
            cost: String::new(),
            pack_size: "1".to_owned(),
            expires_on: String::new(),
        });

    state.receive_purchase();
    let draft = state
        .purchase_editor
        .as_ref()
        .expect("the trip is still open");
    assert!(
        draft.confirm_receive,
        "the first press only arms the button"
    );
    assert_eq!(state.stock[0].on_hand, 0, "nothing reached the shelf");
    assert!(state.purchases.is_empty(), "and no trip was written");

    state.receive_purchase();
    assert!(
        state.purchase_editor.is_none(),
        "the second press records it"
    );
    assert_eq!(state.stock[0].on_hand, 2, "the shelf moved once");
    assert_eq!(
        state.purchases[0].status,
        crate::db::PurchaseStatus::Received
    );

    let _ = std::fs::remove_file(path);
}

/// A count sheet typed at the market can carry a line that brought nothing
/// home. Writing that list must not refuse the whole trip over a zero the
/// receiving step is about to ignore, and the saved plan must not claim to
/// have ordered something nobody bought.
#[test]
fn an_unsaved_trip_can_count_a_line_at_zero() {
    let (mut state, path) = state("receive-unsaved-zero", Currency::UGX);
    seed_product(&state.store, "Soap", 200, None);
    state
        .store
        .conn()
        .execute("UPDATE products SET tracked = 1", [])
        .expect("track everything");
    state.reload_stock();

    state.start_purchase();
    for (product_id, name, qty) in [(1, "Test item", "2"), (2, "Soap", "0")] {
        state
            .purchase_editor
            .as_mut()
            .expect("open")
            .lines
            .push(PurchaseLineDraft {
                product_id,
                name: name.to_owned(),
                qty: qty.to_owned(),
                cost: String::new(),
                pack_size: "1".to_owned(),
                expires_on: String::new(),
            });
    }
    state.receive_purchase();
    state.receive_purchase();

    assert!(
        state
            .purchase_editor
            .as_ref()
            .is_none_or(|draft| draft.error.is_none()),
        "a zero line is not an error"
    );
    let trip = state.purchases.first().expect("the trip");
    assert_eq!(trip.status, crate::db::PurchaseStatus::Received);
    assert_eq!(trip.count_label(), "2 items", "only what was bought");
    let detail = crate::db::purchase_detail(&state.store, trip.id).expect("detail");
    assert_eq!(detail.lines.len(), 1, "the zero never reached the plan");
    let soap = state
        .stock
        .iter()
        .find(|item| item.product_id == 2)
        .expect("soap is still tracked");
    assert_eq!(soap.on_hand, 0, "nothing came home for it");

    let _ = std::fs::remove_file(path);
}

/// The list is the count sheet: a line counted short is recorded as short,
/// a line counted at zero was not bought, and a line taken off the sheet was
/// never bought at all. One trip, three different truths.
#[test]
fn a_received_trip_keeps_the_difference_between_plan_and_count() {
    let (mut state, path) = state("receive-short", Currency::UGX);
    for (name, price) in [("Test item", 100), ("Soap", 200), ("Tea", 300)] {
        seed_product(&state.store, name, price, None);
    }
    state
        .store
        .conn()
        .execute("UPDATE products SET tracked = 1", [])
        .expect("track everything");
    state.reload_stock();
    state.reload_products();

    state.start_purchase();
    for (product_id, name, qty) in [(1, "Test item", "5"), (2, "Soap", "4"), (3, "Tea", "2")] {
        state
            .purchase_editor
            .as_mut()
            .expect("open")
            .lines
            .push(PurchaseLineDraft {
                product_id,
                name: name.to_owned(),
                qty: qty.to_owned(),
                cost: "500".to_owned(),
                pack_size: "1".to_owned(),
                expires_on: String::new(),
            });
    }
    state.save_purchase();
    assert!(state.purchase_editor.is_none(), "saving returns to Restock");
    assert_eq!(state.purchases[0].count_label(), "11 items");

    let id = state.purchases[0].id;
    state.edit_purchase(id);
    let draft = state.purchase_editor.as_mut().expect("reopened");
    draft.lines[0].qty = "2".to_owned(); // short
    draft.lines[1].qty = "0".to_owned(); // not bought
    state.purchase_remove_line(2); // never bought
    state.receive_purchase();
    state.receive_purchase();

    let trip = state.purchases.first().expect("the trip");
    assert_eq!(trip.status, crate::db::PurchaseStatus::Received);
    assert_eq!(trip.count_label(), "2 of 9 bought");
    assert_eq!(trip.cost_minor, Some(1_000), "two at five hundred");

    let detail = crate::db::purchase_detail(&state.store, trip.id).expect("the trip, in full");
    assert_eq!(
        detail.lines.len(),
        2,
        "the line taken off the sheet is gone"
    );
    let soap = state
        .stock
        .iter()
        .find(|item| item.product_id == 2)
        .expect("soap is still tracked");
    assert_eq!(soap.on_hand, 0, "a line counted at zero adds nothing");
    let tea = state
        .stock
        .iter()
        .find(|item| item.product_id == 3)
        .expect("tea is still tracked");
    assert_eq!(tea.on_hand, 0, "a line taken off the sheet adds nothing");

    let _ = std::fs::remove_file(path);
}

/// The dashboard is open to everyone; the scope is what narrows. A cashier
/// reads their own sales, a manager reads the shop's, and an approval buys
/// a cashier the shop view for the same minute it buys anything else.
#[test]
fn a_report_is_scoped_to_the_person_unless_a_manager_is_here() {
    let (mut state, path) = state("scope", Currency::UGX);
    let cashier = state.session.as_ref().expect("signed in").user_id.clone();

    assert_eq!(state.report_scope(), Scope::Person(cashier.as_str()));
    assert_eq!(state.report_scope().user(), Some(cashier.as_str()));
    assert!(!state.report_scope().is_shop());

    // A manager sees the shop without asking anyone.
    state.session.as_mut().expect("signed in").role = crate::auth::Role::Manager;
    assert_eq!(state.report_scope(), Scope::Shop);
    assert_eq!(state.report_scope().user(), None);

    // A cashier with a manager's approval gets the same view for a minute.
    state.session.as_mut().expect("signed in").role = crate::auth::Role::Cashier;
    state.elevated_until = Some(Instant::now() + std::time::Duration::from_secs(60));
    assert!(state.report_scope().is_shop());

    // With nobody signed in there is no shop to report on.
    state.elevated_until = None;
    state.session = None;
    assert_eq!(state.report_scope(), Scope::Person(""));
    assert_eq!(state.report_scope().user(), Some(""));

    let _ = std::fs::remove_file(path);
}

/// The edit field shows an amount without its symbol and the save path
/// parses it back. If the two ever disagree, an operator opens a product,
/// presses Save without touching anything, and the price changes.
#[test]
fn plain_amount_round_trips_through_parse() {
    for currency in [Currency::UGX, Currency::KES] {
        for minor in [0, 1, 100, 2_800, 12_345] {
            let text = plain_amount(minor, currency);
            let parsed = Money::parse(&text, currency)
                .unwrap_or_else(|error| panic!("{currency:?} {minor} -> {text:?}: {error}"));
            assert_eq!(
                parsed.minor(),
                minor,
                "{currency:?} {minor} formatted as {text:?}"
            );
        }
    }
}

/// The case the whole rounding path exists for: 340 shillings is not a
/// tendered amount, 300 is.
#[test]
fn the_due_rounds_to_the_smallest_coin() {
    let (mut state, path) = state("rounding", Currency::UGX);

    state.cart = vec![line("Sugar", 1, 340)];
    assert_eq!(
        state.cart_total_minor(),
        340,
        "the exact total is untouched"
    );
    assert_eq!(state.cart_rounding_minor(), -40);
    assert_eq!(state.due_minor(), 300);

    // An exact multiple does not move at all.
    state.cart = vec![line("Sugar", 1, 300)];
    assert_eq!(state.cart_rounding_minor(), 0);
    assert_eq!(state.due_minor(), 300);

    // A currency with a one-cent coin never rounds, so a card payment in
    // dollars is never a cent short.
    state.currency = Currency::USD;
    state.cart = vec![line("Sugar", 1, 340)];
    assert_eq!(state.cart_rounding_minor(), 0);
    assert_eq!(state.due_minor(), 340);

    let _ = std::fs::remove_file(path);
}

/// Setting five of something should be one key and one Set, not four taps
/// on plus. The prefilled count is replaced by the first key so the cashier
/// does not have to clear a `1` before typing `5`.
#[test]
fn a_cart_quantity_can_be_typed_directly() {
    let (mut state, path) = state("cart-quantity", Currency::UGX);
    state.cart = vec![line("Sugar", 1, 340)];

    state.edit_cart_quantity(0);
    let editor = state.cart_quantity.as_mut().expect("editor open");
    assert_eq!(editor.digits, "1");
    editor.push_digit('5');
    assert_eq!(editor.digits, "5", "the first key replaces the count");
    editor.push_digit('0');
    assert_eq!(editor.value(), Some(50), "later keys append");

    assert_eq!(state.cart[0].qty, 1, "nothing changes until Set");
    state.set_cart_quantity(1, 1, 50);
    assert_eq!(state.cart[0].qty, 50);
    assert!(state.cart_quantity.is_none(), "Set closes the editor");

    let _ = std::fs::remove_file(path);
}

/// Zero is not a quantity. Refusing it must leave the editor open so the
/// cashier can correct a mistyped count instead of losing the row.
#[test]
fn a_zero_cart_quantity_is_refused() {
    let (mut state, path) = state("cart-quantity-zero", Currency::UGX);
    state.cart = vec![line("Sugar", 1, 340)];

    state.edit_cart_quantity(0);
    let editor = state.cart_quantity.as_mut().expect("editor open");
    editor.clear();
    editor.push_digit('0');
    assert_eq!(editor.value(), None);

    state.set_cart_quantity(1, 1, 0);
    assert_eq!(state.cart[0].qty, 1);
    assert!(
        state.cart_quantity.is_some(),
        "the editor stays open for a correction"
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn a_sale_empties_the_cart_and_closes_the_payment() {
    let (mut state, path) = state("paid", Currency::UGX);
    state.cart = vec![line("Sugar", 1, 340), line("Soap", 2, 500)];

    state.start_payment();
    state.payment.as_mut().expect("open").tendered = "2000".to_owned();
    state.confirm_payment();

    assert!(
        state.payment.is_none(),
        "the tender panel closes with the sale"
    );
    assert!(state.cart.is_empty(), "the till is ready for the next sale");

    // The cart is already empty, so the sale is read back from the ledger.
    let sale_id = crate::db::sales(&state.store, 3, 10, None)[0].id;
    let detail = crate::db::sale_detail(&state.store, sale_id).expect("the sale was recorded");
    assert_eq!(detail.total_minor, 1_340);
    assert_eq!(detail.rounding_minor, -40, "1340 rounds down to 1300");
    assert_eq!(detail.paid_minor, 1_300);
    assert_eq!(detail.change_minor, 700);
    assert_eq!(detail.lines.len(), 2);
    assert_eq!(detail.lines[0].name, "Sugar");

    assert_eq!(crate::db::totals(&state.store, 3, None), (1, 1_300));
    // The sale carries the person, not the shop: a till with three cashiers
    // has to be able to say which of them took this money.
    assert_eq!(
        crate::db::sales(&state.store, 3, 10, None)[0]
            .cashier
            .as_deref(),
        Some("Test cashier")
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn an_empty_cash_field_means_the_exact_amount() {
    let (mut state, path) = state("exact", Currency::UGX);
    state.cart = vec![line("Sugar", 1, 300)];

    state.start_payment();
    state.confirm_payment();

    assert!(state.payment.is_none());
    let sale_id = crate::db::sales(&state.store, 3, 10, None)[0].id;
    let detail = crate::db::sale_detail(&state.store, sale_id).expect("the sale was recorded");
    assert_eq!(detail.tendered_minor, 300);
    assert_eq!(detail.change_minor, 0);

    let _ = std::fs::remove_file(path);
}

/// A mistyped tender is the most common mistake at a till. It must cost one
/// field, not the basket.
#[test]
fn a_short_tender_keeps_the_cart_and_writes_nothing() {
    let (mut state, path) = state("short", Currency::UGX);
    state.cart = vec![line("Sugar", 1, 340)];

    state.start_payment();
    state.payment.as_mut().expect("open").tendered = "200".to_owned();
    state.confirm_payment();

    let payment = state.payment.as_ref().expect("still open");
    assert_eq!(payment.error.as_deref(), Some("That is USh 100 short."));
    assert_eq!(state.cart.len(), 1, "the basket is untouched");
    assert_eq!(crate::db::totals(&state.store, 3, None), (0, 0));

    // Correcting the field completes the sale.
    state.payment.as_mut().expect("open").tendered = "500".to_owned();
    state.confirm_payment();
    assert!(state.payment.is_none());
    assert!(state.cart.is_empty());

    let _ = std::fs::remove_file(path);
}

#[test]
fn cancelling_a_payment_keeps_the_cart() {
    let (mut state, path) = state("cancel", Currency::UGX);
    state.cart = vec![line("Sugar", 1, 340)];

    state.start_payment();
    state.close_payment();

    assert!(state.payment.is_none());
    assert_eq!(state.cart.len(), 1);
    assert_eq!(crate::db::totals(&state.store, 3, None), (0, 0));

    let _ = std::fs::remove_file(path);
}

#[test]
fn pay_does_nothing_on_an_empty_cart() {
    let (mut state, path) = state("empty-pay", Currency::UGX);

    state.start_payment();
    assert!(state.payment.is_none());

    let _ = std::fs::remove_file(path);
}

#[test]
fn tapping_a_listed_sale_opens_its_receipt() {
    let (mut state, path) = state("sale-open", Currency::UGX);
    state.cart = vec![line("Sugar", 1, 400)];

    state.start_payment();
    state.confirm_payment();

    let id = crate::db::sales(&state.store, 3, 10, None)[0].id;
    state.open_sale(id);

    let detail = state.sale_open.as_ref().expect("the receipt opened");
    assert_eq!(detail.id, id);
    assert_eq!(detail.lines.len(), 1);
    assert_eq!(detail.lines[0].name, "Sugar");

    state.close_sale();
    assert!(state.sale_open.is_none());

    let _ = std::fs::remove_file(path);
}

#[test]
fn an_unnamed_category_lands_in_general() {
    let (mut state, path) = state("general", Currency::UGX);

    state.product_editor = Some(ProductDraft {
        id: None,
        name: "Hot water".to_owned(),
        price: "100".to_owned(),
        category: "   ".to_owned(),
        category_id: None,
        barcode: String::new(),
        unit_label: "unit".to_owned(),
        pack_size: "1".to_owned(),
        pack_price: String::new(),
        pack_barcode: String::new(),
        derived_price_minor: None,
        price_typed: true,
        tracked: false,
        perishable: false,
        shelf_life_days: String::new(),
        tiers: Vec::new(),
        error: None,
    });
    state.save_product();

    assert!(state.product_editor.is_none(), "the form closed");
    let saved = state
        .products
        .iter()
        .find(|product| product.name == "Hot water")
        .expect("saved with its default category");
    assert_eq!(saved.category, "General");

    let _ = std::fs::remove_file(path);
}

/// The form is where a ladder is typed. It saves with the product, and reopens
/// with the same rungs so changing one is an edit rather than a retype.
#[test]
fn a_quantity_break_typed_into_the_form_saves_with_the_product() {
    let (mut state, path) = state("product-breaks", Currency::UGX);

    state.start_product();
    {
        let draft = state.product_editor.as_mut().expect("open");
        draft.name = "Panadol".to_owned();
        draft.price = "500".to_owned();
        draft.add_tier();
        draft.add_tier();
        draft.tiers[0].qty = "10".to_owned();
        draft.tiers[0].price = "475".to_owned();
        draft.tiers[1].qty = "50".to_owned();
        draft.tiers[1].price = "450".to_owned();
    }
    state.save_product();
    assert!(state.product_editor.is_none(), "the form closed");

    let product = state
        .products
        .iter()
        .find(|product| product.name == "Panadol")
        .expect("saved")
        .clone();
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

    state.edit_product(&product);
    let draft = state.product_editor.as_ref().expect("reopened");
    assert_eq!(draft.tiers.len(), 2);
    assert_eq!(draft.tiers[0].qty, "10");
    assert_eq!(draft.tiers[1].qty, "50");
    let first = Money::parse(draft.tiers[0].price.trim(), state.currency).expect("a price");
    assert_eq!(first.minor(), 475);

    // Saving the untouched form is not a change: the ladder is rewritten from
    // the same numbers rather than doubled or dropped.
    state.save_product();
    let product = state
        .products
        .iter()
        .find(|product| product.name == "Panadol")
        .expect("still there");
    assert_eq!(product.tiers.len(), 2);

    let _ = std::fs::remove_file(path);
}

/// A break the till cannot honour keeps the form open with the reason. Saving
/// it would charge the shop's best customer more than a walk-in.
#[test]
fn a_break_above_the_shelf_price_keeps_the_form_open() {
    let (mut state, path) = state("product-break-high", Currency::UGX);

    state.start_product();
    {
        let draft = state.product_editor.as_mut().expect("open");
        draft.name = "Panadol".to_owned();
        draft.price = "500".to_owned();
        draft.add_tier();
        draft.tiers[0].qty = "10".to_owned();
        draft.tiers[0].price = "600".to_owned();
    }
    state.save_product();

    let draft = state.product_editor.as_ref().expect("the form stays open");
    let error = draft.error.as_deref().expect("an error");
    assert!(error.contains("cannot be above"), "{error}");
    assert!(
        !state
            .products
            .iter()
            .any(|product| product.name == "Panadol"),
        "nothing was written"
    );

    let _ = std::fs::remove_file(path);
}

/// Half a row is a row the operator is still typing. It stops the save and
/// names the missing half instead of guessing a price.
#[test]
fn a_break_row_without_a_price_is_refused() {
    let (mut state, path) = state("product-break-half", Currency::UGX);

    state.start_product();
    {
        let draft = state.product_editor.as_mut().expect("open");
        draft.name = "Panadol".to_owned();
        draft.price = "500".to_owned();
        draft.add_tier();
        draft.tiers[0].qty = "10".to_owned();
    }
    state.save_product();

    let draft = state.product_editor.as_ref().expect("the form stays open");
    assert_eq!(draft.error.as_deref(), Some("A price break needs a price."));
    assert!(
        !state
            .products
            .iter()
            .any(|product| product.name == "Panadol"),
        "nothing was written"
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn a_supplier_needs_a_name_and_survives_archive() {
    let (mut state, path) = state("suppliers", Currency::UGX);

    state.start_supplier();
    state.save_supplier();
    assert!(
        state.supplier_editor.is_some(),
        "a blank name keeps the form open"
    );
    assert_eq!(
        state
            .supplier_editor
            .as_ref()
            .and_then(|draft| draft.error.as_deref()),
        Some("A supplier needs a name.")
    );

    if let Some(draft) = state.supplier_editor.as_mut() {
        draft.name = "  Acme  ".to_owned();
        draft.contact_name = "Amina".to_owned();
        draft.phone = "+256 700".to_owned();
    }
    state.save_supplier();

    assert!(state.supplier_editor.is_none(), "the form closed");
    assert_eq!(state.suppliers.len(), 1);
    assert_eq!(state.suppliers[0].name, "Acme", "the name is trimmed");
    assert_eq!(state.suppliers[0].contact_name.as_deref(), Some("Amina"));
    let id = state.suppliers[0].id.clone();

    state.start_supplier();
    if let Some(draft) = state.supplier_editor.as_mut() {
        draft.name = "acme".to_owned();
    }
    state.save_supplier();
    assert!(
        state.supplier_editor.is_some(),
        "a duplicate name keeps the form open"
    );
    assert_eq!(
        state
            .supplier_editor
            .as_ref()
            .and_then(|draft| draft.error.as_deref()),
        Some("Another supplier already uses that name. Show archived suppliers if it is hidden.")
    );

    state.set_supplier_archived(&id, true);
    assert!(state.suppliers[0].archived);

    state.set_supplier_archived(&id, false);
    assert!(!state.suppliers[0].archived);

    let _ = std::fs::remove_file(path);
}

#[test]
fn category_suggestions_lead_with_the_catalog_and_ignore_case() {
    let catalog = vec!["Drinks".to_owned()];
    let suggestions = category_suggestions(&catalog, "drinks ");

    // "Drinks" holds the field already, so the catalog is skipped, "Pantry"
    // style names never invented here, and the starter list follows in full.
    assert!(
        !suggestions
            .iter()
            .any(|name| name.eq_ignore_ascii_case("Drinks"))
    );
    assert!(suggestions.len() >= 2);
    assert!(
        suggestions[1..]
            .iter()
            .all(|name| STARTER_CATEGORIES.contains(&name.as_str()))
    );

    let catalog = vec!["Pantry".to_owned()];
    let suggestions = category_suggestions(&catalog, "");
    assert_eq!(
        suggestions[0], "Pantry",
        "a catalog name comes before the studio list"
    );
}

#[test]
fn an_import_saves_what_parses_and_counts_the_rest() {
    let (mut state, path) = state("import", Currency::UGX);

    let csv = std::env::temp_dir().join(format!("pos-import-{}.csv", std::process::id()));
    std::fs::write(&csv, "name,category,price,barcode\nSoap 250g,Household,1200,111\nSoap lite,Household,free\nMilo tin,Pantry,9000,111").expect("write csv");
    state.start_import();
    state.import.as_mut().expect("open").path = csv.display().to_string();
    state.import_products();

    assert!(state.import.is_none(), "a partial import still closes");
    assert_eq!(state.products.len(), 2, "one seeded, one imported");
    let soap = state
        .products
        .iter()
        .find(|product| product.name == "Soap 250g")
        .expect("the well-formed row landed");
    assert_eq!(soap.barcode.as_deref(), Some("111"));
    assert_eq!(
        state.notice.as_deref(),
        Some("Imported 1 product, skipped 2.")
    );

    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(csv);
}

#[test]
fn a_chosen_path_means_the_same_from_either_chooser() {
    let (mut state, path) = state("finish-ask", Currency::UGX);

    // An import: the file is read and its folder remembered, exactly as it
    // was when the in-app picker answered the same question.
    let folder = std::env::temp_dir().join(format!("pos-finish-{}", std::process::id()));
    std::fs::create_dir_all(&folder).expect("folder");
    let csv = folder.join("catalog.csv");
    std::fs::write(
        &csv,
        "name,category,price,barcode\nSoap 250g,Household,1200,111\n",
    )
    .expect("write csv");

    state.finish_ask(crate::picker::Purpose::Import, csv);

    assert!(
        state.import.is_none(),
        "the import ran and its modal closed"
    );
    assert_eq!(state.import_dir.as_deref(), Some(folder.as_path()));
    assert_eq!(state.products.len(), 2, "one seeded, one imported");

    // An export: the folder is remembered and the file lands in it.
    state.finish_ask(
        crate::picker::Purpose::Export(crate::picker::ExportKind::AllProducts),
        folder.clone(),
    );

    assert_eq!(state.export_dir.as_deref(), Some(folder.as_path()));
    let written = std::fs::read_dir(&folder)
        .expect("folder")
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "csv"))
        .count();
    assert_eq!(written, 2, "the imported catalog and the export beside it");

    let _ = std::fs::remove_dir_all(&folder);
    let _ = std::fs::remove_file(path);
}

#[test]
fn an_import_that_lands_nothing_keeps_the_modal_and_gives_the_reason() {
    let (mut state, path) = state("import-err", Currency::UGX);

    let csv = std::env::temp_dir().join(format!("pos-import-empty-{}.csv", std::process::id()));
    std::fs::write(&csv, "name,category,price\n").expect("write csv");
    state.start_import();
    state.import.as_mut().expect("open").path = csv.display().to_string();
    state.import_products();

    let error = state
        .import
        .as_ref()
        .expect("still open")
        .error
        .as_deref()
        .expect("a reason");
    assert_eq!(
        error,
        "Nothing was imported: the file has no products in it."
    );

    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(csv);
}

#[test]
fn tracking_and_an_adjustment_go_through_the_state_as_the_screen_does() {
    let (mut state, path) = state("stock-track", Currency::UGX);

    state.start_tracking();
    {
        let draft = state.stock_editor.as_mut().expect("open");
        draft.product_id = Some(1);
        draft.opening = "12".to_owned();
        draft.low_stock = "3".to_owned();
    }
    state.save_tracking();

    assert!(state.stock_editor.is_none(), "a saved form closes");
    assert_eq!(state.stock.len(), 1);
    assert_eq!(state.stock[0].on_hand, 12);
    assert_eq!(state.stock[0].low_stock, 3);

    // An adjustment asks for the count; the state stores the difference.
    state.start_stock_move(1, "Test item", 12, crate::db::StockKind::Adjust);
    {
        let draft = state.stock_move.as_mut().expect("open");
        draft.quantity = "9".to_owned();
        draft.note = "Counted".to_owned();
    }
    state.save_stock_move();

    assert!(state.stock_move.is_none());
    assert_eq!(state.stock[0].on_hand, 9);

    state.open_stock_history(1, "Test item");
    let history = state.stock_history.as_ref().expect("history");
    assert_eq!(history.movements.len(), 2);
    assert_eq!(history.movements[0].delta, -3);
    assert_eq!(history.movements[0].note.as_deref(), Some("Counted"));

    // The catalog is the source of the name Inventory shows, and archiving a
    // product takes it out of both lists. Both changes have to reach the
    // lists without a restart.
    let product = state.products[0].clone();
    state.edit_product(&product);
    state.product_editor.as_mut().expect("open").name = "Renamed item".to_owned();
    state.save_product();
    assert_eq!(state.stock[0].name, "Renamed item");

    state.delete_product(1);
    assert!(state.products.is_empty());
    assert!(state.stock.is_empty());

    let _ = std::fs::remove_file(path);
}

#[test]
fn an_adjustment_that_changes_nothing_keeps_the_form_and_explains() {
    let (mut state, path) = state("stock-zero", Currency::UGX);

    state.start_tracking();
    {
        let draft = state.stock_editor.as_mut().expect("open");
        draft.product_id = Some(1);
        draft.opening = "5".to_owned();
    }
    state.save_tracking();

    state.start_stock_move(1, "Test item", 5, crate::db::StockKind::Adjust);
    state.stock_move.as_mut().expect("open").quantity = "5".to_owned();
    state.save_stock_move();

    let draft = state.stock_move.as_ref().expect("the form stays open");
    assert_eq!(
        draft.error.as_deref(),
        Some("That matches what the till already has.")
    );
    assert_eq!(state.stock[0].on_hand, 5);

    let _ = std::fs::remove_file(path);
}

/// A phone sell is a keyboard scan without the keyboard: it fills the same
/// search field and asks the grid to submit it. If the routing skipped that,
/// the phone would leave a barcode on the screen and nothing in the cart.
#[test]
fn a_phone_sell_fills_the_scan_field_and_asks_it_to_submit() {
    let (mut state, path) = state("phone-sell", Currency::UGX);

    state.receive_scan(ScanEvent {
        barcode: "4006381333931".to_owned(),
        action: ScanAction::Sell,
    });

    assert_eq!(state.module, crate::modules::SELL);
    assert_eq!(state.sell_search, "4006381333931");
    assert!(state.scan_focus);
    assert!(state.scan_submit);

    let _ = std::fs::remove_file(path);
}

/// `Add` opens the same form the Products screen opens, with the barcode
/// already in it, so the operator only has to name and price the item.
#[test]
fn a_manager_add_scan_opens_the_product_form_with_the_barcode() {
    let (mut state, path) = state("phone-add", Currency::UGX);
    state.session.as_mut().expect("signed in").role = crate::auth::Role::Manager;

    state.receive_scan(ScanEvent {
        barcode: " ABC-123 ".to_owned(),
        action: ScanAction::Add,
    });

    assert_eq!(state.module, crate::modules::PRODUCTS);
    let draft = state.product_editor.as_ref().expect("the form opens");
    assert_eq!(draft.barcode, " ABC-123 ");

    let _ = std::fs::remove_file(path);
}

/// A cashier cannot change the catalog, so an add scan must say who can
/// rather than opening a form whose Save would be refused later.
#[test]
fn a_cashier_add_scan_is_refused_with_an_explanation() {
    let (mut state, path) = state("phone-add-refused", Currency::UGX);

    state.receive_scan(ScanEvent {
        barcode: "4006381333931".to_owned(),
        action: ScanAction::Add,
    });

    assert_eq!(state.module, crate::modules::SELL);
    assert!(state.product_editor.is_none());
    assert_eq!(
        state.notice.as_deref(),
        Some("A manager must add products from a phone scan.")
    );

    let _ = std::fs::remove_file(path);
}

/// A pack and a loose tablet are one product but two lines: adding the box
/// must not merge into the loose row, and editing one count leaves the
/// other alone.
#[test]
fn a_pack_and_a_loose_unit_are_two_cart_lines() {
    let (mut state, path) = state("cart-pack", Currency::UGX);
    let id = seed_pack(&state.store, "Amoxicillin", 30, 100, Some(2_800));
    state.reload_products();
    let product = state
        .products
        .iter()
        .find(|product| product.id == id)
        .expect("the pack product")
        .clone();

    state.add_to_cart(&product, 1);
    state.add_to_cart(&product, 100);
    state.add_to_cart(&product, 100);

    assert_eq!(state.cart.len(), 2, "loose and sealed do not merge");
    let loose = &state.cart[0];
    assert_eq!((loose.qty, loose.unit_factor), (1, 1));
    assert_eq!(loose.unit_minor, 30);
    assert_eq!(loose.unit_label.as_deref(), Some("tablet"));
    let pack = &state.cart[1];
    assert_eq!((pack.qty, pack.unit_factor), (2, 100));
    assert_eq!(pack.unit_minor, 2_800);
    assert_eq!(pack.unit_label.as_deref(), Some("pack of 100"));
    assert_eq!(pack.total_minor(), 5_600);

    state.set_cart_quantity(id, 100, 3);
    assert_eq!(state.cart[1].qty, 3, "the box count changed");
    assert_eq!(state.cart[0].qty, 1, "the loose count did not");

    let _ = std::fs::remove_file(path);
}

/// A break is a rule about the count, not a choice at the counter: the line
/// takes the deepest rung it reaches, and takes the price back when the count
/// falls below it.
#[test]
fn a_quantity_break_prices_the_whole_line_at_the_rung_it_reaches() {
    let (mut state, path) = state("cart-break", Currency::UGX);
    let id = seed_ladder(
        &state.store,
        "Panadol",
        500,
        &[
            PriceTier {
                min_qty: 10,
                price_minor: 475,
            },
            PriceTier {
                min_qty: 50,
                price_minor: 450,
            },
        ],
    );
    state.reload_products();
    let product = state
        .products
        .iter()
        .find(|product| product.id == id)
        .expect("the ladder product")
        .clone();

    state.add_to_cart(&product, 1);
    assert_eq!(state.cart[0].unit_minor, 500, "one is the shelf price");
    assert_eq!(state.cart[0].tier_qty, None);
    assert_eq!(state.cart[0].tier_label(), None);

    // Nine are still the shelf price: the break starts at ten, and the whole
    // line moves at once rather than the tenth unit alone.
    state.set_cart_quantity(id, 1, 9);
    assert_eq!(state.cart[0].unit_minor, 500);
    assert_eq!(state.cart[0].total_minor(), 4_500);

    state.set_cart_quantity(id, 1, 10);
    assert_eq!(state.cart[0].unit_minor, 475);
    assert_eq!(state.cart[0].total_minor(), 4_750);
    assert_eq!(state.cart[0].tier_label().as_deref(), Some("10+ price"));

    state.set_cart_quantity(id, 1, 60);
    assert_eq!(state.cart[0].unit_minor, 450);
    assert_eq!(state.cart[0].tier_label().as_deref(), Some("50+ price"));

    // Changed their mind: the shelf price comes back with the count.
    state.set_cart_quantity(id, 1, 3);
    assert_eq!(state.cart[0].unit_minor, 500);
    assert_eq!(state.cart[0].tier_qty, None);

    let _ = std::fs::remove_file(path);
}

/// Scanning is the same path as typing: the tenth scan crosses the break and
/// the line drops without the cashier doing anything else.
#[test]
fn scanning_up_to_a_break_moves_the_whole_line_to_it() {
    let (mut state, path) = state("cart-break-scan", Currency::UGX);
    let id = seed_ladder(
        &state.store,
        "Panadol",
        500,
        &[PriceTier {
            min_qty: 10,
            price_minor: 475,
        }],
    );
    state.reload_products();
    let product = state
        .products
        .iter()
        .find(|product| product.id == id)
        .expect("the ladder product")
        .clone();

    for _ in 0..9 {
        state.add_to_cart(&product, 1);
    }
    assert_eq!(state.cart[0].qty, 9);
    assert_eq!(state.cart[0].unit_minor, 500);

    state.add_to_cart(&product, 1);
    assert_eq!(state.cart[0].qty, 10);
    assert_eq!(state.cart[0].unit_minor, 475);
    assert_eq!(state.cart[0].total_minor(), 4_750);

    let _ = std::fs::remove_file(path);
}

/// A wholesale buyer takes the deepest break on the ladder from the first unit:
/// the price is the shop's floor, not a reward for buying ten.
#[test]
fn wholesale_takes_the_deepest_break_from_one_unit() {
    let (mut state, path) = state("cart-wholesale-floor", Currency::UGX);
    let id = seed_ladder(
        &state.store,
        "Panadol",
        500,
        &[
            PriceTier {
                min_qty: 10,
                price_minor: 475,
            },
            PriceTier {
                min_qty: 50,
                price_minor: 450,
            },
        ],
    );
    state.reload_products();
    let product = state
        .products
        .iter()
        .find(|product| product.id == id)
        .expect("the ladder product")
        .clone();

    state.set_customer_mode(CustomerMode::Wholesale);
    state.add_to_cart(&product, 1);

    assert_eq!(state.cart[0].qty, 1);
    assert_eq!(
        state.cart[0].unit_minor, 450,
        "one unit still pays the floor"
    );
    assert_eq!(
        state.cart[0].tier_qty, None,
        "the mode explains the price, not a rung"
    );
    assert_eq!(state.cart[0].tier_label().as_deref(), Some("Wholesale"));

    let _ = std::fs::remove_file(path);
}

/// The switch reprices what is already in the basket, not only the next scan:
/// a customer who says "actually, wholesale" halfway through must not keep the
/// retail price on the lines already picked.
#[test]
fn switching_mode_reprices_the_lines_already_in_the_cart() {
    let (mut state, path) = state("cart-wholesale-reprice", Currency::UGX);
    let id = seed_ladder(
        &state.store,
        "Panadol",
        500,
        &[PriceTier {
            min_qty: 50,
            price_minor: 450,
        }],
    );
    state.reload_products();
    let product = state
        .products
        .iter()
        .find(|product| product.id == id)
        .expect("the ladder product")
        .clone();

    state.add_to_cart(&product, 1);
    assert_eq!(state.cart[0].unit_minor, 500);

    state.set_customer_mode(CustomerMode::Wholesale);
    assert_eq!(state.cart[0].unit_minor, 450);
    assert_eq!(state.cart[0].tier_label().as_deref(), Some("Wholesale"));

    let _ = std::fs::remove_file(path);
}

/// Back to retail, the count decides again: wholesale was a mode, not a change
/// to the ladder the line was added under.
#[test]
fn switching_back_to_retail_restores_the_count_break() {
    let (mut state, path) = state("cart-wholesale-back", Currency::UGX);
    let id = seed_ladder(
        &state.store,
        "Panadol",
        500,
        &[
            PriceTier {
                min_qty: 10,
                price_minor: 475,
            },
            PriceTier {
                min_qty: 50,
                price_minor: 450,
            },
        ],
    );
    state.reload_products();
    let product = state
        .products
        .iter()
        .find(|product| product.id == id)
        .expect("the ladder product")
        .clone();

    state.set_customer_mode(CustomerMode::Wholesale);
    state.add_to_cart(&product, 1);
    state.set_cart_quantity(id, 1, 10);
    assert_eq!(state.cart[0].unit_minor, 450, "wholesale ignores the count");

    state.set_customer_mode(CustomerMode::Retail);
    assert_eq!(state.cart[0].unit_minor, 475, "ten earns the ten break");
    assert_eq!(state.cart[0].tier_label().as_deref(), Some("10+ price"));

    state.set_cart_quantity(id, 1, 50);
    assert_eq!(state.cart[0].unit_minor, 450);
    assert_eq!(state.cart[0].tier_label().as_deref(), Some("50+ price"));

    let _ = std::fs::remove_file(path);
}

/// A pack's ladder is one rung at the pack price, so wholesale leaves it alone
/// and does not label it: the pack is already the bulk deal.
#[test]
fn wholesale_leaves_a_pack_at_its_pack_price() {
    let (mut state, path) = state("cart-wholesale-pack", Currency::UGX);
    let id = crate::db::insert_product(
        &state.store,
        &crate::db::ProductFields {
            name: "Amoxicillin",
            price_minor: 500,
            category: "Medicine",
            category_id: None,
            barcode: None,
            unit_label: "tablet",
            pack_size: 100,
            pack_price_minor: Some(40_000),
            pack_barcode: None,
            tiers: &[PriceTier {
                min_qty: 10,
                price_minor: 450,
            }],
        },
    )
    .expect("seed pack with a ladder");
    state.reload_products();
    let product = state
        .products
        .iter()
        .find(|product| product.id == id)
        .expect("the pack product")
        .clone();

    state.set_customer_mode(CustomerMode::Wholesale);
    state.add_to_cart(&product, 100);

    assert_eq!(
        state.cart[0].unit_minor, 40_000,
        "the pack price is already the bulk deal"
    );
    assert_eq!(
        state.cart[0].tier_label(),
        None,
        "a pack is not labelled wholesale"
    );

    let _ = std::fs::remove_file(path);
}

/// A product with no breaks charges the same either way, so the line says
/// nothing: a "Wholesale" label would promise a saving that is not there.
#[test]
fn wholesale_without_a_break_is_not_labelled() {
    let (mut state, path) = state("cart-wholesale-plain", Currency::UGX);
    let product = state
        .products
        .iter()
        .find(|product| product.name == "Test item")
        .expect("the seeded product")
        .clone();

    state.set_customer_mode(CustomerMode::Wholesale);
    state.add_to_cart(&product, 1);

    assert_eq!(state.cart[0].unit_minor, product.price_minor);
    assert_eq!(state.cart[0].tier_label(), None);

    let _ = std::fs::remove_file(path);
}

/// The next customer is a walk-in: a wholesale price must not survive the sale
/// that used it.
#[test]
fn a_sale_returns_the_till_to_retail_prices() {
    let (mut state, path) = state("cart-wholesale-reset-sale", Currency::UGX);
    state.cart = vec![line("Sugar", 1, 340)];
    state.set_customer_mode(CustomerMode::Wholesale);

    state.start_payment();
    state.payment.as_mut().expect("open").tendered = "400".to_owned();
    state.confirm_payment();

    assert_eq!(state.customer_mode, CustomerMode::Retail);
    assert!(state.cart.is_empty());

    let _ = std::fs::remove_file(path);
}

/// Clearing the basket is also the end of its terms.
#[test]
fn clearing_the_cart_returns_the_till_to_retail_prices() {
    let (mut state, path) = state("cart-wholesale-reset-clear", Currency::UGX);
    state.cart = vec![line("Sugar", 1, 340)];
    state.set_customer_mode(CustomerMode::Wholesale);

    state.clear_cart();

    assert_eq!(state.customer_mode, CustomerMode::Retail);

    let _ = std::fs::remove_file(path);
}

/// The box is already the bulk deal. A loose ladder must never reprice it, or
/// the two discounts would stack and the box price would mean nothing.
#[test]
fn a_pack_line_ignores_the_loose_ladder() {
    let (mut state, path) = state("cart-pack-break", Currency::UGX);
    let id = crate::db::insert_product(
        &state.store,
        &crate::db::ProductFields {
            name: "Amoxicillin",
            price_minor: 500,
            category: "Medicine",
            category_id: None,
            barcode: None,
            unit_label: "tablet",
            pack_size: 100,
            pack_price_minor: Some(40_000),
            pack_barcode: None,
            tiers: &[PriceTier {
                min_qty: 10,
                price_minor: 450,
            }],
        },
    )
    .expect("seed pack with a ladder");
    state.reload_products();
    let product = state
        .products
        .iter()
        .find(|product| product.id == id)
        .expect("the pack product")
        .clone();

    state.add_to_cart(&product, 1);
    state.set_cart_quantity(id, 1, 20);
    assert_eq!(state.cart[0].unit_minor, 450, "loose follows the ladder");

    state.add_to_cart(&product, 100);
    state.set_cart_quantity(id, 100, 5);
    assert_eq!(state.cart[1].unit_minor, 40_000, "the box keeps its price");
    assert_eq!(state.cart[1].tier_qty, None);

    let _ = std::fs::remove_file(path);
}

/// A box with no price cannot be sold: charging the tablet price for a
/// hundred tablets would be a giveaway, and guessing would be worse.
#[test]
fn a_pack_without_a_price_is_refused() {
    let (mut state, path) = state("cart-pack-unpriced", Currency::UGX);
    let id = seed_pack(&state.store, "Amoxicillin", 30, 100, None);
    state.reload_products();
    let product = state
        .products
        .iter()
        .find(|product| product.id == id)
        .expect("the pack product")
        .clone();

    state.add_to_cart(&product, 100);

    assert!(state.cart.is_empty());
    assert!(
        state
            .notice
            .as_deref()
            .is_some_and(|notice| notice.contains("no pack price")),
        "{:?}",
        state.notice
    );

    let _ = std::fs::remove_file(path);
}

/// The pack price pre-fills the tablet price rounded up, keeps following it
/// while untouched, and stops the moment the operator types their own.
#[test]
fn the_pack_price_helper_yields_to_a_typed_price() {
    let mut draft = ProductDraft::new("Medicine".to_owned(), None);
    draft.unit_label = "tablet".to_owned();
    draft.pack_size = "100".to_owned();
    draft.pack_price = "2800".to_owned();

    draft.sync_price_from_pack(Currency::UGX);
    assert_eq!(draft.price, "28", "2,800 / 100");

    draft.pack_price = "2900".to_owned();
    draft.sync_price_from_pack(Currency::UGX);
    assert_eq!(draft.price, "29", "the helper still owns the field");

    draft.note_price_typed();
    draft.price = "35".to_owned();
    draft.pack_price = "5000".to_owned();
    draft.sync_price_from_pack(Currency::UGX);
    assert_eq!(draft.price, "35", "a typed price is the operator's");
}

/// Clearing the price to retype it must not let the helper refill the box;
/// the next keystroke would land after the helper's digits.
#[test]
fn a_cleared_price_stays_cleared() {
    let mut draft = ProductDraft::new("Medicine".to_owned(), None);
    draft.pack_size = "100".to_owned();
    draft.pack_price = "2800".to_owned();

    draft.sync_price_from_pack(Currency::UGX);
    assert_eq!(draft.price, "28");

    draft.note_price_typed();
    draft.price.clear();
    draft.sync_price_from_pack(Currency::UGX);

    assert_eq!(draft.price, "");
}

/// Rounding up keeps the loose sale from losing money on the box: 2,801
/// over 100 is 28.01, and selling at 28 would give the box away.
#[test]
fn the_pack_price_helper_rounds_up_not_down() {
    let mut draft = ProductDraft::new("Medicine".to_owned(), None);
    draft.pack_size = "100".to_owned();
    draft.pack_price = "2801".to_owned();

    draft.sync_price_from_pack(Currency::UGX);

    assert_eq!(draft.price, "29");
}
