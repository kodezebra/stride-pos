//! Sales: the list of what has been sold, and the receipt behind each sale.
//!
//! A sale is what the till already committed. This screen only reads, so it is
//! the quietest of the six — the drawer summary at the top, the list below, and
//! one modal that shows the lines, tender and change of a single sale.

use super::*;
///
/// The receipt behind a list row. Every figure on it is what was recorded at
/// the time — `unit × qty` as charged, the rounding the customer actually
/// paid — because a customer conversation starts from "this is what the till
/// says", not from a re-derivation that can disagree with the ledger.
pub fn sale_detail_modal(ctx: &egui::Context, state: &mut State) {
    let Some(detail) = state.sale_open.as_ref() else {
        return;
    };
    let currency = state.currency;
    let shop = state.settings.shop_name().to_owned();
    let paper = crate::receipt::Paper::from_receipt(&shop, &state.settings.receipt);
    let copy_text = crate::receipt::plain(detail, &paper, currency);

    let mut close = false;
    let mut copied = false;

    modal(ctx, egui::Id::new("pos_sale_detail")).show(ctx, |ui| {
        ui.set_width(460.0);

        // The same block the customer saw at the till, scrolled when the sale
        // is longer than the window: an owner checking a discrepancy is
        // reading paper, not a summary of paper.
        egui::ScrollArea::vertical()
            .max_height(420.0)
            .show(ui, |ui| {
                crate::receipt::body(ui, detail, &paper, currency, true)
            });

        actions::footer(ui, |ui| {
            if actions::secondary(ui, "Close").clicked() {
                close = true;
            }
            // Same slip, as text: this is the screen an owner opens when a
            // customer is asking about a figure, and the answer has to travel.
            if actions::secondary(ui, "Copy receipt")
                .on_hover_text("Copy this slip as text")
                .clicked()
            {
                ui.ctx().copy_text(copy_text.clone());
                copied = true;
            }
        });
        if copied {
            components::caption(ui, "Copied to the clipboard.");
        }
    });

    // Escape, because an owner checking sale after sale keeps one hand on the
    // keyboard and the list one keypress away.
    if close || ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        state.close_sale();
    }
}

// ---------------------------------------------------------------------------
// Sales
// ---------------------------------------------------------------------------

pub(super) struct Sales;

/// The cash drawer, at the top of Today.
///
/// One card with two states. Open: what the till expects to be in the drawer,
/// and the one number it cannot know — what is actually there. Closed: what was
/// counted and how it compared. Nothing is drawn before the first sale of the
/// day, because there is nothing to count and a card saying so is noise.
///
/// The drawer is the shop's, not the cashier's: the count covers every sale in
/// the shift even when the list below is scoped to one person.
pub(super) fn drawer_card(ui: &mut egui::Ui, state: &mut State) {
    let Some(drawer) = state.drawer.clone() else {
        return;
    };
    let palette = theme::current(ui.ctx());

    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());

        let opened_by = drawer
            .shift
            .opened_by
            .as_deref()
            .map(|name| format!(" · {name}"))
            .unwrap_or_default();
        let count = drawer.totals.sales_count;
        let sales = match count {
            0 => String::new(),
            1 => "1 sale · ".to_owned(),
            _ => format!("{count} sales · "),
        };
        let title = if drawer.shift.is_open() {
            "Cash drawer"
        } else {
            "Day closed"
        };
        ui.horizontal(|ui| {
            ui.label(components::semibold(title).size(TYPE.body));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                components::caption(
                    ui,
                    &format!("{sales}opened {}{opened_by}", drawer.shift.opened_at),
                );
            });
        });
        ui.add_space(SPACING.md);

        // The figures are peers in one row, not a form: three numbers a shopkeeper
        // reads at a glance, the same three every day.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 40.0;
            drawer_figure(
                ui,
                "Cash taken",
                &state.money(drawer.totals.cash_minor),
                None,
            );
            if drawer.shift.float_minor > 0 {
                drawer_figure(
                    ui,
                    "Change float",
                    &state.money(drawer.shift.float_minor),
                    None,
                );
            }
            drawer_figure(
                ui,
                "Expected in drawer",
                &state.money(drawer.expected_minor()),
                None,
            );

            if drawer.shift.is_open() {
                return;
            }
            if let Some(counted) = drawer.shift.counted_minor {
                drawer_figure(ui, "Counted", &state.money(counted), None);
            }
            if let Some(difference) = drawer.difference_minor() {
                let (text, color) = match difference.cmp(&0) {
                    std::cmp::Ordering::Equal => ("Balanced".to_owned(), palette.success),
                    std::cmp::Ordering::Less => (
                        format!("Short {}", state.money(-difference)),
                        palette.danger,
                    ),
                    std::cmp::Ordering::Greater => {
                        (format!("Over {}", state.money(difference)), palette.warning)
                    }
                };
                drawer_figure(ui, "Difference", &text, Some(egui::Color32::from(color)));
            }
        });

        // Who handled this drawer's cash. The shop view is the only one that
        // may see other people's takings, and one cashier's figure is already
        // the total above: show the split only when there is more than one name
        // to compare, which is exactly when a short count needs explaining.
        if state.report_scope().is_shop() {
            let people = crate::db::drawer_by_person(&state.store, &drawer.shift.uid);
            if people.len() > 1 {
                ui.add_space(SPACING.md);
                components::caption(ui, "Cash by person in this drawer");
                ui.add_space(SPACING.xs);
                tally_row(ui, "Person", "Sales", "Cash", true, false);
                for (index, person) in people.iter().enumerate() {
                    tally_row(
                        ui,
                        &person.name,
                        &person.sales.to_string(),
                        &state.money(person.cash_minor),
                        false,
                        index % 2 == 1,
                    );
                }
            }
        }

        if !drawer.shift.is_open() {
            return;
        }

        // The list below can be one person's; the drawer never is. Say so where
        // the two disagree, rather than leaving a cashier to wonder whose cash
        // they are counting.
        if !state.report_scope().is_shop() {
            ui.add_space(SPACING.xs);
            components::caption(
                ui,
                "The drawer is the shop's: this count covers every sale since it opened.",
            );
        }

        // A rule, because what follows is the one thing the till is asking
        // rather than telling.
        ui.add_space(SPACING.md);
        let rule = ui.available_rect_before_wrap();
        ui.painter().hline(
            rule.x_range(),
            rule.top(),
            egui::Stroke::new(1.0_f32, palette.border),
        );
        ui.add_space(SPACING.md);

        let mut close = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.sm;
            components::field_label(ui, "Cash counted");
            let response = text_field(ui, &mut state.drawer_count, "0", 140.0);
            // Enter is the keyboard-first path: count, Enter, done. The button is
            // the same act with a finger.
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                close = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if actions::primary(ui, "Close day").clicked() {
                    close = true;
                }
            });
        });
        if close {
            state.close_day();
        }

        if let Some(error) = state.drawer_error.clone() {
            ui.add_space(SPACING.xs);
            ui.label(
                egui::RichText::new(error)
                    .size(TYPE.small)
                    .color(palette.danger),
            );
        }

        ui.add_space(SPACING.xs);
        components::caption(
            ui,
            if drawer.shift.float_minor > 0 {
                "Type what is in the drawer, then close. Take the takings out and leave the float."
            } else {
                "Type what is in the drawer, then close. Take the takings out."
            },
        );
    });
    ui.add_space(SPACING.lg);
}

/// One figure with its caption above it, in a row of peers.
pub(super) fn drawer_figure(
    ui: &mut egui::Ui,
    label: &str,
    value: &str,
    color: Option<egui::Color32>,
) {
    ui.vertical(|ui| {
        components::field_label(ui, label);
        ui.add_space(2.0);
        let text = components::numeric(value).size(TYPE.heading);
        ui.label(match color {
            Some(color) => text.color(color),
            None => text,
        });
    });
}
