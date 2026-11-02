//! Everything the POS screen tree can change, in one place.
//!
//! One struct, passed as `&mut` to whichever module is drawing. Modules keep no
//! state of their own: switching module must not lose the cart, the search box,
//! or the scroll position, and a module that owns data makes that rule hard to
//! keep. It also means a command in the palette can change anything the user can
//! change, without reaching into widgets.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use money::{Currency, Money};
use ui_kit::theme::Theme;
use ui_shell::egui;

use crate::db::{Product, RecordedSale, SaleLine};
use crate::settings::Settings;

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
}

impl CartLine {
    pub fn total_minor(&self) -> i64 {
        self.qty * self.unit_minor
    }
}

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
    pub barcode: String,
    pub error: Option<String>,
}

impl ProductDraft {
    fn new(category: String) -> Self {
        Self {
            id: None,
            name: String::new(),
            price: String::new(),
            category,
            barcode: String::new(),
            error: None,
        }
    }
}

/// Category names offered in the product form even when the catalog is empty.
///
/// They are a sketch of a grocery until, not a schema: suggestions the owner
/// can tap instead of typing "Drniks" twice. Nothing is written to the
/// database until a product actually uses one, so a shop that names its own
/// shelves simply outgrows the list — and a till whose catalog is untouched
/// still filters on nothing, exactly as before.
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
/// and must not advertise themselves as two. `current` is left out — a chip
/// for the name already in the field is noise.
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

/// The CSV import, open as a modal over the app.
///
/// The path is typed or pasted rather than picked from a dialog: importing is
/// a one-time setup job for the owner, and a file-dialog dependency is not
/// worth what it costs a till this small. Rows that cannot be read are
/// skipped and counted; the summary line is the receipt for the import.
pub struct ImportDraft {
    pub path: String,
    pub error: Option<String>,
}

/// The payment screen, open as a modal over the app.
///
/// Cash received is text, for the same reason a price is: it is mid-edit, and
/// parsing on every keystroke would eat a half-typed `5000`. Empty means "the
/// exact amount", which is what most cash sales are.
pub struct Payment {
    pub tendered: String,
    pub error: Option<String>,
    /// Set once the sale is written. The modal then shows the change instead of
    /// the form, so the cashier reads the number before the drawer opens.
    pub done: Option<RecordedSale>,
    /// The same sale read back with its lines and header. `done` drives the
    /// screen; this is what the receipt says, and it can only be filled from
    /// the ledger, never from the cart that has just been emptied.
    pub detail: Option<crate::db::SaleDetail>,
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

    /// Index into [`crate::modules::all`]. Also the activity-bar selection.
    pub module: usize,
    /// Per-module sidebar visibility, so collapsing Sell does not collapse
    /// Settings. Indexed in parallel with the module list.
    pub sidebar_open: Vec<bool>,
    /// Set when something persisted changed; the shell writes `prefs.json` and
    /// clears it.
    pub prefs_dirty: bool,

    // Per-module view state. Kept here rather than in the module structs for the
    // reason in the module doc: switching away and back should not reset them.
    pub search: String,
    /// Set when the selling screen opens, so the next frame can put the cursor in
    /// the scan field. A scanner types into whatever has focus; a till that needs
    /// one click before the first barcode is a till that "doesn't scan".
    pub scan_focus: bool,
    /// The category both Sell and Products filter by. `None` is "All"; the value
    /// is a category name, because the categories are the shop's, not an index
    /// into a list the app invented.
    pub category: Option<String>,
    pub range: usize,
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
    pub picker: Option<crate::picker::Picker>,
    /// The folder the last import came from, so the next one starts there for
    /// the rest of this session.
    pub import_dir: Option<PathBuf>,
    /// The folder the last export went to, for the same reason.
    pub export_dir: Option<PathBuf>,
    /// A product the operator asked to delete, waiting for confirmation. Inline
    /// in the row rather than a modal: the row is where the mistake would be.
    pub product_delete: Option<i64>,

    /// The payment screen, when it is open.
    pub payment: Option<Payment>,

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

    /// What the Settings screen is editing. Committed by [`State::save_shop_name`],
    /// so a half-typed name is never written to disk.
    pub shop_draft: String,

    /// The shop's configuration, loaded once at startup and written back when
    /// something in Settings changes.
    pub settings: Settings,
    /// Where `settings.json` and the database live, so the shell can persist a
    /// change and the owner screens can write backups without knowing about
    /// paths.
    pub paths: Paths,
    /// The theme currently installed on the context.
    pub theme: Theme,
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
        settings: Settings,
        paths: Paths,
        currency: Currency,
        needs_currency: bool,
    ) -> Self {
        let theme = settings.theme();
        let shop_draft = settings.shop_name.trim().to_owned();
        let products = crate::db::products(&store);
        let categories = crate::db::categories(&store);
        let users = crate::db::users(&store);
        Self {
            store,
            cart: Vec::new(),
            module,
            sidebar_open,
            prefs_dirty: false,
            search: String::new(),
            scan_focus: true,
            category: None,
            range: 0,
            settings_section: 0,
            products,
            categories,
            product_editor: None,
            import: None,
            picker: None,
            import_dir: None,
            export_dir: None,
            product_delete: None,
            payment: None,
            sale_open: None,
            notice: None,
            file_notice: None,
            file_path: None,
            backup_confirm: false,
            shop_draft,
            settings,
            paths,
            theme,
            theme_dirty: false,
            settings_dirty: false,
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
        self.product_editor = None;
        self.product_delete = None;
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

    /// Add one unit of a product, merging with an existing line.
    ///
    /// Scanning the same barcode twice should read "2 × Espresso", not two
    /// lines: the cashier checks quantities, not line counts. The line keeps the
    /// name and price it was added at, so editing the catalog mid-sale cannot
    /// silently reprice something already in the basket.
    pub fn add_to_cart(&mut self, product: &Product) {
        self.notice = None;
        match self
            .cart
            .iter_mut()
            .find(|line| line.product_id == product.id)
        {
            Some(line) => line.qty += 1,
            None => self.cart.push(CartLine {
                product_id: product.id,
                name: product.name.clone(),
                qty: 1,
                unit_minor: product.price_minor,
            }),
        }
    }

    /// Re-read the catalog and the category list after a write.
    ///
    /// If the selected category no longer exists (its last product was deleted
    /// or recategorised), the filter falls back to All rather than showing an
    /// empty grid with a category chip that is not in the sidebar.
    pub fn reload_products(&mut self) {
        self.products = crate::db::products(&self.store);
        self.categories = crate::db::categories(&self.store);

        if let Some(selected) = &self.category
            && !self.categories.iter().any(|category| category == selected)
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
        self.product_editor = Some(ProductDraft::new(category));
        self.product_delete = None;
        self.notice = None;
    }

    /// Open the form on an existing product.
    pub fn edit_product(&mut self, product: &Product) {
        self.product_editor = Some(ProductDraft {
            id: Some(product.id),
            name: product.name.clone(),
            price: plain_amount(product.price_minor, self.currency),
            category: product.category.clone(),
            barcode: product.barcode.clone().unwrap_or_default(),
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
        let price_text = draft.price.trim().to_owned();

        if name.is_empty() {
            return self.editor_error("A product needs a name.");
        }
        // An unnamed category is not worth an error: a product whose shelf has
        // no name yet still sells, so it lands in the default one instead of
        // stopping the save.
        let category = if category.is_empty() {
            "General".to_owned()
        } else {
            category
        };

        let price_minor = match Money::parse(&price_text, self.currency) {
            Ok(price) if price.minor() >= 0 => price.minor(),
            Ok(_) => return self.editor_error("A price cannot be negative."),
            Err(error) => return self.editor_error(error.to_string()),
        };

        let barcode = (!barcode.is_empty()).then_some(barcode);
        if let Some(code) = barcode.as_deref()
            && crate::db::barcode_taken(&self.store, code, id)
        {
            return self.editor_error("Another product already uses that barcode.");
        }

        let result = match id {
            Some(id) => crate::db::update_product(
                &self.store,
                id,
                &name,
                price_minor,
                &category,
                barcode.as_deref(),
            ),
            None => crate::db::insert_product(
                &self.store,
                &name,
                price_minor,
                &category,
                barcode.as_deref(),
            )
            .map(|_| ()),
        };

        match result {
            Ok(()) => {
                self.product_editor = None;
                self.reload_products();
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

    /// Add products from a CSV file, as listed in the import draft.
    ///
    /// Everything the parser accepts is written; anything else — a row the
    /// file spells wrongly, a barcode another product already uses — is
    /// skipped and counted. One bad line must not stop the other hundreds,
    /// and a partial import is honest as long as the summary says what it is:
    /// the modal closes on what landed, and the counts name what did not.
    pub fn import_products(&mut self) {
        let Some(path) = self
            .import
            .as_ref()
            .map(|draft| draft.path.trim().to_owned())
        else {
            return;
        };

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
        let mut added = 0;
        let mut skipped = errors.len();

        for row in rows {
            if row
                .barcode
                .as_deref()
                .is_some_and(|code| crate::db::barcode_taken(&self.store, code, None))
            {
                skipped += 1;
                continue;
            }
            if let Err(error) = crate::db::insert_product(
                &self.store,
                &row.name,
                row.price_minor,
                &row.category,
                row.barcode.as_deref(),
            ) {
                // The catalog's own rejects count like the file's: a row the
                // till cannot take does not stop the others.
                tracing::warn!(error = %error, "a CSV row could not be saved");
                skipped += 1;
            } else {
                added += 1;
            }
        }

        if added == 0 {
            let message = match errors.first() {
                Some(error) => format!("Nothing was imported. {error}"),
                None => "Nothing was imported: the file has no products in it.".to_owned(),
            };
            return self.import_error(message);
        }

        let noun = if added == 1 { "product" } else { "products" };
        let summary = if skipped == 0 {
            format!("Imported {added} {noun}.")
        } else {
            format!("Imported {added} {noun}, skipped {skipped}.")
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

    /// Open the typed-path fallback for the CSV import.
    ///
    /// The normal route is [`State::start_import_picker`]; this stays for a
    /// path pasted from an email, and is what the picker offers when a folder
    /// cannot be read.
    pub fn start_import(&mut self) {
        self.import = Some(ImportDraft {
            path: String::new(),
            error: None,
        });
    }

    /// Open the file chooser on the last folder an import came from.
    pub fn start_import_picker(&mut self) {
        let start = crate::picker::start_dir(self.import_dir.as_deref(), &self.paths.data);
        self.picker = Some(crate::picker::Picker::open(
            crate::picker::Purpose::Import,
            start,
            &self.paths.data,
        ));
    }

    /// Open the folder chooser for one of the exports.
    ///
    /// Nothing is written until the operator confirms a folder, so a mis-click
    /// on "Export CSV" can no longer leave a file somewhere unexpected.
    pub fn start_export(&mut self, kind: crate::picker::ExportKind) {
        let start = crate::picker::start_dir(self.export_dir.as_deref(), &self.paths.data);
        self.picker = Some(crate::picker::Picker::open(
            crate::picker::Purpose::Export(kind),
            start,
            &self.paths.data,
        ));
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
        self.notice = None;
    }

    pub fn cart_count(&self) -> i64 {
        self.cart.iter().map(|line| line.qty).sum()
    }

    pub fn cart_total_minor(&self) -> i64 {
        self.cart.iter().map(CartLine::total_minor).sum()
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

    /// Open the payment screen. Does nothing when the cart is empty or the
    /// screen is already open — both are "the operator pressed Pay twice".
    pub fn start_payment(&mut self) {
        if self.payment.is_some() || self.cart.is_empty() {
            return;
        }
        // Taking money is its own permission, separate from filling a basket: a
        // shop can have someone who only ever puts the order together.
        if !self.permits(crate::auth::Permission::TakePayment) {
            return self.ask_override(crate::auth::Permission::TakePayment, None);
        }
        self.notice = None;
        self.payment = Some(Payment {
            tendered: String::new(),
            error: None,
            done: None,
            detail: None,
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
        if payment.done.is_some() {
            return;
        }

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
            })
            .collect();

        match crate::db::record_sale(
            &self.store,
            &lines,
            self.cart_rounding_minor(),
            tendered_minor,
            self.session
                .as_ref()
                .map(|session| session.user_id.as_str()),
        ) {
            Ok(sale) => {
                self.cart.clear();
                // Read the sale back once, here, so the confirmation can print
                // a receipt instead of four remembered numbers. The cart is
                // already empty: this is the only copy of what was sold.
                let detail = crate::db::sale_detail(&self.store, sale.id);
                if let Some(payment) = self.payment.as_mut() {
                    payment.error = None;
                    payment.done = Some(sale);
                    payment.detail = detail;
                    payment.focus = true;
                }
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

/// How a range reads as a section heading above the cards.
pub fn range_caption(range: usize) -> &'static str {
    match range {
        0 => "Today",
        1 => "Last 7 days",
        2 => "Last 30 days",
        _ => "All time",
    }
}

/// Settings sections. Each one is a page that does not exist yet; the list is
/// real so the sidebar has the right shape from the start.
pub const SETTINGS_SECTIONS: &[&str] = &["Terminal", "Receipt", "Tax", "Staff", "Sync"];

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
        crate::db::insert_product(&store, "Test item", 100, "Test", None).expect("seed product");
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
        }
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

    #[test]
    fn a_sale_empties_the_cart_and_reports_the_change() {
        let (mut state, path) = state("paid", Currency::UGX);
        state.cart = vec![line("Sugar", 1, 340), line("Soap", 2, 500)];

        state.start_payment();
        state.payment.as_mut().expect("open").tendered = "2000".to_owned();
        state.confirm_payment();

        let sale = state
            .payment
            .as_ref()
            .and_then(|payment| payment.done.clone())
            .expect("the sale was recorded");
        assert_eq!(sale.total_minor, 1_340);
        assert_eq!(sale.rounding_minor, -40, "1340 rounds down to 1300");
        assert_eq!(sale.paid_minor(), 1_300);
        assert_eq!(sale.change_minor, 700);

        // The confirmation prints the receipt, so the lines have to come back
        // from the ledger — the cart they came from is already empty.
        let detail = state
            .payment
            .as_ref()
            .and_then(|payment| payment.detail.as_ref())
            .expect("the sale was read back");
        assert_eq!(detail.lines.len(), 2);
        assert_eq!(detail.lines[0].name, "Sugar");
        assert_eq!(detail.change_minor, 700);

        assert!(state.cart.is_empty(), "the till is ready for the next sale");
        assert_eq!(crate::db::totals(&state.store, 3), (1, 1_300));
        // The sale carries the person, not the shop: a till with three cashiers
        // has to be able to say which of them took this money.
        assert_eq!(
            crate::db::sales(&state.store, 3, 10)[0].cashier.as_deref(),
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

        let sale = state
            .payment
            .as_ref()
            .and_then(|payment| payment.done.clone())
            .expect("the sale was recorded");
        assert_eq!(sale.tendered_minor, 300);
        assert_eq!(sale.change_minor, 0);

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
        assert!(payment.done.is_none(), "nothing was recorded");
        assert_eq!(payment.error.as_deref(), Some("That is USh 100 short."));
        assert_eq!(state.cart.len(), 1, "the basket is untouched");
        assert_eq!(crate::db::totals(&state.store, 3), (0, 0));

        // Correcting the field completes the sale.
        state.payment.as_mut().expect("open").tendered = "500".to_owned();
        state.confirm_payment();
        assert!(state.payment.as_ref().expect("open").done.is_some());
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
        assert_eq!(crate::db::totals(&state.store, 3), (0, 0));

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

        let id = crate::db::sales(&state.store, 3, 10)[0].id;
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
            barcode: String::new(),
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
}
