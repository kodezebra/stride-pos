//! Staff identity: who is on the till, what they may do, and how they prove it.
//!
//! This is a shop's staff list, not an account system: three roles, one
//! credential each, and no network. A PIN is not a password for a service — it
//! is the local proof that the person the drawer will be attributed to is the
//! person standing there.
//!
//! The lock screen is not a modal over the workspace. A modal leaves yesterday's
//! takings legible behind it, and a till is read from both sides of the counter.
//!
//! Two rules make the rest of this file small:
//!
//! - **The owner is the back office.** The owner account can do everything, but
//!   an owner who works the till should add a second, cashier record: that is
//!   what keeps "who took this money" a fact, and the password off the counter.
//! - **Approval is not a second login.** A cashier who needs a manager's
//!   permission gets a prompt inside the action, and the elevation lasts a
//!   minute. Signing in as the manager would attribute the sale to them.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};

use ui_kit::components;
use ui_kit::tokens::{SPACING, TYPE};
use ui_shell::egui;
use ui_shell::regions;

use crate::db;
use crate::state::State;

// ---------------------------------------------------------------------------
// Roles and permissions
// ---------------------------------------------------------------------------

/// What a staff member is allowed to do, in ascending order of authority.
///
/// `Ord` is load-bearing: "a manager or above" is `role >= Role::Manager`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Cashier,
    Manager,
    Owner,
}

impl Role {
    pub const ALL: [Role; 3] = [Role::Cashier, Role::Manager, Role::Owner];

    /// The value stored in `users.role`. Stable: it is in a shipped migration.
    pub fn key(self) -> &'static str {
        match self {
            Role::Cashier => "cashier",
            Role::Manager => "manager",
            Role::Owner => "owner",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Role::ALL.into_iter().find(|role| role.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            Role::Cashier => "Cashier",
            Role::Manager => "Manager",
            Role::Owner => "Owner",
        }
    }

    /// The credential shape a new account of this role gets. Not a security
    /// boundary — the hash is the same either way — but the owner's password is
    /// typed rarely and should survive a shoulder, while a PIN is typed a
    /// hundred times a day and must not.
    pub fn credential_kind(self) -> CredentialKind {
        match self {
            Role::Owner => CredentialKind::Password,
            Role::Cashier | Role::Manager => CredentialKind::Pin,
        }
    }

    pub fn permits(self, permission: Permission) -> bool {
        match permission {
            Permission::Sell | Permission::TakePayment | Permission::Lock => true,
            Permission::VoidSale
            | Permission::Refund
            | Permission::Discount
            | Permission::ChangePrice
            | Permission::OpenDrawer
            | Permission::EditSettings
            | Permission::ManageSuppliers
            | Permission::ManageInventory => self >= Role::Manager,
            Permission::ManageUsers | Permission::ChangeCurrency => self == Role::Owner,
        }
    }

    /// How long the till waits before locking itself. The owner is trusted with
    /// the back office and is often away from the screen; a cashier is not.
    pub fn idle_timeout(self) -> Duration {
        match self {
            Role::Owner => Duration::from_secs(15 * 60),
            Role::Cashier | Role::Manager => Duration::from_secs(5 * 60),
        }
    }
}

/// One thing a person can do that the till cares about.
///
/// Named after actions, not screens: a permission that changes when a module is
/// renamed is a permission that will be forgotten in one of its call sites.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Permission {
    Sell,
    TakePayment,
    Lock,
    // Voiding, refunding, discounting and opening the drawer have no screen yet.
    // The four are here because the role matrix is written once: a check added
    // later would be a check somebody has to remember to add.
    #[allow(dead_code)]
    VoidSale,
    #[allow(dead_code)]
    Refund,
    #[allow(dead_code)]
    Discount,
    ChangePrice,
    #[allow(dead_code)]
    OpenDrawer,
    EditSettings,
    ManageUsers,
    ManageSuppliers,
    ManageInventory,
    ChangeCurrency,
}

impl Permission {
    /// How the action reads in an approval prompt: "a manager is needed to …".
    pub fn action(self) -> &'static str {
        match self {
            Permission::Sell => "sell",
            Permission::TakePayment => "take payment",
            Permission::Lock => "lock the till",
            Permission::VoidSale => "void a sale",
            Permission::Refund => "refund a sale",
            Permission::Discount => "give a discount",
            Permission::ChangePrice => "change the catalog",
            Permission::OpenDrawer => "open the drawer",
            Permission::EditSettings => "change settings",
            Permission::ManageUsers => "manage staff",
            Permission::ManageSuppliers => "manage suppliers",
            Permission::ManageInventory => "change stock",
            Permission::ChangeCurrency => "change the currency",
        }
    }

    /// Whether a manager can approve this for someone else.
    ///
    /// The owner's powers are not lent out for a minute: adding a staff account
    /// or changing the currency is a decision the owner makes while signed in as
    /// themselves, and the record should say so.
    pub fn approvable(self) -> bool {
        !matches!(self, Permission::ManageUsers | Permission::ChangeCurrency)
    }
}

// ---------------------------------------------------------------------------
// Credentials
// ---------------------------------------------------------------------------

/// The shape of a secret. It decides which keyboard a screen shows, not how the
/// secret is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialKind {
    /// Six digits.
    Pin,
    /// The owner's password: long, and nothing else.
    Password,
}

/// A PIN is exactly this many digits.
pub const PIN_DIGITS: usize = 6;

/// How short the owner's password may be.
///
/// Eight is a floor, not a recommendation. The owner types it rarely and can
/// write it down, and the lockout — five tries, then a wait that doubles — is
/// what stops a guess; length only has to make the guess expensive.
pub const PASSWORD_MIN: usize = 8;

impl CredentialKind {
    pub fn key(self) -> &'static str {
        match self {
            CredentialKind::Pin => "pin",
            CredentialKind::Password => "password",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "pin" => Some(CredentialKind::Pin),
            "password" => Some(CredentialKind::Password),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            CredentialKind::Pin => "PIN",
            CredentialKind::Password => "Password",
        }
    }

    /// What the empty field says, which is also the whole instruction.
    pub fn hint(self) -> &'static str {
        match self {
            CredentialKind::Pin => "6 digits",
            CredentialKind::Password => "Your password",
        }
    }

    /// Length and alphabet only. Composition rules ("one symbol") make people
    /// write the symbol on the monitor; length is what actually costs an
    /// attacker, so length is all this asks for.
    pub fn validate(self, secret: &str) -> Result<(), &'static str> {
        match self {
            CredentialKind::Pin => {
                if secret.len() != PIN_DIGITS || !secret.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err("A PIN is exactly six digits.");
                }
                Ok(())
            }
            CredentialKind::Password => {
                if secret.chars().count() < PASSWORD_MIN {
                    return Err("Use at least 8 characters. Length beats symbols.");
                }
                Ok(())
            }
        }
    }
}

/// Argon2id at the OWASP minimum: 19 MiB, two passes, one lane.
///
/// The parameters live inside the stored PHC string, so raising them later is a
/// change here plus a rehash on the next successful sign-in — not a migration.
const MEMORY_KIB: u32 = 19 * 1024;
const PASSES: u32 = 2;
const LANES: u32 = 1;

fn argon2() -> Argon2<'static> {
    let params = Params::new(MEMORY_KIB, PASSES, LANES, None)
        .expect("the OWASP Argon2id parameters are in range");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0_u8; N];
    // The OS source, not a time seed: a guessable salt or recovery code is worth
    // less than none, and `getrandom` is already in the tree for `uuid`.
    getrandom::fill(&mut bytes).expect("the OS random source is available");
    bytes
}

/// A fresh six-digit PIN, uniform over the whole space.
///
/// The owner never invents a PIN, because an invented one is the shop's name, a
/// date, or 123456. A raw `% 1_000_000` would favour the low end of the range by
/// the size of the remainder, so the tail of the u32 space is thrown away
/// instead.
pub fn generate_pin() -> String {
    const SPACE: u32 = 1_000_000;
    let limit = u32::MAX - (u32::MAX % SPACE);
    loop {
        let value = u32::from_le_bytes(random_bytes::<4>());
        if value < limit {
            return format!("{:0width$}", value % SPACE, width = PIN_DIGITS);
        }
    }
}

/// Hash a secret into a PHC string, which is what the database stores.
pub fn hash(secret: &str) -> anyhow::Result<String> {
    // `password_hash`'s error is not a `std::error::Error` unless that crate is
    // built with `std`, which it is not here: a till that cannot produce a PHC
    // string has one thing to say about it, and this is it.
    let salt = SaltString::encode_b64(&random_bytes::<16>())
        .map_err(|err| anyhow::anyhow!("could not build a salt: {err}"))?;
    Ok(argon2()
        .hash_password(secret.as_bytes(), &salt)
        .map_err(|err| anyhow::anyhow!("could not hash the secret: {err}"))?
        .to_string())
}

/// Check a secret against a stored PHC string.
///
/// A malformed stored hash returns `false` and a log line rather than an error:
/// the only useful answer at the lock screen is "that is not the PIN".
pub fn verify(secret: &str, stored: &str) -> bool {
    match PasswordHash::new(stored) {
        Ok(parsed) => argon2().verify_password(secret.as_bytes(), &parsed).is_ok(),
        Err(error) => {
            tracing::error!(error = %error, "a stored credential is not a PHC string");
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Lockout
// ---------------------------------------------------------------------------

/// Tries before the till starts making the person wait.
pub const FREE_ATTEMPTS: u32 = 5;

const LOCK_BASE_SECONDS: u64 = 30;
const LOCK_MAX_SECONDS: u64 = 15 * 60;

/// How long the account is locked after `failures` wrong secrets, or `None` when
/// there is no lock yet.
///
/// 30 seconds doubling per further failure, capped at a quarter of an hour. A
/// person who has genuinely forgotten will go for the recovery code long before
/// the cap; someone guessing six digits will not get through.
pub fn lock_seconds(failures: u32) -> Option<u64> {
    let extra = failures.checked_sub(FREE_ATTEMPTS)?;
    let seconds = LOCK_BASE_SECONDS << extra.min(5);
    Some(seconds.min(LOCK_MAX_SECONDS))
}

/// Why a sign-in was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Denied {
    /// Wrong secret, and this many failures are on record.
    Wrong { failures: u32 },
    /// Too many failures: wait this long.
    Locked { seconds: u64 },
    /// The account was switched off.
    Inactive,
}

impl Denied {
    /// One sentence for the lock screen, written for the person typing.
    ///
    /// The remaining count is only worth saying once it is short: telling a
    /// cashier "5 tries left" on every typo turns a mistake into a warning.
    pub fn message(self, user: &db::User) -> String {
        match self {
            Denied::Wrong { failures } => match FREE_ATTEMPTS.saturating_sub(failures) {
                0 => "Wrong. One more and the till will make you wait.".to_owned(),
                1 => "Wrong. One try left.".to_owned(),
                _ => format!("Wrong {}.", user.kind.label().to_lowercase()),
            },
            Denied::Locked { seconds } => format!("Too many tries. Wait {seconds}s."),
            Denied::Inactive => "That account is switched off.".to_owned(),
        }
    }
}

/// Seconds since the epoch, which is how `login_attempts` stores a lock.
///
/// Wall-clock on purpose: the lockout is a local policy about a person standing
/// at a counter, and it must survive the app being closed and reopened. It is
/// the one piece of state in the app that is deliberately not monotonic, and it
/// is deliberately not synced.
pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or(0)
}

/// Verify a secret, counting failures and enforcing the lockout.
///
/// The database owns the failure count because the till can be closed between
/// attempts; the policy lives here.
pub fn attempt(store: &storage::Store, user: &db::User, secret: &str) -> Result<(), Denied> {
    if !user.active {
        return Err(Denied::Inactive);
    }

    let now = unix_now();
    let (failures, locked_until) = db::attempts(store, &user.id);
    if let Some(until) = locked_until
        && until > now
    {
        return Err(Denied::Locked {
            seconds: (until - now) as u64,
        });
    }

    if verify(secret, &user.credential) {
        db::clear_attempts(store, &user.id);
        return Ok(());
    }

    let failures = failures.saturating_add(1);
    let lock = lock_seconds(failures);
    db::record_attempt(
        store,
        &user.id,
        failures,
        lock.map(|seconds| now + seconds as i64),
    );

    match lock {
        Some(seconds) => Err(Denied::Locked { seconds }),
        None => Err(Denied::Wrong { failures }),
    }
}

/// The lockout a selected account is under, in seconds, for the lock screen.
pub fn locked_for(store: &storage::Store, user_id: &str) -> Option<u64> {
    let (_, until) = db::attempts(store, user_id);
    let until = until?;
    let now = unix_now();
    (until > now).then(|| (until - now) as u64)
}

// ---------------------------------------------------------------------------
// The recovery code
// ---------------------------------------------------------------------------

/// The `meta` key the recovery code's hash lives under.
///
/// Per install, not per account: the code is about this till, and an owner who
/// buys a second till writes down a second code.
pub const RECOVERY_KEY: &str = "recovery_code";

/// Crockford's base32: no `I`, `L`, `O` or `U`, so a code read out over the
/// phone or copied off a sticky note cannot turn into a different code.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const CODE_BYTES: usize = 16;
const CODE_GROUP: usize = 5;

/// A fresh one-time recovery code, grouped for reading aloud.
///
/// 16 random bytes are 128 bits and 26 base32 characters; the grouping is
/// cosmetic and [`normalise_code`] accepts the code with or without it.
pub fn new_recovery_code() -> String {
    let raw = base32(&random_bytes::<CODE_BYTES>());
    raw.chars()
        .enumerate()
        .flat_map(|(index, ch)| {
            let dash = (index > 0 && index % CODE_GROUP == 0).then_some('-');
            dash.into_iter().chain(std::iter::once(ch))
        })
        .collect()
}

/// Fold a typed code into the form that was hashed.
///
/// Case, spaces and dashes are noise a person introduces; `I`/`L` → `1` and
/// `O` → `0` are the two confusions Crockford's alphabet exists to survive.
pub fn normalise_code(input: &str) -> String {
    input
        .chars()
        .filter_map(|ch| match ch.to_ascii_uppercase() {
            'I' | 'L' => Some('1'),
            'O' => Some('0'),
            ch if ALPHABET.contains(&(ch as u8)) => Some(ch),
            _ => None,
        })
        .collect()
}

fn base32(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(CODE_BYTES * 2);
    let mut buffer: u32 = 0;
    let mut bits = 0;

    for byte in bytes {
        buffer = (buffer << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 0x1f) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 0x1f) as usize] as char);
    }
    out
}

// ---------------------------------------------------------------------------
// The lock screen
// ---------------------------------------------------------------------------

/// The whole screen while nobody is signed in.
pub fn lock_screen(ui: &mut egui::Ui, state: &mut State) {
    let width = 420.0_f32.min(ui.available_width());
    ui.add_space((ui.available_height() * 0.14).max(SPACING.lg));

    ui.horizontal_top(|ui| {
        ui.add_space(((ui.available_width() - width) * 0.5).max(0.0));
        ui.vertical(|ui| {
            ui.set_width(width);
            ui.label(components::semibold(crate::APP_NAME).size(TYPE.heading));
            ui.add_space(SPACING.xs);
            components::caption(ui, "A till for small shops. Local only.");
            ui.add_space(SPACING.lg);

            if state.setup.code.is_some() {
                setup_code_card(ui, state);
            } else if state.users.is_empty() {
                setup_card(ui, state);
            } else if state.recovery.is_some() {
                recovery_card(ui, state);
            } else {
                sign_in_card(ui, state);
            }
        });
    });
}

fn sign_in_card(ui: &mut egui::Ui, state: &mut State) {
    components::card(ui, |ui| {
        ui.set_width(ui.available_width());
        components::section_title(ui, "Who is on the till?");

        let mut pick = None;
        let mut any = false;
        for user in &state.users {
            if !user.active {
                continue;
            }
            any = true;
            let selected = state.sign_in.user_id.as_deref() == Some(user.id.as_str());
            let row = format!("{}  ·  {}", user.name, user.role.label());
            if regions::nav_item(ui, selected, &row).clicked() {
                pick = Some(user.id.clone());
            }
        }
        if let Some(id) = pick {
            state.choose_user(&id);
        }
        if !any {
            components::caption(
                ui,
                "Every account is switched off. Use the owner's password.",
            );
        }

        let kind = state.selected_kind();
        let Some(kind) = kind else {
            ui.add_space(SPACING.md);
            components::caption(ui, "Choose your name.");
            return;
        };

        ui.add_space(SPACING.md);
        components::field_label(ui, kind.label());
        let response =
            crate::modules::secret_field(ui, &mut state.sign_in.secret, kind.hint(), 240.0);
        if state.sign_in.focus {
            response.request_focus();
            state.sign_in.focus = false;
        }

        let entered =
            response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        let locked = state
            .sign_in
            .locked_until
            .is_some_and(|until| until > unix_now());

        if let Some(error) = state.sign_in.error.clone() {
            ui.add_space(SPACING.sm);
            ui.label(
                egui::RichText::new(error)
                    .size(TYPE.small)
                    .color(ui.visuals().error_fg_color),
            );
        } else if let Some(seconds) = locked_seconds(&state.sign_in.locked_until) {
            ui.add_space(SPACING.sm);
            components::caption(ui, &format!("Too many tries. Try again in {seconds}s."));
        }

        // A locked account has a countdown above and nothing to gain from
        // another try, so the button goes quiet until the wait is over.
        ui.add_space(SPACING.md);
        let mut clicked = false;
        ui.add_enabled_ui(!locked, |ui| {
            clicked = components::primary_button(ui, "Sign in").clicked();
        });
        if !locked && (clicked || entered) {
            state.sign_in();
        }

        ui.add_space(SPACING.sm);
        if ui
            .add(egui::Button::new("Forgot the owner password?").frame(false))
            .clicked()
        {
            state.start_recovery();
        }
    });
}

fn setup_card(ui: &mut egui::Ui, state: &mut State) {
    components::card(ui, |ui| {
        ui.set_width(ui.available_width());
        components::section_title(ui, "Set up this till");
        components::caption(
            ui,
            "The owner account is the way back in: it adds staff, changes the \
             currency, and can recover a forgotten password. An owner who also \
             works the counter should add a cashier account, so the password \
             stays in the back office.",
        );

        ui.add_space(SPACING.md);
        components::field_label(ui, "Name");
        let name = crate::modules::text_field(ui, &mut state.setup.name, "e.g. Amina", 280.0);
        if state.setup.focus {
            name.request_focus();
            state.setup.focus = false;
        }

        ui.add_space(SPACING.sm);
        components::field_label(ui, "Password");
        crate::modules::secret_field(
            ui,
            &mut state.setup.password,
            "At least 8 characters",
            280.0,
        );

        ui.add_space(SPACING.sm);
        components::field_label(ui, "Again");
        crate::modules::secret_field(ui, &mut state.setup.confirm, "The same password", 280.0);

        if let Some(error) = &state.setup.error {
            ui.add_space(SPACING.sm);
            ui.label(
                egui::RichText::new(error)
                    .size(TYPE.small)
                    .color(ui.visuals().error_fg_color),
            );
        }

        ui.add_space(SPACING.md);
        if components::primary_button(ui, "Create the owner").clicked() {
            state.create_owner();
        }
    });
}

fn setup_code_card(ui: &mut egui::Ui, state: &mut State) {
    let code = state.setup.code.clone().unwrap_or_default();
    components::card(ui, |ui| {
        ui.set_width(ui.available_width());
        components::section_title(ui, "Write this down");
        components::caption(
            ui,
            "This is the only way back in if the owner's password is forgotten. \
             It is shown once: the till keeps a hash of it, not the code itself.",
        );

        ui.add_space(SPACING.md);
        ui.label(
            egui::RichText::new(&code)
                .size(TYPE.body)
                .monospace()
                .strong(),
        );

        ui.add_space(SPACING.md);
        if components::primary_button(ui, "I have written it down").clicked() {
            state.acknowledge_code();
        }
    });
}

fn recovery_card(ui: &mut egui::Ui, state: &mut State) {
    let Some(recovery) = state.recovery.as_ref() else {
        return;
    };

    // A rotated code is shown before the way back to the lock screen, because
    // the old one stops working the moment it is used.
    if let Some(code) = recovery.new_code.clone() {
        components::card(ui, |ui| {
            ui.set_width(ui.available_width());
            components::section_title(ui, "Your new recovery code");
            components::caption(ui, "The code you just used is spent. This one replaces it.");
            ui.add_space(SPACING.md);
            ui.label(
                egui::RichText::new(&code)
                    .size(TYPE.body)
                    .monospace()
                    .strong(),
            );
            ui.add_space(SPACING.md);
            if components::primary_button(ui, "I have written it down").clicked() {
                state.finish_recovery();
            }
        });
        return;
    }

    let mut code = recovery.code.clone();
    let mut password = recovery.password.clone();
    let mut confirm = recovery.confirm.clone();
    let mut cancel = false;
    let mut submit = false;

    components::card(ui, |ui| {
        ui.set_width(ui.available_width());
        components::section_title(ui, "Recover the owner account");
        components::caption(
            ui,
            "The recovery code is the one shown when this till was set up. It \
             works once, and then it is replaced.",
        );

        ui.add_space(SPACING.md);
        components::field_label(ui, "Recovery code");
        crate::modules::secret_field(ui, &mut code, "XXXXX-XXXXX-XXXXX-XXXXX-XXXXX-X", 320.0);

        ui.add_space(SPACING.sm);
        components::field_label(ui, "New password");
        crate::modules::secret_field(ui, &mut password, "At least 8 characters", 280.0);

        ui.add_space(SPACING.sm);
        components::field_label(ui, "Again");
        crate::modules::secret_field(ui, &mut confirm, "The same password", 280.0);

        if let Some(error) = &recovery.error {
            ui.add_space(SPACING.sm);
            ui.label(
                egui::RichText::new(error)
                    .size(TYPE.small)
                    .color(ui.visuals().error_fg_color),
            );
        }

        ui.add_space(SPACING.md);
        ui.horizontal(|ui| {
            if components::primary_button(ui, "Recover").clicked() {
                submit = true;
            }
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
        });
    });

    if let Some(draft) = state.recovery.as_mut() {
        draft.code = code;
        draft.password = password;
        draft.confirm = confirm;
    }
    if submit {
        state.recover();
    }
    if cancel {
        state.recovery = None;
        state.sign_in.focus = true;
    }
}

/// The seconds left on a lock, as a countdown the screen can print.
fn locked_seconds(until: &Option<i64>) -> Option<u64> {
    let until = (*until)?;
    let now = unix_now();
    (until > now).then(|| (until - now) as u64)
}

// ---------------------------------------------------------------------------
// Manager approval
// ---------------------------------------------------------------------------

/// The prompt a cashier sees when they reach for something above their role.
///
/// It is drawn over the action, not over the app: the manager's PIN approves
/// this one thing for a minute, and the sale stays attributed to the cashier.
pub fn override_modal(ctx: &egui::Context, state: &mut State) {
    if state.override_request.is_none() {
        return;
    }

    let mut cancel = false;
    let mut submit = false;

    crate::modules::modal(ctx, egui::Id::new("pos_override")).show(ctx, |ui| {
        ui.set_width(380.0);
        let (permission, error, opening, mut secret, mut pick) = {
            let request = state.override_request.as_ref().expect("open");
            (
                request.permission,
                request.error.clone(),
                request.focus,
                request.secret.clone(),
                request.user_id.clone(),
            )
        };

        // Two permissions are never lent to a manager, and a screen that offers
        // one of them with "a manager is needed" would send the cashier to the
        // wrong person.
        let who = if permission.approvable() {
            "A manager is needed"
        } else {
            "The owner is needed"
        };
        let body = if permission.approvable() {
            format!(
                "{} cannot {} alone. A manager or the owner can approve one \
                 minute of it; the sale stays in the cashier's name.",
                state
                    .session
                    .as_ref()
                    .map(|session| session.name.as_str())
                    .unwrap_or("This account"),
                permission.action()
            )
        } else {
            format!(
                "{} cannot {}. Only the owner can.",
                state
                    .session
                    .as_ref()
                    .map(|session| session.name.as_str())
                    .unwrap_or("This account"),
                permission.action()
            )
        };

        ui.label(components::semibold(who).size(TYPE.heading));
        components::caption(ui, &body);

        ui.add_space(SPACING.md);
        for user in &state.users {
            if !user.active || !user.role.permits(permission) {
                continue;
            }
            let selected = pick.as_deref() == Some(user.id.as_str());
            let row = format!("{}  ·  {}", user.name, user.role.label());
            if regions::nav_item(ui, selected, &row).clicked() {
                pick = Some(user.id.clone());
            }
        }

        ui.add_space(SPACING.md);
        // The field matches whoever was picked: a manager types a PIN, the owner
        // types a password. Both are secrets, so both are masked.
        let kind = pick
            .as_deref()
            .and_then(|id| state.users.iter().find(|user| user.id == id))
            .map_or(crate::auth::CredentialKind::Pin, |user| user.kind);
        components::field_label(ui, kind.label());
        let response = crate::modules::secret_field(ui, &mut secret, kind.hint(), 200.0);
        if opening {
            response.request_focus();
        }
        let entered =
            response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));

        if let Some(error) = &error {
            ui.add_space(SPACING.sm);
            ui.label(
                egui::RichText::new(error)
                    .size(TYPE.small)
                    .color(ui.visuals().error_fg_color),
            );
        }

        ui.add_space(SPACING.md);
        ui.horizontal(|ui| {
            if components::primary_button(ui, "Approve").clicked() || entered {
                submit = true;
            }
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
        });

        if let Some(request) = state.override_request.as_mut() {
            request.user_id = pick;
            request.secret = secret;
            // Focus is a first-frame thing: after this the field behaves like any
            // other and a click elsewhere must be able to take the caret.
            request.focus = false;
        }
    });

    if submit {
        state.approve_override();
    }
    if cancel {
        state.override_request = None;
    }
}

// ---------------------------------------------------------------------------
// Settings → Staff
// ---------------------------------------------------------------------------

/// The one moment a generated PIN exists in the clear.
///
/// A card, not a toast: a toast is gone before the owner has found a pen, and
/// the till cannot show the digits again — it stores only the hash. The card
/// stays on the Staff page until it is dismissed.
fn new_pin_card(ui: &mut egui::Ui, state: &mut State) {
    let Some((name, pin)) = state
        .new_pin
        .as_ref()
        .map(|new| (new.name.clone(), new.pin.clone()))
    else {
        return;
    };

    let mut done = false;
    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        components::section_title(ui, &format!("PIN for {name}"));
        components::caption(
            ui,
            "Write it down and hand it over. The till keeps only a hash, so this \
             is the last time the digits exist.",
        );

        ui.add_space(SPACING.md);
        ui.horizontal(|ui| {
            ui.label(components::numeric(&pin).size(TYPE.kpi));
            ui.add_space(SPACING.lg);
            if components::primary_button(ui, "Written down").clicked() {
                done = true;
            }
        });
    });
    ui.add_space(SPACING.lg);

    if done {
        state.new_pin = None;
    }
}

/// The staff list, and the two things an owner does here: add a person and
/// switch one off. Credentials are never shown, not even to the owner — the
/// only time a PIN is visible is the moment it is made, above this list.
pub fn staff_card(ui: &mut egui::Ui, state: &mut State) {
    new_pin_card(ui, state);

    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        components::caption(
            ui,
            "Every sale is signed by the person who took the money. Switch an \
             account off rather than deleting it: the history keeps its name.",
        );

        ui.add_space(SPACING.md);
        let mut toggle = None;
        let mut reset = None;
        for user in &state.users {
            ui.horizontal(|ui| {
                ui.label(components::semibold(&user.name).size(TYPE.body));
                components::caption(ui, user.role.label());
                if !user.active {
                    components::caption(ui, "switched off");
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if user.role == Role::Owner {
                        components::caption(ui, "the owner account");
                        return;
                    }
                    if ui
                        .button(if user.active {
                            "Switch off"
                        } else {
                            "Switch on"
                        })
                        .clicked()
                    {
                        toggle = Some((user.id.clone(), !user.active));
                    }
                    if ui.button("Reset PIN").clicked() {
                        reset = Some(user.id.clone());
                    }
                });
            });
            ui.add_space(SPACING.xs);
        }

        if let Some((id, active)) = toggle {
            state.set_staff_active(&id, active);
        }
        if let Some(id) = reset {
            state.start_reset_pin(&id);
        }

        // The reset confirmation lives under the list: one at a time, like the
        // delete confirmation on a product row. The PIN itself is generated, so
        // there is nothing to type — only to agree.
        if let Some(draft) = state.staff_reset.as_ref() {
            let name = state
                .users
                .iter()
                .find(|user| user.id == draft.user_id)
                .map(|user| user.name.as_str())
                .unwrap_or("this account");
            let mut apply = false;
            let mut cancel = false;

            ui.add_space(SPACING.sm);
            ui.horizontal(|ui| {
                components::caption(ui, &format!("Give {name} a new PIN?"));
                if ui.button("New PIN").clicked() {
                    apply = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
            if let Some(error) = &draft.error {
                ui.label(
                    egui::RichText::new(error)
                        .size(TYPE.small)
                        .color(ui.visuals().error_fg_color),
                );
            }

            if apply {
                state.reset_pin();
            }
            if cancel {
                state.staff_reset = None;
            }
        }
    });

    ui.add_space(SPACING.lg);
    components::section_title(ui, "Add a person");

    let mut add = false;
    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        components::caption(
            ui,
            "Cashiers sell and take payment. Managers also void, refund, discount, \
             edit the catalog and change settings. The till makes a six-digit PIN \
             and shows it once.",
        );

        ui.add_space(SPACING.md);
        ui.horizontal(|ui| {
            components::field_label(ui, "Name");
            crate::modules::text_field(ui, &mut state.staff.name, "e.g. Brian", 220.0);
        });

        ui.add_space(SPACING.sm);
        ui.horizontal(|ui| {
            components::field_label(ui, "Role");
            for role in [Role::Cashier, Role::Manager] {
                if components::choice_card(
                    ui,
                    ui_kit::icons::Icon::USER,
                    role.label(),
                    state.staff.role == role,
                    120.0,
                )
                .clicked()
                {
                    state.staff.role = role;
                }
            }
        });

        if let Some(error) = &state.staff.error {
            ui.add_space(SPACING.sm);
            ui.label(
                egui::RichText::new(error)
                    .size(TYPE.small)
                    .color(ui.visuals().error_fg_color),
            );
        }

        ui.add_space(SPACING.md);
        if components::primary_button(ui, "Add").clicked() {
            add = true;
        }
    });
    if add {
        state.add_staff();
    }

    ui.add_space(SPACING.lg);
    components::section_title(ui, "Owner password");
    password_card(ui, state);
}

/// Change the owner's own password. No "current password" field: the only way
/// to reach this card is already being signed in as the owner.
fn password_card(ui: &mut egui::Ui, state: &mut State) {
    let mut apply = false;
    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        components::caption(
            ui,
            &format!("At least {PASSWORD_MIN} characters. The recovery code is not changed."),
        );

        ui.add_space(SPACING.md);
        ui.horizontal(|ui| {
            components::field_label(ui, "New");
            crate::modules::secret_field(
                ui,
                &mut state.owner_password.password,
                "At least 8 characters",
                240.0,
            );
            ui.add_space(SPACING.sm);
            components::field_label(ui, "Again");
            crate::modules::secret_field(
                ui,
                &mut state.owner_password.confirm,
                "The same password",
                240.0,
            );
        });

        if let Some(error) = &state.owner_password.error {
            ui.add_space(SPACING.sm);
            ui.label(
                egui::RichText::new(error)
                    .size(TYPE.small)
                    .color(ui.visuals().error_fg_color),
            );
        }

        ui.add_space(SPACING.md);
        if components::primary_button(ui, "Change password").clicked() {
            apply = true;
        }
    });
    if apply {
        state.change_owner_password();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_round_trips_through_its_hash() {
        let stored = hash("482913").expect("hash");
        assert!(
            stored.starts_with("$argon2id$"),
            "a PHC string, not a bare hash"
        );
        assert!(verify("482913", &stored));
        assert!(!verify("482914", &stored));
        // Two hashes of the same secret differ: the salt is doing its job.
        assert_ne!(stored, hash("482913").expect("hash"));
    }

    #[test]
    fn a_broken_stored_hash_is_a_refusal_not_a_crash() {
        assert!(!verify("482913", "not-a-phc-string"));
    }

    #[test]
    fn the_lockout_starts_at_five_and_doubles() {
        for failures in 0..FREE_ATTEMPTS {
            assert_eq!(lock_seconds(failures), None, "{failures} failures");
        }
        assert_eq!(lock_seconds(5), Some(30));
        assert_eq!(lock_seconds(6), Some(60));
        assert_eq!(lock_seconds(7), Some(120));
        // The cap keeps a forgotten PIN from locking the shop out for a day.
        assert_eq!(lock_seconds(50), Some(LOCK_MAX_SECONDS));
    }

    #[test]
    fn a_recovery_code_read_aloud_survives_the_confusions() {
        let code = new_recovery_code();
        assert_eq!(normalise_code(&code).len(), 26, "{code}");
        // Lower case, spaces, and the two pairs Crockford's alphabet folds.
        assert_eq!(normalise_code(&code.to_lowercase()), normalise_code(&code));
        assert_eq!(normalise_code("IL0O"), "1100");
        assert_eq!(normalise_code(""), "");
        // A code with its dashes removed is the same code.
        assert_eq!(
            normalise_code(&code.replace('-', " ")),
            normalise_code(&code)
        );
    }

    #[test]
    fn the_owner_is_the_only_role_with_users_and_currency() {
        for role in Role::ALL {
            assert!(role.permits(Permission::Sell), "{role:?} can sell");
            assert_eq!(
                role.permits(Permission::ManageUsers),
                role == Role::Owner,
                "{role:?} and staff accounts"
            );
        }
        assert!(!Role::Cashier.permits(Permission::ChangePrice));
        assert!(Role::Manager.permits(Permission::ChangePrice));
        assert!(!Role::Cashier.permits(Permission::ManageSuppliers));
        assert!(Role::Manager.permits(Permission::ManageSuppliers));
        // Cashiers must be able to see stock; only a manager may change it.
        assert!(!Role::Cashier.permits(Permission::ManageInventory));
        assert!(Role::Manager.permits(Permission::ManageInventory));
        // Nothing above a manager's authority is ever lent out.
        assert!(!Permission::ManageUsers.approvable());
        assert!(!Permission::ChangeCurrency.approvable());
        assert!(Permission::ChangePrice.approvable());
        assert!(Permission::ManageSuppliers.approvable());
        assert!(Permission::ManageInventory.approvable());
    }

    #[test]
    fn a_pin_is_six_digits_and_a_password_is_long() {
        assert!(CredentialKind::Pin.validate("123456").is_ok());
        assert!(CredentialKind::Pin.validate("12345").is_err());
        assert!(CredentialKind::Pin.validate("12345a").is_err());
        assert!(
            CredentialKind::Password
                .validate("correct horse battery")
                .is_ok()
        );
        // The floor is eight characters, exactly.
        assert!(
            CredentialKind::Password
                .validate(&"x".repeat(PASSWORD_MIN))
                .is_ok()
        );
        assert!(
            CredentialKind::Password
                .validate(&"x".repeat(PASSWORD_MIN - 1))
                .is_err()
        );
    }

    #[test]
    fn a_generated_pin_is_a_pin() {
        let pins: Vec<String> = (0..16).map(|_| generate_pin()).collect();
        for pin in &pins {
            assert_eq!(pin.len(), PIN_DIGITS, "{pin}");
            assert!(pin.bytes().all(|byte| byte.is_ascii_digit()), "{pin}");
            // The generator's output has to pass the rule the screen applies.
            assert!(CredentialKind::Pin.validate(pin).is_ok(), "{pin}");
        }
        // Not a proof of randomness, only that the source is not constant.
        assert!(
            pins.iter().any(|pin| pin != &pins[0]),
            "every draw was {}",
            pins[0]
        );
    }

    #[test]
    fn a_cashier_locks_sooner_than_the_owner() {
        assert!(Role::Cashier.idle_timeout() < Role::Owner.idle_timeout());
        assert_eq!(Role::Cashier.idle_timeout(), Role::Manager.idle_timeout());
    }
}
