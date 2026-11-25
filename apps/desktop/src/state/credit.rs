//! Settling a sale that was sold on credit.
//!
//! The customer takes the goods at checkout and comes back with the money
//! later. That later payment is another append-only row against the original
//! sale, so the balance shown here is always derived from what was actually
//! received rather than a number kept alongside it.

use money::Money;

use super::State;

impl State {
    /// Open the settle panel for the customer currently at the till.
    pub fn open_credit_panel(&mut self) {
        if self.customer.is_none() {
            self.notice = Some("Select the customer first.".to_owned());
            return;
        }
        // Taking money later is still taking money. The same permission that
        // opens the checkout tender panel opens this one.
        if !self.permits(crate::auth::Permission::TakePayment) {
            return self.ask_override(crate::auth::Permission::TakePayment, None);
        }
        self.refresh_credit_sales();
        self.credit_tendered.clear();
        self.credit_reference.clear();
        self.credit_error = None;
        self.credit_panel = true;
    }

    pub fn close_credit_panel(&mut self) {
        self.credit_panel = false;
        self.credit_error = None;
        self.credit_sale_id = None;
        self.credit_tendered.clear();
        self.credit_reference.clear();
    }

    /// Re-read what the selected customer still owes.
    pub fn refresh_credit_sales(&mut self) {
        self.credit_sales = match self.customer.as_ref() {
            Some(customer) => crate::db::credit_sales(&self.store, customer.id),
            None => Vec::new(),
        };
        if !self
            .credit_sales
            .iter()
            .any(|sale| Some(sale.id) == self.credit_sale_id)
        {
            self.credit_sale_id = self.credit_sales.first().map(|sale| sale.id);
        }
    }

    /// Record the typed payment against the selected credit sale.
    pub fn record_credit_payment(&mut self) {
        // An approval can expire while the panel is open, so ask again at the
        // moment of the write rather than trusting the earlier check.
        if !self.permits(crate::auth::Permission::TakePayment) {
            return self.ask_override(crate::auth::Permission::TakePayment, None);
        }

        let Some(sale_id) = self.credit_sale_id else {
            self.credit_error = Some("Pick a sale to pay off.".to_owned());
            return;
        };
        let Some(sale) = self.credit_sales.iter().find(|sale| sale.id == sale_id) else {
            self.credit_error = Some("That sale no longer has a balance.".to_owned());
            return;
        };
        let balance_minor = sale.balance_minor;
        let receipt_no = sale.receipt_no.clone();
        let text = self.credit_tendered.trim().to_owned();
        let method = self.credit_method.trim().to_owned();
        let reference = self.credit_reference.trim().to_owned();

        let entered_minor = if text.is_empty() {
            balance_minor
        } else {
            match Money::parse(&text, self.currency) {
                Ok(amount) => amount.minor(),
                Err(error) => {
                    self.credit_error = Some(error.to_string());
                    return;
                }
            }
        };
        if entered_minor <= 0 {
            self.credit_error = Some("Enter an amount.".to_owned());
            return;
        }

        let is_cash = method.eq_ignore_ascii_case("cash");
        if !is_cash && entered_minor > balance_minor {
            self.credit_error = Some(format!(
                "That is {} more than the balance.",
                self.money(entered_minor - balance_minor)
            ));
            return;
        }

        // Cash can cover the balance and give the rest back; a shorter cash
        // payment is a part-payment. Non-cash is exact, like at checkout.
        let amount_minor = entered_minor.min(balance_minor);
        let tendered_minor = if is_cash { entered_minor } else { amount_minor };
        let reference = if is_cash || reference.is_empty() {
            None
        } else {
            Some(reference.as_str())
        };

        // The drawer being counted tonight is the one this payment belongs to,
        // not the drawer the original sale was rung up in.
        let shift_uid = self.ensure_shift();
        let user_id = self.session.as_ref().map(|session| session.user_id.clone());
        if let Err(error) = crate::db::record_credit_payment(
            &self.store,
            &crate::db::CreditPayment {
                sale_id,
                method: &method,
                amount_minor,
                tendered_minor,
                reference,
                shift_uid: shift_uid.as_deref(),
                user_id: user_id.as_deref(),
            },
        ) {
            self.credit_error = Some(error.to_string());
            return;
        }

        let label = receipt_no
            .map(|number| format!("sale {number}"))
            .unwrap_or_else(|| format!("sale #{sale_id}"));
        self.notice = Some(format!(
            "Recorded {} against {label}",
            self.money(amount_minor)
        ));
        self.refresh_credit_sales();
        self.credit_tendered.clear();
        self.credit_reference.clear();
        self.credit_error = None;
        if self.credit_sales.is_empty() {
            self.credit_panel = false;
        }
    }
}
