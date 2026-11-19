//! Supplier and stock state: who supplies the shop, and what came from them.
//!
//! Stock is not its own screen — every movement starts from a supplier or a
//! product — so the drafts that describe a delivery, a track and a correction
//! live with the supplier list that opens them.

use super::*;
/// The supplier form, open as a modal over the app.
///
/// Only the name is required. A shop often knows the rep and the phone number
/// long before it knows an email or a postal address, and a form that refuses
/// half-known suppliers teaches people to type junk.
pub struct SupplierDraft {
    /// `None` while adding, `Some` while editing.
    pub id: Option<String>,
    pub name: String,
    pub contact_name: String,
    pub phone: String,
    pub email: String,
    pub address: String,
    pub notes: String,
    pub error: Option<String>,
}

impl SupplierDraft {
    fn new() -> Self {
        Self {
            id: None,
            name: String::new(),
            contact_name: String::new(),
            phone: String::new(),
            email: String::new(),
            address: String::new(),
            notes: String::new(),
            error: None,
        }
    }

    fn from(supplier: &Supplier) -> Self {
        Self {
            id: Some(supplier.id.clone()),
            name: supplier.name.clone(),
            contact_name: supplier.contact_name.clone().unwrap_or_default(),
            phone: supplier.phone.clone().unwrap_or_default(),
            email: supplier.email.clone().unwrap_or_default(),
            address: supplier.address.clone().unwrap_or_default(),
            notes: supplier.notes.clone().unwrap_or_default(),
            error: None,
        }
    }
}

/// The "start tracking" form, open as a modal over Inventory.
///
/// Tracking is opt-in and starts with a count on the shelf, because an imported
/// catalog has no stock and inventing zero would claim the shop is empty. The
/// picker lists only untracked products; the two numbers are whole units, not
/// money, so they are parsed as integers.
pub struct TrackDraft {
    pub product_id: Option<i64>,
    /// Search inside the picker, separate from the screen's own filter.
    pub search: String,
    pub opening: String,
    pub low_stock: String,
    /// What to keep in stock. Empty means twice the warning line, computed by
    /// the suggestion, which is a better default than making the owner do
    /// arithmetic on a first day.
    pub target_stock: String,
    pub error: Option<String>,
}

impl TrackDraft {
    fn new() -> Self {
        Self {
            product_id: None,
            search: String::new(),
            opening: "0".to_owned(),
            low_stock: "5".to_owned(),
            target_stock: String::new(),
            error: None,
        }
    }
}

/// One manual movement, open as a modal over Inventory.
///
/// `quantity` is a positive amount for receive, return and waste, and the
/// counted total for an adjustment. The sign is derived in `save_stock_move`
/// from the kind, never typed: "we counted forty and the ledger said forty-three"
/// is a `-3` the operator should not have to work out.
pub struct StockMoveDraft {
    pub product_id: i64,
    pub product_name: String,
    /// What the ledger says now, so an adjustment can ask for the count the
    /// operator sees rather than a difference they would have to subtract.
    pub on_hand: i64,
    pub kind: crate::db::StockKind,
    pub quantity: String,
    pub note: String,
    /// The date printed on the pack, as typed. Only a delivery, a return or
    /// stock pulled off a dated bucket carries one.
    pub expires_on: String,
    /// Whether a date is required here: a perishable product's delivery.
    pub perishable: bool,
    /// The shop's usual life, shown as a hint beside the date field.
    pub shelf_life_days: Option<i64>,
    pub error: Option<String>,
}

impl StockMoveDraft {
    fn new(
        product_id: i64,
        product_name: &str,
        on_hand: i64,
        kind: crate::db::StockKind,
        perishable: bool,
        shelf_life_days: Option<i64>,
    ) -> Self {
        Self {
            product_id,
            product_name: product_name.to_owned(),
            on_hand,
            kind,
            quantity: String::new(),
            note: String::new(),
            expires_on: String::new(),
            perishable,
            shelf_life_days,
            error: None,
        }
    }
}

/// A product's recent movements, read once when the modal opens.
pub struct StockHistory {
    pub product_name: String,
    pub movements: Vec<crate::db::StockMovement>,
}

impl State {
    /// Re-read the supplier directory after a write.
    pub fn reload_suppliers(&mut self) {
        self.suppliers = crate::db::suppliers(&self.store);
    }

    /// Open an empty supplier form.
    pub fn start_supplier(&mut self) {
        self.supplier_editor = Some(SupplierDraft::new());
        self.supplier_archive = None;
        self.notice = None;
    }

    /// Open the form on an existing supplier.
    pub fn edit_supplier(&mut self, supplier: &Supplier) {
        self.supplier_editor = Some(SupplierDraft::from(supplier));
        self.supplier_archive = None;
        self.notice = None;
    }

    /// Store the open supplier form.
    ///
    /// Everything is checked before the first write, so a rejected form leaves
    /// the database untouched and the text the operator typed still on screen.
    pub fn save_supplier(&mut self) {
        let Some(draft) = self.supplier_editor.as_ref() else {
            return;
        };

        let id = draft.id.clone();
        let name = draft.name.trim().to_owned();
        let contact_name = draft.contact_name.trim().to_owned();
        let phone = draft.phone.trim().to_owned();
        let email = draft.email.trim().to_owned();
        let address = draft.address.trim().to_owned();
        let notes = draft.notes.trim().to_owned();

        if name.is_empty() {
            return self.supplier_error("A supplier needs a name.");
        }
        if crate::db::supplier_name_taken(&self.store, &name, id.as_deref()) {
            return self.supplier_error(
                "Another supplier already uses that name. Show archived suppliers if it is hidden.",
            );
        }

        let result = match id.as_deref() {
            Some(id) => crate::db::update_supplier(
                &self.store,
                id,
                &crate::db::SupplierFields {
                    name: &name,
                    contact_name: &contact_name,
                    phone: &phone,
                    email: &email,
                    address: &address,
                    notes: &notes,
                },
            ),
            None => crate::db::insert_supplier(
                &self.store,
                &crate::db::SupplierFields {
                    name: &name,
                    contact_name: &contact_name,
                    phone: &phone,
                    email: &email,
                    address: &address,
                    notes: &notes,
                },
            )
            .map(|_| ()),
        };

        match result {
            Ok(()) => {
                self.supplier_editor = None;
                self.reload_suppliers();
                self.notice = Some(format!("Saved {name}."));
            }
            Err(error) => {
                tracing::error!(error = %error, "saving a supplier failed");
                self.supplier_error(format!("Could not save: {error}"));
            }
        }
    }

    /// Archive a supplier, or bring one back into the active list.
    pub fn set_supplier_archived(&mut self, id: &str, archived: bool) {
        match crate::db::set_supplier_archived(&self.store, id, archived) {
            Ok(()) => {
                self.supplier_archive = None;
                self.reload_suppliers();
                self.notice = Some(
                    if archived {
                        "Supplier archived."
                    } else {
                        "Supplier restored."
                    }
                    .to_owned(),
                );
            }
            Err(error) => {
                tracing::error!(error = %error, "archiving a supplier failed");
                self.supplier_archive = None;
                self.notice = Some(format!("Could not change the supplier: {error}"));
            }
        }
    }

    fn supplier_error(&mut self, message: impl Into<String>) {
        if let Some(draft) = self.supplier_editor.as_mut() {
            draft.error = Some(message.into());
        }
    }

    /// Re-read tracked stock after a movement or a sale.
    pub fn reload_stock(&mut self) {
        self.stock = crate::db::stock(&self.store);
        self.expiring = crate::db::expiring(&self.store);
    }

    /// The expiry warning window, from settings.
    pub fn expiry_warning_days(&self) -> i64 {
        self.settings.warn_within_days()
    }

    /// Everything left on one expired date, pulled into waste in one movement.
    pub fn pull_expired(&mut self, product_id: i64, expires_on: &str, product_name: &str) {
        let user_id = self.session.as_ref().map(|session| session.user_id.clone());
        match crate::db::pull_expired(&self.store, product_id, expires_on, user_id.as_deref()) {
            Ok(units) => {
                self.reload_stock();
                self.notice = Some(format!("Pulled {units} of {product_name} into waste."));
            }
            Err(error) => {
                tracing::error!(error = %error, "pulling expired stock failed");
                self.notice = Some(format!("Could not pull it: {error}"));
            }
        }
    }

    /// Open the "start tracking" form.
    pub fn start_tracking(&mut self) {
        self.stock_editor = Some(TrackDraft::new());
        self.stock_move = None;
        self.stock_history = None;
        self.notice = None;
    }

    /// Start tracking the chosen product, with the count on the shelf.
    ///
    /// The opening count is deliberately explicit, even for zero. An imported
    /// catalog has no counts, and a product that starts at zero is a statement
    /// the operator made, not one the app guessed.
    pub fn save_tracking(&mut self) {
        let Some((product_id, opening_text, low_stock_text, target_text)) =
            self.stock_editor.as_ref().map(|draft| {
                (
                    draft.product_id,
                    draft.opening.trim().to_owned(),
                    draft.low_stock.trim().to_owned(),
                    draft.target_stock.trim().to_owned(),
                )
            })
        else {
            return;
        };
        let Some(product_id) = product_id else {
            return self.stock_editor_error("Choose a product to track.");
        };
        let opening = match opening_text.parse::<i64>() {
            Ok(count) if count >= 0 => count,
            Ok(_) => return self.stock_editor_error("Opening count cannot be negative."),
            Err(_) => return self.stock_editor_error("Opening count must be a whole number."),
        };
        let low_stock = match low_stock_text.parse::<i64>() {
            Ok(level) if level >= 0 => level,
            _ => return self.stock_editor_error("Low-stock level must be zero or more."),
        };
        let target_stock = if target_text.is_empty() {
            None
        } else {
            match target_text.parse::<i64>() {
                Ok(target) if target >= low_stock => Some(target),
                _ => {
                    return self
                        .stock_editor_error("Keep-in-stock must be at least the low-stock level.");
                }
            }
        };
        let user_id = self.session.as_ref().map(|session| session.user_id.clone());

        match crate::db::track_product(
            &self.store,
            product_id,
            opening,
            low_stock,
            target_stock,
            user_id.as_deref(),
        ) {
            Ok(()) => {
                self.stock_editor = None;
                self.reload_stock();
                self.notice = Some("Stock tracking started.".to_owned());
            }
            Err(error) => {
                tracing::error!(error = %error, "starting stock tracking failed");
                self.stock_editor_error(format!("Could not start tracking: {error}"));
            }
        }
    }

    /// Open a manual movement form for one tracked product.
    pub fn start_stock_move(
        &mut self,
        product_id: i64,
        product_name: &str,
        on_hand: i64,
        kind: crate::db::StockKind,
    ) {
        let item = self.stock.iter().find(|item| item.product_id == product_id);
        self.stock_move = Some(StockMoveDraft::new(
            product_id,
            product_name,
            on_hand,
            kind,
            item.is_some_and(|item| item.perishable),
            item.and_then(|item| item.shelf_life_days),
        ));
        self.stock_editor = None;
        self.stock_history = None;
        self.notice = None;
    }

    /// Write the manual movement, with the sign the kind implies.
    ///
    /// The database refuses a movement whose sign contradicts its kind, so this
    /// form validates before the write as well: a rejected form keeps the typed
    /// quantity and note on screen.
    pub fn save_stock_move(&mut self) {
        let Some((
            product_id,
            product_name,
            on_hand,
            kind,
            quantity_text,
            note,
            expires_text,
            perishable,
        )) = self.stock_move.as_ref().map(|draft| {
            (
                draft.product_id,
                draft.product_name.clone(),
                draft.on_hand,
                draft.kind,
                draft.quantity.trim().to_owned(),
                draft.note.clone(),
                draft.expires_on.trim().to_owned(),
                draft.perishable,
            )
        })
        else {
            return;
        };
        let counted = match quantity_text.parse::<i64>() {
            Ok(counted) => counted,
            Err(_) => {
                return self.stock_move_error(match kind {
                    crate::db::StockKind::Adjust => "Counted quantity must be a whole number.",
                    _ => "Quantity must be a whole number.",
                });
            }
        };
        let delta = match kind {
            crate::db::StockKind::Receive | crate::db::StockKind::Return => {
                if counted <= 0 {
                    return self.stock_move_error("Enter a positive quantity.");
                }
                counted
            }
            crate::db::StockKind::Waste => {
                if counted <= 0 {
                    return self.stock_move_error("Enter a positive quantity.");
                }
                -counted
            }
            crate::db::StockKind::Adjust => {
                if counted < 0 {
                    return self.stock_move_error("Counted quantity cannot be negative.");
                }
                match counted.checked_sub(on_hand) {
                    Some(0) => {
                        return self.stock_move_error("That matches what the till already has.");
                    }
                    Some(delta) => delta,
                    None => return self.stock_move_error("That count is out of range."),
                }
            }
            crate::db::StockKind::Opening | crate::db::StockKind::Sale => {
                return self.stock_move_error("That movement is written by its own flow.");
            }
        };

        let user_id = self.session.as_ref().map(|session| session.user_id.clone());

        // Only an arrival, a return or stock pulled off a dated bucket names a
        // date. A correction changes the count as a whole and has none.
        let expires_on = match kind {
            crate::db::StockKind::Receive
            | crate::db::StockKind::Return
            | crate::db::StockKind::Waste => {
                if expires_text.is_empty() {
                    None
                } else {
                    match crate::db::parse_expiry_date(&expires_text) {
                        Some(date) => Some(date),
                        None => {
                            return self.stock_move_error("Write the date like 31/12/2026.");
                        }
                    }
                }
            }
            _ => None,
        };
        if kind == crate::db::StockKind::Receive && perishable && expires_on.is_none() {
            return self
                .stock_move_error("This product is perishable; date the delivery from the pack.");
        }

        match crate::db::record_stock_movement(
            &self.store,
            product_id,
            delta,
            kind,
            &note,
            expires_on.as_deref(),
            user_id.as_deref(),
        ) {
            Ok(()) => {
                self.stock_move = None;
                self.reload_stock();
                self.notice = Some(format!("{} recorded for {product_name}.", kind.label()));
            }
            Err(error) => {
                tracing::error!(error = %error, "recording a stock movement failed");
                self.stock_move_error(format!("Could not record it: {error}"));
            }
        }
    }

    /// Read one product's recent movements for the history modal.
    pub fn open_stock_history(&mut self, product_id: i64, product_name: &str) {
        let movements = crate::db::stock_movements(&self.store, product_id);
        self.stock_history = Some(StockHistory {
            product_name: product_name.to_owned(),
            movements,
        });
        self.stock_editor = None;
        self.stock_move = None;
        self.notice = None;
    }

    fn stock_editor_error(&mut self, message: impl Into<String>) {
        if let Some(draft) = self.stock_editor.as_mut() {
            draft.error = Some(message.into());
        }
    }

    fn stock_move_error(&mut self, message: impl Into<String>) {
        if let Some(draft) = self.stock_move.as_mut() {
            draft.error = Some(message.into());
        }
    }
}
