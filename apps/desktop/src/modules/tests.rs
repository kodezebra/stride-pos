//! The tests that outlive any one screen: the module table, the rail order
//! and the permission matrix describe the app as a whole, not a screen.

use super::*;

/// `COUNT` keeps a persisted index inside the list, and the view map is
/// what keeps that index reachable. Both are easy to break when a module is
/// added, and neither breakage is visible until a cashier lands on a screen
/// the rail cannot show them — so pin them here.
#[test]
fn the_module_table_matches_its_index() {
    assert_eq!(
        all().len(),
        COUNT,
        "COUNT must equal the number of modules or a saved index can point past the end"
    );
}

#[test]
fn the_rail_is_four_views_with_the_keys_in_order() {
    let manager = views(crate::auth::Role::Manager);
    assert_eq!(
        manager,
        &[View::Sell, View::Day, View::Shop, View::Settings][..]
    );

    // Four keys, in rail order, with no gaps: F1, F2, F3, F4.
    for (offset, view) in manager.iter().enumerate() {
        assert_eq!(
            view.hotkey() as u8,
            egui::Key::F1 as u8 + offset as u8,
            "{} should be F{}",
            view.label(),
            offset + 1
        );
    }

    let cashier = views(crate::auth::Role::Cashier);
    assert_eq!(cashier, &[View::Sell, View::Day][..]);
    assert!(
        !cashier.contains(&View::Shop) && !cashier.contains(&View::Settings),
        "a cashier rail must not offer the back office"
    );
}

#[test]
fn every_module_is_a_tab_of_exactly_one_view() {
    for view in views(crate::auth::Role::Manager) {
        let tabs = tabs(*view, crate::auth::Role::Manager);
        // A view lands on its first tab, and a single-page view has none to
        // draw — but it still has a landing module.
        if let Some(first) = tabs.first() {
            assert_eq!(
                *first,
                landing(*view),
                "{} lands on its first tab",
                view.label()
            );
        }
        for &index in tabs {
            assert_eq!(
                view_of(index),
                *view,
                "{} is on the wrong view",
                tab_label(index)
            );
        }
    }

    // No module is orphaned: each one is either the landing page of its
    // view or a tab on it.
    for index in 0..COUNT {
        let view = view_of(index);
        assert!(
            landing(view) == index || tabs(view, crate::auth::Role::Manager).contains(&index),
            "{} has no tab to live on",
            tab_label(index)
        );
    }
}

#[test]
fn the_tabs_of_a_view_are_named_and_ordered() {
    assert_eq!(
        tabs(View::Day, crate::auth::Role::Manager),
        &[SALES, DASHBOARD][..]
    );
    assert_eq!(
        tabs(View::Shop, crate::auth::Role::Manager),
        &[PRODUCTS, PURCHASES, SUPPLIERS, CUSTOMERS][..]
    );

    // A cashier reads the day's own sales; the trends that read the shop's
    // whole history are the manager's screen.
    assert_eq!(tabs(View::Day, crate::auth::Role::Cashier), &[SALES][..]);
    assert!(!reachable(crate::auth::Role::Cashier, DASHBOARD));
    assert!(!reachable(crate::auth::Role::Cashier, SETTINGS));
    assert!(reachable(crate::auth::Role::Cashier, SELL));
    assert!(reachable(crate::auth::Role::Manager, SETTINGS));

    for index in 0..COUNT {
        assert!(!tab_label(index).is_empty());
    }
}

#[test]
fn customers_are_a_shop_tab_with_the_old_settings_door() {
    assert_eq!(view_of(CUSTOMERS), View::Shop);
    assert_eq!(tab_label(CUSTOMERS), "Customers");
    assert_eq!(
        permission(CUSTOMERS),
        crate::auth::Permission::EditSettings,
        "moving the page must not widen who can change a customer's rate"
    );
    assert!(reachable(crate::auth::Role::Manager, CUSTOMERS));
    assert!(
        !reachable(crate::auth::Role::Cashier, CUSTOMERS),
        "a cashier has no Shop to manage regulars in"
    );
    assert!(
        !crate::state::SETTINGS_SECTIONS.contains(&"Customers"),
        "the old Settings section must not keep a second door open"
    );
}

fn day(day: &str, weekday: u8, revenue: i64) -> crate::db::DayTotal {
    crate::db::DayTotal {
        day: day.to_owned(),
        weekday,
        sales: 1,
        revenue,
    }
}

/// The axis names the month once and then only numbers the days, so the
/// reading of `dd/mm` is the whole contract: a leading zero reads as a
/// typo, and a misread month captions the chart with the wrong span.
#[test]
fn the_axis_names_the_week_and_numbers_the_month() {
    let week = day("26/10", 1, 0);
    assert_eq!(axis_label(&week, 7), "Mon");
    assert_eq!(axis_label(&week, 8), "Mon");
    assert_eq!(axis_label(&week, 9), "26");
    assert_eq!(axis_label(&day("01/11", 0, 0), 30), "1");
}

#[test]
fn the_caption_says_the_span_once() {
    // Newest first, the order `by_day` hands them over in.
    let crossing = [day("01/11", 0, 0), day("26/10", 1, 0)];
    assert_eq!(period_label(&crossing), "26 Oct – 1 Nov");
    let one_month = [day("31/10", 6, 0), day("12/10", 1, 0)];
    assert_eq!(period_label(&one_month), "12–31 Oct");
    assert_eq!(period_label(&[day("01/11", 0, 0)]), "1 Nov");
    assert_eq!(period_label(&[]), "");
}

#[test]
fn a_day_is_said_with_its_weekday() {
    assert_eq!(long_date(&day("01/11", 0, 0)), "Sun 1 Nov");
}

#[test]
fn money_moves_as_a_percentage_and_counts_as_units() {
    assert_eq!(
        change(1_200, Some(1_000), true),
        ("+20%".to_owned(), Trend::Up)
    );
    assert_eq!(
        change(800, Some(1_000), true),
        ("-20%".to_owned(), Trend::Down)
    );
    assert_eq!(change(3, Some(1), false), ("+2".to_owned(), Trend::Up));
    assert_eq!(change(1, Some(3), false), ("-2".to_owned(), Trend::Down));
    // A window that took nothing cannot be a percentage: the first sale in
    // it is new, and two empty windows have nothing to say.
    assert_eq!(change(5, Some(0), true), ("new".to_owned(), Trend::Up));
    assert_eq!(change(0, Some(0), true), ("—".to_owned(), Trend::Flat));
    // All time has no previous window at all.
    assert_eq!(change(5, None, false), ("—".to_owned(), Trend::Flat));
}

#[test]
fn the_comparison_says_the_window_and_whether_it_is_the_shop() {
    assert_eq!(comparison_caption(0, true), "compared with yesterday");
    assert_eq!(
        comparison_caption(1, true),
        "compared with the 7 days before"
    );
    assert_eq!(comparison_caption(3, true), "everything the shop has sold");
    assert_eq!(
        comparison_caption(1, false),
        "only your sales · compared with the 7 days before"
    );
    assert_eq!(comparison_caption(3, false), "only your sales");
    // Only all time needs to admit that its chart is capped.
    assert_eq!(chart_caption(0), None);
    assert_eq!(chart_caption(3), Some("The latest 30 selling days"));
}

fn product(id: i64, name: &str, barcode: Option<&str>) -> Product {
    Product {
        id,
        name: name.to_owned(),
        price_minor: 1_000,
        category: "Drinks".to_owned(),
        category_id: None,
        barcode: barcode.map(str::to_owned),
        sku: None,
        unit_label: "unit".to_owned(),
        pack_size: 1,
        pack_price_minor: None,
        pack_barcode: None,
        cost_milli: None,
        tiers: Vec::new(),
    }
}

/// Stock sorting is a lens over the same rows, not a different filter. An
/// untracked product has no count, so it goes last in both stock orders
/// instead of pretending to be an empty shelf.
#[test]
fn the_catalog_sorts_by_stock_with_untracked_rows_last() {
    let products = [
        product(1, "High", None),
        product(2, "Low", None),
        product(3, "Untracked", None),
    ];
    let mut high = stock_item(1, "High", "Drinks", false);
    high.on_hand = 40;
    let mut low = stock_item(2, "Low", "Drinks", false);
    low.on_hand = 3;
    let stock = [high, low];

    let visible = |sort| catalog::visible_catalog(&products, &stock, &[], 7, None, "", 0, sort);

    assert_eq!(visible(0), vec![0, 1, 2], "catalog order is the default");
    assert_eq!(visible(1), vec![1, 0, 2], "lowest stock first");
    assert_eq!(visible(2), vec![0, 1, 2], "highest stock first");
    assert_eq!(visible(3), vec![0, 1, 2], "A–Z");
}

/// A barcode is what the scanner sends, and a product *name* can contain the
/// same digits. If the name won, the wrong item would land in the cart and the
/// till would be off by the difference — silently.
#[test]
fn a_scanned_barcode_beats_a_name_that_contains_it() {
    let products = [
        product(1, "Cola 6001234 crate", None),
        product(2, "Cola", Some("6001234")),
    ];
    let visible = visible_products(&products, None, "6001234");

    assert_eq!(visible.len(), 2, "both read as matches before resolution");
    assert_eq!(
        scan_target(&products, "6001234", &visible),
        Scan::One {
            index: 1,
            unit_factor: 1
        }
    );
}

#[test]
fn the_only_product_in_view_is_the_one_a_scan_takes() {
    let products = [product(1, "Cola", None), product(2, "Fanta", None)];
    let visible = visible_products(&products, None, "fan");

    assert_eq!(
        scan_target(&products, "fan", &visible),
        Scan::One {
            index: 1,
            unit_factor: 1
        }
    );
}

/// Two tiles are a choice the cashier has to make. Picking the first would
/// put the wrong product in the cart with no way to tell it had happened.
#[test]
fn two_matches_are_a_question_not_a_choice() {
    let products = [product(1, "Cola 500", None), product(2, "Cola 1L", None)];
    let visible = visible_products(&products, None, "cola");

    assert_eq!(scan_target(&products, "cola", &visible), Scan::Ambiguous(2));
}

/// The sidebar is a browse tool, not a restriction. A cashier who left a
/// category selected must still be able to scan anything on the shelf.
#[test]
fn a_scan_ignores_the_category_sidebar() {
    let products = [product(1, "Cola", Some("6001234"))];
    let visible = visible_products(&products, Some("Food"), "");

    assert!(visible.is_empty(), "the sidebar hides the product");
    assert_eq!(
        scan_target(&products, "6001234", &visible),
        Scan::One {
            index: 0,
            unit_factor: 1
        }
    );
}

/// An exact name is the cashier confirming which of the matches they meant.
#[test]
fn an_exact_name_beats_a_looser_match() {
    let products = [product(1, "Cola 500", None), product(2, "Cola", None)];
    let visible = visible_products(&products, None, "cola");

    assert_eq!(
        scan_target(&products, "Cola", &visible),
        Scan::One {
            index: 1,
            unit_factor: 1
        }
    );
}

#[test]
fn an_unknown_code_takes_nothing() {
    let products = [product(1, "Cola", Some("6001234"))];
    let visible = visible_products(&products, None, "999");

    assert_eq!(scan_target(&products, "999", &visible), Scan::None);
    assert_eq!(scan_target(&products, "   ", &visible), Scan::None);
}

/// The box code is one scan of one thing: the line must carry the pack's
/// size so the shelf loses a hundred tablets, not one. The loose code still
/// buys a single tablet from the same row.
#[test]
fn a_pack_barcode_adds_a_pack_but_the_loose_code_adds_one() {
    let mut medicine = product(1, "Amoxicillin", Some("100100"));
    medicine.unit_label = "tablet".to_owned();
    medicine.pack_size = 100;
    medicine.pack_price_minor = Some(2_800);
    medicine.pack_barcode = Some("100200".to_owned());
    let products = [medicine];

    let loose = visible_products(&products, None, "100100");
    assert_eq!(
        scan_target(&products, "100100", &loose),
        Scan::One {
            index: 0,
            unit_factor: 1
        }
    );

    let pack = visible_products(&products, None, "100200");
    assert_eq!(
        pack,
        vec![0],
        "typing the box code finds the row before the scan resolves"
    );
    assert_eq!(
        scan_target(&products, "100200", &pack),
        Scan::One {
            index: 0,
            unit_factor: 100
        }
    );
}

/// The two product buttons answer different questions: one is "what I am
/// looking at", the other "everything this shop sells". With a filter on,
/// the files must differ in rows, not just in name.
#[test]
fn a_filtered_export_is_shorter_than_the_whole_catalog() {
    let mut products = [
        product(1, "Cola", None),
        product(2, "Bread", None),
        product(3, "Milk", None),
    ];
    products[1].category = "Food".to_owned();
    products[2].category = "Food".to_owned();

    let visible = visible_products(&products, Some("Food"), "");
    let filtered: Vec<Product> = visible
        .iter()
        .map(|&index| products[index].clone())
        .collect();

    let currency = money::tender_from_code("UGX").expect("UGX is a tender");
    assert_eq!(
        crate::export::products_csv(&filtered, currency)
            .lines()
            .count(),
        3,
        "a header and the two Food products"
    );
    assert_eq!(
        crate::export::products_csv(&products, currency)
            .lines()
            .count(),
        4,
        "a header and all three products, whatever the sidebar says"
    );
}

fn supplier(id: &str, name: &str, contact: Option<&str>, phone: Option<&str>) -> Supplier {
    Supplier {
        id: id.to_owned(),
        name: name.to_owned(),
        contact_name: contact.map(str::to_owned),
        phone: phone.map(str::to_owned),
        email: None,
        address: None,
        notes: None,
        archived: false,
    }
}

/// Archiving is the supplier equivalent of switching a staff account off:
/// the row stays, but the next order should not start from it.
#[test]
fn the_directory_hides_archived_suppliers_until_asked() {
    let mut suppliers = [
        supplier("a", "Acme", Some("Amina"), Some("+256 700")),
        supplier("b", "Beta", None, None),
    ];
    suppliers[1].archived = true;

    assert_eq!(
        visible_suppliers(&suppliers, "", false),
        [0],
        "the working list leaves archived suppliers out"
    );
    assert_eq!(
        visible_suppliers(&suppliers, "", true),
        [0, 1],
        "showing archived keeps name order"
    );
}

/// The search is the directory's whole job: a shop may remember the rep's
/// name or the number long before it remembers the company's legal name.
#[test]
fn the_supplier_search_reaches_contact_and_phone() {
    let suppliers = [
        supplier("a", "Acme", Some("Amina"), Some("+256 700")),
        supplier("b", "Beta", None, None),
    ];

    assert_eq!(visible_suppliers(&suppliers, "acme", false), [0]);
    assert_eq!(
        visible_suppliers(&suppliers, "amina", false),
        [0],
        "the contact is searchable"
    );
    assert_eq!(
        visible_suppliers(&suppliers, "700", false),
        [0],
        "the phone number is searchable"
    );
    assert!(visible_suppliers(&suppliers, "nothing", false).is_empty());
}

/// One window width is a layout, two are a contract. The bar shares one row
/// between the page's search field and its actions, so the two ways it can
/// break are a field that eats the buttons at the minimum window and a
/// cluster that runs under the field on a wide one. Both are cheap to catch
/// here and awkward to notice on a screen with the right number of products.
#[test]
fn the_page_bar_fits_its_search_and_its_actions_on_one_row() {
    // 880×560 is the smallest window the app allows: the contents minus the
    // 280 px sidebar and the 24 px workspace margins each side.
    for (window, expected_field) in [(552.0_f32, SEARCH_MIN), (1112.0, SEARCH_MAX)] {
        let ctx = egui::Context::default();
        // The bar draws semibold text, and semibold is a font family the
        // app installs at startup; a bare context would panic on the first
        // word of a button.
        let (palette, dark) = crate::theme::palette_for(ui_kit::theme::Theme::Light);
        ui_kit::theme::install(&ctx, palette, dark);

        let mut query = String::from("milk");
        let mut field = egui::Rect::NOTHING;
        let mut actions = egui::Rect::NOTHING;
        let mut consumed = 0.0;

        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(window, 560.0),
                )),
                ..Default::default()
            },
            |ui| {
                let before = ui.cursor().top();
                let response = action_bar(
                    ui,
                    PageLeft::Search {
                        text: &mut query,
                        hint: "Name, code or barcode",
                        count: Some("3 of 12".to_owned()),
                    },
                    |ui| {
                        let _ = actions::primary_compact(ui, "New product");
                        actions::toolbar_menu(ui, transfer_icon(), "Import / Export", |_| {});
                        actions = ui.min_rect();
                    },
                );
                field = response.expect("the bar has a search field").rect;
                consumed = ui.cursor().top() - before;
            },
        );
        output.drop_without_applying_deltas();

        assert!(
            (field.width() - expected_field).abs() < 0.5,
            "a {window} px bar gave the search {:.1} px, not {expected_field}",
            field.width()
        );
        assert!(
            actions.left() >= field.right(),
            "the actions start at {:.1} but the field ends at {:.1} in a {window} px bar",
            actions.left(),
            field.right()
        );
        assert!(
            actions.right() <= window + 0.5,
            "the actions run past the bar's right edge in a {window} px bar"
        );
        // The field sits on the bar's centre line, not on its top edge: a
        // 52 px row with a 34 px control left-aligned looks like a mistake
        // in both themes.
        assert!(
            (field.center().y - TOOLBAR_HEIGHT / 2.0).abs() < 1.0,
            "the search sits at {:.1} px in a {TOOLBAR_HEIGHT} px bar",
            field.center().y
        );
        assert!(
            (field.height() - SEARCH_HEIGHT).abs() < 0.5,
            "the search is {:.1} px tall, not {SEARCH_HEIGHT}",
            field.height()
        );
        // The bar plus the gap before the page, and nothing beyond that: a
        // bar that claims more pushes the first row of the table down. The
        // gap is the bar's own 8 px on top of the row spacing every panel
        // pays between two widgets.
        assert!(
            (TOOLBAR_HEIGHT..=TOOLBAR_HEIGHT + SPACING.sm + 8.0).contains(&consumed),
            "the bar consumed {consumed} px of a 560 px window"
        );
    }
}

#[test]
fn product_form_summaries_show_only_what_is_set() {
    assert_eq!(pack_summary("1", "", "", ""), "Loose only");
    assert_eq!(break_summary(0), "None");
    assert_eq!(expiry_summary(false, false, ""), "Needs Inventory tracking");
}

#[test]
fn product_form_summaries_describe_hidden_details() {
    assert_eq!(
        pack_summary("20", "2,800", "box-1", ""),
        "Pack of 20 · 2,800 · 1 barcode"
    );
    assert_eq!(
        pack_summary("12", "", "box-1", "unit-1"),
        "Pack of 12 · 2 barcodes"
    );
    assert_eq!(break_summary(1), "1 break");
    assert_eq!(break_summary(3), "3 breaks");
    assert_eq!(expiry_summary(true, false, ""), "Undated stock");
    assert_eq!(expiry_summary(true, true, ""), "Dates every delivery");
    assert_eq!(
        expiry_summary(true, true, "30"),
        "Dates every delivery · 30 days"
    );
}

fn supplier_draft(
    contact: &str,
    phone: &str,
    email: &str,
    address: &str,
    notes: &str,
) -> state::SupplierDraft {
    state::SupplierDraft {
        id: None,
        name: "Acme".to_owned(),
        contact_name: contact.to_owned(),
        phone: phone.to_owned(),
        email: email.to_owned(),
        address: address.to_owned(),
        notes: notes.to_owned(),
        error: None,
    }
}

#[test]
fn supplier_detail_summary_counts_only_filled_fields() {
    let blank = supplier_draft("", "", "", "", "");
    assert_eq!(
        suppliers::supplier_detail_summary(&blank),
        "Nothing filled yet"
    );

    let one = supplier_draft("Amina", "", "", "", "");
    assert_eq!(suppliers::supplier_detail_summary(&one), "Contact");

    let two = supplier_draft("Amina", "+256 700", "", "", "");
    assert_eq!(suppliers::supplier_detail_summary(&two), "Contact, Phone");

    let three = supplier_draft("Amina", "+256 700", "orders@example.com", "", "");
    assert_eq!(
        suppliers::supplier_detail_summary(&three),
        "Contact, Phone +1 more"
    );
}

fn supplier_with_archive(id: &str, name: &str, archived: bool) -> Supplier {
    Supplier {
        id: id.to_owned(),
        name: name.to_owned(),
        contact_name: None,
        phone: None,
        email: None,
        address: None,
        notes: None,
        archived,
    }
}

fn stock_item(id: i64, name: &str, category: &str, perishable: bool) -> StockItem {
    StockItem {
        product_id: id,
        name: name.to_owned(),
        category: category.to_owned(),
        on_hand: 10,
        low_stock: 5,
        cost_milli: None,
        pack_size: 1,
        unit_label: "unit".to_owned(),
        target_stock: None,
        incoming: 0,
        perishable,
        shelf_life_days: None,
    }
}

fn purchase_draft(
    supplier_id: Option<&str>,
    lines: Vec<(i64, &str)>,
    search: &str,
) -> state::PurchaseDraft {
    state::PurchaseDraft {
        id: None,
        supplier_id: supplier_id.map(str::to_owned),
        note: String::new(),
        lines: lines
            .into_iter()
            .map(|(product_id, name)| state::PurchaseLineDraft {
                product_id,
                name: name.to_owned(),
                qty: "1".to_owned(),
                planned_qty: None,
                cost: String::new(),
                pack_size: "1".to_owned(),
                expires_on: String::new(),
            })
            .collect(),
        search: search.to_owned(),
        error: None,
        confirm_delete: false,
        confirm_receive: false,
    }
}

#[test]
fn purchase_derive_keeps_archived_supplier_name_for_old_list() {
    let suppliers = vec![
        supplier_with_archive("acme", "Acme", true),
        supplier_with_archive("bulk", "Bulk Traders", false),
    ];
    let draft = purchase_draft(Some("acme"), vec![], "");

    let derived = purchases::derive_purchase_from(&suppliers, &[], Some(&draft));

    assert_eq!(derived.saved_supplier(), Some("Acme"));
    assert_eq!(
        derived.suppliers(),
        &[("bulk".to_owned(), "Bulk Traders".to_owned())]
    );
}

#[test]
fn purchase_derive_filters_candidates_from_one_snapshot() {
    let stock = vec![
        stock_item(1, "Milk", "Dairy", true),
        stock_item(2, "Bread", "Bakery", false),
        stock_item(3, "Butter", "Dairy", false),
    ];
    let draft = purchase_draft(None, vec![(1, "Milk")], "dai");

    let derived = purchases::derive_purchase_from(&[], &stock, Some(&draft));

    assert_eq!(derived.perishable(), &[1]);
    assert_eq!(
        derived.matches("dai"),
        vec![
            stock_item(1, "Milk", "Dairy", true),
            stock_item(3, "Butter", "Dairy", false),
        ]
    );
    assert!(derived.already_on_list(1));
    assert!(!derived.already_on_list(3));
}

#[test]
fn purchase_picker_offers_unadded_products_first() {
    let stock = vec![
        stock_item(1, "Milk", "Dairy", true),
        stock_item(2, "Bread", "Bakery", false),
        stock_item(3, "Butter", "Dairy", false),
    ];
    let draft = purchase_draft(None, vec![(1, "Milk")], "");

    let derived = purchases::derive_purchase_from(&[], &stock, Some(&draft));

    assert_eq!(
        derived.matches(""),
        vec![
            stock_item(2, "Bread", "Bakery", false),
            stock_item(3, "Butter", "Dairy", false),
            stock_item(1, "Milk", "Dairy", true),
        ]
    );
}

#[test]
fn purchase_picker_action_says_when_a_line_is_a_repeat() {
    assert_eq!(purchases::product_picker_action(false), "Add");
    assert_eq!(purchases::product_picker_action(true), "Add again");
}

#[test]
fn low_stock_action_hides_when_everything_is_handled() {
    let mut low = stock_item(1, "Milk", "Dairy", true);
    low.on_hand = 2;
    let mut listed = stock_item(2, "Bread", "Bakery", false);
    listed.on_hand = 2;
    let draft = purchase_draft(None, vec![(2, "Bread")], "");

    let actionable =
        purchases::derive_purchase_from(&[], &[low.clone(), listed.clone()], Some(&draft));
    assert!(actionable.has_low_stock_to_add());

    let mut incoming = low;
    incoming.incoming = 8;
    let handled = purchases::derive_purchase_from(&[], &[incoming, listed], Some(&draft));
    assert!(!handled.has_low_stock_to_add());

    let none = purchases::derive_purchase_from(
        &[],
        &[stock_item(3, "Butter", "Dairy", false)],
        Some(&draft),
    );
    assert!(!none.has_low_stock_to_add());
}

/// The shop's own code is a search key in the catalog, just like the barcode
/// it may not have.
#[test]
fn the_shop_code_finds_a_product_as_well_as_a_barcode() {
    let mut item = product(1, "Paracetamol 500mg", Some("6001"));
    item.sku = Some("PARA-500".to_owned());
    assert!(catalog::matches(&item, None, "para"));
    assert!(catalog::matches(&item, None, "6001"));
    assert!(!catalog::matches(&item, None, "nothing"));
}

/// The catalog shows the price the shop actually paid and what it leaves, but
/// never invents a cost for a product nobody has received yet.
#[test]
fn cost_detail_shows_the_last_cost_and_the_margin_it_leaves() {
    let mut item = product(1, "Tea", None);
    assert_eq!(
        cost_detail(&item, money::Currency::UGX),
        None,
        "an unreceived product has no cost"
    );

    item.price_minor = 3_000;
    item.cost_milli = Some(1_850_000);
    let detail = cost_detail(&item, money::Currency::UGX).expect("a received product has a cost");
    assert!(
        detail.contains("1850"),
        "cost is money, not milli units: {detail}"
    );
    assert!(
        detail.contains("38% margin"),
        "margin is the share left: {detail}"
    );
}
