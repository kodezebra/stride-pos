//! Regular customers: the shop's list, and the form that keeps it.
//!
//! A customer is a pricing term rather than a loyalty record, so the only
//! editable facts are the name the cashier picks, a phone number to ring, and
//! the rate that comes off the basket. The list is small and read with the
//! catalog, like products.

use super::*;

/// The customer form, open over Settings.
///
/// The rate is typed as a percentage because that is how a shopkeeper says it;
/// it is stored in basis points so half a percent is not lost to rounding. The
/// form keeps the text as typed until Save, so a rejected value does not erase
/// the rest of the row.
pub struct CustomerDraft {
    /// `None` while adding, `Some` while editing.
    pub id: Option<i64>,
    pub name: String,
    pub phone: String,
    pub discount: String,
    pub error: Option<String>,
}

impl CustomerDraft {
    fn new() -> Self {
        Self {
            id: None,
            name: String::new(),
            phone: String::new(),
            discount: String::new(),
            error: None,
        }
    }

    fn from(customer: &Customer) -> Self {
        Self {
            id: Some(customer.id),
            name: customer.name.clone(),
            phone: customer.phone.clone().unwrap_or_default(),
            discount: percent_text(customer.discount_bp),
            error: None,
        }
    }
}

/// A basis-point rate as a shopkeeper writes it: `5%`, `2.5%`, `12.25%`.
///
/// Trailing zeroes are dropped, so a whole-percent shop reads `5%` and not
/// `5.00%`.
pub fn percent_text(bp: i64) -> String {
    let whole = bp / 100;
    let frac = bp % 100;
    if frac == 0 {
        format!("{whole}%")
    } else if frac % 10 == 0 {
        format!("{whole}.{}%", frac / 10)
    } else {
        format!("{whole}.{frac:02}%")
    }
}

/// Parse a typed percentage into basis points.
///
/// Accepts `5`, `5.`, `5.5` and `2.25`, with or without the `%`; an empty field
/// is a customer with no rate. More than two decimals is refused rather than
/// rounded: the rate is the shop's own term, and silently changing it would
/// change what every customer of theirs pays.
pub fn parse_percent(text: &str) -> Result<i64, &'static str> {
    let text = text.trim().trim_end_matches('%').trim();
    if text.is_empty() {
        return Ok(0);
    }
    let (whole, fraction) = match text.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (text, ""),
    };
    if !fraction.chars().all(|ch| ch.is_ascii_digit()) || fraction.len() > 2 {
        return Err("Use at most two decimal places, like 2.5.");
    }
    let whole: i64 = if whole.is_empty() {
        0
    } else {
        whole.parse().map_err(|_| "Use a number, like 5 or 2.5.")?
    };
    if whole < 0 {
        return Err("A discount cannot be negative.");
    }
    let fraction: i64 = match fraction.len() {
        0 => 0,
        1 => fraction.parse::<i64>().unwrap_or(0) * 10,
        _ => fraction.parse::<i64>().unwrap_or(0),
    };
    let bp = whole.saturating_mul(100).saturating_add(fraction);
    if bp > 10_000 {
        return Err("A discount cannot exceed 100%.");
    }
    Ok(bp)
}

impl State {
    /// Re-read the customer list after a write or a catalog sync.
    pub fn reload_customers(&mut self) {
        self.customers = crate::db::customers(&self.store);
        // The attached customer is a copy; refresh its terms so a rate edited
        // mid-basket is the rate that is charged.
        if let Some(attached) = self.customer.as_ref() {
            self.customer = self
                .customers
                .iter()
                .find(|customer| customer.id == attached.id)
                .cloned();
        }
    }

    /// Open an empty customer form.
    pub fn start_customer(&mut self) {
        self.customer_editor = Some(CustomerDraft::new());
        self.notice = None;
    }

    /// Open the form on an existing customer.
    pub fn edit_customer(&mut self, customer: &Customer) {
        self.customer_editor = Some(CustomerDraft::from(customer));
        self.notice = None;
    }

    /// Store the open customer form.
    ///
    /// Everything is checked before the write, so a rejected form leaves the
    /// database untouched and the typed text on screen.
    pub fn save_customer(&mut self) {
        let Some(draft) = self.customer_editor.as_ref() else {
            return;
        };

        let id = draft.id;
        let name = draft.name.trim().to_owned();
        let phone = draft.phone.trim().to_owned();

        if name.is_empty() {
            return self.customer_error("A customer needs a name.");
        }
        let discount_bp = match parse_percent(&draft.discount) {
            Ok(bp) => bp,
            Err(message) => return self.customer_error(message),
        };
        if crate::db::customer_name_taken(&self.store, &name, id) {
            return self.customer_error("Another customer already has that name.");
        }

        match crate::db::save_customer(&self.store, id, &name, Some(&phone), discount_bp) {
            Ok(_) => {
                self.customer_editor = None;
                self.reload_customers();
                self.notice = Some(format!("Saved {name}."));
            }
            Err(error) => {
                tracing::error!(error = %error, "saving a customer failed");
                self.customer_error(format!("Could not save: {error}"));
            }
        }
    }

    /// Hide a customer from the list. Their sales keep the name they were
    /// written under.
    pub fn archive_customer(&mut self, id: i64) {
        match crate::db::archive_customer(&self.store, id) {
            Ok(()) => {
                self.reload_customers();
                self.notice = Some("Customer archived.".to_owned());
            }
            Err(error) => {
                tracing::error!(error = %error, "archiving a customer failed");
                self.notice = Some(format!("Could not archive the customer: {error}"));
            }
        }
    }

    fn customer_error(&mut self, message: impl Into<String>) {
        if let Some(draft) = self.customer_editor.as_mut() {
            draft.error = Some(message.into());
        }
    }
}
