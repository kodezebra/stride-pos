//! Everything the POS screen tree can change, in one place.
//!
//! One struct, passed as `&mut` to whichever module is drawing. Modules keep no
//! state of their own: switching module must not lose the cart, the search box,
//! or the scroll position, and a module that owns data makes that rule hard to
//! keep. It also means a command in the palette can change anything the user can
//! change, without reaching into widgets.

mod suppliers;

pub use suppliers::*;

mod catalog;

pub use catalog::*;

mod cart;

pub use cart::*;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use money::{Currency, Money};
use ui_kit::theme::Theme;
use ui_kit::tokens::Rgb;
use ui_shell::egui;

use crate::db::{Product, SaleLine, StockItem, Supplier};
use crate::scan::{ScanAction, ScanEvent, ScanStatus};
use crate::settings::{ReceiptDetails, Settings};

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

/// The payment panel, open in the cart footer.
///
/// Cash received is text, for the same reason a price is: it is mid-edit, and
/// parsing on every keystroke would eat a half-typed `5000`. Empty means "the
/// exact amount", which is what most cash sales are.
pub struct Payment {
    pub tendered: String,
    pub error: Option<String>,
    /// Put the cursor in the cash field on the first frame.
    pub focus: bool,
}

/// Who is signed in, and what they are allowed to do.
#[derive(Clone, Debug)]
pub struct Session {
    pub user_id: String,
    pub name: String,
    pub role: crate::auth::Role,
}

/// Whose takings a report screen shows.
///
/// The screen is not the permission: every role can open the dashboard, because
/// a cashier has to answer "what did this receipt come to?". What the role
/// decides is the *scope* — one person's sales, or the shop's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope<'a> {
    /// The whole shop: a manager or an owner.
    Shop,
    /// One cashier's own sales, by account id.
    Person(&'a str),
}

impl<'a> Scope<'a> {
    /// The id a query narrows to, or `None` for every account's sales.
    pub fn user(self) -> Option<&'a str> {
        match self {
            Scope::Shop => None,
            Scope::Person(id) => Some(id),
        }
    }

    pub fn is_shop(self) -> bool {
        matches!(self, Scope::Shop)
    }
}

/// The lock screen: the person picked, and the secret being typed.
#[derive(Default)]
pub struct SignIn {
    pub user_id: Option<String>,
    pub secret: String,
    pub error: Option<String>,
    /// The second the selected account is locked until, mirrored from the
    /// database so the screen can count down without querying every frame.
    pub locked_until: Option<i64>,
    /// Put the cursor in the secret field on the next frame.
    pub focus: bool,
}

/// First-run setup: the one account that can create the others.
pub struct Setup {
    pub name: String,
    pub password: String,
    pub confirm: String,
    pub error: Option<String>,
    /// The recovery code, once the owner exists and until it is acknowledged.
    /// While this is set the workspace stays hidden.
    pub code: Option<String>,
    pub focus: bool,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            name: String::new(),
            password: String::new(),
            confirm: String::new(),
            error: None,
            code: None,
            focus: true,
        }
    }
}

/// The forgotten-password flow.
#[derive(Default)]
pub struct Recovery {
    pub code: String,
    pub password: String,
    pub confirm: String,
    pub error: Option<String>,
    /// The replacement code, shown before the way back to the lock screen.
    pub new_code: Option<String>,
}

/// An action waiting for someone with more authority to approve it.
pub struct Override {
    pub permission: crate::auth::Permission,
    /// The module to open once approved, when the request came from navigation.
    pub module: Option<usize>,
    pub user_id: Option<String>,
    pub secret: String,
    pub error: Option<String>,
    pub focus: bool,
}

/// The "add a person" form in Settings.
pub struct StaffDraft {
    pub name: String,
    pub role: crate::auth::Role,
    pub error: Option<String>,
}

impl Default for StaffDraft {
    fn default() -> Self {
        Self {
            name: String::new(),
            role: crate::auth::Role::Cashier,
            error: None,
        }
    }
}

/// A freshly generated PIN, waiting to be read off the screen and handed over.
///
/// Only the hash is stored, so this is the one moment the digits exist in the
/// clear: it stays on the Staff page until the owner says they have written it
/// down, and it is never recoverable afterwards.
pub struct NewPin {
    pub name: String,
    pub pin: String,
}

/// A PIN being replaced for one account. Inline under the staff list: one at a
/// time, like the delete confirmation on a product row.
pub struct ResetPin {
    pub user_id: String,
    pub error: Option<String>,
}

/// The "pair another till" form in Settings.
///
/// The port is text, not a number: a field that silently rewrites what was
/// typed is how a form loses an address. It is parsed once, on Add.
pub struct SyncPeerDraft {
    pub name: String,
    pub host: String,
    pub port: String,
    pub token: String,
}

impl Default for SyncPeerDraft {
    fn default() -> Self {
        Self {
            name: String::new(),
            host: String::new(),
            port: crate::scan::DEFAULT_PORT.to_string(),
            token: String::new(),
        }
    }
}

/// The owner changing their own password.
#[derive(Default)]
pub struct PasswordDraft {
    pub password: String,
    pub confirm: String,
    pub error: Option<String>,
}

impl PasswordDraft {
    pub fn clear(&mut self) {
        self.password.clear();
        self.confirm.clear();
        self.error = None;
    }
}

/// How long an approval lasts. Long enough to finish the action that asked for
/// it, short enough that it cannot be handed to the next person.
const ELEVATION: Duration = Duration::from_secs(60);

/// The two places the till keeps its own files.
///
/// Grouped because they are only ever known together, at boot: the database
/// lives in `data` and `settings.json` in `config`, and both come from
/// `app_boot`'s directories. `Store` deliberately does not remember where it was
/// opened from, so backups and exports ask the state for this.
pub struct Paths {
    pub data: PathBuf,
    pub settings: PathBuf,
}

pub struct State {
    pub store: storage::Store,

    pub cart: Vec<CartLine>,
    /// The cart line whose quantity is being set, if any. One editor at a time:
    /// the modal owns the screen while it is open.
    pub cart_quantity: Option<CartQuantity>,

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
    /// The distinct categories currently in the catalog, for the sidebars.
    pub categories: Vec<String>,
    /// The product form, when it is open.
    pub product_editor: Option<ProductDraft>,
    /// The CSV import modal, when it is open. The typed-path fallback for a
    /// file whose location is already known.
    pub import: Option<ImportDraft>,
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
        let categories = crate::db::categories(&store);
        let suppliers = crate::db::suppliers(&store);
        let stock = crate::db::stock(&store);
        let expiring = crate::db::expiring(&store);
        let purchases = crate::db::purchases(&store);
        let users = crate::db::users(&store);
        let drawer = crate::db::drawer(&store);
        Self {
            store,
            cart: Vec::new(),
            cart_quantity: None,
            module,
            sidebar_open,
            sell_view: SellView::default(),
            prefs_dirty: false,
            sell_search: String::new(),
            catalog_search: String::new(),
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
            categories,
            product_editor: None,
            import: None,
            picker: None,
            dialog: crate::dialog::Chooser::new(),
            import_dir: None,
            export_dir: None,
            product_delete: None,
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
            drawer,
            drawer_count: String::new(),
            drawer_error: None,
            float_draft,
            sale_open: None,
            notice: None,
            file_notice: None,
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

    /// Format an amount in this till's currency: `12345` → `"Ksh 123.45"`.
    ///
    /// The currency is the one fact every screen needs and none of them own, so
    /// it lives here and the screens ask the state rather than passing it down.
    pub fn money(&self, minor: i64) -> String {
        Money::new(minor, self.currency).format()
    }

    /// Record a currency choice. On a failed write the picker stays open with
    /// the reason, because selling in an unrecorded currency is worse than
    /// blocking the till for one more attempt.
    pub fn set_currency(&mut self, currency: Currency) {
        match crate::db::set_currency(&self.store, currency) {
            Ok(()) => {
                self.currency = currency;
                self.needs_currency = false;
                self.currency_picker = false;
                self.currency_error = None;
            }
            Err(error) => {
                tracing::error!(error = %error, "storing the currency failed");
                self.currency_error = Some(format!("Could not save the currency: {error}"));
            }
        }
    }

    /// Switch appearance. The shell installs it on the next frame; screens never
    /// touch global visuals.
    pub fn set_theme(&mut self, theme: Theme) {
        if self.theme != theme {
            self.theme = theme;
            self.settings.set_theme(theme);
            self.theme_dirty = true;
            self.settings_dirty = true;
        }
    }

    /// Switch the one brand accent. The shell derives the palette on the next
    /// frame; screens never touch global visuals.
    pub fn set_brand_accent(&mut self, accent: Option<Rgb>) {
        if self.brand_accent != accent {
            self.brand_accent = accent;
            self.settings.set_brand_accent(accent);
            self.theme_dirty = true;
            self.settings_dirty = true;
        }
    }

    /// Flip light/dark. One place decides, so the icon button, the palette
    /// command and Ctrl+T cannot drift apart.
    pub fn toggle_theme(&mut self) {
        self.set_theme(self.theme.flipped());
    }

    /// Commit the shop name typed in Settings.
    ///
    /// Deliberately not bound straight to `settings.shop_name`: the shell writes
    /// the file whenever the flag is set, so a field wired to the settings would
    /// rewrite `settings.json` on every keystroke, and a crash mid-word would
    /// leave half a shop name as the letterhead of every receipt.
    pub fn save_shop_name(&mut self) {
        let name = self.shop_draft.trim().to_owned();
        self.shop_draft.clone_from(&name);
        if self.settings.shop_name != name {
            self.settings.shop_name = name;
            self.settings_dirty = true;
        }
    }

    /// Commit the name other tills see. Blank is allowed; the protocol falls
    /// back to "Till" so a peer list never shows an empty row.
    pub fn save_sync_device_name(&mut self) {
        let name = self.sync_name_draft.trim().to_owned();
        self.sync_name_draft.clone_from(&name);
        if self.settings.sync.device_name != name {
            self.settings.sync.device_name = name;
            self.settings_dirty = true;
        }
    }

    /// Turn till-to-till sync on or off. The shell restarts the listener and
    /// wakes the worker from the flag, so this only records the decision.
    pub fn set_sync_enabled(&mut self, enabled: bool) {
        if self.settings.sync.enabled != enabled {
            self.settings.sync.enabled = enabled;
            self.settings_dirty = true;
        }
    }

    /// Declare (or stop declaring) this till as the catalog author.
    ///
    /// The flag is a promise to peers, not a lock: replicas read it from the
    /// pull response and ignore catalogs from tills that do not claim it.
    pub fn set_catalog_owner(&mut self, owner: bool) {
        if self.settings.sync.catalog_owner != owner {
            self.settings.sync.catalog_owner = owner;
            self.settings_dirty = true;
        }
    }

    /// Re-read the peer list, including the error and last-seen time the worker
    /// wrote. Called after a pass so the card does not show a stale failure.
    pub fn reload_sync_peers(&mut self) {
        match crate::db::sync_peers(&self.store) {
            Ok(peers) => {
                self.sync_peers = peers;
                self.sync_peer_error = None;
            }
            Err(error) => {
                self.sync_peer_error = Some(format!("Could not read the paired tills: {error}"));
            }
        }
    }

    /// Pair another till from the Sync card.
    pub fn add_sync_peer(&mut self) {
        let port = match self.sync_peer_draft.port.trim().parse::<u16>() {
            Ok(port) if port != 0 => port,
            _ => {
                self.sync_peer_error = Some("Enter a port between 1 and 65535.".to_owned());
                return;
            }
        };

        match crate::db::add_sync_peer(
            &self.store,
            &self.sync_peer_draft.name,
            &self.sync_peer_draft.host,
            port,
            &self.sync_peer_draft.token,
        ) {
            Ok(()) => {
                self.sync_peer_draft = SyncPeerDraft::default();
                self.sync_peer_error = None;
                self.reload_sync_peers();
                // Pairing a till is a request to use it now, not to wait half a
                // minute for the next poll.
                self.sync_now_requested = self.settings.sync.enabled;
            }
            Err(error) => {
                self.sync_peer_error = Some(error.to_string());
            }
        }
    }

    /// Stop pulling from one till. Its cursor goes with the row: re-adding the
    /// till starts a fresh feed at zero. Ingestion is idempotent, so that replay
    /// converges; it is only slower than keeping the cursor.
    pub fn remove_sync_peer(&mut self, id: &str) {
        match crate::db::remove_sync_peer(&self.store, id) {
            Ok(()) => {
                self.sync_peer_error = None;
                self.reload_sync_peers();
            }
            Err(error) => {
                self.sync_peer_error = Some(format!("Could not remove the till: {error}"));
            }
        }
    }

    /// Commit the optional receipt fields typed in Settings.
    ///
    /// They are saved together because they are read as one block on paper: an
    /// address saved without the phone number beside it is a half-edited
    /// letterhead, and the owner would have to notice which field they last
    /// touched.
    pub fn save_receipt_details(&mut self) {
        let details = self.receipt_draft.normalised();
        self.receipt_draft = details.clone();
        if self.settings.receipt != details {
            self.settings.receipt = details;
            self.settings_dirty = true;
        }
    }

    /// Commit the tax rate typed in Settings. The draft is parsed as a whole
    /// percentage; anything that does not parse is ignored rather than written
    /// to disk, so a stray keystroke cannot invent a rate the shop does not pay.
    pub fn save_tax_rate(&mut self) {
        let parsed: Option<i64> = self.tax_draft.trim().parse().ok();
        let rate = parsed.unwrap_or(0).max(0);
        self.tax_draft = rate.to_string();
        if self.settings.tax_rate_percent != rate {
            self.settings.tax_rate_percent = rate;
            self.settings_dirty = true;
        }
    }

    /// Commit the change float typed in Settings.
    ///
    /// It is the cash that sits in the drawer before the first sale, so the
    /// evening count is compared against takings plus this rather than against
    /// takings alone. A figure that does not parse is ignored rather than
    /// written: the float is a statement about real money.
    pub fn save_cash_float(&mut self) {
        let Ok(amount) = Money::parse(self.float_draft.trim(), self.currency) else {
            return;
        };
        let minor = amount.minor().max(0);
        self.float_draft = plain_amount(minor, self.currency);
        if self.settings.cash_float_minor != minor {
            self.settings.cash_float_minor = minor;
            self.settings_dirty = true;
        }
    }

    /// The drawer a sale is about to land in, opened if the day has not started.
    ///
    /// A failure is logged and the sale goes through unattributed. Bookkeeping
    /// must never refuse a sale: a gap in a count is recoverable, a customer
    /// turned away is not.
    fn ensure_shift(&mut self) -> Option<String> {
        if let Some(drawer) = self.drawer.as_ref()
            && drawer.shift.is_open()
        {
            return Some(drawer.shift.uid.clone());
        }

        let user = self.session.as_ref().map(|session| session.user_id.clone());
        match crate::db::ensure_open_shift(
            &self.store,
            self.settings.cash_float_minor,
            user.as_deref(),
        ) {
            Ok(shift) => {
                let uid = shift.uid.clone();
                self.refresh_drawer();
                Some(uid)
            }
            Err(error) => {
                tracing::error!(error = %error, "opening the drawer failed");
                None
            }
        }
    }

    /// Re-read the drawer after a sale or a close, so the card shows the money
    /// that is actually in it.
    pub fn refresh_drawer(&mut self) {
        self.drawer = crate::db::drawer(&self.store);
    }

    /// Count the drawer and close the day.
    ///
    /// One number, one tap: the operator types what is in the drawer and the
    /// till works out what should have been there. A count that does not parse
    /// leaves the field and the card open rather than closing on a guess, and a
    /// second tap cannot overwrite a count that was already made.
    pub fn close_day(&mut self) {
        let Some(drawer) = self.drawer.as_ref() else {
            return;
        };
        if !drawer.shift.is_open() {
            return;
        }

        let counted = match Money::parse(self.drawer_count.trim(), self.currency) {
            Ok(amount) => amount.minor(),
            Err(_) => {
                self.drawer_error = Some("Type the cash you counted.".to_owned());
                return;
            }
        };

        let uid = drawer.shift.uid.clone();
        let user = self.session.as_ref().map(|session| session.user_id.clone());
        match crate::db::close_shift(&self.store, &uid, counted, user.as_deref()) {
            Ok(()) => {
                self.drawer_error = None;
                self.drawer_count.clear();
                self.refresh_drawer();
            }
            Err(error) => {
                tracing::error!(error = %error, "closing the drawer failed");
                self.drawer_error = Some(error.to_string());
            }
        }
    }

    /// Commit the business type chosen in Settings. The value must match one of
    /// the seed files; anything else is ignored. The choice only affects which
    /// category names the product form offers as suggestions — it never inserts
    /// categories automatically.
    ///
    /// Stored lowercased so "Supermarket" and "supermarket" both match the
    /// seeded category list.
    pub fn save_business_type(&mut self) {
        let chosen = self.business_draft.trim().to_ascii_lowercase();
        if chosen.is_empty() {
            self.business_draft = String::new();
            if self.settings.business_type.is_some() {
                self.settings.business_type = None;
                self.settings_dirty = true;
            }
            return;
        }
        if self.settings.business_type.as_deref() != Some(&chosen) {
            self.settings.business_type = Some(chosen.clone());
            self.business_draft = chosen;
            self.settings_dirty = true;
        }
    }

    pub fn select(&mut self, module: usize) {
        if self.module != module {
            self.module = module;
            self.prefs_dirty = true;
            self.notice = None;
            self.file_notice = None;
            self.file_path = None;
            self.backup_confirm = false;
            // Opening the selling screen returns the cursor to the scan field, so
            // the next barcode lands in the cart without a click.
            if module == crate::modules::SELL {
                self.scan_focus = true;
            }
        }
    }

    // -----------------------------------------------------------------------
    // Who is at the till
    // -----------------------------------------------------------------------

    /// Whether the workspace is hidden behind the lock screen.
    ///
    /// Also true while the one-time recovery code is on screen: the code is
    /// shown once, and the takings behind it are none of the next person's
    /// business.
    pub fn locked(&self) -> bool {
        self.session.is_none() || self.setup.code.is_some()
    }

    /// Called once a frame. Locks the till after a stretch with no input.
    ///
    /// Idleness is read from input events rather than from a timer started at
    /// sign-in: a mouse that moves is a person, and a screen nobody has touched
    /// is not. The clock only runs while someone is signed in.
    pub fn tick(&mut self, ctx: &egui::Context) {
        if ctx.input(|input| !input.events.is_empty()) {
            self.last_activity = Instant::now();
        }
        let Some(session) = &self.session else {
            return;
        };
        if self.last_activity.elapsed() >= session.role.idle_timeout() {
            self.lock();
        }
    }

    /// Lock the till now.
    ///
    /// The cart survives: the customer is still at the counter, and rebuilding
    /// the basket would be the real cost of locking. Everything that shows money
    /// closes.
    pub fn lock(&mut self) {
        self.session = None;
        self.elevated_until = None;
        self.override_request = None;
        self.payment = None;
        self.cart_quantity = None;
        self.product_editor = None;
        self.product_delete = None;
        self.supplier_editor = None;
        self.supplier_archive = None;
        self.notice = None;
        self.sign_in = SignIn::default();
        self.recovery = None;
        self.staff_reset = None;
        self.owner_password.clear();
    }

    /// Pick a person on the lock screen, and read their lockout.
    pub fn choose_user(&mut self, user_id: &str) {
        self.sign_in.user_id = Some(user_id.to_owned());
        self.sign_in.secret.clear();
        self.sign_in.error = None;
        self.sign_in.locked_until = crate::auth::locked_for(&self.store, user_id)
            .map(|seconds| crate::auth::unix_now() + seconds as i64);
        self.sign_in.focus = true;
    }

    /// The selected account, when one is selected and still exists.
    fn selected_user(&self) -> Option<&crate::db::User> {
        let id = self.sign_in.user_id.as_deref()?;
        self.users.iter().find(|user| user.id == id)
    }

    /// Which keyboard the selected account needs: a PIN pad or a password.
    pub fn selected_kind(&self) -> Option<crate::auth::CredentialKind> {
        self.selected_user().map(|user| user.kind)
    }

    /// Check the secret on the lock screen and open the workspace.
    pub fn sign_in(&mut self) {
        let Some(user) = self.selected_user().cloned() else {
            return;
        };
        let secret = self.sign_in.secret.clone();

        match crate::auth::attempt(&self.store, &user, &secret) {
            Ok(()) => {
                self.session = Some(Session {
                    user_id: user.id,
                    name: user.name,
                    role: user.role,
                });
                self.sign_in = SignIn::default();
                self.recovery = None;
                self.last_activity = Instant::now();
                // The catalog may have moved on since the last shift, and the
                // shop's own staff are the ones who moved it.
                self.reload_products();
                // A shared till must not inherit a screen the new person cannot
                // use. A cashier who signs in after a manager was in Settings
                // lands on Sell instead of a locked door.
                if !crate::modules::reachable(user.role, self.module) {
                    self.select(crate::modules::SELL);
                }
            }
            Err(denied) => {
                self.sign_in.secret.clear();
                self.sign_in.focus = true;
                self.sign_in.error = Some(denied.message(&user));
                self.sign_in.locked_until = crate::auth::locked_for(&self.store, &user.id)
                    .map(|seconds| crate::auth::unix_now() + seconds as i64);
            }
        }
    }

    /// Create the owner account on a fresh till.
    ///
    /// The recovery code is minted here and shown once. Its hash is stored
    /// before the screen closes, so a till that loses power between the two
    /// still has a password and an owner — only the code is lost, and the code
    /// is the fallback, not the way in.
    pub fn create_owner(&mut self) {
        let name = self.setup.name.trim().to_owned();
        let password = self.setup.password.clone();

        if name.is_empty() {
            return self.setup_error("The owner needs a name.");
        }
        if let Err(message) = crate::auth::CredentialKind::Password.validate(&password) {
            return self.setup_error(message);
        }
        if password != self.setup.confirm {
            return self.setup_error("The two passwords do not match.");
        }

        let credential = match crate::auth::hash(&password) {
            Ok(credential) => credential,
            Err(error) => {
                tracing::error!(error = %error, "hashing the owner password failed");
                return self.setup_error("Could not store that password. Try again.");
            }
        };

        let user = match crate::db::insert_user(
            &self.store,
            &name,
            crate::auth::Role::Owner,
            crate::auth::CredentialKind::Password,
            &credential,
        ) {
            Ok(user) => user,
            Err(error) => {
                tracing::error!(error = %error, "creating the owner failed");
                return self.setup_error("Could not create the owner account.");
            }
        };

        let code = crate::auth::new_recovery_code();
        if let Err(error) = self.store_recovery_code(&code) {
            tracing::error!(error = %error, "storing the recovery code failed");
            return self.setup_error("Could not store the recovery code. Try again.");
        }

        self.users = crate::db::users(&self.store);
        self.session = Some(Session {
            user_id: user.id,
            name: user.name,
            role: user.role,
        });
        self.setup.password.clear();
        self.setup.confirm.clear();
        self.setup.error = None;
        self.setup.code = Some(code);
        self.last_activity = Instant::now();
    }

    /// The operator has written the code down; the workspace can appear.
    pub fn acknowledge_code(&mut self) {
        self.setup = Setup::default();
        self.notice = Some("Till ready. Choose the currency to finish setup.".to_owned());
    }

    /// Store a hash of a recovery code. The code itself is never written down.
    fn store_recovery_code(&self, code: &str) -> anyhow::Result<()> {
        let credential = crate::auth::hash(&crate::auth::normalise_code(code))?;
        crate::db::set_meta(&self.store, crate::auth::RECOVERY_KEY, &credential)
    }

    /// Open the forgotten-password flow.
    pub fn start_recovery(&mut self) {
        self.sign_in.error = None;
        self.recovery = Some(Recovery::default());
    }

    /// Trade the one-time code for a new owner password.
    ///
    /// The code is spent: a new one is minted and shown before the screen
    /// closes, so the piece of paper in the drawer is never valid twice.
    pub fn recover(&mut self) {
        let Some(draft) = self.recovery.as_ref() else {
            return;
        };
        let code = crate::auth::normalise_code(&draft.code);
        let password = draft.password.clone();
        let confirm = draft.confirm.clone();

        let Some(stored) = crate::db::meta(&self.store, crate::auth::RECOVERY_KEY) else {
            return self.recovery_error("This till has no recovery code. Use the password.");
        };
        if !crate::auth::verify(&code, &stored) {
            return self.recovery_error("That recovery code is not right.");
        }
        if let Err(message) = crate::auth::CredentialKind::Password.validate(&password) {
            return self.recovery_error(message);
        }
        if password != confirm {
            return self.recovery_error("The two passwords do not match.");
        }

        let Some(owner) = crate::db::owner(&self.store) else {
            return self.recovery_error("There is no owner account on this till.");
        };

        let credential = match crate::auth::hash(&password) {
            Ok(credential) => credential,
            Err(error) => {
                tracing::error!(error = %error, "hashing the new owner password failed");
                return self.recovery_error("Could not store that password. Try again.");
            }
        };
        if let Err(error) = crate::db::set_user_credential(&self.store, &owner.id, &credential) {
            tracing::error!(error = %error, "replacing the owner password failed");
            return self.recovery_error("Could not store that password. Try again.");
        }
        // A recovered account starts with a clean slate: the failures that led
        // here were about the old password.
        crate::db::clear_attempts(&self.store, &owner.id);

        let new_code = crate::auth::new_recovery_code();
        if let Err(error) = self.store_recovery_code(&new_code) {
            tracing::error!(error = %error, "rotating the recovery code failed");
            return self.recovery_error("The password changed, but the code did not.");
        }

        self.users = crate::db::users(&self.store);
        if let Some(draft) = self.recovery.as_mut() {
            draft.code.clear();
            draft.password.clear();
            draft.confirm.clear();
            draft.error = None;
            draft.new_code = Some(new_code);
        }
    }

    /// The new code is written down; back to the lock screen.
    pub fn finish_recovery(&mut self) {
        self.recovery = None;
        self.sign_in = SignIn::default();
        self.sign_in.focus = true;
    }

    /// Whether the person signed in may do this, in their own right or because a
    /// manager approved it within the last minute.
    pub fn permits(&self, permission: crate::auth::Permission) -> bool {
        let Some(session) = &self.session else {
            return false;
        };
        if session.role.permits(permission) {
            return true;
        }
        let elevated = self
            .elevated_until
            .is_some_and(|until| until > Instant::now());
        elevated && permission.approvable() && crate::auth::Role::Manager.permits(permission)
    }

    /// Whose figures the report screens show.
    ///
    /// `ViewReports` is the scope, not the key: without it the dashboard and the
    /// transaction list still open, and still answer a customer's question, but
    /// only about the cashier's own sales. Approval lifts the scope with the
    /// same minute-long window as any other manager override.
    ///
    /// The lock screen has no session, and gets an id that matches no sale
    /// rather than a shop view: a report drawn while nobody is signed in must
    /// not read the books.
    pub fn report_scope(&self) -> Scope<'_> {
        let Some(session) = &self.session else {
            return Scope::Person("");
        };
        if self.permits(crate::auth::Permission::ViewReports) {
            Scope::Shop
        } else {
            Scope::Person(session.user_id.as_str())
        }
    }

    /// Switch module, asking for approval first when the person cannot open it.
    ///
    /// Every route into a module comes through here — the rail, the F-keys and
    /// the palette — so a permission cannot be enforced on one and forgotten on
    /// another.
    pub fn open(&mut self, module: usize, permission: crate::auth::Permission) {
        if self.permits(permission) {
            self.select(module);
        } else {
            self.ask_override(permission, Some(module));
        }
    }

    /// Ask someone with the authority to approve one action.
    pub fn ask_override(&mut self, permission: crate::auth::Permission, module: Option<usize>) {
        self.notice = None;
        self.override_request = Some(Override {
            permission,
            module,
            user_id: None,
            secret: String::new(),
            error: None,
            focus: true,
        });
    }

    /// Check the approving person's secret and open the elevation window.
    pub fn approve_override(&mut self) {
        let Some(request) = self.override_request.as_ref() else {
            return;
        };
        let permission = request.permission;
        let module = request.module;
        let user_id = request.user_id.clone();
        let secret = request.secret.clone();

        let Some(user) =
            user_id.and_then(|id| self.users.iter().find(|user| user.id == id).cloned())
        else {
            return self.override_error("Choose who is approving.");
        };
        if !user.role.permits(permission) {
            return self.override_error(format!("{} cannot {}.", user.name, permission.action()));
        }

        match crate::auth::attempt(&self.store, &user, &secret) {
            Ok(()) => {
                self.elevated_until = Some(Instant::now() + ELEVATION);
                self.override_request = None;
                self.last_activity = Instant::now();
                self.notice = Some(format!(
                    "{} approved: {} (one minute).",
                    user.name,
                    permission.action()
                ));
                if let Some(module) = module {
                    self.select(module);
                }
            }
            Err(denied) => self.override_error(denied.message(&user)),
        }
    }

    /// Add a cashier or manager.
    ///
    /// The PIN is generated, not typed. A PIN the owner invents is a birthday, a
    /// phone number, or 123456, and it is the one secret in a shop that nobody
    /// would ever change. The digits are shown once, above the staff list.
    pub fn add_staff(&mut self) {
        let name = self.staff.name.trim().to_owned();
        let role = self.staff.role;

        if name.is_empty() {
            return self.staff_error("A person needs a name.");
        }
        if role == crate::auth::Role::Owner {
            return self.staff_error("There is one owner account, created at setup.");
        }
        if self
            .users
            .iter()
            .any(|user| user.name.eq_ignore_ascii_case(&name))
        {
            return self.staff_error("Someone with that name already works here.");
        }

        let pin = crate::auth::generate_pin();
        let credential = match crate::auth::hash(&pin) {
            Ok(credential) => credential,
            Err(error) => {
                tracing::error!(error = %error, "hashing a new PIN failed");
                return self.staff_error("Could not store that PIN. Try again.");
            }
        };

        match crate::db::insert_user(
            &self.store,
            &name,
            role,
            role.credential_kind(),
            &credential,
        ) {
            Ok(_) => {
                self.users = crate::db::users(&self.store);
                self.staff = StaffDraft::default();
                self.new_pin = Some(NewPin { name, pin });
            }
            Err(error) => {
                tracing::error!(error = %error, "adding a person failed");
                self.staff_error("Could not add that person.");
            }
        }
    }

    /// Switch an account on or off. Off means "no longer works here", not
    /// "deleted": their sales keep their name.
    pub fn set_staff_active(&mut self, id: &str, active: bool) {
        match crate::db::set_user_active(&self.store, id, active) {
            Ok(()) => {
                self.users = crate::db::users(&self.store);
                // Nobody switches themselves off, but if a path ever allows it
                // the till must not stay open under a closed account.
                if !active
                    && self
                        .session
                        .as_ref()
                        .is_some_and(|session| session.user_id == id)
                {
                    self.lock();
                }
            }
            Err(error) => {
                tracing::error!(error = %error, "switching an account failed");
                self.notice = Some(format!("Could not change that account: {error}"));
            }
        }
    }

    /// Open the PIN confirmation under one account.
    pub fn start_reset_pin(&mut self, id: &str) {
        self.staff_reset = Some(ResetPin {
            user_id: id.to_owned(),
            error: None,
        });
    }

    /// Replace one account's PIN with a fresh one, shown once.
    pub fn reset_pin(&mut self) {
        let Some(draft) = self.staff_reset.as_ref() else {
            return;
        };
        let id = draft.user_id.clone();
        let name = self
            .users
            .iter()
            .find(|user| user.id == id)
            .map(|user| user.name.clone())
            .unwrap_or_else(|| "the person".to_owned());

        let pin = crate::auth::generate_pin();
        let credential = match crate::auth::hash(&pin) {
            Ok(credential) => credential,
            Err(error) => {
                tracing::error!(error = %error, "hashing a reset PIN failed");
                return self.reset_error("Could not store that PIN. Try again.");
            }
        };

        match crate::db::set_user_credential(&self.store, &id, &credential) {
            Ok(()) => {
                // A reset also clears the lockout: the point of the reset is that
                // the person can get back in.
                crate::db::clear_attempts(&self.store, &id);
                self.staff_reset = None;
                self.users = crate::db::users(&self.store);
                self.new_pin = Some(NewPin { name, pin });
            }
            Err(error) => {
                tracing::error!(error = %error, "replacing a PIN failed");
                self.reset_error("Could not store that PIN. Try again.");
            }
        }
    }

    /// The owner changing their own password.
    ///
    /// No current-password field: they are already signed in, and asking again
    /// only teaches people to pick something they can retype from memory.
    pub fn change_owner_password(&mut self) {
        let password = self.owner_password.password.clone();

        if let Err(message) = crate::auth::CredentialKind::Password.validate(&password) {
            return self.owner_password_error(message);
        }
        if password != self.owner_password.confirm {
            return self.owner_password_error("The two passwords do not match.");
        }

        let Some(owner) = crate::db::owner(&self.store) else {
            return self.owner_password_error("There is no owner account on this till.");
        };

        let credential = match crate::auth::hash(&password) {
            Ok(credential) => credential,
            Err(error) => {
                tracing::error!(error = %error, "hashing the owner password failed");
                return self.owner_password_error("Could not store that password. Try again.");
            }
        };

        match crate::db::set_user_credential(&self.store, &owner.id, &credential) {
            Ok(()) => {
                self.users = crate::db::users(&self.store);
                self.owner_password.clear();
                self.notice = Some("Owner password changed.".to_owned());
            }
            Err(error) => {
                tracing::error!(error = %error, "replacing the owner password failed");
                self.owner_password_error("Could not store that password. Try again.");
            }
        }
    }

    fn setup_error(&mut self, message: &str) {
        self.setup.error = Some(message.to_owned());
    }

    fn recovery_error(&mut self, message: &str) {
        if let Some(draft) = self.recovery.as_mut() {
            draft.error = Some(message.to_owned());
        }
    }

    fn override_error(&mut self, message: impl Into<String>) {
        if let Some(request) = self.override_request.as_mut() {
            request.error = Some(message.into());
        }
    }

    fn staff_error(&mut self, message: &str) {
        self.staff.error = Some(message.to_owned());
    }

    fn reset_error(&mut self, message: &str) {
        if let Some(draft) = self.staff_reset.as_mut() {
            draft.error = Some(message.to_owned());
        }
    }

    fn owner_password_error(&mut self, message: &str) {
        self.owner_password.error = Some(message.to_owned());
    }

    pub fn sidebar_open(&self, module: usize) -> bool {
        self.sidebar_open.get(module).copied().unwrap_or(true)
    }

    pub fn set_sidebar_open(&mut self, module: usize, open: bool) {
        if module >= self.sidebar_open.len() {
            self.sidebar_open.resize(module + 1, true);
        }
        if self.sidebar_open[module] != open {
            self.sidebar_open[module] = open;
            self.prefs_dirty = true;
        }
    }

    pub fn toggle_sidebar(&mut self) {
        let open = self.sidebar_open(self.module);
        self.set_sidebar_open(self.module, !open);
    }

    // -----------------------------------------------------------------------
    // Stock
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // Purchases
    // -----------------------------------------------------------------------

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

    /// Apply a chosen path, from the system dialog or from the till's picker.
    ///
    /// One place, because both routes ask the same question: an import reads
    /// the file it was given, an export writes into the folder it was given.
    pub(crate) fn finish_ask(&mut self, purpose: crate::picker::Purpose, path: PathBuf) {
        match purpose {
            crate::picker::Purpose::Import => {
                self.import_dir = path.parent().map(PathBuf::from);
                self.import = Some(ImportDraft {
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
mod tests {
    use super::*;

    /// A till with a real database, because the payment path writes.
    fn state(name: &str, currency: Currency) -> (State, PathBuf) {
        let path = std::env::temp_dir().join(format!("pos-state-{name}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut store = storage::Store::open(&path).expect("open database");
        store.migrate(crate::db::MIGRATIONS).expect("migrate");
        // A cart only ever holds real products — `sale_lines.product_id` is a
        // foreign key — so the first row is the one `line()` points at.
        seed_product(&store, "Test item", 100, None);
        let mut state = State::new(
            store,
            0,
            Vec::new(),
            Settings::default(),
            Paths {
                data: std::env::temp_dir(),
                settings: PathBuf::from("settings.json"),
            },
            currency,
            false,
        );
        // Paying needs somebody signed in, because the sale records who took the
        // money. The secret is never checked on this path, so a placeholder hash
        // is enough to have a real row for the foreign key.
        let cashier = crate::db::insert_user(
            &state.store,
            "Test cashier",
            crate::auth::Role::Cashier,
            crate::auth::CredentialKind::Pin,
            "$argon2id$placeholder",
        )
        .expect("seed cashier");
        state.session = Some(Session {
            user_id: cashier.id,
            name: cashier.name,
            role: cashier.role,
        });
        (state, path)
    }

    fn line(name: &str, qty: i64, unit_minor: i64) -> CartLine {
        CartLine {
            product_id: 1,
            name: name.to_owned(),
            qty,
            unit_minor,
            unit_factor: 1,
            unit_label: None,
        }
    }

    /// Seed a plain, loose product row, bypassing the form draft.
    fn seed_product(store: &storage::Store, name: &str, price: i64, barcode: Option<&str>) {
        crate::db::insert_product(
            store,
            &crate::db::ProductFields {
                name,
                price_minor: price,
                category: "Test",
                category_id: None,
                barcode,
                unit_label: "unit",
                pack_size: 1,
                pack_price_minor: None,
                pack_barcode: None,
            },
        )
        .expect("seed product");
    }

    /// Seed a pack product: one tablet price, one box price, no codes.
    fn seed_pack(
        store: &storage::Store,
        name: &str,
        price: i64,
        pack_size: i64,
        pack_price: Option<i64>,
    ) -> i64 {
        crate::db::insert_product(
            store,
            &crate::db::ProductFields {
                name,
                price_minor: price,
                category: "Test",
                category_id: None,
                barcode: None,
                unit_label: "tablet",
                pack_size,
                pack_price_minor: pack_price,
                pack_barcode: None,
            },
        )
        .expect("seed pack")
    }

    /// A trip that was never saved is still a trip: planning at the market and
    /// counting in should not need a save in between, and nothing reaches the
    /// shelf except through receiving.
    #[test]
    fn a_trip_planned_at_the_market_is_received_in_one_go() {
        let (mut state, path) = state("receive-unsaved", Currency::UGX);
        state
            .store
            .conn()
            .execute("UPDATE products SET tracked = 1 WHERE id = 1", [])
            .expect("track the seeded item");
        state.reload_stock();

        state.start_purchase();
        state
            .purchase_editor
            .as_mut()
            .expect("open")
            .lines
            .push(PurchaseLineDraft {
                product_id: 1,
                name: "Test item".to_owned(),
                qty: "3".to_owned(),
                cost: "500".to_owned(),
                pack_size: "1".to_owned(),
                expires_on: String::new(),
            });

        assert_eq!(state.stock[0].on_hand, 0, "a plan moves no stock");
        state.receive_purchase();
        state.receive_purchase();

        assert!(state.purchase_editor.is_none(), "the trip is closed");
        let trip = state.purchases.first().expect("one trip");
        assert_eq!(trip.status, crate::db::PurchaseStatus::Received);
        assert_eq!(trip.count_label(), "3 items");
        assert_eq!(trip.cost_minor, Some(1_500), "three at five hundred");
        assert_eq!(state.stock[0].on_hand, 3, "the shelf moved on receiving");

        let _ = std::fs::remove_file(path);
    }

    /// Stock is written once and there is no undo, so the Received button asks
    /// twice: the first press arms it, the second records the trip.
    #[test]
    fn receiving_asks_for_a_second_press() {
        let (mut state, path) = state("receive-confirm", Currency::UGX);
        state
            .store
            .conn()
            .execute("UPDATE products SET tracked = 1 WHERE id = 1", [])
            .expect("track the seeded item");
        state.reload_stock();

        state.start_purchase();
        state
            .purchase_editor
            .as_mut()
            .expect("open")
            .lines
            .push(PurchaseLineDraft {
                product_id: 1,
                name: "Test item".to_owned(),
                qty: "2".to_owned(),
                cost: String::new(),
                pack_size: "1".to_owned(),
                expires_on: String::new(),
            });

        state.receive_purchase();
        let draft = state
            .purchase_editor
            .as_ref()
            .expect("the trip is still open");
        assert!(
            draft.confirm_receive,
            "the first press only arms the button"
        );
        assert_eq!(state.stock[0].on_hand, 0, "nothing reached the shelf");
        assert!(state.purchases.is_empty(), "and no trip was written");

        state.receive_purchase();
        assert!(
            state.purchase_editor.is_none(),
            "the second press records it"
        );
        assert_eq!(state.stock[0].on_hand, 2, "the shelf moved once");
        assert_eq!(
            state.purchases[0].status,
            crate::db::PurchaseStatus::Received
        );

        let _ = std::fs::remove_file(path);
    }

    /// A count sheet typed at the market can carry a line that brought nothing
    /// home. Writing that list must not refuse the whole trip over a zero the
    /// receiving step is about to ignore, and the saved plan must not claim to
    /// have ordered something nobody bought.
    #[test]
    fn an_unsaved_trip_can_count_a_line_at_zero() {
        let (mut state, path) = state("receive-unsaved-zero", Currency::UGX);
        seed_product(&state.store, "Soap", 200, None);
        state
            .store
            .conn()
            .execute("UPDATE products SET tracked = 1", [])
            .expect("track everything");
        state.reload_stock();

        state.start_purchase();
        for (product_id, name, qty) in [(1, "Test item", "2"), (2, "Soap", "0")] {
            state
                .purchase_editor
                .as_mut()
                .expect("open")
                .lines
                .push(PurchaseLineDraft {
                    product_id,
                    name: name.to_owned(),
                    qty: qty.to_owned(),
                    cost: String::new(),
                    pack_size: "1".to_owned(),
                    expires_on: String::new(),
                });
        }
        state.receive_purchase();
        state.receive_purchase();

        assert!(
            state
                .purchase_editor
                .as_ref()
                .is_none_or(|draft| draft.error.is_none()),
            "a zero line is not an error"
        );
        let trip = state.purchases.first().expect("the trip");
        assert_eq!(trip.status, crate::db::PurchaseStatus::Received);
        assert_eq!(trip.count_label(), "2 items", "only what was bought");
        let detail = crate::db::purchase_detail(&state.store, trip.id).expect("detail");
        assert_eq!(detail.lines.len(), 1, "the zero never reached the plan");
        let soap = state
            .stock
            .iter()
            .find(|item| item.product_id == 2)
            .expect("soap is still tracked");
        assert_eq!(soap.on_hand, 0, "nothing came home for it");

        let _ = std::fs::remove_file(path);
    }

    /// The list is the count sheet: a line counted short is recorded as short,
    /// a line counted at zero was not bought, and a line taken off the sheet was
    /// never bought at all. One trip, three different truths.
    #[test]
    fn a_received_trip_keeps_the_difference_between_plan_and_count() {
        let (mut state, path) = state("receive-short", Currency::UGX);
        for (name, price) in [("Test item", 100), ("Soap", 200), ("Tea", 300)] {
            seed_product(&state.store, name, price, None);
        }
        state
            .store
            .conn()
            .execute("UPDATE products SET tracked = 1", [])
            .expect("track everything");
        state.reload_stock();
        state.reload_products();

        state.start_purchase();
        for (product_id, name, qty) in [(1, "Test item", "5"), (2, "Soap", "4"), (3, "Tea", "2")] {
            state
                .purchase_editor
                .as_mut()
                .expect("open")
                .lines
                .push(PurchaseLineDraft {
                    product_id,
                    name: name.to_owned(),
                    qty: qty.to_owned(),
                    cost: "500".to_owned(),
                    pack_size: "1".to_owned(),
                    expires_on: String::new(),
                });
        }
        state.save_purchase();
        assert!(state.purchase_editor.is_none(), "saving returns to Restock");
        assert_eq!(state.purchases[0].count_label(), "11 items");

        let id = state.purchases[0].id;
        state.edit_purchase(id);
        let draft = state.purchase_editor.as_mut().expect("reopened");
        draft.lines[0].qty = "2".to_owned(); // short
        draft.lines[1].qty = "0".to_owned(); // not bought
        state.purchase_remove_line(2); // never bought
        state.receive_purchase();
        state.receive_purchase();

        let trip = state.purchases.first().expect("the trip");
        assert_eq!(trip.status, crate::db::PurchaseStatus::Received);
        assert_eq!(trip.count_label(), "2 of 9 bought");
        assert_eq!(trip.cost_minor, Some(1_000), "two at five hundred");

        let detail = crate::db::purchase_detail(&state.store, trip.id).expect("the trip, in full");
        assert_eq!(
            detail.lines.len(),
            2,
            "the line taken off the sheet is gone"
        );
        let soap = state
            .stock
            .iter()
            .find(|item| item.product_id == 2)
            .expect("soap is still tracked");
        assert_eq!(soap.on_hand, 0, "a line counted at zero adds nothing");
        let tea = state
            .stock
            .iter()
            .find(|item| item.product_id == 3)
            .expect("tea is still tracked");
        assert_eq!(tea.on_hand, 0, "a line taken off the sheet adds nothing");

        let _ = std::fs::remove_file(path);
    }

    /// The dashboard is open to everyone; the scope is what narrows. A cashier
    /// reads their own sales, a manager reads the shop's, and an approval buys
    /// a cashier the shop view for the same minute it buys anything else.
    #[test]
    fn a_report_is_scoped_to_the_person_unless_a_manager_is_here() {
        let (mut state, path) = state("scope", Currency::UGX);
        let cashier = state.session.as_ref().expect("signed in").user_id.clone();

        assert_eq!(state.report_scope(), Scope::Person(cashier.as_str()));
        assert_eq!(state.report_scope().user(), Some(cashier.as_str()));
        assert!(!state.report_scope().is_shop());

        // A manager sees the shop without asking anyone.
        state.session.as_mut().expect("signed in").role = crate::auth::Role::Manager;
        assert_eq!(state.report_scope(), Scope::Shop);
        assert_eq!(state.report_scope().user(), None);

        // A cashier with a manager's approval gets the same view for a minute.
        state.session.as_mut().expect("signed in").role = crate::auth::Role::Cashier;
        state.elevated_until = Some(Instant::now() + std::time::Duration::from_secs(60));
        assert!(state.report_scope().is_shop());

        // With nobody signed in there is no shop to report on.
        state.elevated_until = None;
        state.session = None;
        assert_eq!(state.report_scope(), Scope::Person(""));
        assert_eq!(state.report_scope().user(), Some(""));

        let _ = std::fs::remove_file(path);
    }

    /// The edit field shows an amount without its symbol and the save path
    /// parses it back. If the two ever disagree, an operator opens a product,
    /// presses Save without touching anything, and the price changes.
    #[test]
    fn plain_amount_round_trips_through_parse() {
        for currency in [Currency::UGX, Currency::KES] {
            for minor in [0, 1, 100, 2_800, 12_345] {
                let text = plain_amount(minor, currency);
                let parsed = Money::parse(&text, currency)
                    .unwrap_or_else(|error| panic!("{currency:?} {minor} -> {text:?}: {error}"));
                assert_eq!(
                    parsed.minor(),
                    minor,
                    "{currency:?} {minor} formatted as {text:?}"
                );
            }
        }
    }

    /// The case the whole rounding path exists for: 340 shillings is not a
    /// tendered amount, 300 is.
    #[test]
    fn the_due_rounds_to_the_smallest_coin() {
        let (mut state, path) = state("rounding", Currency::UGX);

        state.cart = vec![line("Sugar", 1, 340)];
        assert_eq!(
            state.cart_total_minor(),
            340,
            "the exact total is untouched"
        );
        assert_eq!(state.cart_rounding_minor(), -40);
        assert_eq!(state.due_minor(), 300);

        // An exact multiple does not move at all.
        state.cart = vec![line("Sugar", 1, 300)];
        assert_eq!(state.cart_rounding_minor(), 0);
        assert_eq!(state.due_minor(), 300);

        // A currency with a one-cent coin never rounds, so a card payment in
        // dollars is never a cent short.
        state.currency = Currency::USD;
        state.cart = vec![line("Sugar", 1, 340)];
        assert_eq!(state.cart_rounding_minor(), 0);
        assert_eq!(state.due_minor(), 340);

        let _ = std::fs::remove_file(path);
    }

    /// Setting five of something should be one key and one Set, not four taps
    /// on plus. The prefilled count is replaced by the first key so the cashier
    /// does not have to clear a `1` before typing `5`.
    #[test]
    fn a_cart_quantity_can_be_typed_directly() {
        let (mut state, path) = state("cart-quantity", Currency::UGX);
        state.cart = vec![line("Sugar", 1, 340)];

        state.edit_cart_quantity(0);
        let editor = state.cart_quantity.as_mut().expect("editor open");
        assert_eq!(editor.digits, "1");
        editor.push_digit('5');
        assert_eq!(editor.digits, "5", "the first key replaces the count");
        editor.push_digit('0');
        assert_eq!(editor.value(), Some(50), "later keys append");

        assert_eq!(state.cart[0].qty, 1, "nothing changes until Set");
        state.set_cart_quantity(1, 1, 50);
        assert_eq!(state.cart[0].qty, 50);
        assert!(state.cart_quantity.is_none(), "Set closes the editor");

        let _ = std::fs::remove_file(path);
    }

    /// Zero is not a quantity. Refusing it must leave the editor open so the
    /// cashier can correct a mistyped count instead of losing the row.
    #[test]
    fn a_zero_cart_quantity_is_refused() {
        let (mut state, path) = state("cart-quantity-zero", Currency::UGX);
        state.cart = vec![line("Sugar", 1, 340)];

        state.edit_cart_quantity(0);
        let editor = state.cart_quantity.as_mut().expect("editor open");
        editor.clear();
        editor.push_digit('0');
        assert_eq!(editor.value(), None);

        state.set_cart_quantity(1, 1, 0);
        assert_eq!(state.cart[0].qty, 1);
        assert!(
            state.cart_quantity.is_some(),
            "the editor stays open for a correction"
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_sale_empties_the_cart_and_closes_the_payment() {
        let (mut state, path) = state("paid", Currency::UGX);
        state.cart = vec![line("Sugar", 1, 340), line("Soap", 2, 500)];

        state.start_payment();
        state.payment.as_mut().expect("open").tendered = "2000".to_owned();
        state.confirm_payment();

        assert!(
            state.payment.is_none(),
            "the tender panel closes with the sale"
        );
        assert!(state.cart.is_empty(), "the till is ready for the next sale");

        // The cart is already empty, so the sale is read back from the ledger.
        let sale_id = crate::db::sales(&state.store, 3, 10, None)[0].id;
        let detail = crate::db::sale_detail(&state.store, sale_id).expect("the sale was recorded");
        assert_eq!(detail.total_minor, 1_340);
        assert_eq!(detail.rounding_minor, -40, "1340 rounds down to 1300");
        assert_eq!(detail.paid_minor, 1_300);
        assert_eq!(detail.change_minor, 700);
        assert_eq!(detail.lines.len(), 2);
        assert_eq!(detail.lines[0].name, "Sugar");

        assert_eq!(crate::db::totals(&state.store, 3, None), (1, 1_300));
        // The sale carries the person, not the shop: a till with three cashiers
        // has to be able to say which of them took this money.
        assert_eq!(
            crate::db::sales(&state.store, 3, 10, None)[0]
                .cashier
                .as_deref(),
            Some("Test cashier")
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_empty_cash_field_means_the_exact_amount() {
        let (mut state, path) = state("exact", Currency::UGX);
        state.cart = vec![line("Sugar", 1, 300)];

        state.start_payment();
        state.confirm_payment();

        assert!(state.payment.is_none());
        let sale_id = crate::db::sales(&state.store, 3, 10, None)[0].id;
        let detail = crate::db::sale_detail(&state.store, sale_id).expect("the sale was recorded");
        assert_eq!(detail.tendered_minor, 300);
        assert_eq!(detail.change_minor, 0);

        let _ = std::fs::remove_file(path);
    }

    /// A mistyped tender is the most common mistake at a till. It must cost one
    /// field, not the basket.
    #[test]
    fn a_short_tender_keeps_the_cart_and_writes_nothing() {
        let (mut state, path) = state("short", Currency::UGX);
        state.cart = vec![line("Sugar", 1, 340)];

        state.start_payment();
        state.payment.as_mut().expect("open").tendered = "200".to_owned();
        state.confirm_payment();

        let payment = state.payment.as_ref().expect("still open");
        assert_eq!(payment.error.as_deref(), Some("That is USh 100 short."));
        assert_eq!(state.cart.len(), 1, "the basket is untouched");
        assert_eq!(crate::db::totals(&state.store, 3, None), (0, 0));

        // Correcting the field completes the sale.
        state.payment.as_mut().expect("open").tendered = "500".to_owned();
        state.confirm_payment();
        assert!(state.payment.is_none());
        assert!(state.cart.is_empty());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cancelling_a_payment_keeps_the_cart() {
        let (mut state, path) = state("cancel", Currency::UGX);
        state.cart = vec![line("Sugar", 1, 340)];

        state.start_payment();
        state.close_payment();

        assert!(state.payment.is_none());
        assert_eq!(state.cart.len(), 1);
        assert_eq!(crate::db::totals(&state.store, 3, None), (0, 0));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn pay_does_nothing_on_an_empty_cart() {
        let (mut state, path) = state("empty-pay", Currency::UGX);

        state.start_payment();
        assert!(state.payment.is_none());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn tapping_a_listed_sale_opens_its_receipt() {
        let (mut state, path) = state("sale-open", Currency::UGX);
        state.cart = vec![line("Sugar", 1, 400)];

        state.start_payment();
        state.confirm_payment();

        let id = crate::db::sales(&state.store, 3, 10, None)[0].id;
        state.open_sale(id);

        let detail = state.sale_open.as_ref().expect("the receipt opened");
        assert_eq!(detail.id, id);
        assert_eq!(detail.lines.len(), 1);
        assert_eq!(detail.lines[0].name, "Sugar");

        state.close_sale();
        assert!(state.sale_open.is_none());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_unnamed_category_lands_in_general() {
        let (mut state, path) = state("general", Currency::UGX);

        state.product_editor = Some(ProductDraft {
            id: None,
            name: "Hot water".to_owned(),
            price: "100".to_owned(),
            category: "   ".to_owned(),
            category_id: None,
            barcode: String::new(),
            unit_label: "unit".to_owned(),
            pack_size: "1".to_owned(),
            pack_price: String::new(),
            pack_barcode: String::new(),
            derived_price_minor: None,
            price_typed: true,
            tracked: false,
            perishable: false,
            shelf_life_days: String::new(),
            error: None,
        });
        state.save_product();

        assert!(state.product_editor.is_none(), "the form closed");
        let saved = state
            .products
            .iter()
            .find(|product| product.name == "Hot water")
            .expect("saved with its default category");
        assert_eq!(saved.category, "General");

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_supplier_needs_a_name_and_survives_archive() {
        let (mut state, path) = state("suppliers", Currency::UGX);

        state.start_supplier();
        state.save_supplier();
        assert!(
            state.supplier_editor.is_some(),
            "a blank name keeps the form open"
        );
        assert_eq!(
            state
                .supplier_editor
                .as_ref()
                .and_then(|draft| draft.error.as_deref()),
            Some("A supplier needs a name.")
        );

        if let Some(draft) = state.supplier_editor.as_mut() {
            draft.name = "  Acme  ".to_owned();
            draft.contact_name = "Amina".to_owned();
            draft.phone = "+256 700".to_owned();
        }
        state.save_supplier();

        assert!(state.supplier_editor.is_none(), "the form closed");
        assert_eq!(state.suppliers.len(), 1);
        assert_eq!(state.suppliers[0].name, "Acme", "the name is trimmed");
        assert_eq!(state.suppliers[0].contact_name.as_deref(), Some("Amina"));
        let id = state.suppliers[0].id.clone();

        state.start_supplier();
        if let Some(draft) = state.supplier_editor.as_mut() {
            draft.name = "acme".to_owned();
        }
        state.save_supplier();
        assert!(
            state.supplier_editor.is_some(),
            "a duplicate name keeps the form open"
        );
        assert_eq!(
            state
                .supplier_editor
                .as_ref()
                .and_then(|draft| draft.error.as_deref()),
            Some(
                "Another supplier already uses that name. Show archived suppliers if it is hidden."
            )
        );

        state.set_supplier_archived(&id, true);
        assert!(state.suppliers[0].archived);

        state.set_supplier_archived(&id, false);
        assert!(!state.suppliers[0].archived);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn category_suggestions_lead_with_the_catalog_and_ignore_case() {
        let catalog = vec!["Drinks".to_owned()];
        let suggestions = category_suggestions(&catalog, "drinks ");

        // "Drinks" holds the field already, so the catalog is skipped, "Pantry"
        // style names never invented here, and the starter list follows in full.
        assert!(
            !suggestions
                .iter()
                .any(|name| name.eq_ignore_ascii_case("Drinks"))
        );
        assert!(suggestions.len() >= 2);
        assert!(
            suggestions[1..]
                .iter()
                .all(|name| STARTER_CATEGORIES.contains(&name.as_str()))
        );

        let catalog = vec!["Pantry".to_owned()];
        let suggestions = category_suggestions(&catalog, "");
        assert_eq!(
            suggestions[0], "Pantry",
            "a catalog name comes before the studio list"
        );
    }

    #[test]
    fn an_import_saves_what_parses_and_counts_the_rest() {
        let (mut state, path) = state("import", Currency::UGX);

        let csv = std::env::temp_dir().join(format!("pos-import-{}.csv", std::process::id()));
        std::fs::write(&csv, "name,category,price,barcode\nSoap 250g,Household,1200,111\nSoap lite,Household,free\nMilo tin,Pantry,9000,111").expect("write csv");
        state.start_import();
        state.import.as_mut().expect("open").path = csv.display().to_string();
        state.import_products();

        assert!(state.import.is_none(), "a partial import still closes");
        assert_eq!(state.products.len(), 2, "one seeded, one imported");
        let soap = state
            .products
            .iter()
            .find(|product| product.name == "Soap 250g")
            .expect("the well-formed row landed");
        assert_eq!(soap.barcode.as_deref(), Some("111"));
        assert_eq!(
            state.notice.as_deref(),
            Some("Imported 1 product, skipped 2.")
        );

        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(csv);
    }

    #[test]
    fn a_chosen_path_means_the_same_from_either_chooser() {
        let (mut state, path) = state("finish-ask", Currency::UGX);

        // An import: the file is read and its folder remembered, exactly as it
        // was when the in-app picker answered the same question.
        let folder = std::env::temp_dir().join(format!("pos-finish-{}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("folder");
        let csv = folder.join("catalog.csv");
        std::fs::write(
            &csv,
            "name,category,price,barcode\nSoap 250g,Household,1200,111\n",
        )
        .expect("write csv");

        state.finish_ask(crate::picker::Purpose::Import, csv);

        assert!(
            state.import.is_none(),
            "the import ran and its modal closed"
        );
        assert_eq!(state.import_dir.as_deref(), Some(folder.as_path()));
        assert_eq!(state.products.len(), 2, "one seeded, one imported");

        // An export: the folder is remembered and the file lands in it.
        state.finish_ask(
            crate::picker::Purpose::Export(crate::picker::ExportKind::AllProducts),
            folder.clone(),
        );

        assert_eq!(state.export_dir.as_deref(), Some(folder.as_path()));
        let written = std::fs::read_dir(&folder)
            .expect("folder")
            .flatten()
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "csv"))
            .count();
        assert_eq!(written, 2, "the imported catalog and the export beside it");

        let _ = std::fs::remove_dir_all(&folder);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_import_that_lands_nothing_keeps_the_modal_and_gives_the_reason() {
        let (mut state, path) = state("import-err", Currency::UGX);

        let csv = std::env::temp_dir().join(format!("pos-import-empty-{}.csv", std::process::id()));
        std::fs::write(&csv, "name,category,price\n").expect("write csv");
        state.start_import();
        state.import.as_mut().expect("open").path = csv.display().to_string();
        state.import_products();

        let error = state
            .import
            .as_ref()
            .expect("still open")
            .error
            .as_deref()
            .expect("a reason");
        assert_eq!(
            error,
            "Nothing was imported: the file has no products in it."
        );

        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(csv);
    }

    #[test]
    fn tracking_and_an_adjustment_go_through_the_state_as_the_screen_does() {
        let (mut state, path) = state("stock-track", Currency::UGX);

        state.start_tracking();
        {
            let draft = state.stock_editor.as_mut().expect("open");
            draft.product_id = Some(1);
            draft.opening = "12".to_owned();
            draft.low_stock = "3".to_owned();
        }
        state.save_tracking();

        assert!(state.stock_editor.is_none(), "a saved form closes");
        assert_eq!(state.stock.len(), 1);
        assert_eq!(state.stock[0].on_hand, 12);
        assert_eq!(state.stock[0].low_stock, 3);

        // An adjustment asks for the count; the state stores the difference.
        state.start_stock_move(1, "Test item", 12, crate::db::StockKind::Adjust);
        {
            let draft = state.stock_move.as_mut().expect("open");
            draft.quantity = "9".to_owned();
            draft.note = "Counted".to_owned();
        }
        state.save_stock_move();

        assert!(state.stock_move.is_none());
        assert_eq!(state.stock[0].on_hand, 9);

        state.open_stock_history(1, "Test item");
        let history = state.stock_history.as_ref().expect("history");
        assert_eq!(history.movements.len(), 2);
        assert_eq!(history.movements[0].delta, -3);
        assert_eq!(history.movements[0].note.as_deref(), Some("Counted"));

        // The catalog is the source of the name Inventory shows, and archiving a
        // product takes it out of both lists. Both changes have to reach the
        // lists without a restart.
        let product = state.products[0].clone();
        state.edit_product(&product);
        state.product_editor.as_mut().expect("open").name = "Renamed item".to_owned();
        state.save_product();
        assert_eq!(state.stock[0].name, "Renamed item");

        state.delete_product(1);
        assert!(state.products.is_empty());
        assert!(state.stock.is_empty());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_adjustment_that_changes_nothing_keeps_the_form_and_explains() {
        let (mut state, path) = state("stock-zero", Currency::UGX);

        state.start_tracking();
        {
            let draft = state.stock_editor.as_mut().expect("open");
            draft.product_id = Some(1);
            draft.opening = "5".to_owned();
        }
        state.save_tracking();

        state.start_stock_move(1, "Test item", 5, crate::db::StockKind::Adjust);
        state.stock_move.as_mut().expect("open").quantity = "5".to_owned();
        state.save_stock_move();

        let draft = state.stock_move.as_ref().expect("the form stays open");
        assert_eq!(
            draft.error.as_deref(),
            Some("That matches what the till already has.")
        );
        assert_eq!(state.stock[0].on_hand, 5);

        let _ = std::fs::remove_file(path);
    }

    /// A phone sell is a keyboard scan without the keyboard: it fills the same
    /// search field and asks the grid to submit it. If the routing skipped that,
    /// the phone would leave a barcode on the screen and nothing in the cart.
    #[test]
    fn a_phone_sell_fills_the_scan_field_and_asks_it_to_submit() {
        let (mut state, path) = state("phone-sell", Currency::UGX);

        state.receive_scan(ScanEvent {
            barcode: "4006381333931".to_owned(),
            action: ScanAction::Sell,
        });

        assert_eq!(state.module, crate::modules::SELL);
        assert_eq!(state.sell_search, "4006381333931");
        assert!(state.scan_focus);
        assert!(state.scan_submit);

        let _ = std::fs::remove_file(path);
    }

    /// `Add` opens the same form the Products screen opens, with the barcode
    /// already in it, so the operator only has to name and price the item.
    #[test]
    fn a_manager_add_scan_opens_the_product_form_with_the_barcode() {
        let (mut state, path) = state("phone-add", Currency::UGX);
        state.session.as_mut().expect("signed in").role = crate::auth::Role::Manager;

        state.receive_scan(ScanEvent {
            barcode: " ABC-123 ".to_owned(),
            action: ScanAction::Add,
        });

        assert_eq!(state.module, crate::modules::PRODUCTS);
        let draft = state.product_editor.as_ref().expect("the form opens");
        assert_eq!(draft.barcode, " ABC-123 ");

        let _ = std::fs::remove_file(path);
    }

    /// A cashier cannot change the catalog, so an add scan must say who can
    /// rather than opening a form whose Save would be refused later.
    #[test]
    fn a_cashier_add_scan_is_refused_with_an_explanation() {
        let (mut state, path) = state("phone-add-refused", Currency::UGX);

        state.receive_scan(ScanEvent {
            barcode: "4006381333931".to_owned(),
            action: ScanAction::Add,
        });

        assert_eq!(state.module, crate::modules::SELL);
        assert!(state.product_editor.is_none());
        assert_eq!(
            state.notice.as_deref(),
            Some("A manager must add products from a phone scan.")
        );

        let _ = std::fs::remove_file(path);
    }

    /// A pack and a loose tablet are one product but two lines: adding the box
    /// must not merge into the loose row, and editing one count leaves the
    /// other alone.
    #[test]
    fn a_pack_and_a_loose_unit_are_two_cart_lines() {
        let (mut state, path) = state("cart-pack", Currency::UGX);
        let id = seed_pack(&state.store, "Amoxicillin", 30, 100, Some(2_800));
        state.reload_products();
        let product = state
            .products
            .iter()
            .find(|product| product.id == id)
            .expect("the pack product")
            .clone();

        state.add_to_cart(&product, 1);
        state.add_to_cart(&product, 100);
        state.add_to_cart(&product, 100);

        assert_eq!(state.cart.len(), 2, "loose and sealed do not merge");
        let loose = &state.cart[0];
        assert_eq!((loose.qty, loose.unit_factor), (1, 1));
        assert_eq!(loose.unit_minor, 30);
        assert_eq!(loose.unit_label.as_deref(), Some("tablet"));
        let pack = &state.cart[1];
        assert_eq!((pack.qty, pack.unit_factor), (2, 100));
        assert_eq!(pack.unit_minor, 2_800);
        assert_eq!(pack.unit_label.as_deref(), Some("pack of 100"));
        assert_eq!(pack.total_minor(), 5_600);

        state.set_cart_quantity(id, 100, 3);
        assert_eq!(state.cart[1].qty, 3, "the box count changed");
        assert_eq!(state.cart[0].qty, 1, "the loose count did not");

        let _ = std::fs::remove_file(path);
    }

    /// A box with no price cannot be sold: charging the tablet price for a
    /// hundred tablets would be a giveaway, and guessing would be worse.
    #[test]
    fn a_pack_without_a_price_is_refused() {
        let (mut state, path) = state("cart-pack-unpriced", Currency::UGX);
        let id = seed_pack(&state.store, "Amoxicillin", 30, 100, None);
        state.reload_products();
        let product = state
            .products
            .iter()
            .find(|product| product.id == id)
            .expect("the pack product")
            .clone();

        state.add_to_cart(&product, 100);

        assert!(state.cart.is_empty());
        assert!(
            state
                .notice
                .as_deref()
                .is_some_and(|notice| notice.contains("no pack price")),
            "{:?}",
            state.notice
        );

        let _ = std::fs::remove_file(path);
    }

    /// The pack price pre-fills the tablet price rounded up, keeps following it
    /// while untouched, and stops the moment the operator types their own.
    #[test]
    fn the_pack_price_helper_yields_to_a_typed_price() {
        let mut draft = ProductDraft::new("Medicine".to_owned(), None);
        draft.unit_label = "tablet".to_owned();
        draft.pack_size = "100".to_owned();
        draft.pack_price = "2800".to_owned();

        draft.sync_price_from_pack(Currency::UGX);
        assert_eq!(draft.price, "28", "2,800 / 100");

        draft.pack_price = "2900".to_owned();
        draft.sync_price_from_pack(Currency::UGX);
        assert_eq!(draft.price, "29", "the helper still owns the field");

        draft.note_price_typed();
        draft.price = "35".to_owned();
        draft.pack_price = "5000".to_owned();
        draft.sync_price_from_pack(Currency::UGX);
        assert_eq!(draft.price, "35", "a typed price is the operator's");
    }

    /// Clearing the price to retype it must not let the helper refill the box;
    /// the next keystroke would land after the helper's digits.
    #[test]
    fn a_cleared_price_stays_cleared() {
        let mut draft = ProductDraft::new("Medicine".to_owned(), None);
        draft.pack_size = "100".to_owned();
        draft.pack_price = "2800".to_owned();

        draft.sync_price_from_pack(Currency::UGX);
        assert_eq!(draft.price, "28");

        draft.note_price_typed();
        draft.price.clear();
        draft.sync_price_from_pack(Currency::UGX);

        assert_eq!(draft.price, "");
    }

    /// Rounding up keeps the loose sale from losing money on the box: 2,801
    /// over 100 is 28.01, and selling at 28 would give the box away.
    #[test]
    fn the_pack_price_helper_rounds_up_not_down() {
        let mut draft = ProductDraft::new("Medicine".to_owned(), None);
        draft.pack_size = "100".to_owned();
        draft.pack_price = "2801".to_owned();

        draft.sync_price_from_pack(Currency::UGX);

        assert_eq!(draft.price, "29");
    }
}
