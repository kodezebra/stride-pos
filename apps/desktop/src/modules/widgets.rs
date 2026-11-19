//! The pieces more than one screen draws.
//!
//! A dialog frame, a form field, a label/amount row, the row shapes the money
//! tables share, and the small action icons. They live here rather than beside
//! the first screen that needed them because a second screen needed them too,
//! and a copy per screen is how the cart and the payment screen drift apart.

use super::*;
/// One label/amount row: the cart's totals and the payment screen's due.
///
/// Shared with `checkout` on purpose — the amount the cashier reads on the cart
/// and the amount on the payment screen must sit on the same pixel.
pub(crate) fn amount_row(ui: &mut egui::Ui, label: &str, amount: &str, strong: bool) {
    let palette = theme::current(ui.ctx());
    // One right-to-left row of a *fixed* height, amount first: the total must
    // end on the same pixel as the Pay button above it, and the row must claim
    // only its own line. A bare `with_layout` child starts out as tall as the
    // space left in the pane, which would push every later row to the bottom.
    let height = if strong {
        TYPE.total * 1.4
    } else {
        TYPE.small * 1.8
    };
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), height),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            let text = components::numeric(amount);
            let text = if strong {
                text.size(TYPE.total).color(palette.text)
            } else {
                text.size(TYPE.small).color(palette.text_muted)
            };
            ui.label(text);

            if strong {
                ui.label(components::semibold(label).size(TYPE.body));
            } else {
                components::caption(ui, label);
            }
        },
    );
}

/// One row of a money table: identifier, when, amount.
///
/// Drawn by rectangle rather than with `Grid` because the amount must sit on the
/// card's right edge: a left-to-right grid sizes every column to its content, so
/// a column of totals ends up ragged in the middle of the pane.
/// One row of a sales table: sale number, when, by whom, and the amount paid.
///
/// Data rows answer clicks, which is what makes a sale worth talking about:
/// a list can only abbreviate, and the reverberating truth — lines, tender
/// and change — lives one tap away in the detail modal.
pub(super) fn table_row(
    ui: &mut egui::Ui,
    id: &str,
    when: &str,
    by: &str,
    amount: &str,
    header: bool,
    stripe: bool,
) -> egui::Response {
    let palette = theme::current(ui.ctx());
    let width = ui.available_width();
    let height = if header { 30.0 } else { ROW_HEIGHT };
    let sense = if header {
        egui::Sense::hover()
    } else {
        egui::Sense::click()
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), sense);

    let painter = ui.painter();
    if stripe {
        painter.rect_filled(
            rect,
            egui::CornerRadius::ZERO,
            egui::Color32::from(palette.surface_alt),
        );
    }
    // The hover highlight replaces the stripe rather than layering on it, so
    // a row being reached for reads the same on even and odd rows.
    if response.hovered() {
        painter.rect_filled(
            rect,
            egui::CornerRadius::ZERO,
            egui::Color32::from(palette.surface),
        );
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    let color = if header {
        egui::Color32::from(palette.text_muted)
    } else {
        egui::Color32::from(palette.text)
    };
    let size = if header { TYPE.small } else { TYPE.body };
    let proportional = if header {
        theme::semibold()
    } else {
        egui::FontFamily::Proportional
    };
    let mono = theme::numeric();
    let y = rect.center().y;

    let left = painter.layout_no_wrap(id.to_owned(), egui::FontId::new(size, mono.clone()), color);
    painter.galley(
        egui::pos2(rect.left(), y - left.size().y * 0.5),
        left,
        color,
    );

    let middle = painter.layout_no_wrap(
        when.to_owned(),
        egui::FontId::new(size, proportional.clone()),
        color,
    );
    painter.galley(
        egui::pos2(rect.left() + width * 0.30, y - middle.size().y * 0.5),
        middle,
        color,
    );

    // Who took it. Empty on a till that has not been set up with staff yet, and
    // on sales from before the column existed.
    let who = painter.layout_no_wrap(
        by.to_owned(),
        egui::FontId::new(size, proportional.clone()),
        color,
    );
    painter.galley(
        egui::pos2(rect.left() + width * 0.55, y - who.size().y * 0.5),
        who,
        color,
    );

    let right = painter.layout_no_wrap(amount.to_owned(), egui::FontId::new(size, mono), color);
    painter.galley(
        egui::pos2(rect.right() - right.size().x, y - right.size().y * 0.5),
        right,
        color,
    );

    if header {
        painter.hline(
            rect.x_range(),
            rect.max.y,
            egui::Stroke::new(1.0_f32, palette.border),
        );
    }

    response
}

/// One row of a three-column tally: a name, a count, and the money.
///
/// Shared by "By person" (who, sales, take) and "Top items" (what, units,
/// revenue) on purpose: both are ranked lists whose middle column is a count
/// the shopkeeper glances at, and neither lines up under `table_row`'s four
/// columns — there is no time cell to reuse.
pub(super) fn tally_row(
    ui: &mut egui::Ui,
    name: &str,
    count: &str,
    amount: &str,
    header: bool,
    stripe: bool,
) {
    let palette = theme::current(ui.ctx());
    let width = ui.available_width();
    let height = if header { 30.0 } else { ROW_HEIGHT };
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());

    let painter = ui.painter();
    if stripe {
        painter.rect_filled(
            rect,
            egui::CornerRadius::ZERO,
            egui::Color32::from(palette.surface_alt),
        );
    }

    let color = if header {
        egui::Color32::from(palette.text_muted)
    } else {
        egui::Color32::from(palette.text)
    };
    let size = if header { TYPE.small } else { TYPE.body };
    let proportional = if header {
        theme::semibold()
    } else {
        egui::FontFamily::Proportional
    };
    let mono = theme::numeric();
    let y = rect.center().y;

    let left = painter.layout_no_wrap(
        name.to_owned(),
        egui::FontId::new(size, proportional),
        color,
    );
    painter.galley(
        egui::pos2(rect.left(), y - left.size().y * 0.5),
        left,
        color,
    );

    let middle = painter.layout_no_wrap(
        count.to_owned(),
        egui::FontId::new(size, mono.clone()),
        color,
    );
    painter.galley(
        egui::pos2(rect.left() + width * 0.45, y - middle.size().y * 0.5),
        middle,
        color,
    );

    let right = painter.layout_no_wrap(amount.to_owned(), egui::FontId::new(size, mono), color);
    painter.galley(
        egui::pos2(rect.right() - right.size().x, y - right.size().y * 0.5),
        right,
        color,
    );

    if header {
        painter.hline(
            rect.x_range(),
            rect.max.y,
            egui::Stroke::new(1.0_f32, palette.border),
        );
    }
}

/// One row of the By person table: a name, sales, cash, and takings.
///
/// Four columns, so it cannot reuse [`tally_row`]'s three: cash and takings are
/// both money and must line up as columns, or the eye has to compare two
/// figures that wander with the name beside them.
pub(super) fn person_row(
    ui: &mut egui::Ui,
    person: &str,
    sales: &str,
    cash: &str,
    takings: &str,
    header: bool,
    stripe: bool,
) {
    let palette = theme::current(ui.ctx());
    let width = ui.available_width();
    let height = if header { 30.0 } else { ROW_HEIGHT };
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());

    let painter = ui.painter();
    if stripe {
        painter.rect_filled(
            rect,
            egui::CornerRadius::ZERO,
            egui::Color32::from(palette.surface_alt),
        );
    }

    let color = if header {
        egui::Color32::from(palette.text_muted)
    } else {
        egui::Color32::from(palette.text)
    };
    let size = if header { TYPE.small } else { TYPE.body };
    let proportional = if header {
        theme::semibold()
    } else {
        egui::FontFamily::Proportional
    };
    let mono = theme::numeric();
    let y = rect.center().y;

    let name = painter.layout_no_wrap(
        person.to_owned(),
        egui::FontId::new(size, proportional),
        color,
    );
    painter.galley(
        egui::pos2(rect.left(), y - name.size().y * 0.5),
        name,
        color,
    );

    for (fraction, text) in [(0.42, sales), (0.66, cash)] {
        let cell = painter.layout_no_wrap(
            text.to_owned(),
            egui::FontId::new(size, mono.clone()),
            color,
        );
        painter.galley(
            egui::pos2(rect.left() + width * fraction, y - cell.size().y * 0.5),
            cell,
            color,
        );
    }

    let right = painter.layout_no_wrap(takings.to_owned(), egui::FontId::new(size, mono), color);
    painter.galley(
        egui::pos2(rect.right() - right.size().x, y - right.size().y * 0.5),
        right,
        color,
    );

    if header {
        painter.hline(
            rect.x_range(),
            rect.max.y,
            egui::Stroke::new(1.0_f32, palette.border),
        );
    }
}

/// The edit glyph for a catalogue row.
///
/// `settings-2` is the edit affordance the desktop uses. It is in the Lucide
/// set but not yet in `ui-kit`'s vocabulary, so it is resolved by name — the
/// same lookup a settings file uses. The fallback means a renamed glyph costs a
/// different icon, never a missing button.
pub(super) fn edit_icon() -> Icon {
    Icon::named("settings-2").unwrap_or(Icon::SETTINGS)
}

/// A dialog frame, with the same breathing room everywhere.
///
/// egui's popup frame is sized for a menu: fine for a list of choices, cramped
/// for a form that is read and typed into. Every modal in the till goes through
/// here, so the product form, the receipt and the payment screen sit the same
/// distance from their edge — and at the dialog radius rather than the menu's.
pub(crate) fn modal(ctx: &egui::Context, id: egui::Id) -> egui::Modal {
    egui::Modal::new(id).frame(
        egui::Frame::popup(&ctx.global_style())
            .inner_margin(egui::Margin::same(SPACING.xl as i8))
            .corner_radius(egui::CornerRadius::same(RADII.lg as u8)),
    )
}

/// A single-line field at a fixed width, matching the height of `search_field`.
pub(crate) fn text_field(
    ui: &mut egui::Ui,
    text: &mut String,
    hint: &str,
    width: f32,
) -> egui::Response {
    field(ui, text, hint, width, false)
}

/// The same field, with the characters replaced by dots.
///
/// A PIN typed at a counter has a queue in front of it. The field still reports
/// what it holds to the caller; only the screen forgets.
pub(crate) fn secret_field(
    ui: &mut egui::Ui,
    text: &mut String,
    hint: &str,
    width: f32,
) -> egui::Response {
    field(ui, text, hint, width, true)
}

/// A field with a fixed prefix inside its frame, for an amount in one currency.
///
/// `TextEdit` has no prefix API, so the left margin is widened to make room and
/// the code is painted over the frame. The number then starts where the code
/// ends instead of floating in a box that looks empty.
pub(crate) fn prefixed_field(
    ui: &mut egui::Ui,
    text: &mut String,
    prefix: &str,
    hint: &str,
    width: f32,
) -> egui::Response {
    let palette = theme::current(ui.ctx());
    let font = egui::FontId::proportional(TYPE.body);
    let prefix_width = ui
        .painter()
        .layout_no_wrap(prefix.to_owned(), font.clone(), ui.visuals().text_color())
        .size()
        .x;
    let left = (12.0 + prefix_width + 6.0)
        .round()
        .clamp(0.0, i8::MAX as f32) as i8;

    ui.scope(|ui| {
        ui.visuals_mut().widgets.inactive.bg_stroke =
            egui::Stroke::new(1.0_f32, egui::Color32::from(palette.border_strong));
        let edit = egui::TextEdit::singleline(text)
            .hint_text(hint)
            .margin(egui::Margin {
                left,
                right: 12,
                top: 12,
                bottom: 12,
            });
        let response = ui.add_sized([width, 44.0], edit);
        ui.painter().text(
            egui::pos2(response.rect.min.x + 12.0, response.rect.center().y),
            egui::Align2::LEFT_CENTER,
            prefix,
            font,
            egui::Color32::from(palette.text_muted),
        );
        response
    })
    .inner
}

pub(super) fn field(
    ui: &mut egui::Ui,
    text: &mut String,
    hint: &str,
    width: f32,
    secret: bool,
) -> egui::Response {
    let palette = theme::current(ui.ctx());
    ui.scope(|ui| {
        ui.visuals_mut().widgets.inactive.bg_stroke =
            egui::Stroke::new(1.0_f32, egui::Color32::from(palette.border_strong));
        let edit = egui::TextEdit::singleline(text)
            .hint_text(hint)
            .margin(egui::Margin::symmetric(12, 12));
        ui.add_sized(
            [width, 44.0],
            if secret { edit.password(true) } else { edit },
        )
    })
    .inner
}

/// The archive glyph for a directory row.
///
/// `archive` is in the Lucide set but not in `ui-kit`'s vocabulary, so it is
/// resolved by name. The fallback means a renamed glyph costs a different icon,
/// never a missing button.
pub(super) fn archive_icon() -> Icon {
    Icon::named("archive").unwrap_or(Icon::BOXES)
}

pub(super) fn restore_icon() -> Icon {
    Icon::named("archive-restore").unwrap_or(Icon::REFUND)
}

/// The movement glyph. Two arrows say "in or out"; the tooltip names the four
/// reasons a movement can have.
pub(super) fn move_icon() -> Icon {
    Icon::named("arrow-right-left").unwrap_or(Icon::BOXES)
}

/// The history glyph, resolved by name like the directory's archive icon: the
/// vocabulary can move without taking a button with it.
pub(super) fn history_icon() -> Icon {
    Icon::named("history").unwrap_or(Icon::RECEIPT)
}
