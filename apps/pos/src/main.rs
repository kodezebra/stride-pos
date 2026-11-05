//! Stride POS.
//!
//! `main` wires crates together and nothing else. Product logic lives in the
//! modules below it, so this file stays readable as the app grows:
//!
//! | file          | role                                              |
//! |---------------|---------------------------------------------------|
//! | `actions.rs`  | the one action hierarchy: primary, secondary, quiet |
//! | `db.rs`       | schema and the few reads the shell needs           |
//! | `export.rs`   | the books, written out as CSV                      |
//! | `state.rs`    | everything the UI can change, in one struct        |
//! | `modules.rs`  | the modules and their sidebar/workspace             |
//! | `picker.rs`   | choosing a file to import, or a folder to export into |
//! | `auth.rs`     | staff, PINs, the lock screen, approvals            |
//! | `checkout.rs` | the payment modal: cash in, change out             |
//! | `commands.rs` | the command palette's contents                     |
//! | `prefs.rs`    | window/UI habits, persisted across restarts        |
//! | `receipt.rs`  | the receipt, shared by the confirmation and Sales  |
//! | `scan.rs`     | the phone scanner: pairing, token, LAN listener    |
//! | `settings.rs` | terminal configuration: appearance, later tax/printer |
//! | `shell.rs`    | the frame: rail, sidebar, workspace, status bar    |

mod actions;
mod auth;
mod checkout;
mod commands;
mod currencies;
mod db;
mod export;
mod firewall;
mod import;
mod modules;
mod picker;
mod prefs;
mod receipt;
mod scan;
mod settings;
mod shell;
mod state;
mod theme;

use std::process::ExitCode;

use crate::theme::{DARK, LIGHT};
use ui_shell::regions::{self, Window};

/// The one brand string. Window title and receipt header both read it, so a
/// rename touches this line and nothing else.
pub const APP_NAME: &str = "Stride POS";

fn main() -> ExitCode {
    app_boot::run("pos", |ctx| {
        let mut store = storage::Store::open(ctx.data_path("pos.db"))?;
        // Claim before migrating: a file that belongs to another app is refused
        // before this one writes a single table into it.
        store.claim_application_id(db::APPLICATION_ID)?;
        // An intermediate build of the supplier directory created the table
        // before its columns were settled; repair that shape before the
        // forward-only migrations decide there is nothing left to do.
        db::repair_suppliers(&store)?;
        store.migrate(db::MIGRATIONS)?;

        let settings_path = ctx.config_path("settings.json");
        let settings: settings::Settings = config::load_or_default(&settings_path)?;
        let theme = settings.theme();

        let prefs_path = ctx.config_path("prefs.json");
        let prefs: prefs::Prefs = config::load_or_default(&prefs_path)?;

        // A prefs file from a build with more modules must not index past the
        // end of the list. Clamp rather than crash: the user's habit is worth
        // less than their till opening.
        let module = prefs.module.min(modules::COUNT.saturating_sub(1));

        // The currency belongs to the install, stored in the database with the
        // amounts it names. Absent means this is a fresh till: default the frame
        // to UGX and let the shell demand a choice before anything is priced.
        let stored = db::currency(&store);
        let needs_currency = stored.is_none();
        let currency = stored
            .unwrap_or_else(|| money::tender_from_code("UGX").expect("UGX is an ISO 4217 tender"));

        let state = state::State::new(
            store,
            module,
            prefs.sidebar,
            settings,
            state::Paths {
                data: ctx.dirs.data.clone(),
                settings: settings_path,
            },
            currency,
            needs_currency,
        );
        let app = shell::PosShell::new(state, prefs_path);

        regions::run_app(
            APP_NAME,
            theme,
            LIGHT,
            DARK,
            Window::default(),
            app,
            |ui, app| app.draw(ui),
        )?;

        Ok(())
    })
}
