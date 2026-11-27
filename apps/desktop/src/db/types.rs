//! The row types the rest of the app sees.
//!
//! These are the shapes a query returns and a screen draws: one struct per
//! thing the shop talks about. They live apart from the queries so a caller can
//! name a `Product` without pulling in the SQL that fills it.

pub struct Sale {
    pub id: i64,
    /// The number on the paper. `None` on a sale written before numbers
    /// carried a device; the list falls back to the row id, which is what that
    /// slip said.
    pub receipt_no: Option<String>,
    /// What the customer actually paid: the exact total plus cash rounding.
    pub paid_minor: i64,
    /// Rendered in the till's local time, minute precision (`01/11 09:15`):
    /// the stored value stays UTC and the query translates and trims. A
    /// cashier reading the list wants "when the sale happened" at a glance,
    /// not the database's epoch count.
    pub created_at: String,
    /// The account that took the money, by name.
    ///
    /// Read through the join, not copied onto the sale: renaming a person should
    /// rename their history. Sales written before the till had staff, and sales
    /// whose account was later removed, are `None`.
    pub cashier: Option<String>,
}

/// One sale as the CSV export writes it.
///
/// Separate from [`Sale`] because the two read different things: the table caps
/// at a hundred rows and trims the timestamp to fit a column, while a file is
/// meant to be complete and re-readable. Sharing one query would mean exporting
/// whatever the screen happened to show.
pub struct ExportSale {
    pub id: i64,
    /// The number on the paper the customer holds, so a slip quoted at the
    /// counter can be found in the file. Empty on sales written before numbers
    /// carried a device.
    pub receipt_no: Option<String>,
    /// Local time, whole seconds: a file is compared against another system's
    /// books, so the minute the screen shows is not precise enough.
    pub created_at: String,
    pub cashier: Option<String>,
    /// The customer's name, when one was attached to the sale.
    pub customer: Option<String>,
    /// What the customer was charged before cash rounding: the lines less the
    /// basket discount. The gross is `total_minor + discount_minor`.
    pub total_minor: i64,
    /// The basket discount taken off the lines.
    pub discount_minor: i64,
    /// `retail` or `wholesale`.
    pub price_mode: String,
    /// The cash adjustment, signed.
    pub rounding_minor: i64,
    /// `total_minor + rounding_minor`: what the customer paid.
    pub paid_minor: i64,
    /// One method, or several joined with `+` once a sale can be split.
    pub method: String,
}

/// One line of a sale, on its way into the database.
///
/// The name and the unit price are the ones on the receipt, not a lookup: a
/// sale records what it sold, and the catalog may have moved on since.
pub struct SaleLine {
    /// The catalog row, when there is still one. Reporting only.
    pub product_id: Option<i64>,
    pub name: String,
    pub qty: i64,
    pub unit_minor: i64,
    /// How many base units one line unit held at the moment of sale: `1` for a
    /// loose tablet, `pack_size` for a sealed pack. Stock moves by the product
    /// of this and `qty`.
    pub unit_factor: i64,
    /// What the line's unit was called, e.g. `tablet` or `pack of 100`. `None`
    /// keeps a receipt printed by an older build reading exactly as it did.
    pub unit_label: Option<String>,
}

/// One tender as the checkout collected it, on its way into `payments`.
///
/// `amount_minor` is what the tender settles; `tendered_minor` is what the
/// customer handed over. They differ only for cash that overpaid and produced
/// change: a 20,000 note against a 12,400 bill settles 12,400 and returns
/// 7,600. A mobile-money tender is exact, so the two are equal and change is 0.
pub struct PaymentLine<'a> {
    /// The stored key: `cash`, `mtn_momo`, `airtel_money`.
    pub method: &'a str,
    /// What this tender contributes towards the amount due.
    pub amount_minor: i64,
    /// What the customer handed over. Equals `amount_minor` unless this is a
    /// cash tender that overpaid.
    pub tendered_minor: i64,
    /// The phone-side transaction id, when the cashier recorded one.
    pub reference: Option<&'a str>,
}

/// The money and attribution of one sale, as the checkout collected it.
///
/// A struct rather than positional arguments because five of these are `i64`
/// amounts: a call site that swapped the discount for the tax would compile and
/// write a sale that reconciles with nothing.
pub struct SaleTerms<'a> {
    /// The cash adjustment, signed: `paid = net + rounding`.
    pub rounding_minor: i64,
    /// The tax collected, included in the net total.
    pub tax_minor: i64,
    /// Cash handed over, for a sale paid in one cash tender. Ignored when
    /// `payments` is non-empty, which is how the till now pays.
    pub tendered_minor: i64,
    /// Every tender, in the order it was taken. Empty means the legacy single
    /// cash sale: `tendered_minor` is then the whole of it.
    pub payments: &'a [PaymentLine<'a>],
    /// The basket discount taken off the lines.
    pub discount_minor: i64,
    /// The rate that produced it, in basis points.
    pub discount_bp: i64,
    /// When `true`, the sale was recorded "on credit": the customer takes the
    /// goods now and pays later. Zero tenders are allowed and `paid_minor`
    /// reflects whatever cash was actually received (which may be zero).
    pub is_on_credit: bool,
    /// `retail` or `wholesale`.
    pub price_mode: &'a str,
    /// The customer row, when one is still in the catalog.
    pub customer_id: Option<i64>,
    /// The customer's name as it was at the sale.
    pub customer_name: Option<&'a str>,
    /// The cashier the money is attributed to.
    pub user_id: Option<&'a str>,
    /// The drawer the cash went into.
    pub shift_uid: Option<&'a str>,
}

impl Default for SaleTerms<'_> {
    fn default() -> Self {
        Self {
            rounding_minor: 0,
            tax_minor: 0,
            tendered_minor: 0,
            payments: &[],
            discount_minor: 0,
            discount_bp: 0,
            is_on_credit: false,
            price_mode: "retail",
            customer_id: None,
            customer_name: None,
            user_id: None,
            shift_uid: None,
        }
    }
}

/// A sale after it is written.
///
/// The amounts are the ones the cashier reads out, so a caller does not have to
/// re-derive them from a cart that is now empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedSale {
    pub id: i64,
    /// The row's portable identity: what a peer uses to recognise a replay.
    pub uid: String,
    /// What the customer was charged before cash rounding: the line sum less
    /// the basket discount, tax-inclusive.
    pub total_minor: i64,
    /// The cash adjustment, signed: `paid = total + rounding`.
    pub rounding_minor: i64,
    /// The tax the sale collected, included in `total_minor`: `pre_tax =
    /// total - tax`. Zero when the till charges no tax.
    pub tax_minor: i64,
    /// The basket discount, in money: `gross = total + discount`.
    pub discount_minor: i64,
    /// The rate the discount was taken at, in basis points. `0` on a wholesale
    /// sale, where the ladder replaced the rate.
    pub discount_bp: i64,
    /// `retail` or `wholesale`.
    pub price_mode: String,
    /// The customer's name as it was at the sale, when one was attached.
    pub customer_name: Option<String>,
    pub tendered_minor: i64,
    pub change_minor: i64,
}

impl RecordedSale {
    /// What the customer paid, and what the drawer gained.
    #[cfg(test)]
    pub fn paid_minor(&self) -> i64 {
        self.total_minor + self.rounding_minor
    }
}

/// A drawer session: the stretch of time one count of the cash covers.
///
/// Not a rota and not a person. It opens by itself when the first sale of the
/// day is rung up and closes when somebody counts the drawer, so the till never
/// asks the operator to start work before serving a customer. Who took each sale
/// is already on `sales.user_id`; this knows only about money.
#[derive(Clone, Debug)]
pub struct Shift {
    /// Portable identity, carried on every sale the shift contains.
    pub uid: String,
    /// Local time, minute precision: the card reads "opened 08:14".
    pub opened_at: String,
    pub opened_by: Option<String>,
    /// The change that was in the drawer before the first sale.
    pub float_minor: i64,
    /// `None` while the drawer is still open.
    pub closed_at: Option<String>,
    pub counted_minor: Option<i64>,
}

impl Shift {
    pub fn is_open(&self) -> bool {
        self.closed_at.is_none()
    }
}

/// What one shift's cash adds up to.
#[derive(Clone, Debug, Default)]
pub struct DrawerTotals {
    /// Cash that went into the drawer: what customers paid, not what they
    /// handed over — the change left the drawer in the same breath.
    pub cash_minor: i64,
    pub sales_count: i64,
}

/// A shift and its cash, read together because the card never shows one
/// without the other.
#[derive(Clone, Debug)]
pub struct Drawer {
    pub shift: Shift,
    pub totals: DrawerTotals,
}

impl Drawer {
    /// What the drawer should hold: the float plus every cash sale.
    pub fn expected_minor(&self) -> i64 {
        self.shift
            .float_minor
            .saturating_add(self.totals.cash_minor)
    }

    /// Counted minus expected. Positive is a surplus, `None` until counted.
    pub fn difference_minor(&self) -> Option<i64> {
        self.shift
            .counted_minor
            .map(|counted| counted - self.expected_minor())
    }
}

/// One rung of a product's price ladder: from `min_qty` loose units upward, one
/// costs `price_minor`.
///
/// The shelf price is not a rung. It is the price of one, and it is the rung
/// below the first break, so [`min_qty`](Self::min_qty) is always two or more.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PriceTier {
    pub min_qty: i64,
    pub price_minor: i64,
}

/// One thing the shop sells, as the catalog stores it.
///
/// `price_minor` is in the till's currency, like every other amount: there is no
/// second unit and nothing to convert at checkout.
///
/// `category` is the denormalized name printed on receipts and reports — it is
/// copied at save time so a receipt never changes when a category is renamed.
/// `category_id` links to [`Category`] so the product form can offer a real
/// dropdown and renaming a category cascades to every product that uses it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Product {
    pub id: i64,
    pub name: String,
    /// Price of one base unit — the smallest thing that can leave the shelf.
    pub price_minor: i64,
    pub category: String,
    pub category_id: Option<i64>,
    /// Optional, and unique when present: two products claiming one barcode is
    /// a scanning bug rather than a preference.
    pub barcode: Option<String>,
    /// The shop's own code for the row, unique among live products when set.
    /// Unlike a barcode it is a reorder key: a CSV "update" file matches on it
    /// first, with the barcode used only for rows that have no SKU yet.
    pub sku: Option<String>,
    /// What one base unit is called: `tablet`, `bottle`, `box`. Shown on the
    /// receipt and on a loose cart line; `unit` when the shop has no better name.
    pub unit_label: String,
    /// How many base units one sealed pack holds. `1` means the product has no
    /// pack of its own and is sold only by the base unit.
    pub pack_size: i64,
    /// Price of one sealed pack. `None` when the shop does not sell packs, or
    /// has not priced one yet.
    pub pack_price_minor: Option<i64>,
    /// The barcode on the box. Separate from [`Self::barcode`], which is the
    /// code on the loose goods; scanning either resolves to this row, with the
    /// pack code buying a whole pack.
    pub pack_barcode: Option<String>,
    /// What one base unit costs, in thousandths of a minor unit. The pack's
    /// cost divided by its size rarely lands on a whole amount — a 1,850 pack
    /// of 100 is 18.5 a tablet — and rounding the stored cost would make every
    /// profit figure drift. `None` until the shop records a purchase.
    pub cost_milli: Option<i64>,
    /// Loose-unit price breaks, ascending by quantity. Empty when the shop has
    /// none, and always empty for a pack: the box price is already the bulk
    /// deal.
    pub tiers: Vec<PriceTier>,
}

/// A regular customer, and the rate the till takes off their basket.
///
/// Not a loyalty card: the shop knows the name and the terms it gives, and the
/// till needs both at the counter. `discount_bp` is basis points (`500` is
/// five percent) because a rate is not always a whole percent. A customer with
/// `0` is someone the shop tracks by name and buys at retail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Customer {
    pub id: i64,
    pub name: String,
    pub phone: Option<String>,
    /// The rate taken off the basket, in basis points. `0` is a customer with
    /// no discount.
    pub discount_bp: i64,
}

/// One of the shops or people the shop buys from.
///
/// The contact fields are optional because a till should not refuse a supplier
/// just because nobody wrote down an email. `name` is the only thing that has
/// to exist: it is what a purchase order would print.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Supplier {
    pub id: String,
    pub name: String,
    pub contact_name: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub address: Option<String>,
    pub notes: Option<String>,
    pub archived: bool,
}

/// A category the shop uses to group products.
///
/// Categories live in their own table so renaming one updates every product
/// that points at it, and so the product form can offer a real dropdown
/// instead of re-deriving the list from the catalog each frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Category {
    pub id: i64,
    pub name: String,
    pub sort_order: i64,
    pub color: Option<String>,
}

/// A shelf as the sidebars show it: its name, its place, and how many live
/// products sit on it.
///
/// The `categories` table owns the name and the order. A product whose text
/// names a shelf the table has never seen is still shown: a CSV import can
/// arrive with names no one has tapped into a form yet, and hiding those
/// shelves would make their products reachable only by search.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CategorySummary {
    pub name: String,
    pub sort_order: i64,
    pub count: usize,
}

/// The editable fields of a supplier, as the form holds them.
///
/// One struct rather than six string arguments: the insert and update paths
/// must agree on which column is which, and a caller that transposes phone and
/// email is a bug the compiler cannot see in a positional list.
///
/// The values are raw form text; [`insert_supplier`] and [`update_supplier`]
/// trim them and store a blank optional field as `NULL`.
pub struct SupplierFields<'a> {
    pub name: &'a str,
    pub contact_name: &'a str,
    pub phone: &'a str,
    pub email: &'a str,
    pub address: &'a str,
    pub notes: &'a str,
}

/// Why a movement happened.
///
/// The stored spelling is stable: it is in a shipped migration and in every row
/// already written. The label is the till's wording and may change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StockKind {
    /// The count taken when tracking started.
    Opening,
    /// Written by [`record_sale`], one row per sold line.
    Sale,
    /// Goods arriving from a supplier.
    Receive,
    /// Stock coming back onto the shelf.
    Return,
    /// Spoiled, broken, or otherwise unsellable.
    Waste,
    /// A correction after a physical count, positive or negative.
    Adjust,
}

impl StockKind {
    pub const ALL: [StockKind; 6] = [
        StockKind::Opening,
        StockKind::Sale,
        StockKind::Receive,
        StockKind::Return,
        StockKind::Waste,
        StockKind::Adjust,
    ];

    pub fn key(self) -> &'static str {
        match self {
            StockKind::Opening => "opening",
            StockKind::Sale => "sale",
            StockKind::Receive => "receive",
            StockKind::Return => "return",
            StockKind::Waste => "waste",
            StockKind::Adjust => "adjust",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            StockKind::Opening => "Opening count",
            StockKind::Sale => "Sale",
            StockKind::Receive => "Received",
            StockKind::Return => "Returned",
            StockKind::Waste => "Waste",
            StockKind::Adjust => "Adjustment",
        }
    }
}

/// One tracked product and what the ledger says is on the shelf.
///
/// `on_hand` is always `SUM(delta)`, never a stored count. Negative is allowed:
/// the till must keep selling when the shelf and the database disagree, and an
/// adjustment is how the shop says so afterwards.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StockItem {
    pub product_id: i64,
    pub name: String,
    pub category: String,
    pub on_hand: i64,
    pub low_stock: i64,
    /// What one base unit cost, in thousandths of a minor unit. Kept exact so a
    /// pack of 100 costing 1,850 does not lose the half-unit. `None` until a
    /// purchase has recorded a price.
    pub cost_milli: Option<i64>,
    /// How many base units one pack holds; the purchase form prefills its
    /// "units per pack" from here.
    pub pack_size: i64,
    /// What one base unit is called, so the stock screen can say "tablets".
    pub unit_label: String,
    /// How many the shop wants to keep. `None` means "twice the warning line",
    /// which is a sensible default for a shop that never sets one.
    pub target_stock: Option<i64>,
    /// Ordered but not yet received, across planned purchases, counted in base
    /// units. Suggestions subtract this so a trip in progress cannot be ordered
    /// twice.
    pub incoming: i64,
    /// Whether the product goes off, so every delivery must be dated.
    pub perishable: bool,
    /// The shop's usual life for a delivery, in days. Only a prefill for a
    /// repacked or undated delivery: a date printed on the pack always wins.
    pub shelf_life_days: Option<i64>,
}

impl StockItem {
    /// The per-base-unit cost rounded to a whole minor unit, for display and
    /// for a loose purchase line. The stored figure stays exact.
    pub fn unit_cost_minor(&self) -> Option<i64> {
        self.cost_milli.map(|cost| (cost + 500) / 1000)
    }

    /// The per-pack cost rounded to a whole minor unit. Prefills a purchase line
    /// priced by the pack; the shop's own invoice is the authority, this is only
    /// the last one it paid.
    pub fn pack_cost_minor(&self) -> Option<i64> {
        self.cost_milli
            .map(|cost| (cost * self.pack_size + 500) / 1000)
    }

    pub fn is_out(&self) -> bool {
        self.on_hand <= 0
    }

    pub fn is_low(&self) -> bool {
        !self.is_out() && self.on_hand <= self.low_stock
    }

    /// The level a restock aims for: the shop's own target, or twice the
    /// warning line. Never below the warning line itself.
    pub fn target(&self) -> i64 {
        self.target_stock
            .unwrap_or_else(|| self.low_stock.saturating_mul(2))
            .max(self.low_stock)
    }

    /// How many to buy to reach [`Self::target`], after what is already on the
    /// shelf and already on its way.
    pub fn suggested(&self) -> i64 {
        (self.target() - self.on_hand - self.incoming).max(0)
    }

    /// Whether this is worth putting on the next list: at or under the warning
    /// line, with nothing already planned to cover it.
    pub fn needs_restock(&self) -> bool {
        self.suggested() > 0 && self.on_hand + self.incoming <= self.low_stock
    }
}

/// One movement from any product, for the shop-wide history view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlobalStockMovement {
    pub id: i64,
    pub product_name: String,
    /// Signed: positive puts stock on the shelf, negative takes it off.
    pub delta: i64,
    pub kind: StockKind,
    pub note: Option<String>,
    pub created_at: String,
    pub user: Option<String>,
}

/// One line of a product's stock history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StockMovement {
    pub id: i64,
    /// Signed: positive puts stock on the shelf, negative takes it off.
    pub delta: i64,
    pub kind: StockKind,
    pub note: Option<String>,
    pub created_at: String,
    /// Who recorded it, when the movement names a staff account.
    pub user: Option<String>,
    /// The date on the pack this movement carried, when it was a delivery or a
    /// return. `None` for sales, adjustments and undated stock.
    pub expires_on: Option<String>,
}

/// Where a purchase is in the only lifecycle it has: a shopping list, then a
/// record of a trip.
///
/// There is no "ordered", "sent" or "part received" state. The shop buys in
/// person, so a list is a list until the goods are in hand, and the moment they
/// are in hand the trip is over. Receiving is one act, not a series of
/// deliveries to track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PurchaseStatus {
    Planned,
    Received,
}

impl PurchaseStatus {
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "planned" => Some(PurchaseStatus::Planned),
            "received" => Some(PurchaseStatus::Received),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PurchaseStatus::Planned => "To buy",
            PurchaseStatus::Received => "Received",
        }
    }
}

/// A restocking trip, as the list shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Purchase {
    pub id: i64,
    pub supplier_id: Option<String>,
    /// The supplier's current name, read through the join. `None` when the
    /// trip names no supplier, which is the common case at a market.
    pub supplier: Option<String>,
    pub status: PurchaseStatus,
    pub note: Option<String>,
    /// Local time, minute precision, like the sale list.
    pub created_at: String,
    /// Local time when the last line was received, if it is finished.
    pub received_at: Option<String>,
    pub line_count: i64,
    /// Sum of what was ordered, across every line.
    pub unit_count: i64,
    /// Sum of what has arrived so far.
    pub received_count: i64,
    /// What the trip is expected to cost, from lines that have a price.
    /// `None` when no line has one, rather than a misleading zero.
    pub cost_minor: Option<i64>,
}

impl Purchase {
    /// The count under the title: what came in on a received trip, what is on
    /// the list while it is still to buy.
    ///
    /// A trip that came home short says so — "6 of 10 bought" is the number the
    /// owner wants — but one that came home with extra is just "12 items".
    pub fn count_label(&self) -> String {
        let (shown, planned) = match self.status {
            PurchaseStatus::Planned => (self.unit_count, self.unit_count),
            PurchaseStatus::Received => (self.received_count, self.unit_count),
        };
        let items = if shown == 1 { "item" } else { "items" };
        if shown < planned {
            format!("{shown} of {planned} bought")
        } else {
            format!("{shown} {items}")
        }
    }
}

/// One line of a purchase.
///
/// `name` is the snapshot taken when the line was written, not a join: the
/// catalog may have been renamed or archived since, and the trip must keep
/// reading as what was bought.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PurchaseLine {
    pub id: i64,
    /// The catalog row, when it still exists. `None` after a product is deleted.
    pub product_id: Option<i64>,
    pub name: String,
    pub qty_ordered: i64,
    pub qty_received: i64,
    pub unit_cost_minor: Option<i64>,
    /// How many base units one ordered unit holds. The shop may buy a sealed
    /// box one week and loose tablets the next; the line remembers which.
    pub pack_size: i64,
    /// The date printed on the pack, when the delivery carried one. A plan may
    /// hold the same product twice with two dates.
    pub expires_on: Option<String>,
}

/// A purchase and its lines, for the editor and the receiving screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PurchaseDetail {
    pub purchase: Purchase,
    pub lines: Vec<PurchaseLine>,
}

/// One line on its way into a new or edited plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PurchaseLineInput {
    pub product_id: i64,
    /// How many ordered units — packs, when `pack_size` is above one.
    pub qty: i64,
    pub unit_cost_minor: Option<i64>,
    /// How many base units one ordered unit holds.
    pub pack_size: i64,
    /// The date printed on the pack, when one is known while planning. Optional
    /// here; required at receiving time for a perishable product.
    pub expires_on: Option<String>,
}

/// One line counted at receiving time.
///
/// `line_id` is `None` for something picked up that was not on the plan, which
/// the receive screen allows: a market trip always brings one extra.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceiveLine {
    pub line_id: Option<i64>,
    pub product_id: i64,
    /// How many ordered units arrived — packs, when `pack_size` is above one.
    pub qty: i64,
    pub unit_cost_minor: Option<i64>,
    /// How many base units one ordered unit holds. Snapshotted onto the line so
    /// a later change to the product's pack does not rewrite what was bought.
    pub pack_size: i64,
    /// The date printed on the pack. Required for a perishable product, even
    /// when it is in the past: the shop may be counting stock it already has.
    pub expires_on: Option<String>,
}

/// Revenue, cost and coverage for a range.
///
/// Only lines with a recorded cost contribute: counting an uncosted line's
/// revenue as pure profit would overstate the margin, and guessing its cost
/// would invent one. The counts tell the screen how much of the range it can
/// speak for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Margin {
    /// Item revenue at the shelf price, less each sale's discount. The
    /// discount is allocated across the sale's lines in proportion to their
    /// gross, so the profit figure is what the shop actually kept.
    pub revenue_minor: i64,
    /// Everything the range discounted, retail and wholesale alike.
    pub discount_minor: i64,
    pub cost_minor: i64,
    pub costed_lines: i64,
    pub total_lines: i64,
}

impl Margin {
    pub fn profit_minor(&self) -> i64 {
        self.revenue_minor - self.cost_minor
    }

    pub fn has_cost(&self) -> bool {
        self.costed_lines > 0
    }

    pub fn is_complete(&self) -> bool {
        self.total_lines > 0 && self.costed_lines == self.total_lines
    }
}

/// Someone who works the till.
///
/// The credential is carried around because the lock screen needs to check it;
/// nothing renders it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct User {
    pub id: String,
    pub name: String,
    pub role: crate::auth::Role,
    pub credential: String,
    pub kind: crate::auth::CredentialKind,
    pub active: bool,
}

/// A peer this till pulls from or a phone this till remembers.
///
/// The token is the peer's own pairing code. It is stored as the canonical ten
/// characters so the client can pass it to `lan-http` without normalising on
/// every request.
#[derive(Clone, Debug)]
pub struct SyncPeer {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub token: String,
    pub last_pulled_seq: i64,
    pub last_catalog_version: i64,
    pub last_seen_at: Option<String>,
    pub last_error: Option<String>,
}

impl SyncPeer {
    /// What the peer list prints as the address, or why it cannot be dialled.
    pub fn address(&self) -> String {
        match (&self.host, self.port) {
            (Some(host), Some(port)) => format!("{host}:{port}"),
            _ => "No address".to_owned(),
        }
    }

    pub fn display_name(&self) -> &str {
        let name = self.name.trim();
        if name.is_empty() { &self.id } else { name }
    }
}

/// One dated slice of a product's shelf, after FEFO has consumed what was sold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpiryBucket {
    pub product_id: i64,
    pub name: String,
    pub unit_label: String,
    /// `None` for stock received without a date, which is consumed last.
    pub expires_on: Option<String>,
    pub on_hand: i64,
    /// Days from today; negative is already past. `None` for undated stock.
    pub days_left: Option<i64>,
}

impl ExpiryBucket {
    pub fn is_expired(&self) -> bool {
        matches!(self.days_left, Some(days) if days < 0)
    }

    pub fn expires_within(&self, days: i64) -> bool {
        matches!(self.days_left, Some(left) if left <= days)
    }
}

/// The hour that rang the most sales, and how many.
///
/// Ranked by sale count, because "busiest" on a till means the queue, not the
/// takings; revenue breaks a tie so the answer is stable when two hours drew
/// the same number of customers.
pub struct PeakHour {
    /// `0`–`23`, terminal clock.
    pub hour: u8,
    pub sales: i64,
}

/// One line of a sale, as recorded at the time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetailLine {
    /// The name on the receipt. The catalog may have moved on since.
    pub name: String,
    /// Units sold. Always at least one; respected rather than implied.
    pub qty: i64,
    /// Per-unit price in minor units, as charged.
    pub unit_minor: i64,
    /// `qty * unit` as recorded, not recomputed: the sale is the record of
    /// what was charged, and recomputing would only invite disagreement.
    pub total_minor: i64,
    /// How many base units one sold unit held: `1` loose, `pack_size` sealed.
    pub unit_factor: i64,
    /// The label on the line at the time, when it was anything but `unit`.
    pub unit_label: Option<String>,
}

/// One tender as the sale detail reads it back: what was handed over, under
/// which method, and what came back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaidBy {
    /// The stored key: `cash`, `mtn_momo`, `airtel_money`.
    pub method: String,
    /// What this tender contributed towards the amount due.
    pub amount_minor: i64,
    /// What the customer handed over.
    pub tendered_minor: i64,
    /// What came back from this tender. Only cash can produce change.
    pub change_minor: i64,
    /// The phone-side transaction id, when one was recorded.
    pub reference: Option<String>,
}

/// The whole record of one sale: its lines, its money, and who took it.
///
/// The list answers "how much came in"; this answers "what exactly was for
/// that" — the receipt a customer might come back about, or the discrepancy an
/// owner is looking for at the end of a shift.
///
/// Problems `IO`: a tender is recorded, never verified — the cashier confirms
/// the customer's phone before adding a mobile-money row, and the till cannot
/// know whether the network agreed. Payments are summed in Rust (not `SUM` in
/// SQL) so a split sale shows each method and its amount honestly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaleDetail {
    pub id: i64,
    /// The number on the paper: `<device>-<day>-<seq>`, or `None` on a sale
    /// written before numbers carried a device. The receipt prints the row id
    /// in that case, because that is what the slip in the customer's hand says.
    pub receipt_no: Option<String>,
    pub cashier: Option<String>,
    /// Local time, like the list shows: the stored value stays UTC.
    pub created_at: String,
    /// The pre-rounding amount the customer was charged: the line sum less the
    /// basket discount, tax-inclusive.
    pub total_minor: i64,
    /// The basket discount taken off the lines, in money. `0` on a sale rung
    /// up before discounts, and on a wholesale sale.
    pub discount_minor: i64,
    /// The rate the discount was taken at, in basis points.
    pub discount_bp: i64,
    /// `retail` or `wholesale`.
    pub price_mode: String,
    /// The customer on the sale, as named at the time.
    pub customer_name: Option<String>,
    /// The customer row, when one was attached at the time of sale.
    pub customer_id: Option<i64>,
    /// Whether this sale was sold on credit: goods taken now, paid later.
    pub is_on_credit: bool,
    /// Cash rounding on top of `total` — `0` most of the time, and what the
    /// customer paid above the exact total otherwise.
    pub rounding_minor: i64,
    /// `total + rounding`: the full amount charged for this sale.
    pub paid_minor: i64,
    /// The tax the sale collected, included in `total_minor`. `0` when the
    /// till charges no tax.
    pub tax_minor: i64,
    /// " + "-joined payment methods, one per distinct method. One sale; one
    /// day, "cash + mobile".
    pub method: String,
    /// Every tender in the order it was taken, so a receipt can print each
    /// method and its own amount rather than one combined figure.
    pub payments: Vec<PaidBy>,
    /// What was handed over, summed across payments.
    pub tendered_minor: i64,
    /// What went back, summed across payments.
    pub change_minor: i64,
    /// What has actually been received against this sale so far: the sum of
    /// `payments.amount_minor`. On a settled sale this equals `paid_minor`;
    /// on a credit sale it can be less (or zero).
    pub settled_minor: i64,
    /// What is still owed: `paid_minor - settled_minor`, floored at zero.
    /// Non-zero only while a credit sale is unsettled.
    pub balance_minor: i64,
    pub lines: Vec<DetailLine>,
}

/// A payment taken later against a credit sale.
///
/// It carries its own drawer and cashier: the customer may come back on a
/// different day, to a different till, and that money belongs to the shift
/// that took it rather than the one that rang up the sale.
pub struct CreditPayment<'a> {
    pub sale_id: i64,
    pub method: &'a str,
    /// What this payment contributes to the balance.
    pub amount_minor: i64,
    /// What the customer handed over. Cash may overpay and give change.
    pub tendered_minor: i64,
    pub reference: Option<&'a str>,
    pub shift_uid: Option<&'a str>,
    pub user_id: Option<&'a str>,
}

/// One credit sale still owed money, shown when a customer comes back to pay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreditSale {
    pub id: i64,
    pub receipt_no: Option<String>,
    /// Local time, like the sales list shows.
    pub created_at: String,
    /// The full amount charged.
    pub total_minor: i64,
    /// What has been received so far.
    pub settled_minor: i64,
    /// What remains: `total_minor - settled_minor`.
    pub balance_minor: i64,
}

/// One local day's totals — the trend behind the range.
///
/// Revenue is `total + rounding`, the drawer figure `totals` and the list
/// already agree on. Days are newest first. Fixed ranges carry every day in the
/// window, zero days included, so the bars are a calendar; all time carries the
/// thirty most recent days that sold, because it has no calendar to draw.
pub struct DayTotal {
    /// `dd/mm`, terminal clock — SQLite's `strftime` has no month names.
    /// Grouped on the full date, so two sales on the same day share one row.
    pub day: String,
    /// `0`–`6`, Sunday first, as `strftime('%w')` numbers them. Carried beside
    /// the date because a week reads better as names, and only the calendar
    /// knows which name a date wears.
    pub weekday: u8,
    pub sales: i64,
    pub revenue: i64,
}

/// The range's bestselling lines, ranked by revenue.
///
/// Ranked by the recorded line total, before rounding: rounding belongs to
/// the sale as a whole, not to any one item. Grouped by the name on the line,
/// not by product id — a deleted product's receipts keep their history, and
/// there is no join to break when the catalog changes.
pub struct ItemTotal {
    pub name: String,
    pub qty: i64,
    pub revenue: i64,
}

// ---------------------------------------------------------------------------
// Payments taken
// ---------------------------------------------------------------------------

/// What one payment method brought in over a range.
pub struct MethodTotal {
    /// `cash` today; the column exists so card and mobile money need no change
    /// here when they arrive.
    pub method: String,
    /// Sales that used the method, not payments: one sale may be split across
    /// two methods, and counting payment rows would report it twice.
    pub sales: i64,
    /// What stayed in the drawer, rounding included. Summing `tendered_minor`
    /// would count the change that went back out.
    pub amount_minor: i64,
}

// ---------------------------------------------------------------------------
// By person
// ---------------------------------------------------------------------------

/// What one cashier rang up, over a range or over one drawer.
pub struct PersonTotal {
    /// The account's name, or `No cashier` for a sale written before anyone was
    /// signed in. Never empty: a blank cell in a money table reads as a bug.
    pub name: String,
    pub sales: i64,
    /// Cash that went into the drawer, rounding included.
    pub cash_minor: i64,
    /// Everything the person took, however it was paid.
    pub takings_minor: i64,
}
