//! Everything the POS screen tree can change, in one place.
//!
//! One struct, passed as `&mut` to whichever module is drawing. Modules keep no
//! state of their own: switching module must not lose the cart, the search box,
//! or the scroll position, and a module that owns data makes that rule hard to
//! keep. It also means a command in the palette can change anything the user can
//! change, without reaching into widgets.

mod prefs;

pub use prefs::*;

mod day;

mod sync;

pub use sync::*;

mod staff;

pub use staff::*;

mod purchasing;

pub use purchasing::*;

mod suppliers;

pub use suppliers::*;

mod catalog;

pub use catalog::*;

mod cart;

pub use cart::*;

mod credit;

mod customers;

pub use customers::*;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use money::{Currency, Money};
use ui_kit::theme::Theme;
use ui_kit::tokens::Rgb;
use ui_shell::egui;

use crate::db::{
    CategorySummary, Customer, PaymentLine, PriceTier, Product, SaleLine, SaleTerms, StockItem,
    Supplier,
};
use crate::scan::{ScanAction, ScanEvent, ScanStatus};
use crate::settings::{ReceiptDetails, Settings};

/// Category names offered in the product form even when the catalog is empty.
///
/// They are a sketch of a grocery until, not a schema: suggestions the owner
/// can tap instead of typing "Drniks" twice. Nothing is written to the
/// database until a product actually uses one, so a shop that names its own
/// shelves simply outgrows the list — and a till whose catalog is untouched
/// still filters on nothing, exactly as before.
#[allow(dead_code)]
pub const STARTER_CATEGORIES: &[&str] = &[
    "Grains, Rice & Flours",
    "Spices, Masalas & Seasonings",
    "Tea, Coffee & Cocoa",
    "Pasta, Oils & Sauces",
    "Fresh Milk & Yogurt",
    "Bakery & Breads",
    "Fresh Produce",
    "Soft Drinks & Water",
    "Snacks & Sweets",
    "Laundry & Cleaning",
    "Personal Hygiene",
    "Loose / Weighed Items",
    "Services & Digital",
];

/// Suggested names for the product form's category field: what the catalog
/// already uses first, then the starter list. Case is ignored when
/// de-duplicating, because "Drinks" and "drinks" are meant to be one category
/// and must not advertise themselves as two. `current` is left out — a
/// suggestion for the name already in the field is noise.
#[allow(dead_code)]
pub fn category_suggestions(categories: &[String], current: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let current = current.trim();

    for source in categories
        .iter()
        .map(String::as_str)
        .chain(STARTER_CATEGORIES.iter().copied())
    {
        if names.iter().any(|name| name.eq_ignore_ascii_case(source)) {
            continue;
        }
        if source.eq_ignore_ascii_case(current) {
            continue;
        }
        names.push(source.to_owned());
    }

    names
}

/// One tender already taken in the payment panel.
///
/// `amount_minor` is what it contributes to the amount due; `tendered_minor` is
/// what the customer handed over. They differ only when cash overpays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tender {
    /// The stored key: `cash`, `mtn_momo`, `airtel_money`.
    pub method: String,
    pub amount_minor: i64,
    pub tendered_minor: i64,
    /// The phone-side transaction id, when the cashier recorded one.
    pub reference: Option<String>,
}

/// The payment panel, open in the cart footer.
///
/// Cash received is text, for the same reason a price is: it is mid-edit, and
/// parsing on every keystroke would eat a half-typed `5000`. Empty means "the
/// whole balance", which is what most sales are.
pub struct Payment {
    /// Tenders taken so far, in the order they were taken. Empty until the
    /// first Add; the sale is written when the sum reaches the amount due.
    pub tenders: Vec<Tender>,
    /// The method the next tender will use. `cash` unless the cashier changed
    /// it.
    pub method: String,
    pub tendered: String,
    /// The phone-side transaction id being typed for a non-cash tender.
    pub reference: String,
    pub error: Option<String>,
    /// Put the cursor in the amount field on the first frame.
    pub focus: bool,
    /// When `true`, the sale is recorded "on credit": zero tenders are allowed
    /// and `paid_minor` reflects whatever cash was actually received (may be zero).
    /// The outstanding balance is tracked as `total_minor - paid_minor`.
    pub is_on_credit: bool,
}

/// The tenders the till can record, in the order the panel offers them.
///
/// Mobile money is recorded, not processed: the cashier checks the customer's
/// phone for the confirmation and then adds the row. The till has no way to
/// know whether the network agreed, and the panel says so on screen.
pub const PAYMENT_METHODS: &[(&str, &str)] = &[
    ("cash", "Cash"),
    ("mtn_momo", "MTN MoMo"),
    ("airtel_money", "Airtel Money"),
];

/// The human name of a stored method key.
///
/// Unknown keys are title-cased with underscores turned into spaces, so a
/// method synced from a newer peer still reads as words rather than a key.
pub fn payment_method_label(method: &str) -> String {
    if let Some((_, label)) = PAYMENT_METHODS
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(method))
    {
        return (*label).to_owned();
    }
    let mut words = method.split('_').filter(|word| !word.is_empty());
    let mut label = String::new();
    if let Some(first) = words.next() {
        let mut chars = first.chars();
        if let Some(ch) = chars.next() {
            label.extend(ch.to_uppercase());
            label.push_str(chars.as_str());
        }
        for word in words {
            label.push(' ');
            label.push_str(word);
        }
    }
    if label.is_empty() {
        method.to_owned()
    } else {
        label
    }
}

/// How long an approval lasts. Long enough to finish the action that asked for
/// it, short enough that it cannot be handed to the next person.
const ELEVATION: Duration = Duration::from_secs(60);

pub struct State {
    pub store: storage::Store,

    pub cart: Vec<CartLine>,
    /// Parked baskets held at the counter while another customer is served.
    pub parked_carts: Vec<ParkedCart>,
    /// Which price ladder the cart is selling on. A session choice, not a sale
    /// fact: the lines keep the unit price they were charged at.
    pub customer_mode: CustomerMode,
    /// The regular customer this basket belongs to, when the cashier picked
    /// one. Their rate is what `effective_discount_bp` reads.
    pub customer: Option<Customer>,
    /// The regular customers the shop keeps, by name. Read with the catalog and
    /// re-read after a write, like the product list.
    pub customers: Vec<Customer>,
    /// The customer form, when it is open.
    pub customer_editor: Option<CustomerDraft>,
    /// The cart line whose quantity is being set, if any. One editor at a time:
    /// the modal owns the screen while it is open.
    pub cart_quantity: Option<CartQuantity>,
    /// The compact cart row showing its detail, keyed by product and pack size.
    /// View state only: it is never persisted and never travels with a sale.
    pub cart_expanded: Option<(i64, i64)>,

    /// Index into [`crate::modules::all`]. Also the activity-bar selection.
    pub module: usize,
    /// Per-module sidebar visibility, so collapsing Sell does not collapse
    /// Settings. Indexed in parallel with the module list.
    pub sidebar_open: Vec<bool>,
    /// How the Sell screen lays out its products. The toggle sits beside the
    /// scan field, because that is where the cashier already looks.
    pub sell_view: SellView,
    /// Set when something persisted changed; the shell writes `prefs.json` and
    /// clears it.
    pub prefs_dirty: bool,

    // Per-module view state. Kept here rather than in the module structs for the
    // reason in the module doc: switching away and back should not reset them.
    /// The till's scan or search field.
    ///
    /// Separate from [`Self::catalog_search`] on purpose: they filter two
    /// different lists, and a cashier typing into the till must not silently
    /// filter the Products list — or the export built from it.
    pub sell_search: String,
    /// The Products list's query.
    pub catalog_search: String,
    /// How the Products list is ordered: catalog order, lowest stock first,
    /// highest stock first, then A–Z. A view choice, never written to a product.
    pub catalog_sort: usize,
    /// Set when the selling screen opens, so the next frame can put the cursor in
    /// the scan field. A scanner types into whatever has focus; a till that needs
    /// one click before the first barcode is a till that "doesn't scan".
    pub scan_focus: bool,
    /// A phone scan asked the product panel to submit the next frame. The search
    /// field is already filled with the barcode; this is the Enter key the phone
    /// cannot press, and it is consumed by the panel that draws Sell.
    pub scan_submit: bool,
    /// What the LAN listener is doing, mirrored from the shell so Settings can
    /// show it without the screen owning the worker thread.
    pub scanner_status: ScanStatus,
    /// What the sync worker last reported, for the Sync card and status bar.
    pub sync_status: crate::sync::SyncStatus,
    /// The tills this one pulls from, re-read after a pass so a peer's error
    /// and last-seen time are current.
    pub sync_peers: Vec<crate::db::SyncPeer>,
    /// The "pair another till" fields, committed only by Add.
    pub sync_peer_draft: SyncPeerDraft,
    /// Why pairing a till failed. Shown in the card, cleared by the next try.
    pub sync_peer_error: Option<String>,
    /// The Sync card asked for a pass now. Consumed by the shell.
    pub sync_now_requested: bool,
    /// The device name as typed. Committed by [`State::save_sync_device_name`]
    /// so a half-typed name is not what peers see.
    pub sync_name_draft: String,
    /// Whether the host firewall will let a phone reach the port that is
    /// listening. Mirrored from the shell for the same reason as the status.
    pub firewall: crate::firewall::Snapshot,
    /// A privileged step is waiting on the polkit dialog.
    pub firewall_busy: bool,
    /// Why the last attempt to open the port failed.
    pub firewall_error: Option<String>,
    /// The Settings card asked the shell to open the port. Consumed by the shell.
    pub firewall_allow_requested: bool,
    /// The category both Sell and Products filter by. `None` is "All"; the value
    /// is a category name, because the categories are the shop's, not an index
    /// into a list the app invented.
    pub category: Option<String>,
    pub range: usize,
    /// Trends has its own window, because one day is not a trend. Index into
    /// [`TREND_RANGES`], which starts where `RANGES` stops being a single day.
    pub trend_range: usize,
    pub settings_section: usize,

    /// The catalog, read once and re-read after every write. Filtering happens
    /// in memory: a few hundred products are cheaper to filter than to query
    /// once per frame, and the grid redraws every frame.
    pub products: Vec<Product>,
    /// Products the shop removed, kept for the archive view and restore.
    pub archived: Vec<Product>,
    /// Whether the Catalog sidebar is showing the archive instead of the
    /// shelves. A view, not a filter: an archived row has no stock to facet.
    pub show_archived: bool,
    /// The shelves the sidebars show, in the shop's order, with live counts.
    /// The first of the two category views in the database: this one is ordered
    /// and counted for drawing, `categories_list` is the raw form/sync view.
    pub categories: Vec<CategorySummary>,
    /// The product form, when it is open.
    pub product_editor: Option<ProductDraft>,
    /// The warning-line form for selected products, when it is open.
    pub warning_editor: Option<WarningDraft>,
    /// The CSV import modal, when it is open. The typed-path fallback for a
    /// file whose location is already known.
    pub import: Option<ImportDraft>,
    /// Which import the next file chooser answer is for. Kept here rather than
    /// in the picker's purpose so the system dialog and the in-app picker ask
    /// the same question without either learning about catalog semantics.
    pub import_mode: ImportMode,
    /// The file chooser, when an import or an export is choosing a location.
    /// Only one picker exists: closing it returns to whichever screen asked.
    /// The system's chooser runs outside the app (`dialog.rs`) and only lands
    /// here when the computer has none to show.
    pub picker: Option<crate::picker::Picker>,
    /// The computer's own file chooser, while one is open.
    pub dialog: crate::dialog::Chooser,
    /// The folder the last import came from, so the next one starts there for
    /// the rest of this session.
    pub import_dir: Option<PathBuf>,
    /// The folder the last export went to, for the same reason.
    pub export_dir: Option<PathBuf>,
    /// A product the operator asked to delete, waiting for confirmation. Inline
    /// in the row rather than a modal: the row is where the mistake would be.
    pub product_delete: Option<i64>,
    /// Products ticked in the catalog for a bulk action. Transient: changing
    /// module or running the action clears it, so a later visit starts clean.
    pub selected_products: Vec<i64>,

    /// The supplier directory, read once and re-read after every write. Like the
    /// catalog it is small enough to filter in memory.
    pub suppliers: Vec<Supplier>,
    /// Search across name, contact and phone.
    pub supplier_search: String,
    /// Whether archived suppliers stay in the list. Off by default: a supplier
    /// nobody uses any more is not what the next order should start from.
    pub show_archived_suppliers: bool,
    /// The supplier form, when it is open.
    pub supplier_editor: Option<SupplierDraft>,
    /// A supplier waiting for an archive/restore confirmation, by id.
    pub supplier_archive: Option<String>,

    /// Tracked stock, by product, with the ledger's sum. Re-read after every
    /// sale and every movement: a cached count would be the thing that
    /// disagrees with the ledger.
    pub stock: Vec<StockItem>,
    /// `0` all, `1` low, `2` out, `3` expiring. The filter a shopkeeper uses
    /// standing in front of the shelf, not a report.
    pub stock_filter: usize,
    /// What is left on every dated slice of a perishable product's stock,
    /// earliest date first. Derived from the ledger by FEFO, never stored.
    pub expiring: Vec<crate::db::ExpiryBucket>,
    /// The "start tracking" form, when it is open.
    pub stock_editor: Option<TrackDraft>,
    /// A receive/return/waste/adjust form, when it is open.
    pub stock_move: Option<StockMoveDraft>,
    /// One product's movements, read when the history modal opens.
    pub stock_history: Option<StockHistory>,

    /// Restocking trips, newest first. A purchase is a plan or a received
    /// record; it never edits stock except through the receiving screen.
    pub purchases: Vec<crate::db::Purchase>,
    /// Search across the supplier and note. Product names on the lines are not
    /// searched yet.
    pub purchase_search: String,
    /// `0` all, `1` to buy, `2` received.
    pub purchase_filter: usize,
    /// The list being created or edited, open as the Restock page.
    pub purchase_editor: Option<PurchaseDraft>,
    /// A received trip being read.
    pub purchase_view: Option<crate::db::PurchaseDetail>,

    /// The payment screen, when it is open.
    pub payment: Option<Payment>,

    // ---- settling a credit sale --------------------------------------------
    /// Outstanding credit sales for the customer currently at the till.
    pub credit_sales: Vec<crate::db::CreditSale>,
    /// Which of those sales the operator is paying off right now.
    pub credit_sale_id: Option<i64>,
    /// The amount being typed for a later payment.
    pub credit_tendered: String,
    /// The method being used for a later payment.
    pub credit_method: String,
    /// Phone-side transaction id, when the method is not cash.
    pub credit_reference: String,
    /// Why the last later payment failed.
    pub credit_error: Option<String>,
    /// Whether the settle panel is open.
    pub credit_panel: bool,

    /// The drawer this till is accountable for: the open shift, or one counted
    /// earlier today. `None` before the first sale of the day, which is also
    /// when there is nothing to count.
    pub drawer: Option<crate::db::Drawer>,
    /// The cash count as typed at close. Committed by [`State::close_day`], so
    /// a half-typed figure is never compared against the takings.
    pub drawer_count: String,
    /// Why the last close failed. Shown in the card rather than a dialog, for
    /// the same reason a rejected tender stays in the cart.
    pub drawer_error: Option<String>,
    /// The change float as typed in Settings. Committed by
    /// [`State::save_cash_float`].
    pub float_draft: String,

    /// The sale the operator tapped in a list, read in full for the detail
    /// modal. `None` while every list is just a list.
    pub sale_open: Option<crate::db::SaleDetail>,

    /// One-line feedback for an action that has no flow behind it yet. Cleared
    /// by the next action, so it never turns into a permanent lie in the UI.
    pub notice: Option<String>,
    /// Feedback from an action that produced a file: a backup, an export. Kept
    /// apart from `notice`, which only renders under the cart, so a message from
    /// Settings or a modal would otherwise be invisible. Each screen that sets
    /// it also renders it; navigation clears it.
    pub file_notice: Option<String>,
    /// The file the last [`State::file_notice`] is about, so the screen can
    /// offer the path for copying without parsing it back out of a sentence.
    pub file_path: Option<PathBuf>,
    /// Result of the one-click "apply suggested categories" action, shown in
    /// the business-type card. It reports a write the owner asked for, never
    /// one the app did on its own.
    pub category_seed_notice: Option<String>,
    /// A backup asked for but not yet written. Inline rather than a modal, for
    /// the same reason deleting a product asks in its row: the card is where
    /// the button was.
    pub backup_confirm: bool,
    /// Why the last automatic backup did not happen, if it did not.
    ///
    /// Kept for the Backup card rather than shown as a toast: a shop whose
    /// copies have been failing for a month needs to know that on the screen
    /// where it can be fixed, not in a line that vanished with the sale behind
    /// it. Cleared by the next copy that works.
    pub auto_backup_error: Option<String>,

    /// What the Settings screen is editing. Committed by [`State::save_shop_name`],
    /// so a half-typed name is never written to disk.
    pub shop_draft: String,
    /// The receipt fields as typed. Committed together by
    /// [`State::save_receipt_details`], for the same reason as the shop name.
    pub receipt_draft: ReceiptDetails,
    /// The tax rate as typed. Committed by [`State::save_tax_rate`] so a
    /// half-typed percentage is never written to disk.
    pub tax_draft: String,
    /// The business type as typed. Committed by [`State::save_business_type`]
    /// so a half-chosen value is never written to disk.
    pub business_draft: String,
    /// Whether the operator chose "Other" in the business-type picker and is
    /// typing a custom value instead of one of the seeded types.
    pub business_showing_other: bool,

    /// The shop's configuration, loaded once at startup and written back when
    /// something in Settings changes.
    pub settings: Settings,
    /// Where `settings.json` and the database live, so the shell can persist a
    /// change and the owner screens can write backups without knowing about
    /// paths.
    pub paths: Paths,
    /// The theme currently installed on the context.
    pub theme: Theme,
    /// The brand accent installed over that theme, or `None` for the palette's
    /// built-in accent.
    pub brand_accent: Option<Rgb>,
    /// Set when the theme changed and the context still needs reinstalling.
    /// The shell owns `install`, so the screen only records the intent.
    pub theme_dirty: bool,
    /// Set when `settings` changed and needs writing.
    pub settings_dirty: bool,

    /// The one currency this till prices in. Stored in the database, not in
    /// `settings.json`: it belongs with the amounts it names.
    pub currency: Currency,
    /// No currency has been stored yet: the picker is modal and the till is not
    /// usable until the operator chooses. Defaults to UGX for the first frame.
    pub needs_currency: bool,
    /// Settings asked to change the currency while the till is still empty.
    pub currency_picker: bool,
    /// Why the last currency write failed, shown inside the picker.
    pub currency_error: Option<String>,

    // ---- who is at the till -------------------------------------------------
    /// Who is signed in. `None` means the lock screen is up.
    pub session: Option<Session>,
    /// Every account, re-read after each write. Kept in memory because the lock
    /// screen needs the list before anyone is signed in, and because it is a
    /// handful of rows.
    pub users: Vec<crate::db::User>,
    /// The lock screen's fields.
    pub sign_in: SignIn,
    /// First-run owner setup.
    pub setup: Setup,
    /// The forgotten-password flow, when it is open.
    pub recovery: Option<Recovery>,
    /// An action waiting for a manager's approval.
    pub override_request: Option<Override>,
    /// Until this instant, the person signed in may do what a manager approved.
    elevated_until: Option<Instant>,
    /// Last input seen, for the idle lock.
    last_activity: Instant,
    /// The "add a person" form in Settings.
    pub staff: StaffDraft,
    /// A generated PIN the owner has not written down yet.
    pub new_pin: Option<NewPin>,
    /// The PIN being reset for one account.
    pub staff_reset: Option<ResetPin>,
    /// The owner changing their own password.
    pub owner_password: PasswordDraft,
}

impl State {
    pub fn new(
        store: storage::Store,
        module: usize,
        sidebar_open: Vec<bool>,
        mut settings: Settings,
        paths: Paths,
        currency: Currency,
        needs_currency: bool,
    ) -> Self {
        let theme = settings.theme();
        let brand_accent = settings.brand_accent();
        // Startup installs the static palette before the first frame. If a
        // brand accent was saved, reinstall on that first frame so the very
        // first paint is the one the shop chose.
        let theme_dirty = brand_accent.is_some();
        let shop_draft = settings.shop_name.trim().to_owned();
        let receipt_draft = settings.receipt.clone();
        let tax_draft = settings.tax_rate_percent.to_string();
        let float_draft = plain_amount(settings.cash_float_minor, currency);
        let business_draft = settings.business_type.clone().unwrap_or_default();
        // A till upgraded from a build without the scanner has no token yet. A
        // token is not a pairing decision, so generate it on boot and let the
        // settings write below persist it; enabling the listener stays manual.
        // Both secrets are filled here so the listener can start with whichever
        // feature the shop turns on first.
        let scanner_token_created = settings.scanner.ensure_token();
        let sync_token_created = settings.sync.ensure_token();
        let sync_name_draft = settings.sync.device_name.trim().to_owned();
        let sync_peers = crate::db::sync_peers(&store).unwrap_or_default();
        let products = crate::db::products(&store);
        let archived = crate::db::archived_products(&store);
        let categories = crate::db::categories(&store);
        let suppliers = crate::db::suppliers(&store);
        let stock = crate::db::stock(&store);
        let expiring = crate::db::expiring(&store);
        let purchases = crate::db::purchases(&store);
        let users = crate::db::users(&store);
        let drawer = crate::db::drawer(&store);
        let customers = crate::db::customers(&store);
        Self {
            store,
            cart: Vec::new(),
            parked_carts: Vec::new(),
            customer_mode: CustomerMode::Retail,
            customer: None,
            customers,
            customer_editor: None,
            cart_quantity: None,
            cart_expanded: None,
            module,
            sidebar_open,
            sell_view: SellView::default(),
            prefs_dirty: false,
            sell_search: String::new(),
            catalog_search: String::new(),
            catalog_sort: 0,
            scan_focus: true,
            scan_submit: false,
            scanner_status: ScanStatus::Off,
            sync_status: crate::sync::SyncStatus::default(),
            sync_peers,
            sync_peer_draft: SyncPeerDraft::default(),
            sync_peer_error: None,
            sync_now_requested: false,
            sync_name_draft,
            firewall: crate::firewall::Snapshot::Unprobed,
            firewall_busy: false,
            firewall_error: None,
            firewall_allow_requested: false,
            category: None,
            range: 0,
            // A month is the shortest window a shopkeeper calls a trend.
            trend_range: 1,
            settings_section: 0,
            products,
            archived,
            show_archived: false,
            categories,
            product_editor: None,
            warning_editor: None,
            import: None,
            import_mode: ImportMode::Add,
            picker: None,
            dialog: crate::dialog::Chooser::new(),
            import_dir: None,
            export_dir: None,
            product_delete: None,
            selected_products: Vec::new(),
            suppliers,
            supplier_search: String::new(),
            show_archived_suppliers: false,
            supplier_editor: None,
            supplier_archive: None,
            stock,
            stock_filter: 0,
            expiring,
            stock_editor: None,
            stock_move: None,
            stock_history: None,
            purchases,
            purchase_search: String::new(),
            purchase_filter: 0,
            purchase_editor: None,
            purchase_view: None,
            payment: None,
            credit_sales: Vec::new(),
            credit_sale_id: None,
            credit_tendered: String::new(),
            credit_method: "cash".to_owned(),
            credit_reference: String::new(),
            credit_error: None,
            credit_panel: false,
            drawer,
            drawer_count: String::new(),
            drawer_error: None,
            float_draft,
            sale_open: None,
            notice: None,
            file_notice: None,
            category_seed_notice: None,
            file_path: None,
            backup_confirm: false,
            auto_backup_error: None,
            shop_draft,
            receipt_draft,
            tax_draft,
            business_draft,
            business_showing_other: false,
            settings,
            paths,
            theme,
            brand_accent,
            theme_dirty,
            settings_dirty: scanner_token_created || sync_token_created,
            currency,
            needs_currency,
            currency_picker: false,
            currency_error: None,
            session: None,
            users,
            sign_in: SignIn::default(),
            setup: Setup::default(),
            recovery: None,
            override_request: None,
            elevated_until: None,
            last_activity: Instant::now(),
            staff: StaffDraft::default(),
            staff_reset: None,
            new_pin: None,
            owner_password: PasswordDraft::default(),
        }
    }

    // -----------------------------------------------------------------------
    // Who is at the till
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // Stock
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // Purchases
    // -----------------------------------------------------------------------
}

/// The amount without its symbol, for putting in an edit field: `2800` in
/// Uganda, `28.00` in Kenya.
///
/// [`Money::format`] is the one canonical rendering; this is the inverse the
/// product form needs, and the test below pins the two together so a currency
/// where they disagree fails here rather than in an operator's hands.
pub(crate) fn plain_amount(minor: i64, currency: Currency) -> String {
    let formatted = Money::new(minor, currency).format();
    match formatted.rsplit_once(' ') {
        Some((_, amount)) => amount.to_owned(),
        // A glyph symbol such as `$` is glued to the digits; drop everything
        // before the number instead.
        None => formatted
            .trim_start_matches(|ch: char| !ch.is_ascii_digit() && ch != '-')
            .to_owned(),
    }
}

/// Date ranges offered by Dashboard and Sales. The index is what `db` turns
/// into a SQL window, so the label and the numbers can never drift apart.
pub const RANGES: &[&str] = &["Today", "7 days", "30 days", "All time"];

/// The windows Trends offers. Today is not among them: a chart of one day is a
/// number with a border, and the comparison would be against yesterday alone.
///
/// The index is an offset into [`RANGES`] from its first multi-day window, so
/// Trends and Today share one definition of what "7 days" means.
pub const TREND_RANGES: &[&str] = &["7 days", "30 days", "All time"];

/// The `RANGES` index a Trends window points at.
pub fn trend_window(trend_range: usize) -> usize {
    1 + trend_range.min(TREND_RANGES.len() - 1)
}

/// How a range reads as a section heading above the cards.
pub fn range_caption(range: usize) -> &'static str {
    match range {
        0 => "Today",
        1 => "Last 7 days",
        2 => "Last 30 days",
        _ => "All time",
    }
}

/// Settings sections, in sidebar order. Appearance leads: it changes the whole
/// till, while every section after it is about one part of the shop.
pub const SETTINGS_SECTIONS: &[&str] = &[
    "Appearance",
    "Terminal",
    "Receipt",
    "Scanner",
    "Money",
    "Staff",
    "Sync",
];

#[cfg(test)]
mod tests;
