//! Catalog state: the product list, the product editor, and file import/export.
//!
//! The editor and the importer both end in the same two things — a reload of
//! the visible products, or an error string the modal can show — so they share
//! a file. The file dialogs live here too because the only flows that use them
//! start on this screen.

use super::*;
/// The product form, open as a modal over the app.
///
/// Price is held as typed text, not as a number: the operator is mid-edit, and
/// parsing on every keystroke would either eat a half-typed `28.` or need a
/// second "committed" copy of the field. It is parsed once, on save.
pub struct ProductDraft {
    /// `None` while adding, `Some` while editing.
    pub id: Option<i64>,
    pub name: String,
    pub price: String,
    pub category: String,
    #[allow(dead_code)]
    pub category_id: Option<i64>,
    pub barcode: String,
    /// The shop's own code, as typed. Blank means no code yet.
    pub sku: String,
    /// What one base unit is called: `tablet`, `sachet`, `unit`.
    pub unit_label: String,
    /// How many base units one sealed pack holds, as typed.
    pub pack_size: String,
    /// The sealed pack's price, as typed. Empty when there is no pack.
    pub pack_price: String,
    /// The barcode printed on the box.
    pub pack_barcode: String,
    /// The tablet price this form last filled from the pack price. `None` while
    /// empty or after the operator clears it.
    pub(super) derived_price_minor: Option<i64>,
    /// Set for good the first time the operator touches the price field. A
    /// cleared field stays theirs: the helper must not grab an empty box back
    /// and turn the next keystroke into "283".
    pub(super) price_typed: bool,
    /// Whether the ledger tracks this product. Only a tracked product can be
    /// perishable, because a date has to sit on a movement.
    pub tracked: bool,
    /// Whether the product goes off, so every delivery must be dated.
    pub perishable: bool,
    /// The shop's usual life for a delivery, as typed. A prefill for repacked
    /// goods only; the date printed on the pack always wins.
    pub shelf_life_days: String,
    /// The loose-unit price breaks, in the order the operator added them. Each
    /// row is text for the same reason the price is: half-typed is not wrong
    /// yet.
    pub tiers: Vec<TierDraft>,
    pub error: Option<String>,
}

/// How many price breaks the form offers. Three is enough for the shops this
/// till serves — a small break, a box-sized break, and a wholesale break — and
/// a cap keeps the modal's height fixed. More rungs are a pricing policy, not a
/// price list, and belong in a spreadsheet the shop already understands.
pub const MAX_PRICE_BREAKS: usize = 3;

/// One price-break row in the product form.
pub struct TierDraft {
    /// The first loose-unit count the break applies to, as typed.
    pub qty: String,
    /// The price of one loose unit from that count up, as typed.
    pub price: String,
}

impl TierDraft {
    fn new() -> Self {
        Self {
            qty: String::new(),
            price: String::new(),
        }
    }
}

impl ProductDraft {
    pub(super) fn new(category: String, category_id: Option<i64>) -> Self {
        Self {
            id: None,
            name: String::new(),
            price: String::new(),
            category,
            category_id,
            barcode: String::new(),
            sku: String::new(),
            unit_label: "unit".to_owned(),
            pack_size: "1".to_owned(),
            pack_price: String::new(),
            pack_barcode: String::new(),
            derived_price_minor: None,
            price_typed: false,
            tracked: false,
            perishable: false,
            shelf_life_days: String::new(),
            tiers: Vec::new(),
            error: None,
        }
    }

    /// Add an empty break row, if the form has room for it.
    pub fn add_tier(&mut self) {
        if self.tiers.len() < MAX_PRICE_BREAKS {
            self.tiers.push(TierDraft::new());
        }
    }

    /// Drop one break row. The rest keep their order.
    pub fn remove_tier(&mut self, index: usize) {
        if index < self.tiers.len() {
            self.tiers.remove(index);
        }
    }

    /// A pack size typed into the form, or `None` while it is not a number.
    pub(super) fn parsed_pack_size(&self) -> Option<i64> {
        match self.pack_size.trim() {
            "" => Some(1),
            text => text.parse::<i64>().ok().filter(|size| *size >= 1),
        }
    }

    /// Fill the loose price from the pack price, rounded up to the next unit.
    ///
    /// A shop that prices a box at 2,800 for 100 knows each tablet must be at
    /// least 28 to break even on the box; rounding up rather than down keeps
    /// the loose sale from losing money. The helper only owns a price it filled
    /// itself: once the operator types their own, this leaves it alone.
    pub fn sync_price_from_pack(&mut self, currency: Currency) {
        let Some(size) = self.parsed_pack_size().filter(|size| *size > 1) else {
            return;
        };
        let Ok(pack_price) = Money::parse(self.pack_price.trim(), currency) else {
            return;
        };
        if pack_price.minor() < 0 {
            return;
        }
        if self.price_typed {
            return;
        }
        let each = pack_price.minor() / size + i64::from(pack_price.minor() % size != 0);
        if self.derived_price_minor != Some(each) {
            self.price = plain_amount(each, currency);
            self.derived_price_minor = Some(each);
        }
    }

    /// Stop the pack-price helper from touching a price the operator typed.
    ///
    /// This sticks even if they clear the field afterwards; it is their field
    /// now, and the helper refilling it would fight the next keystroke.
    pub fn note_price_typed(&mut self) {
        self.price_typed = true;
        self.derived_price_minor = None;
    }
}

/// What a CSV file is for: new rows, or prices and details for rows already
/// in the catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportMode {
    Add,
    Update,
}

/// The CSV import, open as a modal over the app.
///
/// The path is typed or pasted rather than picked from a dialog: importing is
/// a one-time setup job for the owner, and a file-dialog dependency is not
/// worth what it costs a till this small. Rows that cannot be read are
/// skipped and counted; the summary line is the receipt for the import.
pub struct ImportDraft {
    pub mode: ImportMode,
    pub path: String,
    pub error: Option<String>,
}

impl State {
    /// Re-read the catalog and the category list after a write.
    ///
    /// If the selected category no longer exists (its last product was deleted
    /// or recategorised), the filter falls back to All rather than showing an
    /// empty grid with a category chip that is not in the sidebar.
    pub fn reload_products(&mut self) {
        self.products = crate::db::products(&self.store);
        self.categories = crate::db::categories(&self.store);

        if let Some(selected) = &self.category
            && !self
                .categories
                .iter()
                .any(|category| category.name == *selected)
        {
            self.category = None;
        }
    }

    /// Open an empty product form, pre-filled with the category being viewed.
    pub fn start_product(&mut self) {
        let category = self
            .category
            .clone()
            .unwrap_or_else(|| "General".to_owned());
        // Find the category id if this category already exists in the table.
        let category_id = crate::db::categories_list(&self.store)
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(&category))
            .map(|c| c.id);
        self.product_editor = Some(ProductDraft::new(category, category_id));
        self.product_delete = None;
        self.notice = None;
    }

    /// Route a scan that arrived from the phone through the same paths a
    /// keyboard scanner uses.
    ///
    /// Phone scans are accepted only while the till is usable and no modal owns
    /// the screen. Silently filling a hidden form, or adding to a cart behind a
    /// payment sheet, would make the phone feel haunted; a scan that arrives at
    /// a busy till is dropped instead of changing state nobody can see.
    pub fn receive_scan(&mut self, event: ScanEvent) {
        if self.locked()
            || self.needs_currency
            || self.payment.is_some()
            || self.override_request.is_some()
            || self.product_editor.is_some()
            || self.supplier_editor.is_some()
            || self.stock_editor.is_some()
            || self.stock_move.is_some()
            || self.stock_history.is_some()
            || self.sale_open.is_some()
            || self.import.is_some()
            || self.picker.is_some()
        {
            return;
        }

        match event.action {
            ScanAction::Sell => {
                self.select(crate::modules::SELL);
                self.sell_search = event.barcode;
                self.scan_focus = true;
                // The page bar consumes this the next time it draws, exactly as
                // if the scanner had pressed Enter.
                self.scan_submit = true;
                self.notice = None;
            }
            ScanAction::Add => {
                if !self.permits(crate::auth::Permission::ChangePrice) {
                    self.select(crate::modules::SELL);
                    self.notice = Some("A manager must add products from a phone scan.".to_owned());
                    return;
                }
                self.select(crate::modules::PRODUCTS);
                self.start_product();
                if let Some(draft) = &mut self.product_editor {
                    draft.barcode = event.barcode;
                }
            }
        }
    }

    /// Open the form on an existing product.
    pub fn edit_product(&mut self, product: &Product) {
        let stock = self
            .stock
            .iter()
            .find(|item| item.product_id == product.id)
            .cloned();
        self.product_editor = Some(ProductDraft {
            id: Some(product.id),
            name: product.name.clone(),
            price: plain_amount(product.price_minor, self.currency),
            category: product.category.clone(),
            category_id: product.category_id,
            barcode: product.barcode.clone().unwrap_or_default(),
            sku: product.sku.clone().unwrap_or_default(),
            unit_label: product.unit_label.clone(),
            pack_size: product.pack_size.to_string(),
            pack_price: product
                .pack_price_minor
                .map(|price| plain_amount(price, self.currency))
                .unwrap_or_default(),
            pack_barcode: product.pack_barcode.clone().unwrap_or_default(),
            derived_price_minor: None,
            // An existing price is the shop's, not the helper's.
            price_typed: true,
            tracked: stock.is_some(),
            perishable: stock.as_ref().is_some_and(|item| item.perishable),
            shelf_life_days: stock
                .as_ref()
                .and_then(|item| item.shelf_life_days)
                .map(|days| days.to_string())
                .unwrap_or_default(),
            tiers: product
                .tiers
                .iter()
                .map(|tier| TierDraft {
                    qty: tier.min_qty.to_string(),
                    price: plain_amount(tier.price_minor, self.currency),
                })
                .collect(),
            error: None,
        });
        self.product_delete = None;
        self.notice = None;
    }

    /// Parse and store the open form.
    ///
    /// Everything is validated before the first write, so a rejected form leaves
    /// the database untouched and the text the operator typed still on screen.
    pub fn save_product(&mut self) {
        let Some(draft) = self.product_editor.as_ref() else {
            return;
        };

        let id = draft.id;
        let name = draft.name.trim().to_owned();
        let category = draft.category.trim().to_owned();
        let barcode = draft.barcode.trim().to_owned();
        let sku = draft.sku.trim().to_owned();
        let price_text = draft.price.trim().to_owned();
        let unit_label = draft.unit_label.trim().to_owned();
        let pack_size = draft.parsed_pack_size();
        let pack_price_text = draft.pack_price.trim().to_owned();
        let pack_barcode = draft.pack_barcode.trim().to_owned();

        if name.is_empty() {
            return self.editor_error("A product needs a name.");
        }
        let Some(pack_size) = pack_size else {
            return self.editor_error("A pack size must be a whole number of one or more.");
        };
        // An unnamed category is not worth an error: a product whose shelf has
        // no name yet still sells, so it lands in the default one instead of
        // stopping the save.
        let category = if category.is_empty() {
            "General".to_owned()
        } else {
            category
        };

        // Look up or create the category row, then link the product to it.
        let category_id = match crate::db::find_or_insert_category(&self.store, &category) {
            Ok(id) => Some(id),
            Err(error) => {
                tracing::error!(error = %error, category = %category, "could not resolve category");
                None
            }
        };

        let price_minor = match Money::parse(&price_text, self.currency) {
            Ok(price) if price.minor() >= 0 => price.minor(),
            Ok(_) => return self.editor_error("A price cannot be negative."),
            Err(error) => return self.editor_error(error.to_string()),
        };

        // A pack of one is the base unit, so a second price for it is not a
        // second price; the database drops it deliberately. Below two, whatever
        // was typed in the pack fields is ignored rather than rejected, because
        // the form keeps stale text when the operator changes the size back.
        let pack_price_minor = if pack_size <= 1 || pack_price_text.is_empty() {
            None
        } else {
            match Money::parse(&pack_price_text, self.currency) {
                Ok(price) if price.minor() >= 0 => Some(price.minor()),
                Ok(_) => return self.editor_error("A pack price cannot be negative."),
                Err(error) => return self.editor_error(error.to_string()),
            }
        };

        // A break row is optional, and a row the operator left alone is not a
        // row at all. Half a row is a mistake they can still see and fix.
        let mut tiers = Vec::with_capacity(draft.tiers.len());
        for row in &draft.tiers {
            let qty_text = row.qty.trim();
            let price_text = row.price.trim();
            if qty_text.is_empty() && price_text.is_empty() {
                continue;
            }
            if qty_text.is_empty() {
                return self.editor_error("A price break needs the quantity it starts at.");
            }
            if price_text.is_empty() {
                return self.editor_error("A price break needs a price.");
            }
            let min_qty = match qty_text.parse::<i64>() {
                Ok(qty) if qty >= 2 => qty,
                _ => return self.editor_error("A price break starts at two units or more."),
            };
            let price_minor = match Money::parse(price_text, self.currency) {
                Ok(price) if price.minor() >= 0 => price.minor(),
                Ok(_) => return self.editor_error("A price break cannot be negative."),
                Err(error) => return self.editor_error(error.to_string()),
            };
            tiers.push(PriceTier {
                min_qty,
                price_minor,
            });
        }

        let barcode = (!barcode.is_empty()).then_some(barcode);
        let sku = (!sku.is_empty()).then_some(sku);
        let pack_barcode = (pack_size > 1 && !pack_barcode.is_empty()).then_some(pack_barcode);
        for code in [barcode.as_deref(), pack_barcode.as_deref()]
            .into_iter()
            .flatten()
        {
            if crate::db::barcode_taken(&self.store, code, id) {
                return self.editor_error("Another product already uses that barcode.");
            }
        }
        if let Some(sku) = sku.as_deref()
            && crate::db::sku_taken(&self.store, sku, id)
        {
            return self.editor_error("Another product already uses that shop code.");
        }

        let fields = crate::db::ProductFields {
            name: &name,
            price_minor,
            category: &category,
            category_id,
            barcode: barcode.as_deref(),
            sku: sku.as_deref(),
            unit_label: &unit_label,
            pack_size,
            pack_price_minor,
            pack_barcode: pack_barcode.as_deref(),
            tiers: &tiers,
        };

        let perishable = draft.perishable;
        let tracked = draft.tracked;
        if perishable && !tracked {
            return self.editor_error(
                "Only a product the ledger tracks can be perishable. Start tracking it from Inventory first.",
            );
        }
        let shelf_life_days = if perishable {
            match draft.shelf_life_days.trim() {
                "" => None,
                text => match text.parse::<i64>() {
                    Ok(days) if days > 0 => Some(days),
                    _ => return self.editor_error("A shelf life must be a whole number of days."),
                },
            }
        } else {
            None
        };

        let result = match id {
            Some(id) => crate::db::update_product(&self.store, id, &fields).map(|()| id),
            None => crate::db::insert_product(&self.store, &fields),
        };

        match result {
            Ok(id) => {
                // Perishability lives beside tracking, not on the catalog
                // fields: saving a name must not silently clear it, and an
                // untracked product has nowhere to keep a date.
                if let Err(error) =
                    crate::db::set_perishability(&self.store, id, perishable, shelf_life_days)
                {
                    tracing::error!(error = %error, "saving perishability failed");
                    return self
                        .editor_error(format!("Saved {name}, but could not set expiry: {error}"));
                }
                self.product_editor = None;
                self.reload_products();
                // A tracked product's name and category are shown in Inventory;
                // re-read the ledger so the list does not keep the old spelling.
                self.reload_stock();
                self.notice = Some(format!("Saved {name}."));
            }
            Err(error) => {
                tracing::error!(error = %error, "saving a product failed");
                self.editor_error(format!("Could not save: {error}"));
            }
        }
    }

    /// Remove a product the operator confirmed.
    pub fn delete_product(&mut self, id: i64) {
        match crate::db::delete_product(&self.store, id) {
            Ok(()) => {
                self.product_delete = None;
                self.reload_products();
                // Archiving hides the product from the catalog; without this,
                // Inventory would keep painting its row.
                self.reload_stock();
                self.notice = Some("Product removed.".to_owned());
            }
            Err(error) => {
                tracing::error!(error = %error, "deleting a product failed");
                self.product_delete = None;
                self.notice = Some(format!("Could not remove the product: {error}"));
            }
        }
    }

    fn editor_error(&mut self, message: impl Into<String>) {
        if let Some(draft) = self.product_editor.as_mut() {
            draft.error = Some(message.into());
        }
    }

    /// Add or update products from a CSV file, as listed in the import draft.
    ///
    /// Everything the parser accepts is attempted; anything else — a row the
    /// file spells wrongly, a barcode another product already uses, an update
    /// that names no product — is skipped and counted. One bad line must not
    /// stop the other hundreds, and a partial run is honest because the summary
    /// says what landed and what did not.
    pub fn import_products(&mut self) {
        let Some(draft) = self.import.as_ref() else {
            return;
        };
        let (mode, path) = (draft.mode, draft.path.trim().to_owned());
        if path.is_empty() {
            return self.import_error("Type or paste the file's path first.");
        }
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                tracing::warn!(error = %error, "reading a CSV import failed");
                return self.import_error(format!(
                    "Could not read {path}. Check the path and try again."
                ));
            }
        };

        let (rows, errors) = crate::import::parse(&text, self.currency);
        let mut reasons = errors;
        let mut added = 0;
        let mut updated = 0;
        let mut skipped = reasons.len();
        // One read for the whole update file, rather than a query per row.
        let catalog = if mode == ImportMode::Update {
            crate::db::products(&self.store)
        } else {
            Vec::new()
        };

        for row in rows {
            // An update names a product by its own code first, falling back to
            // a barcode for a catalog that has no codes yet. Add mode always
            // writes a new row and never looks for a match.
            let target = if mode == ImportMode::Update {
                let found = crate::db::find_by_code(
                    &self.store,
                    row.sku.as_deref(),
                    row.barcode.as_deref(),
                );
                if found.is_none() {
                    reasons.push("a row named no live product by SKU or barcode".to_owned());
                    skipped += 1;
                    continue;
                }
                found
            } else {
                None
            };

            // A blank code or barcode cell in an update keeps the product's
            // old one. In add mode there is nothing to keep, so blank is none.
            let previous = target.and_then(|id| catalog.iter().find(|item| item.id == id));
            let sku = row
                .sku
                .clone()
                .or_else(|| previous.and_then(|p| p.sku.clone()));
            let barcode = row
                .barcode
                .clone()
                .or_else(|| previous.and_then(|p| p.barcode.clone()));

            // Both barcode columns share one namespace: checking only the loose
            // one would let a pack code collide with another product's code.
            // In an update, the product itself is not a collision.
            let codes = [barcode.as_deref(), row.pack_barcode.as_deref()];
            if codes
                .into_iter()
                .flatten()
                .any(|code| crate::db::barcode_taken(&self.store, code, target))
            {
                skipped += 1;
                continue;
            }
            if let Some(code) = sku.as_deref()
                && crate::db::sku_taken(&self.store, code, target)
            {
                skipped += 1;
                continue;
            }
            // Import also creates the shelf, so the sidebar the products land
            // on is the same shelf the product form offers. A failure here is
            // not a reason to drop a good product: the text still names it.
            let category_id = match crate::db::find_or_insert_category(&self.store, &row.category) {
                Ok(id) => Some(id),
                Err(error) => {
                    tracing::warn!(error = %error, "could not resolve an imported category");
                    None
                }
            };
            let fields = crate::db::ProductFields {
                name: &row.name,
                price_minor: row.price_minor,
                category: &row.category,
                category_id,
                barcode: barcode.as_deref(),
                sku: sku.as_deref(),
                unit_label: &row.unit_label,
                pack_size: row.pack_size,
                pack_price_minor: row.pack_price_minor,
                pack_barcode: row.pack_barcode.as_deref(),
                tiers: &row.tiers,
            };
            let result = match target {
                Some(id) => crate::db::update_product(&self.store, id, &fields).map(|()| id),
                None => crate::db::insert_product(&self.store, &fields),
            };
            match result {
                Ok(_) => {
                    if target.is_some() {
                        updated += 1;
                    } else {
                        added += 1;
                    }
                }
                Err(error) => {
                    // The catalog's own rejects count like the file's: a row the
                    // till cannot take does not stop the others.
                    tracing::warn!(error = %error, "a CSV row could not be saved");
                    skipped += 1;
                }
            }
        }

        let (done, verb, title, nothing) = match mode {
            ImportMode::Add => (
                added,
                "Imported",
                "Nothing was imported.",
                "Nothing was imported: the file has no products in it.",
            ),
            ImportMode::Update => (
                updated,
                "Updated",
                "Nothing was updated.",
                "Nothing was updated: no row named a live product.",
            ),
        };
        if done == 0 {
            let message = match reasons.first() {
                Some(error) => format!("{title} {error}"),
                None => nothing.to_owned(),
            };
            return self.import_error(message);
        }

        let noun = if done == 1 { "product" } else { "products" };
        let summary = if skipped == 0 {
            format!("{verb} {done} {noun}.")
        } else {
            format!("{verb} {done} {noun}, skipped {skipped}.")
        };
        self.import = None;
        self.reload_products();
        self.notice = Some(summary);
    }

    fn import_error(&mut self, message: impl Into<String>) {
        if let Some(draft) = self.import.as_mut() {
            draft.error = Some(message.into());
        }
    }

    /// Open the typed-path fallback for whichever import the operator chose,
    /// including the one offered by the in-app file picker. A path pasted from
    /// an email reaches the catalog this way.
    pub(crate) fn start_csv(&mut self, mode: ImportMode) {
        self.import_mode = mode;
        self.import = Some(ImportDraft {
            mode,
            path: String::new(),
            error: None,
        });
    }

    /// Ask for the CSV to import, in the computer's own file chooser.
    pub fn start_import_picker(&mut self, mode: ImportMode) {
        self.import_mode = mode;
        self.ask_for_path(crate::picker::Purpose::Import);
    }

    /// Ask where one of the exports goes, in the computer's own file chooser.
    ///
    /// Nothing is written until the operator chooses, so a mis-click on
    /// "Export CSV" cannot leave a file somewhere unexpected.
    pub fn start_export(&mut self, kind: crate::picker::ExportKind) {
        self.ask_for_path(crate::picker::Purpose::Export(kind));
    }

    /// Ask the computer for a path, and fall back to the till's own chooser
    /// when it has none to show.
    fn ask_for_path(&mut self, purpose: crate::picker::Purpose) {
        // One question at a time: a second dialog would open behind the first.
        if self.dialog.busy() {
            return;
        }
        let start = self.start_dir_for(purpose);
        if crate::dialog::available() && self.dialog.ask(purpose, start.clone()) {
            return;
        }
        self.open_picker(purpose, start);
    }

    /// The till's own chooser, which needs nothing from the desktop.
    fn open_picker(&mut self, purpose: crate::picker::Purpose, start: PathBuf) {
        self.picker = Some(crate::picker::Picker::open(
            purpose,
            start,
            &self.paths.data,
        ));
    }

    /// The folder a question starts in: the last one of its kind, then the
    /// operator's documents, then the till's own data folder.
    fn start_dir_for(&self, purpose: crate::picker::Purpose) -> PathBuf {
        let last = match purpose {
            crate::picker::Purpose::Import => self.import_dir.as_deref(),
            crate::picker::Purpose::Export(_) => self.export_dir.as_deref(),
        };
        crate::picker::start_dir(last, &self.paths.data)
    }

    /// Whether a file question is waiting for the operator.
    pub fn dialog_busy(&self) -> bool {
        self.dialog.busy()
    }

    /// Take the answer to a file question, once the operator has given one.
    ///
    /// Called once a frame: the dialog is the system's window and reports back
    /// on its own thread.
    pub fn poll_dialog(&mut self) {
        let Some(answer) = self.dialog.poll() else {
            return;
        };
        if let Some(path) = answer.path {
            self.finish_ask(answer.purpose, path);
            return;
        }
        // Cancelled, or never opened at all. Nothing can tell those apart from
        // the result, but the clock can: see `dialog::INSTANT`.
        if answer.open_for >= crate::dialog::INSTANT {
            return;
        }
        tracing::warn!(
            purpose = ?answer.purpose,
            "the system file chooser did not open; using the till's own"
        );
        let start = self.start_dir_for(answer.purpose);
        self.open_picker(answer.purpose, start);
    }

    /// Apply a chosen path, from the system dialog or from the till's picker.
    ///
    /// One place, because both routes ask the same question: an import reads
    /// the file it was given, an export writes into the folder it was given.
    pub(crate) fn finish_ask(&mut self, purpose: crate::picker::Purpose, path: PathBuf) {
        match purpose {
            crate::picker::Purpose::Import => {
                self.import_dir = path.parent().map(PathBuf::from);
                self.import = Some(ImportDraft {
                    mode: self.import_mode,
                    path: path.display().to_string(),
                    error: None,
                });
                self.import_products();
            }
            crate::picker::Purpose::Export(kind) => {
                self.export_dir = Some(path.clone());
                crate::modules::run_export(self, kind, &path);
            }
        }
    }
}
