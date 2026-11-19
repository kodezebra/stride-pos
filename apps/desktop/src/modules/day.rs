//! Day: the dashboard. What the shop took, and how that compares.
//!
//! Every number on this screen is a comparison — today against yesterday, this
//! week against last — so the formatting helpers that turn two counts into a
//! caption live here beside the cards that show them.

use super::*;
// ---------------------------------------------------------------------------
// Dashboard
// ---------------------------------------------------------------------------

pub(super) struct Dashboard;

impl Module for Dashboard {
    fn sidebar(&mut self, ui: &mut egui::Ui, state: &mut State) {
        components::section_title(ui, "Range");
        for (index, label) in state::TREND_RANGES.iter().enumerate() {
            if regions::nav_item(ui, state.trend_range == index, label).clicked() {
                state.trend_range = index;
            }
        }
    }

    fn workspace(&mut self, ui: &mut egui::Ui, state: &mut State) {
        let range = state::trend_window(state.trend_range);
        let scope = state.report_scope();
        let shop = scope.is_shop();
        let user = scope.user().map(str::to_owned);
        let user = user.as_deref();

        let (count, takings) = crate::db::totals(&state.store, range, user);
        let previous = crate::db::previous_totals(&state.store, range, user);
        let margin = crate::db::margin(&state.store, range, user);

        components::caption(
            ui,
            &format!(
                "{} · {}",
                state::range_caption(range),
                comparison_caption(range, shop)
            ),
        );
        ui.add_space(SPACING.md);

        if count == 0 {
            components::empty_state_card(
                ui,
                "No sales yet in this range",
                "Take a sale, and the shop's numbers start here.",
                200.0,
            );
            return;
        }

        // Six figures and no more. A shopkeeper asks four questions of a month:
        // what came in, what was kept, how many people paid, and what each of
        // them spent. The other two — how they paid, and what they bought — are
        // tables below, because a total cannot answer them.
        let (takings_sub, takings_trend) =
            change(takings, previous.map(|(_, revenue)| revenue), true);
        let (sales_sub, sales_trend) = change(count, previous.map(|(count, _)| count), false);
        let mut cards = vec![
            Kpi {
                label: "Takings",
                value: state.money(takings),
                sub: takings_sub,
                trend: takings_trend,
            },
            Kpi {
                label: "Sales",
                value: count.to_string(),
                sub: sales_sub,
                trend: sales_trend,
            },
            Kpi {
                label: "Average basket",
                value: state.money(takings / count),
                sub: "per sale".to_owned(),
                trend: Trend::Flat,
            },
        ];
        // Margin only where the shop records costs, and only over the items that
        // have one: a percentage across uncosted sales would be a guess wearing
        // a decimal point.
        if margin.has_cost() && margin.revenue_minor > 0 {
            let profit = margin.profit_minor();
            cards.push(Kpi {
                label: "Gross margin",
                value: format!(
                    "{:.0}%",
                    profit as f64 * 100.0 / margin.revenue_minor as f64
                ),
                sub: format!("{} kept", state.money(profit)),
                trend: Trend::Flat,
            });
        }
        kpi_row(ui, &cards);

        if margin.has_cost() && !margin.is_complete() {
            components::caption(
                ui,
                &format!(
                    "Margin covers {} of {} sale lines; the rest were sold before \
                     their cost was known.",
                    margin.costed_lines, margin.total_lines
                ),
            );
        }

        // Cash or not, because that is what decides whether the drawer has to be
        // counted tonight.
        ui.add_space(SPACING.lg);
        components::section_title(ui, "How it was paid");
        let methods = crate::db::by_method(&state.store, range, user);
        components::card(ui, |ui| {
            ui.set_min_width(ui.available_width());
            tally_row(ui, "Method", "Taken", "Share", true, false);
            for (index, method) in methods.iter().enumerate() {
                let share = if takings > 0 {
                    format!(
                        "{:.0}%",
                        method.amount_minor as f64 * 100.0 / takings as f64
                    )
                } else {
                    "—".to_owned()
                };
                tally_row(
                    ui,
                    &method_label(&method.method, method.sales),
                    &state.money(method.amount_minor),
                    &share,
                    false,
                    index % 2 == 1,
                );
            }
        });

        // Who took it, and how much of it was cash. The shop view is the only
        // one that can see other people's takings; a cashier's own numbers are
        // already the whole page.
        if shop {
            ui.add_space(SPACING.lg);
            components::section_title(ui, "By person");
            let people = crate::db::by_person(&state.store, range, user);
            components::card(ui, |ui| {
                ui.set_min_width(ui.available_width());
                person_row(ui, "Person", "Sales", "Cash", "Takings", true, false);
                for (index, person) in people.iter().enumerate() {
                    person_row(
                        ui,
                        &person.name,
                        &person.sales.to_string(),
                        &state.money(person.cash_minor),
                        &state.money(person.takings_minor),
                        false,
                        index % 2 == 1,
                    );
                }
            });
        }

        // One chart, because a shopkeeper reads a shape, not a spreadsheet. All
        // time has no calendar to draw, so its chart is capped and says so.
        ui.add_space(SPACING.lg);
        components::section_title(ui, "By day");
        if let Some(caption) = chart_caption(range) {
            components::caption(ui, caption);
        }
        let days = crate::db::by_day(&state.store, range, user);
        components::card(ui, |ui| {
            ui.set_min_width(ui.available_width());
            day_chart(ui, state, &days);
        });

        // Two sentences, because the numbers above do not say themselves out
        // loud: when the money came in, and what took the most of it.
        let busiest = days
            .iter()
            .max_by_key(|day| day.revenue)
            .filter(|day| day.revenue > 0);
        let peak = crate::db::peak_hour(&state.store, range, user);
        if busiest.is_some() || peak.is_some() {
            ui.add_space(SPACING.lg);
            components::card(ui, |ui| {
                ui.set_min_width(ui.available_width());
                if let Some(busiest) = busiest {
                    ui.label(
                        egui::RichText::new(format!(
                            "{} took the most: {}, {:.0}% of takings.",
                            weekday_sentence(busiest.weekday),
                            state.money(busiest.revenue),
                            busiest.revenue as f64 * 100.0 / takings as f64
                        ))
                        .size(TYPE.body),
                    );
                }
                if let Some(peak) = peak {
                    if busiest.is_some() {
                        ui.add_space(SPACING.xs);
                    }
                    ui.label(
                        egui::RichText::new(format!(
                            "Most sales land around {:02}:00 — {} of them.",
                            peak.hour, peak.sales
                        ))
                        .size(TYPE.body),
                    );
                }
            });
        }

        // What actually sells. Ranked by the money each name took, with the
        // units beside it — neither number alone is the story: many cheap units
        // can out-earn a few dear ones, and either way the owner wants both.
        ui.add_space(SPACING.lg);
        components::section_title(ui, if shop { "Top items" } else { "Your top items" });
        let items = crate::db::top_items(&state.store, range, 5, user);
        if items.is_empty() {
            components::empty_state_card(
                ui,
                "No items sold in this range",
                "Once the till rings things up, the best movers rank here.",
                160.0,
            );
        } else {
            components::card(ui, |ui| {
                ui.set_min_width(ui.available_width());
                tally_row(ui, "Item", "Sold", "Revenue", true, false);
                for (index, item) in items.iter().enumerate() {
                    tally_row(
                        ui,
                        &item.name,
                        &item.qty.to_string(),
                        &state.money(item.revenue),
                        false,
                        index % 2 == 1,
                    );
                }
            });
        }
    }
}

/// How a figure moved against the window before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Trend {
    Up,
    Down,
    Flat,
}

/// One card at the top of the dashboard.
pub(super) struct Kpi {
    label: &'static str,
    value: String,
    /// The change against the previous window, or a plain reading of the figure
    /// ("per sale").
    sub: String,
    trend: Trend,
}

/// Cards laid out by width arithmetic, not by wrapping.
///
/// `horizontal_wrapped` decides where to break from the *parent's* available
/// width, which on a panel that already overflowed stays wrong for the rest of
/// the frame — the row spills past the window edge and drags every centred
/// widget below it out of place. Columns here are a function of the panel width,
/// so the row always sums to what is on screen.
pub(super) fn kpi_row(ui: &mut egui::Ui, cards: &[Kpi]) {
    let gap = SPACING.sm;
    let available = ui.available_width();
    // Three figures in a four-column grid would leave a hole where the fourth
    // belongs, so the row never has more columns than it has cards.
    let columns = (((available + gap) / (140.0 + gap)).floor() as usize)
        .clamp(1, 4)
        .min(cards.len().max(1));
    let card = ((available - (columns as f32 - 1.0) * gap) / columns as f32).floor();

    for chunk in cards.chunks(columns) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for kpi in chunk {
                kpi_card(ui, card, kpi);
            }
        });
        ui.add_space(gap);
    }
}

pub(super) fn kpi_card(ui: &mut egui::Ui, width: f32, kpi: &Kpi) {
    components::card(ui, |ui| {
        // The frame adds its margin around this width, so the card lands on the
        // column width instead of one margin too wide.
        ui.set_width((width - 2.0 * SPACING.lg).max(60.0));
        components::field_label(ui, kpi.label);
        ui.add_space(SPACING.xs);
        ui.label(components::numeric(&kpi.value).size(TYPE.kpi));
        ui.add_space(SPACING.xs);
        let palette = theme::current(ui.ctx());
        let color = match kpi.trend {
            Trend::Up => egui::Color32::from(palette.success),
            Trend::Down => egui::Color32::from(palette.danger),
            Trend::Flat => egui::Color32::from(palette.text_muted),
        };
        ui.label(egui::RichText::new(&kpi.sub).size(TYPE.small).color(color));
    });
}

/// A figure's change against the window before it, and the direction to paint.
///
/// Money moves as a percentage and counts as units: "+12%" is how a shopkeeper
/// reads a week's takings, and "+3" is how they read three more sales. A window
/// that took nothing cannot be a percentage, so the first sale in it is `new`;
/// two empty windows have nothing to compare and show a dash. All time passes no
/// previous window at all, and reads the same.
pub(super) fn change(now: i64, before: Option<i64>, money: bool) -> (String, Trend) {
    let Some(before) = before else {
        return ("—".to_owned(), Trend::Flat);
    };
    if before == 0 {
        return if now == 0 {
            ("—".to_owned(), Trend::Flat)
        } else {
            ("new".to_owned(), Trend::Up)
        };
    }

    let delta = now - before;
    if delta == 0 {
        return ("—".to_owned(), Trend::Flat);
    }
    let trend = if delta > 0 { Trend::Up } else { Trend::Down };

    if money {
        let percent = (delta as f64 * 100.0 / before as f64).round() as i64;
        if percent == 0 {
            ("—".to_owned(), Trend::Flat)
        } else {
            (format!("{percent:+}%"), trend)
        }
    } else {
        (format!("{delta:+}"), trend)
    }
}

/// What the cards are being compared with, said once under the page title.
///
/// A continuation, not a sentence: it is joined to the range name on the same
/// line, so the page reads "Last 7 days · compared with the 7 days before"
/// rather than stacking two headings.
///
/// All time has no previous window, so its line says what the figures are
/// instead of promising a comparison.
pub(super) fn comparison_caption(range: usize, shop: bool) -> &'static str {
    if shop {
        match range {
            0 => "compared with yesterday",
            1 => "compared with the 7 days before",
            2 => "compared with the 30 days before",
            _ => "everything the shop has sold",
        }
    } else {
        match range {
            0 => "only your sales · compared with yesterday",
            1 => "only your sales · compared with the 7 days before",
            2 => "only your sales · compared with the 30 days before",
            _ => "only your sales",
        }
    }
}

/// All time has no calendar to draw, so its chart is capped and says so.
pub(super) fn chart_caption(range: usize) -> Option<&'static str> {
    match range {
        0..=2 => None,
        _ => Some("The latest 30 selling days"),
    }
}

/// `cash` as `Cash · 12 sales`: the column stores what the payment code writes,
/// and the page prints it the way a person would, with the count that says
/// whether the money came in one big sale or a dozen small ones.
pub(super) fn method_label(method: &str, sales: i64) -> String {
    let mut chars = method.chars();
    let name: String = match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    };
    if sales == 1 {
        format!("{name} · 1 sale")
    } else {
        format!("{name} · {sales} sales")
    }
}

/// The range's daily revenue as a bar chart.
///
/// Painted rather than laid out widget by widget: a chart is one shape whose
/// parts must share a baseline and one scale, and independent rows cannot
/// promise either. `days` arrives newest first; the chart draws oldest on the
/// left, so the range reads left to right like a calendar.
pub(super) fn day_chart(ui: &mut egui::Ui, state: &State, days: &[crate::db::DayTotal]) {
    let palette = theme::current(ui.ctx());
    let mono = theme::numeric();
    let width = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, CHART_HEIGHT), egui::Sense::hover());
    let painter = ui.painter();

    let hue = chart_hue(ui);
    // The slot wash is the series colour at a whisper, not the accent wash:
    // the highlight has to belong to the bar under it.
    let wash = egui::Color32::from_rgba_unmultiplied(hue.r(), hue.g(), hue.b(), 26);

    let peak = days.iter().map(|day| day.revenue).max().unwrap_or_default();
    let (axis_max, step) = nice_axis(peak);

    let muted = egui::Color32::from(palette.text_muted);

    // The month is said once, over the plot, so the ticks below can be bare
    // day numbers. `26 27 28` under a "26 Oct – 1 Nov" is a calendar; `26/10`
    // ten times over is a spreadsheet.
    let caption = painter.layout_no_wrap(
        period_label(days),
        egui::FontId::new(TYPE.small, mono.clone()),
        muted,
    );
    let caption_height = caption.size().y;
    painter.galley(
        egui::pos2(rect.right() - caption.size().x, rect.top()),
        caption,
        muted,
    );

    // A gutter for the value labels, the caption's row, and a strip under the
    // bars for the dates.
    let plot = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 46.0, rect.top() + caption_height + 10.0),
        egui::pos2(rect.right(), rect.bottom() - 24.0),
    );

    // Four steps of a round size. The baseline is the axis and gets the
    // stronger hairline; the rest are hints behind the bars.
    let ticks = (axis_max / step) as usize;
    for tick in 0..=ticks {
        let fraction = tick as f32 / ticks as f32;
        let y = plot.bottom() - plot.height() * fraction;
        painter.hline(
            plot.x_range(),
            y,
            egui::Stroke::new(
                1.0,
                if tick == 0 {
                    egui::Color32::from(palette.border_strong)
                } else {
                    egui::Color32::from(palette.border)
                },
            ),
        );
        let galley = painter.layout_no_wrap(
            compact_amount(step * tick as i64, state),
            egui::FontId::new(TYPE.small, mono.clone()),
            muted,
        );
        painter.galley(
            egui::pos2(
                plot.left() - 10.0 - galley.size().x,
                y - galley.size().y * 0.5,
            ),
            galley,
            muted,
        );
    }

    // One slot per day, with air on both sides of the bar so neighbouring days
    // never read as one block. The pointer selects a slot, so the highlight
    // and the tooltip always agree with the bar under it.
    let count = days.len();
    let slot = plot.width() / count as f32;
    let bar_width = (slot * 0.56).clamp(3.0, 36.0);
    let hovered = response
        .hover_pos()
        .filter(|pos| plot.contains(*pos))
        .map(|pos| (((pos.x - plot.left()) / slot) as usize).min(count - 1));

    for (index, day) in days.iter().rev().enumerate() {
        let centre = plot.left() + slot * (index as f32 + 0.5);

        if hovered == Some(index) {
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(centre - slot * 0.5 + 1.0, plot.top()),
                    egui::pos2(centre + slot * 0.5 - 1.0, plot.bottom()),
                ),
                egui::CornerRadius::same(6),
                wash,
            );
        }

        let fraction = (day.revenue as f32 / axis_max as f32).clamp(0.0, 1.0);
        let top = plot.bottom() - plot.height() * fraction;
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(centre - bar_width * 0.5, top),
                egui::pos2(centre + bar_width * 0.5, plot.bottom()),
            ),
            egui::CornerRadius {
                nw: 4,
                ne: 4,
                sw: 0,
                se: 0,
            },
            hue,
        );
    }

    // As many dates as fit without colliding, counted from the newest day so
    // the right edge is always labelled. A week is read as a rhythm, so its
    // days are named; a month is read as a calendar, so it is numbered. The
    // step comes from the widest label the axis will draw, so a long range
    // thins out instead of running the numbers together.
    let widest = days
        .iter()
        .map(|day| axis_label(day, count))
        .max_by_key(|label| label.len())
        .unwrap_or_default();
    let label_width = painter
        .layout_no_wrap(widest, egui::FontId::new(TYPE.small, mono.clone()), muted)
        .size()
        .x
        + 12.0;
    let every = (count as f32 * label_width / plot.width()).ceil().max(1.0) as usize;
    for (index, day) in days.iter().rev().enumerate() {
        if !(count - 1 - index).is_multiple_of(every) {
            continue;
        }
        let centre = plot.left() + slot * (index as f32 + 0.5);
        let galley = painter.layout_no_wrap(
            axis_label(day, count),
            egui::FontId::new(TYPE.small, mono.clone()),
            muted,
        );
        painter.galley(
            egui::pos2(centre - galley.size().x * 0.5, plot.bottom() + 6.0),
            galley,
            muted,
        );
    }

    if let Some(index) = hovered {
        let day = &days[count - 1 - index];
        let centre = plot.left() + slot * (index as f32 + 0.5);
        let fraction = (day.revenue as f32 / axis_max as f32).clamp(0.0, 1.0);
        let top = plot.bottom() - plot.height() * fraction;
        day_callout(painter, plot, centre, top, day, state, &palette);
    }
}

/// The chart's own colour.
///
/// Deliberately not `accent`: in this app blue is the colour of things you can
/// press, and a bar is a recorded fact, not a button. Violet also stays clear
/// of the green that means "paid" and the red that means "wrong", so a bar can
/// never be misread as a state.
pub(super) fn chart_hue(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        egui::Color32::from_rgb(0xa5, 0x96, 0xff)
    } else {
        egui::Color32::from_rgb(0x6a, 0x5c, 0xe8)
    }
}

/// The hover card over a bar: two lines, pinned to the bar rather than to the
/// pointer.
///
/// egui's default tooltip follows the cursor, so on a chart it drifts off the
/// bar it describes and covers its neighbours. This one sits directly above
/// the bar, stays inside the plot, and is only as tall as a date and a figure —
/// the three stacked labels of the old tooltip read as a paragraph.
pub(super) fn day_callout(
    painter: &egui::Painter,
    plot: egui::Rect,
    centre: f32,
    bar_top: f32,
    day: &crate::db::DayTotal,
    state: &State,
    palette: &ui_kit::tokens::Palette,
) {
    let text = egui::Color32::from(palette.text);
    let muted = egui::Color32::from(palette.text_muted);

    let heading = painter.layout_no_wrap(
        format!("{} · {} sales", long_date(day), day.sales),
        egui::FontId::new(TYPE.small, theme::semibold()),
        muted,
    );
    let amount = painter.layout_no_wrap(
        state.money(day.revenue),
        egui::FontId::new(TYPE.body, theme::numeric()),
        text,
    );
    let heading_size = heading.size();
    let amount_size = amount.size();

    let pad_x = SPACING.sm;
    let pad_y = 7.0;
    let gap = 2.0;
    let width = heading_size.x.max(amount_size.x) + pad_x * 2.0;
    let height = heading_size.y + gap + amount_size.y + pad_y * 2.0;

    // Above the bar when the plot has room, otherwise inside the bar's top; and
    // never past either edge, so the card cannot hang over the axis labels.
    let left = (centre - width * 0.5).clamp(plot.left(), (plot.right() - width).max(plot.left()));
    let above = bar_top - SPACING.sm - height;
    let top = if above >= plot.top() {
        above
    } else {
        (bar_top + SPACING.sm).min(plot.bottom() - height)
    };
    let rect = egui::Rect::from_min_size(egui::pos2(left, top), egui::vec2(width, height));
    let corner = egui::CornerRadius::same(RADII.sm as u8);

    painter.rect_filled(rect, corner, egui::Color32::from(palette.surface));
    painter.rect_stroke(
        rect,
        corner,
        egui::Stroke::new(1.0_f32, egui::Color32::from(palette.border_strong)),
        egui::StrokeKind::Inside,
    );
    painter.galley(
        egui::pos2(rect.left() + pad_x, rect.top() + pad_y),
        heading,
        muted,
    );
    painter.galley(
        egui::pos2(
            rect.left() + pad_x,
            rect.top() + pad_y + heading_size.y + gap,
        ),
        amount,
        text,
    );
}

/// A round axis that fits `peak`: a step of a 1/2/2.5/5 × 10ⁿ size, chosen so
/// about [`AXIS_STEPS`] of them span the data, then rounded up to a whole step
/// above the tallest bar.
///
/// The top of the axis is the next step, not a fixed multiple: a peak of
/// 258,700 gets 300,000 with 100,000 gridlines, so the bars use most of the
/// height instead of floating under an empty band.
pub(super) fn nice_axis(peak: i64) -> (i64, i64) {
    if peak <= 0 {
        return (1, 1);
    }
    let rough = peak as f64 / AXIS_STEPS as f64;
    let magnitude = 10f64.powf(rough.log10().floor());
    let normal = rough / magnitude;
    let nice = if normal <= 1.0 {
        1.0
    } else if normal <= 2.0 {
        2.0
    } else if normal <= 2.5 {
        2.5
    } else if normal <= 5.0 {
        5.0
    } else {
        10.0
    };
    let step = (nice * magnitude).round().max(1.0) as i64;
    ((peak + step - 1) / step * step, step)
}

/// A gridline value in the till's currency, short enough for a 40 px gutter:
/// `250K`, `1.2M`, `900`.
pub(super) fn compact_amount(minor: i64, state: &State) -> String {
    let scale = money::Money::new(0, state.currency).scale();
    let value = minor as f64 / 10f64.powi(scale as i32);
    if value >= 1_000_000.0 {
        format!("{:.1}M", value / 1_000_000.0)
    } else if value >= 10_000.0 {
        format!("{:.0}K", value / 1_000.0)
    } else if value >= 1_000.0 {
        format!("{:.1}K", value / 1_000.0)
    } else {
        format!("{value:.0}")
    }
}

/// One date-axis label. A week is read as a rhythm, so its days are named; a
/// month is read as a calendar, so it is numbered. The month itself is said
/// once over the plot, so no tick repeats it.
pub(super) fn axis_label(day: &crate::db::DayTotal, count: usize) -> String {
    if count <= 8 {
        weekday_name(day.weekday).to_owned()
    } else {
        day_of_month(day).to_owned()
    }
}

/// The span the chart covers: `26 Oct – 1 Nov`, or one month said once.
pub(super) fn period_label(days: &[crate::db::DayTotal]) -> String {
    match days {
        [] => String::new(),
        [only] => format!("{} {}", day_of_month(only), month_name(only)),
        [newest, .., oldest] => {
            if month_name(newest) == month_name(oldest) {
                format!(
                    "{}–{} {}",
                    day_of_month(oldest),
                    day_of_month(newest),
                    month_name(newest)
                )
            } else {
                format!(
                    "{} {} – {} {}",
                    day_of_month(oldest),
                    month_name(oldest),
                    day_of_month(newest),
                    month_name(newest)
                )
            }
        }
    }
}

/// `Sun 1 Nov` — the way a day is said out loud, for the hover card.
pub(super) fn long_date(day: &crate::db::DayTotal) -> String {
    format!(
        "{} {} {}",
        weekday_name(day.weekday),
        day_of_month(day),
        month_name(day)
    )
}

/// `dd/mm` without the zero the calendar pads it with: an axis tick says `9`.
pub(super) fn day_of_month(day: &crate::db::DayTotal) -> &str {
    match day.day.get(..2).map(|day| day.trim_start_matches('0')) {
        Some(day) if !day.is_empty() => day,
        _ => "0",
    }
}

pub(super) fn month_name(day: &crate::db::DayTotal) -> &'static str {
    let month = day
        .day
        .get(3..5)
        .and_then(|month| month.parse::<usize>().ok())
        .unwrap_or(1);
    MONTHS[month.clamp(1, MONTHS.len()) - 1]
}

pub(super) fn weekday_name(weekday: u8) -> &'static str {
    WEEKDAYS[(weekday as usize).min(WEEKDAYS.len() - 1)]
}

/// The same day, spelled out, for the sentences under the chart. A tick can say
/// `Fri`; a sentence cannot.
pub(super) fn weekday_sentence(weekday: u8) -> &'static str {
    const NAMES: [&str; 7] = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    NAMES[(weekday as usize).min(NAMES.len() - 1)]
}

/// The glyph for the catalog's transfer menu: in and out are the same file
/// moving, so one picture covers both. Resolved by name because the shared
/// vocabulary does not carry a file-transfer icon yet; the fallback costs a
/// different picture, never a missing action.
pub(super) fn transfer_icon() -> Icon {
    Icon::named("arrow-up-down").unwrap_or(Icon::DOCUMENT)
}
/// Height of the daily-revenue chart, axis labels and date strip included.
pub(super) const CHART_HEIGHT: f32 = 232.0;

/// Steps in the chart's value axis.
pub(super) const AXIS_STEPS: usize = 4;

/// `%w` names, Sunday first, because that is how SQLite numbers the week.
pub(super) const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// Months for the span over the plot — `strftime` hands back numbers only.
pub(super) const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
