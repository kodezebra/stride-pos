//! The till's action vocabulary.
//!
//! A button is not a visual style; it is a claim about what happens next. The
//! till keeps four kinds so a cashier can predict the weight of an action before
//! reading it:
//!
//! - [`primary`] completes the screen's purpose. One per view, rightmost in a
//!   footer.
//! - [`secondary`] is an alternate or dismissive action: Cancel, Copy, Back to
//!   cart. Never more visual weight than the primary.
//! - [`quiet`] is a tertiary route that should stay available without competing
//!   with the decision: a fallback path, a template, a password reminder.
//! - [`secondary_compact`] and [`quiet_compact`] are the same claims in a row
//!   or a settings card, where a full-height action would make the table jump.
//!
//! Footer actions are added primary first because the footer lays out right to
//! left; the first button is therefore the rightmost one on screen. Tertiary
//! routes belong above the footer, not beside the decision.
//!
//! Words are part of the contract too. `Cancel` abandons unsaved edits or a
//! pending choice. `Close` dismisses a finished or read-only view. `Back to
//! cart` leaves payment without pretending the sale was cancelled. A new verb
//! is a new outcome, not a new style.

use ui_kit::components;
use ui_kit::theme;
use ui_kit::tokens::{RADII, SPACING, TYPE};
use ui_shell::egui;

/// Height of an action that carries a whole screen or dialog.
const MAIN: f32 = 44.0;
/// Height of an action inside a list row, a field row, or a settings card.
const COMPACT: f32 = 36.0;

/// The one emphasised action on a screen or in a dialog footer.
pub fn primary(ui: &mut egui::Ui, label: &str) -> egui::Response {
    components::primary_button(ui, label)
}

/// An alternate or dismissive action, at dialog weight.
pub fn secondary(ui: &mut egui::Ui, label: &str) -> egui::Response {
    text_button(ui, label, MAIN, false)
}

/// An alternate or dismissive action inside a row or settings card.
pub fn secondary_compact(ui: &mut egui::Ui, label: &str) -> egui::Response {
    text_button(ui, label, COMPACT, false)
}

/// A tertiary route, at dialog weight.
pub fn quiet(ui: &mut egui::Ui, label: &str) -> egui::Response {
    text_button(ui, label, MAIN, true)
}

/// A tertiary route inside a row or settings card.
pub fn quiet_compact(ui: &mut egui::Ui, label: &str) -> egui::Response {
    text_button(ui, label, COMPACT, true)
}

/// A menu trigger that carries secondary weight beside a primary action.
///
/// egui gives menus their own button; this wrapper matches its height and label
/// to the rest of the footer/header actions so a "More" does not read as a
/// different kind of control from the button next to it.
pub fn menu_button(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    let button = egui::Button::new(
        egui::RichText::new(label)
            .size(TYPE.button)
            .family(theme::semibold()),
    )
    .min_size(egui::vec2(0.0, MAIN))
    .corner_radius(egui::CornerRadius::same(RADII.sm as u8));

    let _ = egui::containers::menu::MenuButton::from_button(button).ui(ui, add);
}

/// A dialog footer: primary rightmost, then secondary, then quiet.
///
/// Add the primary first. The layout runs right to left, so the first call is
/// the rightmost button and every reader gets the same decision at the same
/// edge of the dialog.
pub fn footer(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(SPACING.md);
    end(ui, add);
}

/// The same right alignment without the leading space, for a confirmation that
/// lives inside a settings card rather than a dialog.
pub fn end(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = SPACING.sm;
        add(ui);
    });
}

fn text_button(ui: &mut egui::Ui, label: &str, height: f32, quiet: bool) -> egui::Response {
    let palette = theme::current(ui.ctx());
    let text = if quiet {
        egui::RichText::new(label)
            .size(TYPE.button)
            .color(palette.text_muted)
    } else {
        egui::RichText::new(label)
            .size(TYPE.button)
            .family(theme::semibold())
    };

    let mut button = egui::Button::new(text)
        .min_size(egui::vec2(0.0, height))
        .corner_radius(egui::CornerRadius::same(RADII.sm as u8));
    if quiet {
        button = button.frame(false);
    }
    ui.add(button)
}
