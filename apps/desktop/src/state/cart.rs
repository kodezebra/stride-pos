//! The cart: the sale in progress, and the payment that closes it.
//!
//! The cart is the only place a sale is built, so the money arithmetic that
//! decides what is owed (rounding, tax, change) lives beside the lines it
//! reads rather than with the sale once it is written.

use super::*;

/// Which price ladder the till is selling on.
///
/// Retail prices from the product's quantity breaks: the price is a property of
/// the count, so ten units can cost less each than one. Wholesale pins every
/// loose unit to the deepest rung the shop offers, whatever the count — a
/// wholesale buyer negotiates for the floor price, not for one unit at a time.
///
/// The mode belongs to the sale in progress, not to the catalog: switching it
/// reprices the lines already in the basket, and the next sale starts retail
/// again so a wholesale price cannot leak to a walk-in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CustomerMode {
    #[default]
    Retail,
    Wholesale,
}

impl CustomerMode {
    /// The words on the switch. A cashier has to read the mode from across the
    /// counter, so it says what the customer is charged, not the internal name.
    pub fn label(self) -> &'static str {
        match self {
            CustomerMode::Retail => "Retail prices",
            CustomerMode::Wholesale => "Wholesale prices",
        }
    }
}

/// One line on the till. Money is integer minor units, always.
///
/// The line keeps the product id and a copy of the name and price: a sale has to
/// be able to say what was sold at what price even after the catalog changes.
#[derive(Clone, Debug)]
pub struct CartLine {
    pub product_id: i64,
    pub name: String,
    pub qty: i64,
    /// Price per unit, in minor units.
    pub unit_minor: i64,
    /// How many base units one line unit holds: `1` for a loose unit, the pack
    /// size for a sealed pack. Stock leaves the shelf by this, and the line is
    /// merged by `(product_id, unit_factor)` so a pack and a loose unit never
    /// collapse into one row.
    pub unit_factor: i64,
    /// What that unit was called at the time, when it was anything but `unit`.
    pub unit_label: Option<String>,
    /// The ladder this line was added under, ascending: the shelf price at one
    /// unit, then the shop's breaks. A pack line has a single rung — its price
    /// is already the bulk deal and a break on top would stack two discounts.
    ///
    /// Kept on the line, not looked up on every frame, for the same reason the
    /// name and price are: editing the catalog mid-sale cannot silently reprice
    /// something already in the basket.
    pub tiers: Vec<PriceTier>,
    /// The break that set `unit_minor`, when one did. `None` on the shelf price.
    pub tier_qty: Option<i64>,
    /// Whether this line is priced wholesale: the deepest rung, whatever the
    /// count. Kept on the line so the steppers and the quantity keypad keep the
    /// customer's terms without threading a mode through every call.
    pub wholesale: bool,
}

impl CartLine {
    pub fn total_minor(&self) -> i64 {
        self.qty * self.unit_minor
    }

    /// Set the count and price the line again.
    ///
    /// Repricing is the whole point of a quantity break: the break is a
    /// property of the count, so every path that changes the count goes through
    /// here rather than assigning `qty`.
    pub fn set_qty(&mut self, qty: i64) {
        self.qty = qty;
        self.reprice();
    }

    /// Price this line wholesale, or put it back on the retail ladder.
    ///
    /// The flag lives on the line, so a later `set_qty` keeps the terms: the
    /// cashier who bumps a wholesale line's count must not silently move it to
    /// retail prices.
    pub fn set_wholesale(&mut self, wholesale: bool) {
        self.wholesale = wholesale;
        self.reprice();
    }

    /// Price the line from its count, or from the wholesale flag.
    ///
    /// Retail takes the deepest rung this line reaches; below the first break
    /// the shelf price stands, which is why the ladder carries a rung at one
    /// unit. Wholesale takes the last rung outright: the ladder is ascending,
    /// so the last is the floor.
    fn reprice(&mut self) {
        if self.wholesale {
            if let Some(tier) = self.tiers.last() {
                self.unit_minor = tier.price_minor;
                // No rung explains the price: the mode does. Printing "50+
                // price" on a single unit would be a lie.
                self.tier_qty = None;
            }
            return;
        }
        let Some(tier) = self
            .tiers
            .iter()
            .rev()
            .find(|tier| tier.min_qty <= self.qty)
        else {
            return;
        };
        self.unit_minor = tier.price_minor;
        self.tier_qty = (tier.min_qty > 1).then_some(tier.min_qty);
    }

    /// What to call the price on this line, when it is not the shelf price:
    /// `10+ price`, or `Wholesale`.
    ///
    /// A pack's ladder is one rung at its own price, so "Wholesale" on a pack
    /// would explain nothing: the pack price is already the bulk deal. The same
    /// goes for a product with no breaks — wholesale and retail charge the same,
    /// and a label would promise a saving the line does not show.
    pub fn tier_label(&self) -> Option<String> {
        if self.wholesale {
            let floor = self.tiers.last()?.price_minor;
            let shelf = self.tiers.first()?.price_minor;
            return (self.unit_factor == 1 && floor < shelf).then(|| "Wholesale".to_owned());
        }
        self.tier_qty.map(|qty| format!("{qty}+ price"))
    }
}

/// The quantity editor for one cart line, open as a modal over the sell screen.
///
/// The count is held as typed text, not as a number: the cashier may clear it
/// and start again, and a half-typed quantity is not a quantity yet. The first
/// key replaces the prefilled current count, so setting an item from one to five
/// is one tap rather than a Clear and a five.
#[derive(Debug)]
pub struct CartQuantity {
    /// The product the row belongs to. A product can appear twice — loose and
    /// as a pack — so the factor is part of the identity too.
    pub product_id: i64,
    /// Which of the product's lines this editor is for.
    pub unit_factor: i64,
    pub digits: String,
    /// True until the first key: tapping `5` should mean five, not fifteen.
    pub replace: bool,
    /// True on the first frame, so the edit field can take focus and select
    /// the current count. Typing `5` then replaces it instead of appending.
    pub focus: bool,
    /// Set when the cashier tried to commit something that is not a quantity.
    pub error: Option<String>,
}

impl CartQuantity {
    pub fn new(product_id: i64, unit_factor: i64, qty: i64) -> Self {
        Self {
            product_id,
            unit_factor,
            digits: qty.to_string(),
            replace: true,
            focus: true,
            error: None,
        }
    }

    /// The positive quantity on screen, or `None` while it is empty or zero.
    /// The keypad and the edit field both stay inside four digits.
    pub fn value(&self) -> Option<i64> {
        let qty = self.digits.parse::<i64>().ok()?;
        (qty > 0).then_some(qty)
    }

    pub fn push_digit(&mut self, digit: char) {
        if self.replace {
            self.digits.clear();
            self.replace = false;
        }
        if self.digits.len() < 4 {
            self.digits.push(digit);
        }
        self.error = None;
    }

    pub fn backspace(&mut self) {
        self.replace = false;
        self.digits.pop();
        self.error = None;
    }

    pub fn clear(&mut self) {
        self.replace = false;
        self.digits.clear();
        self.error = None;
    }
}

/// How the Sell screen arranges the products it can sell.
///
/// The choice is a view, not a preference: it changes nothing about the cart or
/// the prices, and it is kept for the session so a cashier who prefers keys is
/// not sent back to the default every time they open the till.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SellView {
    /// One product per row. Long names stay readable; fewer fit on screen.
    List,
    /// Touch-sized tiles several per row. Trades name length for reach.
    #[default]
    Grid,
}

impl State {
    /// Add one unit of a product, merging with an existing line.
    ///
    /// `unit_factor` is `1` for a loose unit and the pack size for a sealed
    /// pack. A pack and a loose unit are different lines even though they are
    /// the same catalog row: the price and what leaves the shelf differ, and
    /// merging them would erase that. Scanning the same barcode twice still
    /// reads "2 × Espresso", not two lines: the cashier checks quantities, not
    /// line counts. The line keeps the name and ladder it was added under, so
    /// editing the catalog mid-sale cannot silently reprice something already
    /// in the basket.
    pub fn add_to_cart(&mut self, product: &Product, unit_factor: i64) {
        self.notice = None;
        let unit_factor = unit_factor.max(1);
        let (unit_minor, unit_label, tiers) = if unit_factor > 1 {
            let Some(pack_price) = product.pack_price_minor else {
                self.notice = Some(format!(
                    "{} has no pack price yet; set one in the catalog.",
                    product.name
                ));
                return;
            };
            (
                pack_price,
                Some(format!("pack of {unit_factor}")),
                // A pack is the bulk deal. The ladder is one rung so the line
                // reprices like every other line, without a break to reach.
                vec![PriceTier {
                    min_qty: 1,
                    price_minor: pack_price,
                }],
            )
        } else {
            let label = (product.unit_label != "unit").then(|| product.unit_label.clone());
            let mut tiers = Vec::with_capacity(product.tiers.len() + 1);
            tiers.push(PriceTier {
                min_qty: 1,
                price_minor: product.price_minor,
            });
            tiers.extend(product.tiers.iter().copied());
            (product.price_minor, label, tiers)
        };

        match self
            .cart
            .iter_mut()
            .find(|line| line.product_id == product.id && line.unit_factor == unit_factor)
        {
            Some(line) => {
                let qty = line.qty + 1;
                line.set_qty(qty);
            }
            None => {
                let mut line = CartLine {
                    product_id: product.id,
                    name: product.name.clone(),
                    qty: 1,
                    unit_minor,
                    unit_factor,
                    unit_label,
                    tiers,
                    tier_qty: None,
                    wholesale: self.customer_mode == CustomerMode::Wholesale,
                };
                // A wholesale line is not priced by its count, and the line is
                // born at one unit, so the shelf price it was built with is not
                // the price it charges.
                line.reprice();
                self.cart.push(line);
            }
        }
    }

    /// Open the quantity editor for one cart line.
    ///
    /// The index is the row the cashier tapped; the editor remembers the product
    /// rather than the row, so it still points at the right line if the cart
    /// changes.
    pub fn edit_cart_quantity(&mut self, index: usize) {
        if let Some(line) = self.cart.get(index) {
            self.cart_quantity = Some(CartQuantity::new(
                line.product_id,
                line.unit_factor,
                line.qty,
            ));
        }
    }

    pub fn close_cart_quantity(&mut self) {
        self.cart_quantity = None;
    }

    /// Commit a typed quantity to a cart line.
    ///
    /// A zero, a negative number or a stale product is ignored rather than
    /// removing the line: the modal has a separate Cancel, and deleting stock
    /// because a field was empty is not a safe default. The caller decides how
    /// to surface the rejection.
    pub fn set_cart_quantity(&mut self, product_id: i64, unit_factor: i64, qty: i64) {
        if qty <= 0 {
            return;
        }
        let Some(line) = self
            .cart
            .iter_mut()
            .find(|line| line.product_id == product_id && line.unit_factor == unit_factor)
        else {
            return;
        };
        line.set_qty(qty);
        self.cart_quantity = None;
    }

    /// Read one sale's full record for the detail modal.
    ///
    /// The list abbreviates by design; this is the answer to "what was in
    /// sale 4?". An id that points nowhere is logged and ignored — a row can
    /// only be clicked while it is on screen, so nothing to show is not an
    /// operator-visible error.
    pub fn open_sale(&mut self, id: i64) {
        match crate::db::sale_detail(&self.store, id) {
            Some(detail) => self.sale_open = Some(detail),
            None => tracing::warn!(sale = id, "a clicked sale could not be read"),
        }
    }

    pub fn close_sale(&mut self) {
        self.sale_open = None;
    }

    pub fn clear_cart(&mut self) {
        self.cart.clear();
        self.payment = None;
        self.cart_quantity = None;
        self.notice = None;
        // The next customer is a walk-in until the till is told otherwise.
        self.customer_mode = CustomerMode::Retail;
        self.customer = None;
    }

    /// Switch the whole cart between retail and wholesale pricing.
    ///
    /// Every line is repriced at once: a wholesale buyer who scans an item
    /// after the switch must not be charged the retail price just because the
    /// line is new. The mode is a property of the sale in progress, not a
    /// per-line choice the cashier can leave half-applied.
    pub fn set_customer_mode(&mut self, mode: CustomerMode) {
        self.customer_mode = mode;
        for line in &mut self.cart {
            line.set_wholesale(mode == CustomerMode::Wholesale);
        }
    }

    /// The other mode, for a switch that has only two states.
    pub fn toggle_customer_mode(&mut self) {
        let mode = match self.customer_mode {
            CustomerMode::Retail => CustomerMode::Wholesale,
            CustomerMode::Wholesale => CustomerMode::Retail,
        };
        self.set_customer_mode(mode);
    }

    pub fn cart_count(&self) -> i64 {
        self.cart.iter().map(|line| line.qty).sum()
    }

    /// The sum of the lines, before any basket discount: the price on the
    /// shelf for everything in the cart.
    pub fn cart_gross_minor(&self) -> i64 {
        self.cart.iter().map(CartLine::total_minor).sum()
    }

    /// The rate taken off this basket, in basis points.
    ///
    /// A wholesale basket is priced by the ladder, which already gives the
    /// customer the shop's floor; stacking the regular-customer rate on top
    /// would discount the same sale twice. So the rate is suppressed there, and
    /// the sell screen says so rather than silently charging more.
    pub fn effective_discount_bp(&self) -> i64 {
        if self.customer_mode == CustomerMode::Wholesale {
            return 0;
        }
        self.customer
            .as_ref()
            .map_or(0, |customer| customer.discount_bp)
    }

    /// The basket discount in money: the rate applied to the gross, rounded
    /// down to the minor unit so the shop never gives away a fraction of one.
    pub fn cart_discount_minor(&self) -> i64 {
        let gross = self.cart_gross_minor();
        let bp = self.effective_discount_bp();
        if gross <= 0 || bp <= 0 {
            return 0;
        }
        let discount = i128::from(gross) * i128::from(bp) / 10_000;
        i64::try_from(discount).unwrap_or(gross)
    }

    /// What the customer is charged before cash rounding: the line sum less the
    /// basket discount.
    pub fn cart_total_minor(&self) -> i64 {
        self.cart_gross_minor() - self.cart_discount_minor()
    }

    /// Attach a regular customer to this basket, or clear the attachment.
    ///
    /// A customer with a rate is a discount, so it asks for the discount
    /// permission. The approval cannot resume the action, so the cashier taps
    /// the name again once a manager has approved — the same rule the wholesale
    /// switch follows. Clearing needs no permission: taking a discount off is
    /// never the risky direction.
    pub fn set_customer(&mut self, customer: Option<Customer>) {
        let Some(customer) = customer else {
            self.customer = None;
            return;
        };
        if customer.discount_bp > 0 && !self.permits(crate::auth::Permission::Discount) {
            return self.ask_override(crate::auth::Permission::Discount, None);
        }
        self.customer = Some(customer);
    }

    /// The cash adjustment for this cart: how far the exact total is from the
    /// nearest coin the shop can hand back. Negative when the customer pays
    /// less than the exact total, which is the common case — 340 shillings
    /// against a 100 shilling coin rounds down to 300.
    ///
    /// The exact total is never changed; only what the drawer takes.
    pub fn cart_rounding_minor(&self) -> i64 {
        let total = self.cart_total_minor();
        let step = crate::currencies::cash_step(self.currency);
        match Money::new(total, self.currency).round_to_step(step) {
            Some(rounded) => rounded.minor() - total,
            // Only an overflowing total cannot round; it also cannot be paid.
            None => 0,
        }
    }

    /// What the customer must hand over.
    pub fn due_minor(&self) -> i64 {
        self.cart_total_minor() + self.cart_rounding_minor()
    }

    /// The tax on the cart, given the till's tax rate. Prices are tax-inclusive:
    /// the shelf price is what the customer pays, so the tax element is backed
    /// out of the total. `0` when the rate is `0`.
    ///
    /// Integer division rounds the pre-tax amount down, which slightly
    /// under-collects tax on odd figures — acceptable for a small till, and
    /// `pre_tax + tax` always equals the total.
    pub fn cart_tax_minor(&self) -> i64 {
        let rate = self.settings.tax_rate_percent;
        if rate <= 0 {
            return 0;
        }
        let total = self.cart_total_minor();
        let pre_tax = total
            .checked_mul(100)
            .and_then(|v| v.checked_div(100 + rate))
            .unwrap_or(0);
        total - pre_tax
    }

    /// Whether the basket is being paid for right now.
    ///
    /// The payment exists only while the tender panel is open. A written sale
    /// clears it before the next frame, so the till is never frozen behind a
    /// finished sale.
    pub fn is_tendering(&self) -> bool {
        self.payment.is_some()
    }

    /// Open the payment screen. Does nothing when the cart is empty or the
    /// screen is already open — both are "the operator pressed Pay twice".
    pub fn start_payment(&mut self) {
        if self.payment.is_some() || self.cart.is_empty() {
            return;
        }
        // Paying is a decision about the whole basket. A quantity still being
        // edited is not part of that decision yet.
        self.cart_quantity = None;
        // Taking money is its own permission, separate from filling a basket: a
        // shop can have someone who only ever puts the order together.
        if !self.permits(crate::auth::Permission::TakePayment) {
            return self.ask_override(crate::auth::Permission::TakePayment, None);
        }
        self.notice = None;
        self.payment = Some(Payment {
            tendered: String::new(),
            error: None,
            focus: true,
        });
    }

    /// Parse the cash received and write the sale.
    ///
    /// Everything is checked before the first write: a rejected tender leaves
    /// the cart exactly as it was, because a cashier who mistyped 500 instead of
    /// 5000 should retype one field, not rebuild the basket.
    pub fn confirm_payment(&mut self) {
        let Some(payment) = self.payment.as_ref() else {
            return;
        };

        let due_minor = self.due_minor();
        let tendered_text = payment.tendered.trim().to_owned();
        let tendered_minor = if tendered_text.is_empty() {
            due_minor
        } else {
            match Money::parse(&tendered_text, self.currency) {
                Ok(amount) => amount.minor(),
                Err(error) => return self.payment_error(error.to_string()),
            }
        };

        if tendered_minor < due_minor {
            return self.payment_error(format!(
                "That is {} short.",
                self.money(due_minor - tendered_minor)
            ));
        }

        let lines: Vec<SaleLine> = self
            .cart
            .iter()
            .map(|line| SaleLine {
                product_id: Some(line.product_id),
                name: line.name.clone(),
                qty: line.qty,
                unit_minor: line.unit_minor,
                unit_factor: line.unit_factor,
                unit_label: line.unit_label.clone(),
            })
            .collect();

        // The drawer the cash is about to land in. Opened here, on the way into
        // the first sale of the day, so nobody has to press "start shift"
        // before serving a customer.
        let shift_uid = self.ensure_shift();

        let terms = SaleTerms {
            rounding_minor: self.cart_rounding_minor(),
            tax_minor: self.cart_tax_minor(),
            tendered_minor,
            discount_minor: self.cart_discount_minor(),
            discount_bp: self.effective_discount_bp(),
            price_mode: match self.customer_mode {
                CustomerMode::Retail => "retail",
                CustomerMode::Wholesale => "wholesale",
            },
            customer_id: self.customer.as_ref().map(|customer| customer.id),
            customer_name: self
                .customer
                .as_ref()
                .map(|customer| customer.name.as_str()),
            user_id: self
                .session
                .as_ref()
                .map(|session| session.user_id.as_str()),
            shift_uid: shift_uid.as_deref(),
        };

        let written = crate::db::record_sale(&self.store, &lines, &terms);

        match written {
            Ok(_) => {
                self.cart.clear();
                // The sale is over, so its pricing terms are too: the next
                // customer is a walk-in until the till is told otherwise.
                self.customer_mode = CustomerMode::Retail;
                self.customer = None;
                // The sale is written, so the tender panel has done its job.
                // Leaving a change readout behind would make an empty cart look
                // like a cart that never cleared; the change is on the sale.
                self.payment = None;
                // Hand the keyboard back to the scanner: the next barcode is
                // the next customer's first item.
                self.scan_focus = true;
                // The sale wrote one negative movement per tracked line, in the
                // same transaction. Re-read the sums now so the Inventory list
                // agrees with what just happened.
                self.reload_stock();
                // The drawer gained this sale's cash; the card on Today must
                // say so without waiting for a restart.
                self.refresh_drawer();
            }
            Err(error) => {
                tracing::error!(error = %error, "recording a sale failed");
                self.payment_error(format!("Could not record the sale: {error}"));
            }
        }
    }

    /// Close the payment screen, sale or not. The cart survives a cancel: the
    /// operator aborted the payment, not the sale.
    pub fn close_payment(&mut self) {
        self.payment = None;
    }

    fn payment_error(&mut self, message: impl Into<String>) {
        if let Some(payment) = self.payment.as_mut() {
            payment.error = Some(message.into());
        }
    }
}
