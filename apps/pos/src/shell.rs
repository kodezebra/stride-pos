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
//! | `F1`..`F5`  | Switch module             |
//! | `F6`..`F8`  | Reserved: inventory, suppliers, purchase orders |
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

use crate::currencies;
use crate::modules::{self, Module};
use crate::prefs::Prefs;
use crate::state::State;

pub struct PosShell {
    state: State,
    modules: Vec<Box<dyn Module>>,
    /// Built once from `modules` so the rail and the tooltips cannot drift from
    /// the hotkey router: both read the same `Module` methods.
    rail: Vec<RailItem>,
    commands: Vec<Command<State>>,
    palette: Palette,
    prefs_path: PathBuf,
}

impl PosShell {
    pub fn new(state: State, prefs_path: PathBuf) -> Self {
        let modules = modules::all();
        let rail = modules
            .iter()
            .map(|module| RailItem {
                icon: module.icon(),
                label: module.title(),
                hotkey: module.hotkey().name(),
            })
            .collect();

        Self {
            state,
            modules,
            rail,
            commands: crate::commands::all(),
            palette: Palette::default(),
            prefs_path,
        }
    }

    /// Entry point, handed to [`ui_shell::regions::run_app`].
    pub fn draw(&mut self, ui: &mut egui::Ui) {
        // A theme switch takes effect here, at the top of the next frame. The
        // shell is the only place that installs global visuals; the Settings
        // screen only records the intent on `state`.
        if self.state.theme_dirty {
            ui_kit::theme::install(ui.ctx(), self.state.theme);
            self.state.theme_dirty = false;
        }

        // Time passes even when nobody touches anything: the idle clock is what
        // puts the till on the lock screen.
        self.state.tick(ui.ctx());

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
        // The bulk sibling of the product editor: a CSV import is one owner's
        // setup job, but it still owns the screen while it reads its file.
        crate::modules::import_modal(ui.ctx(), &mut self.state);
        // The receipt behind a sales row, one tap from either list that shows
        // sales; it owns the screen while it holds the floor.
        crate::modules::sale_detail_modal(ui.ctx(), &mut self.state);
        // Same reason: paying is the one flow that writes a sale, and it must own
        // the screen while the cashier counts notes.
        crate::checkout::payment_modal(ui.ctx(), &mut self.state);
        // And the one flow that lends a manager's authority to a cashier.
        crate::auth::override_modal(ui.ctx(), &mut self.state);
        self.persist();
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
                ui.add_space(SPACING.md);
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
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
        ctx.input(|input| {
            for (index, module) in self.modules.iter().enumerate() {
                if input.key_pressed(module.hotkey()) {
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
        // Not while another modal owns the keyboard: the product form's Enter
        // saves, and a payment opening behind it would be a second, invisible
        // thing to dismiss.
        if take_payment && self.state.product_editor.is_none() && self.state.payment.is_none() {
            self.state.start_payment();
        }
    }

    fn draw_frame(&mut self, ui: &mut egui::Ui) {
        // Borrow the fields separately: the closures below need `state` mutably
        // while the frame reads `modules` and `rail`.
        let Self {
            state,
            modules,
            rail,
            ..
        } = self;

        let active = state.module;

        regions::status(ui, |ui| status_bar(ui, state));

        if let Some(next) = regions::rail(ui, rail, active) {
            modules::open(state, next);
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
            if module.scrolls() {
                egui::ScrollArea::vertical()
                    .id_salt(("page", active))
                    .auto_shrink([false, false])
                    .show(ui, |ui| module.workspace(ui, state));
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
            let path = self.state.settings_path.clone();
            match config::save(&path, &self.state.settings) {
                Ok(()) => self.state.settings_dirty = false,
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

    // Left cluster: what the terminal knows about itself. "local only" is the
    // truth today — there is no hub and no cloud — and it is what the operator
    // needs to see before they trust the queue depth that will sit next to it.
    regions::status_item(ui, Icon::UNSYNCED, "local only", muted);

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
