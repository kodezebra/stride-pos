//! Catalog: the product list, the stock badges on it, and the forms that edit it.
//!
//! The list is the shop's memory of what it sells. Everything here reads the
//! same two facts — the product row and its stock — so the filters, the scanner
//! target, the export and the editor all live beside the list they serve.

use super::*;
// ---------------------------------------------------------------------------
// Products
// ---------------------------------------------------------------------------

pub(super) struct Products;

/// Which category both Sell and Products are filtered to.
///
/// One function, so the two sidebars cannot offer different filters over the
/// same grid.
pub(super) fn categories_sidebar(ui: &mut egui::Ui, state: &mut State) {
    components::section_title(ui, "Categories");
    if regions::nav_item(ui, state.category.is_none(), "All").clicked() {
        state.category = None;
    }

    // Cloned because the click mutates `state.category` while the list is being
    // drawn from `state.categories`; both are fields of the same struct.
    let categories = state.categories.clone();
    for category in &categories {
        let selected = state.category.as_deref() == Some(category.as_str());
        if regions::nav_item(ui, selected, category).clicked() {
            state.category = Some(category.clone());
        }
    }
}

/// Write one of the exports into the folder the picker chose.
///
/// The rows are rebuilt from the screen's current filters at the moment the
/// folder is confirmed. The modal blocks the screen while it is open, so the
/// file still matches what the operator was looking at. The template is the
/// exception: it is not a snapshot of anything, only a blank catalog to fill
/// in.
pub(crate) fn run_export(state: &mut State, kind: ExportKind, dir: &Path) {
    // "Export CSV" is what is on screen; "Export all" is the whole catalog.
    // The names of the files carry the difference, because a folder with both
    // in it must not make the operator open them to find out which one they
    // got.
    match kind {
        ExportKind::Products => {
            let needle = state.catalog_search.trim().to_lowercase();
            let visible = visible_catalog(
                &state.products,
                &state.stock,
                &state.expiring,
                state.expiry_warning_days(),
                state.category.as_deref(),
                &needle,
                state.stock_filter,
            );
            let rows: Vec<Product> = visible
                .iter()
                .map(|&index| state.products[index].clone())
                .collect();
            export_products(state, &rows, "products", dir);
        }
        ExportKind::AllProducts => {
            let rows = state.products.clone();
            export_products(state, &rows, "products-all", dir);
        }
        ExportKind::Sales => export_sales(state, dir),
        ExportKind::Template => save_template(state, dir),
    }
}

/// The line under a screen that just wrote a file, with its path one press
/// away.
///
/// A path printed in a sentence is hard to retype, and a till's exports often
/// end up in an email to the accountant, so the copy is the point.
pub(super) fn file_feedback(ui: &mut egui::Ui, state: &State) {
    let Some(notice) = state.file_notice.as_deref() else {
        return;
    };
    ui.horizontal_wrapped(|ui| {
        components::caption(ui, notice);
        if let Some(path) = state.file_path.as_deref()
            && actions::quiet_compact(ui, "Copy path").clicked()
        {
            ui.ctx().copy_text(path.display().to_string());
        }
    });
}

/// Write a product export and say where it went.
///
/// The path is the point: an operator who does not know where the file is has
/// not been helped. A failure lands in the same line a success would have, so a
/// missing export cannot look like a button that did nothing.
pub(super) fn export_products(state: &mut State, rows: &[Product], name: &str, dir: &Path) {
    let body = crate::export::products_csv(rows, state.currency);
    save_export(state, name, &body, rows.len(), ("product", "products"), dir);
}

/// Write the sales export for the range the screen is showing.
///
/// The file reads the same scope as the list: a cashier who cannot see a
/// colleague's takings on screen cannot export them either.
pub(super) fn export_sales(state: &mut State, dir: &Path) {
    let scope = state.report_scope();
    let rows = match crate::db::export_sales(&state.store, state.range, scope.user()) {
        Ok(rows) => rows,
        Err(err) => {
            state.file_path = None;
            state.file_notice = Some(format!("Export failed: {err:#}"));
            return;
        }
    };
    let body = crate::export::sales_csv(&rows, state.currency);
    save_export(state, "sales", &body, rows.len(), ("sale", "sales"), dir);
}

/// Write the blank catalog with two example rows, and point the next import at
/// the folder it went into.
///
/// The fixed name is deliberate: saving twice replaces the template instead of
/// filling the folder with copies, which is also why it carries no stamp.
pub(super) fn save_template(state: &mut State, dir: &Path) {
    let body = crate::export::template_csv(state.currency);
    match crate::export::write(dir, "products-template.csv", &body) {
        Ok(path) => {
            // The next step after saving a template is editing it somewhere
            // else and picking it back up, so start the import where it landed.
            state.import_dir = path.parent().map(Path::to_path_buf);
            state.file_path = Some(path.clone());
            state.file_notice = Some(format!(
                "Saved a template with two example rows to {}",
                path.display()
            ));
        }
        Err(err) => {
            state.file_path = None;
            state.file_notice = Some(format!("Export failed: {err:#}"));
        }
    }
}

/// Name a file after the second it was written and report it in `file_notice`.
///
/// The stamp comes from SQLite, the same clock that dates the sales inside the
/// file, so two exports cannot disagree about when the books were taken.
pub(super) fn save_export(
    state: &mut State,
    name: &str,
    body: &str,
    count: usize,
    nouns: (&str, &str),
    dir: &Path,
) {
    let stamp = match crate::db::stamp(&state.store) {
        Ok(stamp) => stamp,
        Err(err) => {
            state.file_path = None;
            state.file_notice = Some(format!("Export failed: {err:#}"));
            return;
        }
    };

    let noun = if count == 1 { nouns.0 } else { nouns.1 };
    let file = format!("{name}-{stamp}.csv");
    match crate::export::write(dir, &file, body) {
        Ok(path) => {
            state.file_path = Some(path.clone());
            state.file_notice = Some(format!("Exported {count} {noun} to {}", path.display()));
        }
        Err(err) => {
            state.file_path = None;
            state.file_notice = Some(format!("Export failed: {err:#}"));
        }
    }
}

/// Whether a product passes the category and search filters.
pub(super) fn matches(product: &Product, category: Option<&str>, needle: &str) -> bool {
    let in_category = category.is_none_or(|selected| product.category == selected);
    let in_search = needle.is_empty()
        || product.name.to_lowercase().contains(needle)
        || product
            .barcode
            .as_deref()
            .is_some_and(|barcode| barcode.to_lowercase().contains(needle))
        // The code on the box finds the same row: a cashier typing it should
        // see the product before the last digit resolves the scan.
        || product
            .pack_barcode
            .as_deref()
            .is_some_and(|barcode| barcode.to_lowercase().contains(needle));
    in_category && in_search
}

/// Indices of the products the grid shows, in catalog order.
///
/// Its own function because the scan path needs the same answer the grid drew:
/// if the two ever disagreed, a scan could add a product that is not on screen.
pub(super) fn visible_products(
    products: &[Product],
    category: Option<&str>,
    needle: &str,
) -> Vec<usize> {
    products
        .iter()
        .enumerate()
        .filter(|(_, product)| matches(product, category, needle))
        .map(|(index, _)| index)
        .collect()
}

/// The ledger row for a product, if the shop counts it.
///
/// `db::stock` only returns tracked products, so a missing row means "not
/// tracked", not "zero". The catalog draws a muted dash for that case rather
/// than a badge that would read as a count.
pub(super) fn stock_item(stock: &[StockItem], product_id: i64) -> Option<&StockItem> {
    stock.iter().find(|item| item.product_id == product_id)
}

/// The soonest date this product still has stock on, if any.
pub(super) fn earliest_expiry(expiring: &[ExpiryBucket], product_id: i64) -> Option<&ExpiryBucket> {
    expiring
        .iter()
        .filter(|bucket| bucket.product_id == product_id && bucket.on_hand > 0)
        .min_by(|a, b| a.expires_on.cmp(&b.expires_on))
}

/// Indices of the products the catalog shows: the category and search filters,
/// then the stock facet.
///
/// Its own function because the toolbar's count and the rows must agree; a
/// filter that counted one set and drew another is the kind of bug a reader
/// blames on the till.
pub(super) fn visible_catalog(
    products: &[Product],
    stock: &[StockItem],
    expiring: &[ExpiryBucket],
    warn_days: i64,
    category: Option<&str>,
    needle: &str,
    filter: usize,
) -> Vec<usize> {
    products
        .iter()
        .enumerate()
        .filter(|(_, product)| {
            matches(product, category, needle)
                && match filter {
                    1 => stock_item(stock, product.id).is_some_and(StockItem::is_low),
                    2 => stock_item(stock, product.id).is_some_and(StockItem::is_out),
                    // "Expiring" means something is on the shelf with a date
                    // inside the warning window, including already past it.
                    // A date with nothing left is not stock to sell first.
                    3 => expiring.iter().any(|bucket| {
                        bucket.product_id == product.id && bucket.expires_within(warn_days)
                    }),
                    _ => true,
                }
        })
        .map(|(index, _)| index)
        .collect()
}

/// What a scanned code or a typed name resolved to.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Scan {
    /// The product and which of its units was chosen: `1` loose, the pack size
    /// when the code on the box matched.
    One {
        index: usize,
        unit_factor: i64,
    },
    Ambiguous(usize),
    None,
}

/// Resolve a scan (or a typed name) to exactly one product, or say why not.
///
/// The order is the contract:
/// 1. an exact barcode wins outright, **ignoring the category filter** — a scan
///    is an explicit instruction, so a sidebar the cashier forgot about must not
///    turn it into "no product matches". The pack barcode is checked too, and
///    resolves to the pack, not the loose unit;
/// 2. an exact name within what the cashier can see;
/// 3. the only product the current filter shows;
/// 4. nothing, or a count so the message can say how many.
pub(super) fn scan_target(products: &[Product], needle: &str, visible: &[usize]) -> Scan {
    let needle = needle.trim().to_lowercase();
    if needle.is_empty() {
        return Scan::None;
    }

    // Barcodes are unique across both columns when present, so `position` is
    // enough here. Loose first: a product with no pack never reaches the second.
    if let Some(index) = products.iter().position(|product| {
        product
            .barcode
            .as_deref()
            .is_some_and(|barcode| barcode.trim().to_lowercase() == needle)
    }) {
        return Scan::One {
            index,
            unit_factor: 1,
        };
    }
    if let Some(index) = products.iter().position(|product| {
        product
            .pack_barcode
            .as_deref()
            .is_some_and(|barcode| barcode.trim().to_lowercase() == needle)
    }) {
        return Scan::One {
            index,
            unit_factor: products[index].pack_size.max(1),
        };
    }

    let named: Vec<usize> = visible
        .iter()
        .copied()
        .filter(|&index| products[index].name.trim().to_lowercase() == needle)
        .collect();

    if named.len() > 1 {
        return Scan::Ambiguous(named.len());
    }
    if let Some(&index) = named.first() {
        return Scan::One {
            index,
            unit_factor: 1,
        };
    }

    match visible {
        [] => Scan::None,
        [index] => Scan::One {
            index: *index,
            unit_factor: 1,
        },
        _ => Scan::Ambiguous(visible.len()),
    }
}

pub(super) enum ProductAction {
    Edit(i64),
    AskDelete(i64),
    Delete(i64),
    CancelDelete,
    MoveStock { id: i64, name: String, on_hand: i64 },
    StockHistory { id: i64, name: String },
}

pub(super) fn product_header(ui: &mut egui::Ui) {
    // "On hand" has no column header on purpose: the pill rides beside the name,
    // attached to the product it counts. A right-aligned header would drift with
    // the number of row actions, and a column of numbers would still need a
    // tooltip to say whether it meant stock or sold.
    ui.horizontal(|ui| {
        components::table_header(ui, "Product");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            components::table_header(ui, "Price");
        });
    });
    components::divider(ui);
}

/// One product: name, stock and category on the left, price and actions on the
/// right.
///
/// The row returns what was clicked instead of mutating the state, because the
/// caller is iterating `state.products` and cannot borrow it mutably yet.
pub(super) fn product_row(
    ui: &mut egui::Ui,
    product: &Product,
    stock: Option<&StockItem>,
    state: &State,
    deleting: bool,
    can_manage: bool,
) -> Option<ProductAction> {
    let mut action = None;

    ui.horizontal(|ui| {
        ui.set_min_height(52.0);

        // Keep the name clear of the price and row actions. Four 44-point
        // buttons are the widest case (a tracked product), and the reserve is
        // fixed so the price column does not move when a row gains stock.
        const ACTIONS_WIDTH: f32 = 352.0;
        let text_width = (ui.available_width() - ACTIONS_WIDTH).max(120.0);
        ui.allocate_ui_with_layout(
            egui::vec2(text_width, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                product_identity(
                    ui,
                    product,
                    stock,
                    &state.expiring,
                    state.expiry_warning_days(),
                )
            },
        );

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let price =
                ui.label(components::numeric(state.money(product.price_minor)).size(TYPE.body));
            if product.pack_size > 1
                && let Some(pack_price) = product.pack_price_minor
            {
                price.on_hover_text(format!(
                    "Pack of {}: {}",
                    product.pack_size,
                    state.money(pack_price)
                ));
            }

            // Icon-only actions: the row is about the product, and two words of
            // chrome per line crowd the price. The tooltip carries the word.
            if deleting {
                if components::icon_button(ui, Icon::CHECK, 44.0)
                    .on_hover_text("Delete")
                    .clicked()
                {
                    action = Some(ProductAction::Delete(product.id));
                }
                if components::icon_button(ui, Icon::CLOSE, 44.0)
                    .on_hover_text("Cancel")
                    .clicked()
                {
                    action = Some(ProductAction::CancelDelete);
                }
                components::caption(ui, "Remove this product?");
            } else {
                if components::icon_button(ui, Icon::TRASH, 44.0)
                    .on_hover_text("Delete")
                    .clicked()
                {
                    action = Some(ProductAction::AskDelete(product.id));
                }
                if components::icon_button(ui, edit_icon(), 44.0)
                    .on_hover_text("Edit")
                    .clicked()
                {
                    action = Some(ProductAction::Edit(product.id));
                }
                // Stock actions belong to products the shop actually counts. An
                // untracked product has no ledger to move or read; the toolbar's
                // "Track stock" is the way in.
                if let Some(item) = stock {
                    if components::icon_button(ui, history_icon(), 44.0)
                        .on_hover_text("Stock history")
                        .clicked()
                    {
                        action = Some(ProductAction::StockHistory {
                            id: product.id,
                            name: product.name.clone(),
                        });
                    }
                    if can_manage
                        && components::icon_button(ui, move_icon(), 44.0)
                            .on_hover_text("Move stock: receive, return, waste or adjust")
                            .clicked()
                    {
                        action = Some(ProductAction::MoveStock {
                            id: product.id,
                            name: product.name.clone(),
                            on_hand: item.on_hand,
                        });
                    }
                }
            }
        });
    });
    components::divider(ui);

    action
}

/// The product's identity: name and stock pill, then category and barcode.
pub(super) fn product_identity(
    ui: &mut egui::Ui,
    product: &Product,
    stock: Option<&StockItem>,
    expiring: &[ExpiryBucket],
    warn_days: i64,
) {
    ui.horizontal_wrapped(|ui| {
        ui.label(components::semibold(&product.name).size(TYPE.body));
        let (text, color, hint) = match stock {
            Some(item) => {
                let (color, hint) = stock_badge(ui, item);
                (item.on_hand.to_string(), color, hint)
            }
            // A dash, not a zero: the product has no ledger, and a count of
            // zero would be a statement the shop never made.
            None => (
                "—".to_owned(),
                theme::current(ui.ctx()).text_muted,
                "Stock is not tracked for this product.".to_owned(),
            ),
        };
        // Give the pill a slot exactly as wide as its text plus the frame's
        // padding. Without it, `horizontal_wrapped` hands the pill whatever few
        // pixels are left at the end of the row, and the label wraps "64" onto
        // two lines. With it, the whole pill moves to the next line when the
        // name has used the row.
        let text_width = ui.ctx().fonts_mut(|fonts| {
            fonts
                .layout_no_wrap(
                    text.clone(),
                    egui::FontId::new(TYPE.small, egui::FontFamily::Proportional),
                    egui::Color32::WHITE,
                )
                .size()
                .x
        });
        let badge = ui
            .allocate_ui_with_layout(
                egui::vec2(text_width + 13.0, 0.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| components::badge(ui, &text, color),
            )
            .response;
        badge.on_hover_text(hint);

        // Only the soonest date gets a badge: a perishable with four dates on
        // the shelf needs one warning, not four. The Stock screen lists them
        // all.
        if let Some(bucket) = earliest_expiry(expiring, product.id)
            && let Some(days) = bucket.days_left
            && days <= warn_days
        {
            let palette = theme::current(ui.ctx());
            let count = if bucket.unit_label == "unit" {
                bucket.on_hand.to_string()
            } else {
                format!("{} {}", bucket.on_hand, bucket.unit_label)
            };
            let date = bucket.expires_on.as_deref().unwrap_or("?");
            let (text, color, hint) = if days < 0 {
                (
                    "Expired".to_owned(),
                    palette.danger,
                    format!("{count} dated {date} has expired. Pull it into waste."),
                )
            } else if days == 0 {
                (
                    "Expires today".to_owned(),
                    palette.danger,
                    format!("{count} dated {date} expires today."),
                )
            } else {
                (
                    format!("{days}d left"),
                    palette.warning,
                    format!("{count} dated {date} expires in {days} days."),
                )
            };
            ui.scope(|ui| components::badge(ui, &text, color))
                .response
                .on_hover_text(hint);
        }
    });

    let mut detail = if product.category.is_empty() {
        "Uncategorised".to_owned()
    } else {
        product.category.clone()
    };
    if product.pack_size > 1 {
        detail.push_str(&format!(" · pack of {}", product.pack_size));
    }
    if let Some(barcode) = &product.barcode {
        detail.push_str(" · ");
        detail.push_str(barcode);
    }
    if let Some(barcode) = &product.pack_barcode {
        detail.push_str(" · ");
        detail.push_str(barcode);
    }
    components::caption(ui, &detail);
}

/// "12" for units, "12 boxes" when the product names what it is counted in.
pub(super) fn count_label(count: i64, unit_label: &str) -> String {
    if unit_label == "unit" {
        count.to_string()
    } else {
        format!("{count} {unit_label}")
    }
}

/// The stock pill's colour and the sentence behind it.
pub(super) fn stock_badge(ui: &egui::Ui, item: &StockItem) -> (Rgb, String) {
    let palette = theme::current(ui.ctx());
    let unit = |count: i64| count_label(count, &item.unit_label);
    if item.is_out() {
        (
            palette.danger,
            format!("Out of stock · warns at {}", unit(item.low_stock)),
        )
    } else if item.is_low() {
        (
            palette.danger,
            format!("Low stock · warns at {}", unit(item.low_stock)),
        )
    } else {
        (
            palette.success,
            format!(
                "{} on hand · warns at {}",
                unit(item.on_hand),
                unit(item.low_stock)
            ),
        )
    }
}

/// The add/edit form, as a modal drawn by the shell.
///
/// A modal rather than a card above the list: the catalog is a list you click
/// through, and a form pinned above it gets pushed off screen at the narrow end
/// of the window — and, worse, is silently replaced when the operator clicks
/// New product or another row's Edit, losing whatever was typed. A modal makes
/// "one product at a time" explicit and Cancel always means the same thing.
///
/// Called from `shell` after the frame, like the currency picker, so the
/// backdrop covers the screen and the list behind it cannot be clicked.
pub fn product_editor_modal(ctx: &egui::Context, state: &mut State) {
    let currency = state.currency;
    // Suggested category names: categories the shop has already created, then
    // the seed list for this till's business type. Case-insensitive
    // de-duplication keeps "Vitamins" and "vitamins" as one shelf.
    let suggestions: Vec<String> = match state.product_editor.as_ref() {
        Some(draft) => {
            let mut names: Vec<String> = crate::db::categories_list(&state.store)
                .iter()
                .map(|category| category.name.clone())
                .collect();
            if let Some(business_type) = &state.settings.business_type {
                for seed in category_seeds::seed_names(business_type) {
                    if !names.iter().any(|name| name.eq_ignore_ascii_case(&seed)) {
                        names.push(seed);
                    }
                }
            }
            // Remove the category already in the field: suggesting it is noise.
            let current = draft.category.trim();
            names.retain(|name| !name.eq_ignore_ascii_case(current));
            names
        }
        None => Vec::new(),
    };
    let Some(draft) = state.product_editor.as_mut() else {
        return;
    };

    let mut save = false;
    let mut cancel = false;

    modal(ctx, egui::Id::new("pos_product_editor")).show(ctx, |ui| {
        ui.set_width(460.0);

        let heading = if draft.id.is_some() {
            "Edit product"
        } else {
            "New product"
        };
        ui.label(components::semibold(heading).size(TYPE.heading));
        components::caption(
            ui,
            "The price is stored exactly as entered. The till rounds cash at payment.",
        );

        ui.add_space(SPACING.md);
        components::field_label(ui, "Name");
        ui.add_space(SPACING.xs);
        components::search_field(ui, &mut draft.name, "Espresso");

        // Pack first: a shop buys and prices a box, then works out what one
        // tablet must cost. The base unit is still what the till stores and
        // sells loose, so both prices live on this one row.
        ui.add_space(SPACING.md);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.md;

            ui.vertical(|ui| {
                components::field_label(ui, "Sold as");
                ui.add_space(SPACING.xs);
                text_field(ui, &mut draft.unit_label, "unit", 110.0);
            });

            ui.vertical(|ui| {
                components::field_label(ui, "Units per pack");
                ui.add_space(SPACING.xs);
                text_field(ui, &mut draft.pack_size, "1", 90.0);
            });

            ui.vertical(|ui| {
                components::field_label(ui, &format!("Pack price ({})", currency.code()));
                ui.add_space(SPACING.xs);
                text_field(ui, &mut draft.pack_price, "Optional", 130.0);
            });
        });
        ui.add_space(SPACING.xs);
        components::caption(
            ui,
            "A pack of one means the product is sold only by the unit.",
        );

        ui.add_space(SPACING.md);
        components::field_label(ui, "Pack barcode (optional)");
        ui.add_space(SPACING.xs);
        text_field(ui, &mut draft.pack_barcode, "On the box", 350.0);

        // Price and category share a row because the modal has the width for
        // it; the barcode gets its own, since it is scanned rather than typed.
        ui.add_space(SPACING.md);
        let each = {
            let label = draft.unit_label.trim();
            if label.is_empty() {
                "unit".to_owned()
            } else {
                label.to_owned()
            }
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.md;

            ui.vertical(|ui| {
                components::field_label(ui, &format!("Price per {each} ({})", currency.code()));
                ui.add_space(SPACING.xs);
                let response = text_field(ui, &mut draft.price, "0", 150.0);
                // A typed price belongs to the operator: stop the pack helper
                // from replacing it as they keep adjusting the pack price.
                if response.changed() {
                    draft.note_price_typed();
                }
            });

            ui.vertical(|ui| {
                components::field_label(ui, "Category");
                ui.add_space(SPACING.xs);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = SPACING.xs;
                    text_field(ui, &mut draft.category, "General", 148.0);
                    // Existing names in a dropdown instead of a wall of chips:
                    // the suggestion list stays one click away without pushing
                    // the barcode field off the modal. The field stays editable,
                    // because a shop must still be able to name a new shelf.
                    egui::ComboBox::from_id_salt("pos_product_category")
                        .width(220.0)
                        .selected_text("Pick…")
                        .show_ui(ui, |ui| {
                            if suggestions.is_empty() {
                                components::caption(ui, "No other categories yet.");
                            }
                            for name in &suggestions {
                                if ui.selectable_label(false, name).clicked() {
                                    draft.category = name.clone();
                                    ui.close();
                                }
                            }
                        });
                });
            });
        });

        // Fill the loose price from the pack price on the first pass, and keep
        // it in step while the operator tunes the pack. Once they type a price
        // of their own the helper steps aside.
        draft.sync_price_from_pack(currency);

        // Breaks sit under the price they modify: the shelf price is the first
        // rung, and a ladder without its base is a list of numbers with no
        // ground under it. Three rows is the cap, so the modal keeps its shape.
        ui.add_space(SPACING.md);
        components::field_label(
            ui,
            &format!("Quantity breaks (price per {each}, {})", currency.code()),
        );
        ui.add_space(SPACING.xs);
        if draft.tiers.is_empty() {
            components::caption(ui, "None. Every unit costs the price above.");
        }
        let mut remove = None;
        for (index, row) in draft.tiers.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = SPACING.sm;
                components::caption(ui, "From");
                text_field(ui, &mut row.qty, "10", 70.0);
                components::caption(ui, "units, each");
                text_field(ui, &mut row.price, "0", 110.0);
                if actions::quiet_compact(ui, "Remove").clicked() {
                    remove = Some(index);
                }
            });
            ui.add_space(SPACING.xs);
        }
        if let Some(index) = remove {
            draft.remove_tier(index);
        }
        if draft.tiers.len() < MAX_PRICE_BREAKS
            && actions::quiet_compact(ui, "Add a break").clicked()
        {
            draft.add_tier();
        }
        components::caption(
            ui,
            "A break prices the whole line once the count reaches it, and cannot cost \
             more than the price below it.",
        );

        ui.add_space(SPACING.md);
        components::field_label(ui, "Barcode (optional)");
        ui.add_space(SPACING.xs);
        text_field(ui, &mut draft.barcode, "Scanned or typed", 350.0);

        // Expiry lives beside tracking, not on the catalog fields. Tracking is
        // started from Inventory, so this form shows the flag and explains why
        // it is off rather than offering a switch the ledger cannot honour.
        ui.add_space(SPACING.md);
        components::field_label(ui, "Expiry");
        ui.add_space(SPACING.xs);
        ui.add_enabled_ui(draft.tracked, |ui| {
            ui.checkbox(&mut draft.perishable, "Goes off — date every delivery");
            ui.add_space(SPACING.xs);
            ui.add_enabled_ui(draft.perishable, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = SPACING.md;
                    ui.vertical(|ui| {
                        components::field_label(ui, "Usual shelf life (days, optional)");
                        ui.add_space(SPACING.xs);
                        text_field(ui, &mut draft.shelf_life_days, "e.g. 30", 130.0);
                    });
                });
            });
        });
        components::caption(
            ui,
            if draft.tracked {
                "The date printed on the pack always wins. The shelf life is a reminder \
                 for repacked or undated goods."
            } else {
                "Track this product from Inventory before dating its stock."
            },
        );

        if let Some(error) = &draft.error {
            ui.add_space(SPACING.sm);
            ui.label(
                egui::RichText::new(error)
                    .size(TYPE.small)
                    .color(ui.visuals().error_fg_color),
            );
        }

        actions::footer(ui, |ui| {
            if actions::primary(ui, "Save").clicked() {
                save = true;
            }
            if actions::secondary(ui, "Cancel").clicked() {
                cancel = true;
            }
        });
    });

    // Keyboard, because this form is typed into: Enter saves from any field and
    // Escape leaves without saving, which is what the buttons do.
    if ctx.input(|input| input.key_pressed(egui::Key::Enter)) {
        save = true;
    }
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        cancel = true;
    }

    // Applied outside the borrow of `state.product_editor`: saving reads the
    // catalog and writes the store, which needs the whole state.
    if save {
        state.save_product();
    }
    if cancel {
        state.product_editor = None;
    }
}

/// The CSV import, drawn with the other foreground layers in `shell.rs`.
///
/// The normal route is the picker in [`crate::picker`]; the path field here is
/// the fallback for one pasted from an email, and the place an owner with no
/// file to start from can save a template. The modal closes on a successful
/// import and stays open on anything the till could not read, with the reason
/// it could not.
pub fn import_modal(ctx: &egui::Context, state: &mut State) {
    let Some(draft) = state.import.as_mut() else {
        return;
    };

    let mut import = false;
    let mut cancel = false;
    let mut template = false;

    modal(ctx, egui::Id::new("pos_import")).show(ctx, |ui| {
        ui.set_width(460.0);

        ui.label(components::semibold("Import products from CSV").size(TYPE.heading));
        components::caption(
            ui,
            "Rows are read as name, category, price and an optional barcode. Headings \
             are honoured when present, so a file from the shop's spreadsheet works as is; \
             prices are read in this till's currency.",
        );
        components::caption(
            ui,
            "Rows that cannot be read are skipped and counted, never half-imported.",
        );

        ui.add_space(SPACING.md);
        components::field_label(ui, "File path");
        ui.add_space(SPACING.xs);
        text_field(ui, &mut draft.path, "/home/shop/products.csv", 350.0);

        if let Some(error) = &draft.error {
            ui.add_space(SPACING.sm);
            ui.label(
                egui::RichText::new(error)
                    .size(TYPE.small)
                    .color(ui.visuals().error_fg_color),
            );
        }

        ui.add_space(SPACING.sm);
        ui.horizontal(|ui| {
            if actions::quiet_compact(ui, "Save a template").clicked() {
                template = true;
            }
            components::caption(
                ui,
                "A CSV with these columns and two example rows to replace.",
            );
        });

        actions::footer(ui, |ui| {
            if actions::primary(ui, "Import").clicked() {
                import = true;
            }
            if actions::secondary(ui, "Cancel").clicked() {
                cancel = true;
            }
        });
    });

    // Saving a template is not an import: close this modal and open the folder
    // chooser instead. Only one modal is drawn per frame, so this one has to
    // come down before the picker goes up.
    if template {
        state.import = None;
        state.start_export(ExportKind::Template);
        return;
    }

    // Keyboard, matching the product form: Enter runs the import and Escape
    // leaves without touching the catalog.
    if ctx.input(|input| input.key_pressed(egui::Key::Enter)) {
        import = true;
    }
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        cancel = true;
    }

    if import {
        state.import_products();
    }
    if cancel {
        state.import = None;
    }
}
impl Module for Products {
    fn sidebar(&mut self, ui: &mut egui::Ui, state: &mut State) {
        // Stock is a second facet, not a second screen. The old Stock tab asked
        // the same question over the same rows; folding it here means one list,
        // one search and one place to answer "what is low?".
        //
        // It sits above the categories because it is three rows and the
        // categories are unbounded: below them a shop with sixty categories
        // would bury the filter a manager reaches for every morning.
        components::section_title(ui, "Stock");
        for (index, label) in ["All stock", "Low stock", "Out of stock", "Expiring"]
            .into_iter()
            .enumerate()
        {
            if regions::nav_item(ui, state.stock_filter == index, label).clicked() {
                state.stock_filter = index;
            }
        }
        ui.add_space(SPACING.sm);
        components::caption(ui, "Low, out and expiring count tracked products only.");

        // Categories are facets: they narrow the list without being the list's
        // question. The question — "which product?" — is the bar's search field,
        // in the same place as on every other list.
        ui.add_space(SPACING.md);
        categories_sidebar(ui, state);
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, state: &mut State) {
        // A locked page has no bar: a search field over a card that says "ask a
        // manager" is a control that cannot do anything.
        if !state.permits(crate::auth::Permission::ChangePrice) {
            return;
        }

        // The count only appears while a filter is on. With no filter every row
        // is on screen, so "48 of 48" would be a number the reader already knows.
        let filtered = !state.catalog_search.trim().is_empty()
            || state.category.is_some()
            || state.stock_filter != 0;
        let count = filtered.then(|| {
            let needle = state.catalog_search.trim().to_lowercase();
            let visible = visible_catalog(
                &state.products,
                &state.stock,
                &state.expiring,
                state.expiry_warning_days(),
                state.category.as_deref(),
                &needle,
                state.stock_filter,
            );
            format!("{} of {}", visible.len(), state.products.len())
        });

        // Offer the way in only while there is something left to count. A button
        // that opens "every product is tracked" is a dead end.
        let untracked = state
            .products
            .iter()
            .filter(|product| stock_item(&state.stock, product.id).is_none())
            .count();
        let can_track = state.permits(crate::auth::Permission::ManageInventory) && untracked > 0;

        let mut new = false;
        let mut track = false;
        let mut import = false;
        let mut template = false;
        let mut export_visible = false;
        let mut export_all = false;
        action_bar(
            ui,
            PageLeft::Search {
                text: &mut state.catalog_search,
                hint: "Name or barcode",
                count,
            },
            |ui| {
                if actions::primary_compact(ui, "New product").clicked() {
                    new = true;
                }
                if can_track && actions::secondary_compact(ui, "Track stock").clicked() {
                    track = true;
                }
                // Import and both exports are setup actions, not everyday ones.
                // One menu keeps them a click away without letting four
                // file-shaped icons compete with the catalog's primary action
                // for the bar's width.
                actions::toolbar_menu(ui, transfer_icon(), "Import / Export", |ui| {
                    ui.set_min_width(210.0);
                    if ui
                        .button("Import from CSV…")
                        .on_hover_text(
                            "Read name, category, price and barcode. A barcode that \
                             starts with 0 may need a text column in Excel.",
                        )
                        .clicked()
                    {
                        import = true;
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .button("Save a template…")
                        .on_hover_text(
                            "A blank CSV with the columns the import reads and two \
                             example rows",
                        )
                        .clicked()
                    {
                        template = true;
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .button("Export this view")
                        .on_hover_text(
                            "Save the products on screen with the columns the import reads",
                        )
                        .clicked()
                    {
                        export_visible = true;
                        ui.close();
                    }
                    if ui
                        .button("Export all products")
                        .on_hover_text("Save every product with the columns the import reads")
                        .clicked()
                    {
                        export_all = true;
                        ui.close();
                    }
                });
            },
        );

        if new {
            state.start_product();
        }
        if track {
            state.start_tracking();
        }
        if import {
            state.start_import_picker();
        }
        if export_visible {
            state.start_export(ExportKind::Products);
        }
        if export_all {
            state.start_export(ExportKind::AllProducts);
        }
        if template {
            state.start_export(ExportKind::Template);
        }
    }

    fn workspace(&mut self, ui: &mut egui::Ui, state: &mut State) {
        if !state.permits(crate::auth::Permission::ChangePrice) {
            return locked_card(ui, crate::auth::Permission::ChangePrice);
        }

        // Seeing stock is part of the catalog; changing it is a separate gate.
        // The page gate already keeps cashiers out, but the row still asks, so
        // a future role that may read prices cannot silently move stock.
        let can_manage = state.permits(crate::auth::Permission::ManageInventory);

        let needle = state.catalog_search.trim().to_lowercase();
        let visible = visible_catalog(
            &state.products,
            &state.stock,
            &state.expiring,
            state.expiry_warning_days(),
            state.category.as_deref(),
            &needle,
            state.stock_filter,
        );

        file_feedback(ui, state);
        if let Some(notice) = state.notice.as_deref() {
            components::caption(ui, notice);
        }

        if state.products.is_empty() {
            components::empty_state_card(
                ui,
                "No products yet",
                "Add the first product. Products become keys on the Sell screen.",
                200.0,
            );
            return;
        }

        if state.stock.is_empty() {
            components::caption(
                ui,
                "Stock is not tracked yet. Track a product to subtract sales from a count.",
            );
        } else {
            let tracked = state.stock.len();
            let low = state.stock.iter().filter(|item| item.is_low()).count();
            let out = state.stock.iter().filter(|item| item.is_out()).count();
            components::caption(
                ui,
                &format!("{tracked} tracked · {low} low · {out} out of stock"),
            );
        }

        // Dates worth acting on now, whatever the catalog filter says. Expired
        // stock is not a report line: it is on the shelf, and the shop has to
        // take it off. The card only lists the warning window, so it is quiet
        // until a date gets close.
        let warn = state.expiry_warning_days();
        let due: Vec<ExpiryBucket> = state
            .expiring
            .iter()
            .filter(|bucket| bucket.expires_within(warn))
            .cloned()
            .collect();
        let mut pull = None;
        if !due.is_empty() {
            ui.add_space(SPACING.sm);
            components::card(ui, |ui| {
                ui.set_min_width(ui.available_width());
                let palette = theme::current(ui.ctx());
                components::section_title(ui, "Expiring stock");
                components::caption(
                    ui,
                    &format!(
                        "Dates inside {warn} days, earliest first. Pull what is past \
                         into waste so it leaves the shelf."
                    ),
                );
                for bucket in due.iter().take(8) {
                    ui.horizontal(|ui| {
                        ui.set_min_height(40.0);
                        let (label, color) = match bucket.days_left {
                            Some(days) if days < 0 => ("Expired".to_owned(), palette.danger),
                            Some(0) => ("Today".to_owned(), palette.danger),
                            Some(days) => (format!("{days}d"), palette.warning),
                            None => (String::new(), palette.text_muted),
                        };
                        components::badge(ui, &label, color);
                        ui.label(components::semibold(&bucket.name).size(TYPE.body));
                        components::caption(ui, bucket.expires_on.as_deref().unwrap_or(""));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if bucket.is_expired()
                                && can_manage
                                && actions::secondary_compact(ui, "Pull to waste").clicked()
                            {
                                pull = Some((
                                    bucket.product_id,
                                    bucket.expires_on.clone().unwrap_or_default(),
                                    bucket.name.clone(),
                                ));
                            }
                            components::caption(
                                ui,
                                &count_label(bucket.on_hand, &bucket.unit_label),
                            );
                        });
                    });
                }
                if due.len() > 8 {
                    components::caption(
                        ui,
                        &format!(
                            "{} more — choose Expiring in the sidebar to see them all.",
                            due.len() - 8
                        ),
                    );
                }
            });
        }
        if let Some((product_id, expires_on, name)) = pull {
            state.pull_expired(product_id, &expires_on, &name);
        }

        if visible.is_empty() {
            let narrowed = !needle.is_empty() || state.category.is_some();
            let (title, body) = if narrowed {
                (
                    "Nothing matches",
                    "Clear the search or pick another category.",
                )
            } else {
                match state.stock_filter {
                    1 if state.stock.is_empty() => (
                        "No stock tracked yet",
                        "Track a product to start counting it.",
                    ),
                    1 => (
                        "Nothing is low",
                        "Every tracked product is above its warning line.",
                    ),
                    2 if state.stock.is_empty() => (
                        "No stock tracked yet",
                        "Track a product to start counting it.",
                    ),
                    2 => (
                        "Nothing is out",
                        "Every tracked product has stock on the shelf.",
                    ),
                    3 if state.stock.is_empty() => (
                        "No stock tracked yet",
                        "Track a product to start counting it.",
                    ),
                    3 => (
                        "Nothing is expiring",
                        "No dated stock is inside the warning window.",
                    ),
                    _ => (
                        "Nothing matches",
                        "Clear the search or pick another category.",
                    ),
                }
            };
            components::empty_state_card(ui, title, body, 160.0);
            return;
        }

        let mut action = None;
        // The page scrolls (the shell owns it), so the catalog card is laid out
        // whole: a list inside a list would fight the page for the wheel.
        components::card(ui, |ui| {
            ui.set_min_width(ui.available_width());
            product_header(ui);
            for &product_index in &visible {
                let product = &state.products[product_index];
                let stock = stock_item(&state.stock, product.id);
                let deleting = state.product_delete == Some(product.id);
                if let Some(row_action) =
                    product_row(ui, product, stock, state, deleting, can_manage)
                {
                    action = Some(row_action);
                }
            }
        });

        match action {
            Some(ProductAction::Edit(id)) => {
                if let Some(product) = state.products.iter().find(|product| product.id == id) {
                    let product = product.clone();
                    state.edit_product(&product);
                }
            }
            Some(ProductAction::AskDelete(id)) => state.product_delete = Some(id),
            Some(ProductAction::Delete(id)) => state.delete_product(id),
            Some(ProductAction::CancelDelete) => state.product_delete = None,
            Some(ProductAction::MoveStock { id, name, on_hand }) => {
                state.start_stock_move(id, &name, on_hand, StockKind::Receive);
            }
            Some(ProductAction::StockHistory { id, name }) => {
                state.open_stock_history(id, &name);
            }
            None => {}
        }
    }
}
