//! Staff and access: who is signed in, what they may open, and the PIN paths.
//!
//! Sign-in, the recovery code, owner setup, overrides and PIN resets are one
//! story told in order — a shop starts with setup, signs staff in, and
//! occasionally has to let a manager approve something a cashier cannot. The
//! session, the scope it grants and the error each step shows live together.

use super::*;
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

impl State {
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

    /// Whether the workspace is hidden behind the lock screen.
    ///
    /// Also true while the one-time recovery code is on screen: the code is
    /// shown once, and the takings behind it are none of the next person's
    /// business.
    pub fn locked(&self) -> bool {
        self.session.is_none() || self.setup.code.is_some()
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
}
