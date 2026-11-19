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

use std::path::PathBuf;
use std::time::{Duration, Instant};

use money::{Currency, Money};
use ui_kit::theme::Theme;
use ui_kit::tokens::Rgb;
use ui_shell::egui;

use crate::db::{Product, SaleLine, StockItem, Supplier};
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

/// How long an approval lasts. Long enough to finish the action that asked for
/// it, short enough that it cannot be handed to the next person.
const ELEVATION: Duration = Duration::from_secs(60);

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

    // -----------------------------------------------------------------------
    // Who is at the till
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // Stock
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // Purchases
    // -----------------------------------------------------------------------

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
