//! Purchasing: a purchase order from first line to received stock.
//!
//! A purchase is built as a draft, saved, and later received — three states of
//! the same document. The line arithmetic that sums it, the receiving that
//! writes stock, and the error the modal shows all read that one draft.

use super::*;
/// One line in the purchase editor.
///
/// Quantity and cost are text while they are being typed: parsing on every
/// keystroke would eat a half-typed `12` and a blank cost means "not known",
/// which is a real answer, not an error.
pub struct PurchaseLineDraft {
    pub product_id: i64,
    pub name: String,
    pub qty: String,
    pub cost: String,
    /// How many base units one ordered unit holds. Prefilled from the product
    /// but editable: the shop may buy loose this week and a boxed pack next.
    pub pack_size: String,
    /// The date printed on the pack, as typed. Optional while planning, required
    /// when a perishable delivery is received.
    pub expires_on: String,
}

/// A purchase being planned or edited, open as the Restock page.
pub struct PurchaseDraft {
    /// `None` while creating, `Some` while editing a plan.
    pub id: Option<i64>,
    pub supplier_id: Option<String>,
    pub note: String,
    pub lines: Vec<PurchaseLineDraft>,
    /// Search inside the product picker, separate from the list's filter.
    pub search: String,
    pub error: Option<String>,
    /// First click arms deletion, second deletes; the row is where the mistake
    /// would be, so the confirmation lives in the form.
    pub confirm_delete: bool,
    /// First press of Received arms it, the second records the trip. Stock is
    /// written once and there is no undo, so the irreversible press asks twice
    /// for the same reason deleting does.
    pub confirm_receive: bool,
}

impl State {
    /// Re-read the trip list after a write.
    pub fn reload_purchases(&mut self) {
        self.purchases = crate::db::purchases(&self.store);
    }

    /// Open a blank plan for a new trip.
    pub fn start_purchase(&mut self) {
        self.purchase_editor = Some(PurchaseDraft {
            id: None,
            supplier_id: None,
            note: String::new(),
            lines: Vec::new(),
            search: String::new(),
            error: None,
            confirm_delete: false,
            confirm_receive: false,
        });
        self.purchase_view = None;
        self.notice = None;
    }

    /// Open an existing list for editing.
    ///
    /// A received trip is a record, not a list: it opens read-only, because
    /// replacing a line a movement points at would orphan the ledger.
    pub fn edit_purchase(&mut self, id: i64) {
        let Some(detail) = crate::db::purchase_detail(&self.store, id) else {
            self.notice = Some("That purchase is no longer there.".to_owned());
            return;
        };
        if detail.purchase.status == crate::db::PurchaseStatus::Received {
            self.purchase_view = Some(detail);
            self.purchase_editor = None;
            self.notice = None;
            return;
        }

        let currency = self.currency;
        let lines = detail
            .lines
            .iter()
            .map(|line| PurchaseLineDraft {
                product_id: line.product_id.unwrap_or_default(),
                name: line.name.clone(),
                qty: line.qty_ordered.to_string(),
                cost: line
                    .unit_cost_minor
                    .map(|cost| plain_amount(cost, currency))
                    .unwrap_or_default(),
                pack_size: line.pack_size.to_string(),
                expires_on: line.expires_on.clone().unwrap_or_default(),
            })
            .collect();

        self.purchase_editor = Some(PurchaseDraft {
            id: Some(detail.purchase.id),
            supplier_id: detail.purchase.supplier_id.clone(),
            note: detail.purchase.note.clone().unwrap_or_default(),
            lines,
            search: String::new(),
            error: None,
            confirm_delete: false,
            confirm_receive: false,
        });
        self.purchase_view = None;
        self.notice = None;
    }

    /// Add one tracked product to the plan being edited.
    pub fn purchase_add_product(&mut self, product_id: i64) {
        let Some((name, cost, pack_size)) = self
            .stock
            .iter()
            .find(|item| item.product_id == product_id)
            .map(|item| {
                let cost = if item.pack_size > 1 {
                    item.pack_cost_minor()
                } else {
                    item.unit_cost_minor()
                };
                (item.name.clone(), cost, item.pack_size)
            })
        else {
            return;
        };
        let currency = self.currency;
        let Some(draft) = self.purchase_editor.as_mut() else {
            return;
        };
        draft.lines.push(PurchaseLineDraft {
            product_id,
            name,
            qty: "1".to_owned(),
            cost: cost
                .map(|cost| plain_amount(cost, currency))
                .unwrap_or_default(),
            pack_size: pack_size.to_string(),
            // Typed from the pack at receiving time; a plan may hold the same
            // product twice only when the two deliveries are dated apart.
            expires_on: String::new(),
        });
        draft.confirm_receive = false;
        draft.error = None;
    }

    /// Add a product to the restock list from the catalog row.
    ///
    /// A row is a single product, so it never makes sense to write stock here:
    /// the shop is planning a trip, and receiving stays a separate act on the
    /// Restock page. Starting the list if none is open and moving to that page
    /// means the button's result is always visible — the action cannot look as
    /// though it did nothing.
    pub fn restock_product(&mut self, product_id: i64) {
        let name = self
            .stock
            .iter()
            .find(|item| item.product_id == product_id)
            .map(|item| item.name.clone());
        let Some(name) = name else {
            self.notice = Some("Track this product before adding it to a restock list.".to_owned());
            return;
        };

        if self.purchase_editor.is_none() {
            self.start_purchase();
        }
        let already = self
            .purchase_editor
            .as_ref()
            .is_some_and(|draft| draft.lines.iter().any(|line| line.product_id == product_id));
        if !already {
            self.purchase_add_product(product_id);
        }
        // `select` clears the old page's notice, so the receipt for this click
        // is written after the move.
        self.select(crate::modules::PURCHASES);
        self.notice = Some(if already {
            format!("{name} is already on the restock list.")
        } else {
            format!("Added {name} to the restock list.")
        });
    }

    /// Add every ticked catalog row to the restock list in one act.
    ///
    /// Untracked rows and rows already on the plan are counted, not guessed at:
    /// a shop that ticks eight things should hear "added six, skipped two"
    /// rather than wonder why the list is short. The selection is spent by the
    /// action, and the page changes so the result is immediately visible.
    pub fn restock_selected(&mut self) {
        let picked = std::mem::take(&mut self.selected_products);
        if picked.is_empty() {
            return;
        }
        let mut added = Vec::new();
        let mut skipped = 0;
        for id in picked {
            let tracked = self.stock.iter().any(|item| item.product_id == id);
            let already = self
                .purchase_editor
                .as_ref()
                .is_some_and(|draft| draft.lines.iter().any(|line| line.product_id == id));
            if tracked && !already {
                added.push(id);
            } else {
                skipped += 1;
            }
        }
        if added.is_empty() {
            self.notice = Some(format!(
                "Nothing was added: {skipped} selected {} already on the list or untracked.",
                if skipped == 1 {
                    "product is"
                } else {
                    "products are"
                }
            ));
            return;
        }
        if self.purchase_editor.is_none() {
            self.start_purchase();
        }
        for id in &added {
            self.purchase_add_product(*id);
        }
        self.select(crate::modules::PURCHASES);
        let noun = if added.len() == 1 {
            "product"
        } else {
            "products"
        };
        self.notice = Some(if skipped == 0 {
            format!("Added {} {noun} to the restock list.", added.len())
        } else {
            format!(
                "Added {} {noun} to the restock list, skipped {skipped}.",
                added.len()
            )
        });
    }

    /// Start (or extend) a restock list with everything below its warning line.
    ///
    /// The toolbar's bulk path to the same list the row button feeds. An open
    /// plan is kept and filled rather than thrown away, so a half-built trip
    /// never disappears because somebody took the shortcut.
    pub fn restock_low_stock(&mut self) {
        if !self.stock.iter().any(|item| item.needs_restock()) {
            self.notice = Some("Nothing is low, and nothing is already on its way.".to_owned());
            return;
        }
        if self.purchase_editor.is_none() {
            self.start_purchase();
        }
        self.purchase_add_low_stock();
        self.select(crate::modules::PURCHASES);
        self.notice = Some("Added low products to the restock list.".to_owned());
    }

    /// Fill the plan with everything at or below its warning line.
    ///
    /// The quantity aims at the product's target minus what is on the shelf and
    /// already on its way, so a trip cannot order the same gap twice. Anything
    /// already on the list is left alone; the operator can adjust it.
    pub fn purchase_add_low_stock(&mut self) {
        let Some(draft) = self.purchase_editor.as_ref() else {
            return;
        };
        let already: Vec<i64> = draft.lines.iter().map(|line| line.product_id).collect();
        let currency = self.currency;
        let suggestions: Vec<(i64, String, i64, Option<i64>, i64)> = self
            .stock
            .iter()
            .filter(|item| item.needs_restock() && !already.contains(&item.product_id))
            .map(|item| {
                let pack_size = item.pack_size.max(1);
                // The suggestion is in base units; a purchase line counts
                // ordered units. Ordering whole packs round up, so the trip
                // still covers the gap.
                let units = item.suggested();
                let qty = if pack_size > 1 {
                    (units + pack_size - 1) / pack_size
                } else {
                    units
                };
                let cost = if pack_size > 1 {
                    item.pack_cost_minor()
                } else {
                    item.unit_cost_minor()
                };
                (item.product_id, item.name.clone(), qty, cost, pack_size)
            })
            .collect();

        let Some(draft) = self.purchase_editor.as_mut() else {
            return;
        };
        if suggestions.is_empty() {
            draft.error = Some(if already.is_empty() {
                "Nothing is low, and nothing is already on its way.".to_owned()
            } else {
                "Everything that is low is already on this list.".to_owned()
            });
            return;
        }

        for (product_id, name, qty, cost, pack_size) in suggestions {
            draft.lines.push(PurchaseLineDraft {
                product_id,
                name,
                qty: qty.to_string(),
                cost: cost
                    .map(|cost| plain_amount(cost, currency))
                    .unwrap_or_default(),
                pack_size: pack_size.to_string(),
                expires_on: String::new(),
            });
        }
        draft.confirm_receive = false;
        draft.error = None;
    }

    pub fn purchase_remove_line(&mut self, index: usize) {
        if let Some(draft) = self.purchase_editor.as_mut() {
            if index < draft.lines.len() {
                draft.lines.remove(index);
            }
            draft.confirm_delete = false;
            draft.confirm_receive = false;
        }
    }

    /// Delete the list on screen. Only a trip that never arrived can go, and
    /// only after a second tap: the same button is the undo of itself.
    pub fn delete_purchase(&mut self) {
        let Some(draft) = self.purchase_editor.as_ref() else {
            return;
        };
        let Some(id) = draft.id else {
            // Never saved, so there is nothing to delete.
            self.purchase_editor = None;
            return;
        };
        if !draft.confirm_delete {
            if let Some(draft) = self.purchase_editor.as_mut() {
                draft.confirm_delete = true;
            }
            return;
        }

        match crate::db::delete_planned_purchase(&self.store, id) {
            Ok(()) => {
                self.purchase_editor = None;
                self.reload_purchases();
                // The deleted list was "on its way", so the shelf's incoming
                // count and the low-stock warnings just changed.
                self.reload_stock();
                self.notice = Some("List deleted.".to_owned());
            }
            Err(error) => {
                tracing::error!(error = %error, "deleting a purchase failed");
                self.purchase_error(format!("Could not delete it: {error}"));
            }
        }
    }

    /// The draft's lines as the database wants them.
    ///
    /// `allow_zero` is for receiving: a line counted at zero was not bought, and
    /// that is a fact worth keeping. A list being saved needs a positive
    /// quantity on every line, because "buy zero of this" is not a plan.
    fn draft_lines(&self, allow_zero: bool) -> Result<Vec<crate::db::PurchaseLineInput>, String> {
        let Some(draft) = self.purchase_editor.as_ref() else {
            return Err("There is no list to save.".to_owned());
        };
        let mut lines = Vec::with_capacity(draft.lines.len());
        for line in &draft.lines {
            let qty = match line.qty.trim().parse::<i64>() {
                Ok(qty) if qty > 0 || (allow_zero && qty == 0) => qty,
                _ => {
                    return Err(if allow_zero {
                        format!("{} needs a whole quantity of zero or more.", line.name)
                    } else {
                        format!("{} needs a whole quantity of one or more.", line.name)
                    });
                }
            };
            let unit_cost_minor = self
                .parse_cost(&line.cost)
                .map_err(|message| format!("{}: {message}", line.name))?;
            let pack_size = match line.pack_size.trim() {
                "" => 1,
                text => match text.parse::<i64>() {
                    Ok(size) if size >= 1 => size,
                    _ => {
                        return Err(format!(
                            "{} needs a whole pack size of one or more.",
                            line.name
                        ));
                    }
                },
            };
            let expires_on = match line.expires_on.trim() {
                "" => None,
                text => match crate::db::parse_expiry_date(text) {
                    Some(date) => Some(date),
                    None => return Err(format!("{}: write the date like 31/12/2026.", line.name)),
                },
            };
            if allow_zero && qty > 0 {
                let perishable = self
                    .stock
                    .iter()
                    .any(|item| item.product_id == line.product_id && item.perishable);
                if perishable && expires_on.is_none() {
                    return Err(format!(
                        "{} is perishable; date the delivery from the pack.",
                        line.name
                    ));
                }
            }

            lines.push(crate::db::PurchaseLineInput {
                product_id: line.product_id,
                qty,
                unit_cost_minor,
                pack_size,
                expires_on,
            });
        }
        Ok(lines)
    }

    /// Create or update the list the draft describes, and return its id.
    fn write_list(&self, lines: &[crate::db::PurchaseLineInput]) -> anyhow::Result<i64> {
        let Some(draft) = self.purchase_editor.as_ref() else {
            anyhow::bail!("there is no list to save");
        };
        let user_id = self.session.as_ref().map(|session| session.user_id.clone());
        match draft.id {
            Some(id) => crate::db::update_purchase(
                &self.store,
                id,
                draft.supplier_id.as_deref(),
                &draft.note,
                lines,
            )
            .map(|()| id),
            None => crate::db::create_purchase(
                &self.store,
                draft.supplier_id.as_deref(),
                &draft.note,
                lines,
                user_id.as_deref(),
            ),
        }
    }

    /// Save the list and go back to Restock.
    pub fn save_purchase(&mut self) {
        let lines = match self.draft_lines(false) {
            Ok(lines) => lines,
            Err(message) => return self.purchase_error(message),
        };
        if lines.is_empty() {
            return self.purchase_error("Add at least one product to the list.");
        }

        let existing = self
            .purchase_editor
            .as_ref()
            .is_some_and(|draft| draft.id.is_some());
        match self.write_list(&lines) {
            Ok(_) => {
                self.purchase_editor = None;
                self.reload_purchases();
                self.reload_stock();
                self.notice = Some(if existing {
                    "List updated.".to_owned()
                } else {
                    "List saved.".to_owned()
                });
            }
            Err(error) => {
                tracing::error!(error = %error, "saving a purchase failed");
                self.purchase_error(format!("Could not save the list: {error}"));
            }
        }
    }

    /// Close the trip: write what was actually bought and add it to stock.
    ///
    /// The list on screen is the count sheet. A line's quantity is what came
    /// home — fewer than planned is normal, more is a market with a good deal —
    /// and the same number is what enters the shelf. The trip is over when this
    /// returns; there is no half-received state to come back to.
    ///
    /// A saved list is not rewritten first: its quantities are the plan, and the
    /// numbers on screen are what arrived. A list that was never saved is
    /// written and closed in one write, so a failure at the counter cannot
    /// leave a saved plan behind.
    pub fn receive_purchase(&mut self) {
        let lines = match self.draft_lines(true) {
            Ok(lines) => lines,
            Err(message) => return self.purchase_error(message),
        };
        if lines.is_empty() {
            return self.purchase_error("Add at least one product to the trip.");
        }
        if !lines.iter().any(|line| line.qty > 0) {
            return self
                .purchase_error("Type what came home on at least one line, or delete the list.");
        }

        // Stock is written once and there is no undo, so the first press only
        // arms the button and the second records the trip — the same rule as
        // deleting a list. The validation above runs first: a typo in a count
        // should be reported, not hidden behind a confirmation.
        let armed = self
            .purchase_editor
            .as_ref()
            .is_some_and(|draft| draft.confirm_receive);
        if !armed {
            if let Some(draft) = self.purchase_editor.as_mut() {
                draft.confirm_receive = true;
            }
            return;
        }

        let (note, supplier_id) = self
            .purchase_editor
            .as_ref()
            .map(|draft| (draft.note.clone(), draft.supplier_id.clone()))
            .unwrap_or_default();
        let user_id = self.session.as_ref().map(|session| session.user_id.clone());

        let unsaved = self
            .purchase_editor
            .as_ref()
            .is_some_and(|draft| draft.id.is_none());
        if unsaved {
            // A count sheet may carry a line that brought nothing home, but a
            // plan may not: "buy zero of this" is not something the list can
            // say. Only the bought lines are written, and they are written with
            // the trip itself so a failure cannot leave a saved plan behind.
            let bought: Vec<_> = lines.into_iter().filter(|line| line.qty > 0).collect();
            match crate::db::create_and_receive(
                &self.store,
                supplier_id.as_deref(),
                &note,
                &bought,
                user_id.as_deref(),
            ) {
                Ok(_) => {
                    let units: i64 = bought.iter().map(|line| line.qty * line.pack_size).sum();
                    self.purchase_received(units);
                }
                Err(error) => {
                    tracing::error!(error = %error, "receiving a new purchase failed");
                    self.purchase_error(format!("Could not receive it: {error}"));
                }
            }
            return;
        }

        let Some(id) = self.purchase_editor.as_ref().and_then(|draft| draft.id) else {
            return;
        };
        let Some(detail) = crate::db::purchase_detail(&self.store, id) else {
            return self.purchase_error("That purchase is no longer there.");
        };
        // Line ids are matched by product, not carried in the draft: saving a
        // list rewrites its rows, so an id captured when the page opened may
        // already be gone.
        let mut existing: Vec<(i64, i64)> = detail
            .lines
            .iter()
            .filter_map(|line| line.product_id.map(|product_id| (product_id, line.id)))
            .collect();

        let mut received = Vec::with_capacity(lines.len());
        for line in lines {
            let line_id = existing
                .iter()
                .position(|(product_id, _)| *product_id == line.product_id)
                .map(|index| existing.remove(index).1);
            received.push(crate::db::ReceiveLine {
                line_id,
                product_id: line.product_id,
                qty: line.qty,
                unit_cost_minor: line.unit_cost_minor,
                pack_size: line.pack_size,
                expires_on: line.expires_on,
            });
        }
        // A line that is no longer on the sheet was a planning mistake. It can
        // go, because nothing was ever received against it.
        let remove: Vec<i64> = existing.into_iter().map(|(_, line_id)| line_id).collect();

        match crate::db::receive_purchase(
            &self.store,
            id,
            supplier_id.as_deref(),
            &received,
            &remove,
            &note,
            user_id.as_deref(),
        ) {
            Ok(()) => {
                let units: i64 = received.iter().map(|line| line.qty * line.pack_size).sum();
                self.purchase_received(units);
            }
            Err(error) => {
                tracing::error!(error = %error, "receiving a purchase failed");
                self.purchase_error(format!("Could not receive it: {error}"));
            }
        }
    }

    /// Close the Restock page after a trip was recorded, and say what went on
    /// the shelf. Shared by a saved list and one created at the counter: both
    /// end in the same place.
    fn purchase_received(&mut self, units: i64) {
        self.purchase_editor = None;
        self.purchase_view = None;
        self.reload_purchases();
        self.reload_stock();
        self.reload_products();
        self.notice = Some(format!(
            "Received {units} unit{}.",
            if units == 1 { "" } else { "s" }
        ));
    }

    /// An empty cost is a missing fact, not zero. Anything else must parse as
    /// an amount in this till's currency and cannot be negative.
    fn parse_cost(&self, text: &str) -> Result<Option<i64>, String> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(None);
        }
        match Money::parse(text, self.currency) {
            Ok(cost) if cost.minor() >= 0 => Ok(Some(cost.minor())),
            Ok(_) => Err("a cost cannot be negative.".to_owned()),
            Err(error) => Err(format!("a cost must be an amount like 1200 ({error}).")),
        }
    }

    fn purchase_error(&mut self, message: impl Into<String>) {
        if let Some(draft) = self.purchase_editor.as_mut() {
            draft.error = Some(message.into());
        }
    }
}
