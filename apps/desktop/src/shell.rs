//! The POS frame.
//!
//! Sell is not a page inside the school layout with different labels; it is a
//! different arrangement: no header, a 52 px activity rail, a sidebar that
//! belongs to the module, and a cart pinned to the right of the workspace. That
//! is why this app composes [`ui_shell::regions`] instead of using
//! [`ui_shell::Shell`] — same chrome, different skeleton.
//!
//! Keyboard first, because a till is operated without looking at a menu:
//!
//! | Key         | Action                    |
//! |-------------|---------------------------|
//! | `F1`..`F7`  | Switch to a visible module |
//! | `F8`        | Reserved: purchase orders |
//! | `Ctrl+B`    | Collapse/expand sidebar   |
//! | `Ctrl+P`    | Command palette           |
//! | `Ctrl+Enter`| Take payment              |
//! | `Ctrl+T`    | Light/dark                |

use std::path::PathBuf;

use money::Currency;
use ui_kit::components;
use ui_kit::icons::Icon;
use ui_kit::theme::Theme;
use ui_kit::tokens::{SPACING, TYPE};
use ui_shell::egui;
use ui_shell::regions::{self, Command, Palette, RailItem, Sidebar, size};

use crate::actions;
use crate::auth::Role;
use crate::currencies;
use crate::modules::{self, Module};
use crate::prefs::Prefs;
use crate::state::State;
use crate::theme::palette_for;

pub struct PosShell {
    state: State,
    modules: Vec<Box<dyn Module>>,
    commands: Vec<Command<State>>,
    /// The role the command list was built for. The rail is derived every
    /// frame, but the palette owns boxed closures and is rebuilt only when the
    /// signed-in person changes.
    commands_role: Option<Role>,
    palette: Palette,
    prefs_path: PathBuf,
    /// The shop's one LAN listener. Owned by the frame, not by Settings: it
    /// must keep running while the operator is on any other screen.
    lan: crate::lan::Service,
    /// The background loop that pulls from other tills. Same lifetime, same
    /// reason: sync is a property of the process, not of the card that set it.
    sync: crate::sync::service::Service,
    /// Whether the host firewall is dropping the port the listener is on, and
    /// the privileged step that opens it. Owned here for the same reason.
    firewall: crate::firewall::Service,
}

impl PosShell {
    pub fn new(state: State, prefs_path: PathBuf) -> Self {
        // One database, two extra connections: the listener and the worker each
        // open their own so neither can hold up checkout.
        let db_path = state.paths.data.join("pos.db");
        Self {
            state,
            modules: modules::all(),
            commands: crate::commands::all(Role::Cashier),
            commands_role: Some(Role::Cashier),
            palette: Palette::default(),
            prefs_path,
            lan: crate::lan::Service::new(db_path.clone()),
            sync: crate::sync::service::Service::new(db_path),
            firewall: crate::firewall::Service::new(),
        }
    }

    /// Entry point, handed to [`ui_shell::regions::run_app`].
    pub fn draw(&mut self, ui: &mut egui::Ui) {
        // A theme switch takes effect here, at the top of the next frame. The
        // shell is the only place that installs global visuals; the Settings
        // screen only records the intent on `state`. The palette is the POS's
        // own — ui-kit never invents brand colors.
        if self.state.theme_dirty {
            let (palette, dark) = palette_for(self.state.theme);
            if let Some(accent) = self.state.brand_accent {
                ui_kit::theme::install_accent(ui.ctx(), palette, accent, dark);
            } else {
                ui_kit::theme::install(ui.ctx(), palette, dark);
            }
            self.state.theme_dirty = false;
        }

        // Time passes even when nobody touches anything: the idle clock is what
        // puts the till on the lock screen.
        self.state.tick(ui.ctx());

        // The phone scanner and till-to-till sync live for the process, not for
        // the screen that enabled them. Sync first so a switch flipped on the
        // Settings page takes effect on the next frame, then hand every scan to
        // the same routing a keyboard scan uses.
        self.lan.sync(
            &self.state.settings.scanner,
            &self.state.settings.sync,
            self.state.settings.tax_rate_percent,
        );
        let events = self.lan.drain();
        let had_scan = !events.is_empty();
        for event in events {
            self.state.receive_scan(event);
        }
        self.state.scanner_status = self.lan.status().clone();

        // The worker parks when sync is off and is woken by the toggle. "Sync
        // now" from the card is a one-shot request, not a mode.
        self.sync.set_enabled(self.state.settings.sync.enabled);
        if std::mem::take(&mut self.state.sync_now_requested) {
            self.sync.now();
        }
        let sync_events = self.sync.drain();
        let had_sync = !sync_events.is_empty();
        for event in &sync_events {
            self.sync.observe(event);
            if let crate::sync::SyncEvent::CatalogSettings(catalog) = event
                && self.state.settings.tax_rate_percent != catalog.tax_rate_percent
            {
                // Settings change on the UI thread only; the worker hands the
                // value over instead of writing the file itself.
                self.state.settings.tax_rate_percent = catalog.tax_rate_percent;
                self.state.settings_dirty = true;
            }
        }
        if had_sync {
            // The worker records per-peer errors and last-seen times; a pass
            // that just finished is the moment to show them.
            self.state.reload_sync_peers();
        }
        self.state.sync_status = self.sync.status().clone();
        // A scan or a peer that arrived while the till was idle still lands
        // within a single frame: ask egui to repaint immediately so the cart
        // and the card update instead of waiting for the next touch.
        if had_scan || had_sync {
            ui.ctx().request_repaint();
        }

        // A healthy listener can still be unreachable, because the host
        // firewall drops the packet first. Probe the port the listener is on,
        // and let Settings ask for it to be opened. The port is shared, so one
        // probe covers scanning and sync.
        let port = (self.state.settings.scanner.enabled || self.state.settings.sync.enabled)
            .then_some(self.state.settings.scanner.port);
        self.firewall.sync(port);
        if std::mem::take(&mut self.state.firewall_allow_requested) {
            self.firewall.allow();
        }
        self.firewall.poll();
        let firewall = self.firewall.status();
        self.state.firewall = firewall.snapshot;
        self.state.firewall_busy = firewall.busy;
        self.state.firewall_error = firewall.error;

        // Locked is a whole screen, not a modal over the workspace: the takings
        // behind it are the reason for the lock, and a modal leaves them legible
        // around its edges.
        if self.state.locked() {
            // The panels below paint their own fills, so the normal frame needs
            // no background of its own. Locked, there are no panels: the root
            // `Ui` is bare, and without this the lock screen would float on
            // whatever the compositor happens to have behind the window.
            let palette = ui_kit::theme::current(ui.ctx());
            ui.painter().rect_filled(
                ui.max_rect(),
                egui::CornerRadius::ZERO,
                egui::Color32::from(palette.bg),
            );
            crate::auth::lock_screen(ui, &mut self.state);
            self.persist();
            return;
        }

        // The palette and hotkey router are role-scoped. Rebuild them at the
        // first frame after a new person signs in, not on every frame.
        self.sync_role();

        // First run is a single question: nothing behind the modal is reachable,
        // so module hotkeys and the palette stay off until it is answered.
        let setup = self.state.needs_currency;
        if !setup {
            self.handle_keys(ui.ctx());
        }
        self.draw_frame(ui);
        // Drawn after the frame so the modal's backdrop covers it. `Modal` puts
        // itself in the foreground layer, but a same-frame repaint is cheaper
        // than waiting for one more.
        if !setup {
            self.palette.show(ui.ctx(), &self.commands, &mut self.state);
        }
        self.currency_picker(ui.ctx());
        // The product form is owned by the Products module but drawn here, with
        // the rest of the foreground: a modal inside a module's workspace would
        // be clipped to that panel and could not dim the list behind it.
        crate::modules::product_editor_modal(ui.ctx(), &mut self.state);
        // The supplier directory's form, owned by its module for the same
        // reason and drawn in the same foreground layer.
        crate::modules::supplier_editor_modal(ui.ctx(), &mut self.state);
        // Inventory's three surfaces: starting a count, recording one movement,
        // and reading the ledger behind one product. Drawn in that order so a
        // history modal never opens underneath a movement form.
        crate::modules::track_modal(ui.ctx(), &mut self.state);
        crate::modules::stock_move_modal(ui.ctx(), &mut self.state);
        crate::modules::stock_history_modal(ui.ctx(), &mut self.state);
        // Choosing the file to read or the folder to write into. Drawn before
        // the typed import form so that "Type the path instead" can hand the
        // screen over in the same frame, without a flash of both.
        crate::picker::picker_modal(ui.ctx(), &mut self.state);
        // The bulk sibling of the product editor: a CSV import is one owner's
        // setup job, but it still owns the screen while it reads its file.
        crate::modules::import_modal(ui.ctx(), &mut self.state);
        // The receipt behind a sales row, one tap from either list that shows
        // sales; it owns the screen while it holds the floor.
        crate::modules::sale_detail_modal(ui.ctx(), &mut self.state);
        // Setting a cart line's count, which must finish before the basket can
        // be paid for: the modal is drawn under payment so a shortcut cannot
        // leave a quantity editor floating over a sale in progress.
        crate::modules::cart_quantity_modal(ui.ctx(), &mut self.state);
        // Same reason: paying is the one flow that writes a sale, and it must own
        // the screen while the cashier counts notes.
        crate::checkout::payment_modal(ui.ctx(), &mut self.state);
        // And the one flow that lends a manager's authority to a cashier.
        crate::auth::override_modal(ui.ctx(), &mut self.state);
        self.persist();
    }

    /// The role whose navigation the frame should show. Locked means no
    /// session, and the most restrictive rail is the safe default.
    fn current_role(&self) -> Role {
        self.state
            .session
            .as_ref()
            .map(|session| session.role)
            .unwrap_or(Role::Cashier)
    }

    fn sync_role(&mut self) {
        let role = self.current_role();
        if self.commands_role != Some(role) {
            self.commands = crate::commands::all(role);
            self.commands_role = Some(role);
        }
    }

    /// The currency chooser: required on first run, optional from Settings while
    /// the till still has no sales.
    ///
    /// A modal because a till with no currency cannot price anything, and on
    /// first run it is not dismissible: there is no "later" for a setting every
    /// amount on the screen depends on.
    fn currency_picker(&mut self, ctx: &egui::Context) {
        // The owner picks the currency: setup comes first, so the till has
        // somebody responsible for its amounts before it stores any.
        if self.state.session.is_none() {
            return;
        }

        let required = self.state.needs_currency;
        if !required && !self.state.currency_picker {
            return;
        }

        // Read what the closure needs into locals: it borrows `self.state`
        // immutably while the write happens after the modal closes.
        let current = self.state.currency;
        let error = self.state.currency_error.clone();
        let mut chosen = None;
        let mut cancel = false;

        crate::modules::modal(ctx, egui::Id::new("pos_currency_picker")).show(ctx, |ui| {
            ui.set_width(400.0);

            ui.label(components::semibold("Currency").size(TYPE.heading));
            components::caption(
                ui,
                "One currency per till. It is stored with the database and locked \
                 after the first sale.",
            );

            ui.add_space(SPACING.md);

            egui::ScrollArea::vertical()
                .max_height(300.0)
                .show(ui, |ui| {
                    for (title, choices) in currencies::GROUPS {
                        currency_group(ui, title, choices, current, &mut chosen);
                    }
                });

            if let Some(error) = &error {
                ui.add_space(SPACING.sm);
                ui.label(
                    egui::RichText::new(error)
                        .size(TYPE.small)
                        .color(ui.visuals().error_fg_color),
                );
            }

            if !required {
                actions::footer(ui, |ui| {
                    if actions::secondary(ui, "Cancel").clicked() {
                        cancel = true;
                    }
                });
            }
        });

        if cancel {
            self.state.currency_picker = false;
        }
        if let Some(currency) = chosen {
            self.state.set_currency(currency);
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        let mut jump = None;
        let order = modules::rail_order(self.current_role());
        ctx.input(|input| {
            // Walk the visible rail, not the whole module table: an F-key for a
            // destination this role cannot use should do nothing, not open a
            // manager prompt from a key the cashier pressed by accident.
            for &index in order {
                if input.key_pressed(self.modules[index].hotkey()) {
                    jump = Some(index);
                }
            }
        });
        // Through the same gate as the rail and the palette: an F-key is a route
        // into a module like any other.
        if let Some(index) = jump {
            modules::open(&mut self.state, index);
        }

        let (toggle_sidebar, open_palette, toggle_theme, lock, take_payment) = ctx.input(|input| {
            let command = input.modifiers.command;
            (
                command && input.key_pressed(egui::Key::B),
                command && input.key_pressed(egui::Key::P),
                command && input.key_pressed(egui::Key::T),
                command && input.key_pressed(egui::Key::L),
                (command && input.key_pressed(egui::Key::Enter))
                    || input.key_pressed(egui::Key::F12),
            )
        });

        if toggle_sidebar {
            self.state.toggle_sidebar();
        }
        if open_palette {
            self.palette.request_open();
        }
        if toggle_theme {
            self.state.toggle_theme();
        }
        // Locking wins over anything else pressed in the same frame: whatever was
        // about to happen, it should happen to someone who is signed in.
        if lock {
            self.state.lock();
            return;
        }
        // Not while another modal owns the keyboard: the product and supplier
        // forms save on Enter, and a payment opening behind one would be a
        // second, invisible thing to dismiss.
        if take_payment
            && self.state.product_editor.is_none()
            && self.state.supplier_editor.is_none()
            && self.state.stock_editor.is_none()
            && self.state.stock_move.is_none()
            && self.state.stock_history.is_none()
            && self.state.cart_quantity.is_none()
            && self.state.payment.is_none()
        {
            self.state.start_payment();
        }
    }

    fn draw_frame(&mut self, ui: &mut egui::Ui) {
        // Borrow the fields separately: the closures below need `state` mutably
        // while the frame reads `modules`.
        let Self { state, modules, .. } = self;

        let active = state.module;
        let role = state
            .session
            .as_ref()
            .map(|session| session.role)
            .unwrap_or(Role::Cashier);
        let order = modules::rail_order(role);
        let rail: Vec<RailItem> = order
            .iter()
            .map(|&index| {
                let module = &modules[index];
                RailItem {
                    icon: module.icon(),
                    label: module.title(),
                    hotkey: module.hotkey().name(),
                }
            })
            .collect();
        let selected = order
            .iter()
            .position(|&index| index == active)
            .unwrap_or(usize::MAX);

        regions::status(ui, |ui| status_bar(ui, state));

        // The rail's index is the visible row, not the module index. The order
        // is role-filtered, so those two numbers only agree for a manager.
        if let Some(next) = regions::rail(ui, &rail, selected) {
            modules::open(state, order[next]);
        }

        let module = &mut modules[active];
        let spec = match module.fixed_sidebar() {
            Some(width) => Sidebar::new(width).fixed(),
            None => Sidebar::default(),
        };

        // `show_collapsible` flips this flag when the user drags the sidebar
        // shut, so the state is committed after the frame rather than read once.
        let mut open = state.sidebar_open(active);
        regions::sidebar(ui, spec, &mut open, |ui| module.sidebar(ui, state));
        state.set_sidebar_open(active, open);

        regions::workspace(ui, |ui| {
            // Pages scroll as a whole: the dashboard's 30-day chart and the
            // staff list both run past the fold, and a page that cannot scroll
            // hides the bottom of itself. Sell opts out — its cart is pinned to
            // the pane, and an unbounded height would un-pin it.
            let module = &mut modules[active];
            // Pinned actions sit outside the scroll area: a long list must not
            // take "New product" or "Export" past the fold with it.
            module.toolbar(ui, state);
            if module.scrolls() {
                egui::ScrollArea::vertical()
                    .id_salt(("page", active))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        module.workspace(ui, state);
                        // The panel margin frames the viewport, not the end of
                        // the page. Without this, a long list finishes flush
                        // against the bottom edge and looks cut off.
                        ui.add_space(SPACING.lg);
                    });
            } else {
                module.workspace(ui, state);
            }
        });
    }

    fn persist(&mut self) {
        if self.state.prefs_dirty {
            let prefs = Prefs {
                module: self.state.module,
                sidebar: self.state.sidebar_open.clone(),
            };

            match config::save(&self.prefs_path, &prefs) {
                Ok(()) => self.state.prefs_dirty = false,
                Err(err) => {
                    // Clear the flag anyway: retrying every frame would rewrite
                    // the log 60 times a second, and a config directory that
                    // cannot be written is not something this loop can fix.
                    self.state.prefs_dirty = false;
                    tracing::warn!(error = %err, path = %self.prefs_path.display(), "could not save prefs");
                }
            }
        }

        if self.state.settings_dirty {
            let path = self.state.paths.settings.clone();
            match config::save(&path, &self.state.settings) {
                Ok(()) => {
                    self.state.settings_dirty = false;
                    // A phone's catalog snapshot carries the tax rate and the
                    // cash step, and those live in settings rather than a table.
                    // Table writes bump this by trigger; this one needs a hand.
                    if let Err(err) = crate::db::bump_catalog_version(&self.state.store) {
                        tracing::warn!(error = %err, "could not bump the catalog version");
                    }
                }
                Err(err) => {
                    self.state.settings_dirty = false;
                    tracing::warn!(error = %err, path = %path.display(), "could not save settings");
                }
            }
        }
    }
}

fn status_bar(ui: &mut egui::Ui, state: &mut State) {
    let muted = ui.visuals().weak_text_color();
    let palette = ui_kit::theme::current(ui.ctx());

    // Left cluster: what the terminal knows about itself. A till that syncs still
    // sells alone; this segment says whether it is currently keeping company.
    let sync = &state.sync_status;
    let (sync_icon, sync_text, sync_color) = if !state.settings.sync.enabled {
        (Icon::UNSYNCED, "local only".to_owned(), muted)
    } else if sync.error.is_some() {
        (
            Icon::OFFLINE,
            "sync error".to_owned(),
            egui::Color32::from(palette.danger),
        )
    } else if sync.syncing {
        (Icon::ONLINE, "syncing…".to_owned(), muted)
    } else {
        (Icon::ONLINE, format!("{} till(s)", sync.peers), muted)
    };
    regions::status_item(ui, sync_icon, &sync_text, sync_color);

    let cart = format!(
        "{} item(s) - {}",
        state.cart_count(),
        state.money(state.cart_total_minor())
    );

    // Shortcut hints live where the eye already is: the bar is always visible,
    // so the keys are learned once instead of being buried in a help screen.
    //
    // They also give way first. The right cluster is drawn last and right
    // aligned, so on a narrow bar it does not push the hints aside — it paints
    // over them. Measuring it first means the hints drop out instead: the
    // takings are why the bar exists, and every shortcut is in the palette.
    let cashier = state
        .session
        .as_ref()
        .map(|session| format!("{} · {}", session.name, session.role.label()))
        .unwrap_or_else(|| "no cashier".to_owned());
    let reserved = segment_width(ui, &cart)
        + segment_width(ui, &cashier)
        + text_width(ui, concat!("v", env!("CARGO_PKG_VERSION")), false)
        + size::STATUS_ICON
        + 4.0
        + SPACING.lg * 3.0;

    hint(ui, "Ctrl+P", "commands", muted);
    if hint_fits(ui, "Ctrl+B", "sidebar", reserved) {
        hint(ui, "Ctrl+B", "sidebar", muted);
    }
    if hint_fits(ui, "Ctrl+L", "lock", reserved) {
        hint(ui, "Ctrl+L", "lock", muted);
    }

    regions::status_end(ui, |ui| {
        // Right-to-left layout: register in reverse of the visual order.
        ui.label(
            egui::RichText::new(concat!("v", env!("CARGO_PKG_VERSION")))
                .size(TYPE.status)
                .weak(),
        );
        theme_toggle(ui, state);
        regions::status_item(ui, Icon::USER, &cashier, muted);
        regions::status_item(ui, Icon::CART, &cart, muted);
    });
}

/// Width of a label at status size. The bar measures before it draws so a hint
/// can step aside instead of being painted over.
fn text_width(ui: &egui::Ui, text: &str, mono: bool) -> f32 {
    let family = if mono {
        egui::FontFamily::Monospace
    } else {
        egui::FontFamily::Proportional
    };
    ui.painter()
        .layout_no_wrap(
            text.to_owned(),
            egui::FontId::new(TYPE.status, family),
            egui::Color32::WHITE,
        )
        .size()
        .x
}

/// One status segment: icon, gap, label.
fn segment_width(ui: &egui::Ui, text: &str) -> f32 {
    size::STATUS_ICON + SPACING.sm + text_width(ui, text, false)
}

/// Whether one more hint still leaves room for the right cluster.
fn hint_fits(ui: &egui::Ui, key: &str, action: &str, reserved: f32) -> bool {
    let width = text_width(ui, key, true) + text_width(ui, action, false) + SPACING.lg * 2.0;
    ui.available_width() - width > reserved
}

/// A key and what it does, in one muted segment.
fn hint(ui: &mut egui::Ui, key: &str, action: &str, muted: egui::Color32) {
    ui.horizontal_centered(|ui| {
        ui.label(
            egui::RichText::new(key)
                .size(TYPE.status)
                .monospace()
                .color(ui.visuals().strong_text_color()),
        );
        ui.label(egui::RichText::new(action).size(TYPE.status).color(muted));
    });
}

/// Appearance toggle. Settings still exists for the people who look there, but
/// flipping the lights should not cost a screen change.
fn theme_toggle(ui: &mut egui::Ui, state: &mut State) {
    let dark = state.theme == Theme::Dark;
    let icon = if dark { Icon::SUN } else { Icon::MOON };
    let tip = if dark {
        "Switch to light (Ctrl+T)"
    } else {
        "Switch to dark (Ctrl+T)"
    };

    if components::icon_button(ui, icon, size::STATUS_ICON + 4.0)
        .on_hover_text(tip)
        .clicked()
    {
        state.toggle_theme();
    }
}

/// One titled block of country rows.
///
/// `chosen` is an out-parameter because the modal's closure borrows the state
/// immutably; the write happens once the modal has closed.
fn currency_group(
    ui: &mut egui::Ui,
    title: &str,
    choices: &[currencies::Choice],
    current: Currency,
    chosen: &mut Option<Currency>,
) {
    components::section_title(ui, title);
    for choice in choices {
        // A country ISO lists no regular currency for is skipped rather than
        // shown as a row that cannot be picked.
        let Some(currency) = choice.currency() else {
            continue;
        };
        let row = format!("{}  -  {}", choice.label, currency.code());
        if regions::nav_item(ui, currency == current, &row).clicked() {
            *chosen = Some(currency);
        }
    }
    ui.add_space(SPACING.sm);
}
