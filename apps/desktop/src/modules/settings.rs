//! Settings: the shop's own details, the terminal, sync, and the receipt face.
//!
//! One screen of small cards. Each card owns one decision the shopkeeper makes
//! once and then stops thinking about, so each one is drawn by its own function
//! and only the screen has to know they exist.

use super::*;
// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

pub(super) struct Settings;

/// The appearance controls: one card, two rows.
///
/// The toolbar already names the section, so the card starts with the first
/// setting instead of repeating the title. Both choices apply immediately and
/// are written to settings by `State`.
pub(super) fn appearance_card(ui: &mut egui::Ui, state: &mut State) {
    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());

        setting_row(
            ui,
            "Theme",
            "Light or dark, across the whole till.",
            THEME_CONTROL_WIDTH,
            |ui| {
                if let Some(chosen) = theme_segmented(ui, state.theme) {
                    state.set_theme(chosen);
                }
            },
        );

        components::divider(ui);

        setting_row(
            ui,
            "Accent colour",
            "Tints buttons, links and highlights.",
            ACCENT_CONTROL_WIDTH,
            |ui| {
                if let Some(accent) = accent_picker(ui, state.brand_accent) {
                    state.set_brand_accent(accent);
                }
            },
        );
    });
}

/// The width of one accent circle, and of the quiet "Default" chip beside them.
pub(super) const SWATCH_SIZE: f32 = 40.0;

pub(super) const DEFAULT_CHIP_WIDTH: f32 = 78.0;

/// Exact widths let [`setting_row`] reserve the right column before the control
/// is drawn, so the title and hint never reflow when a control changes state.
pub(super) const THEME_CONTROL_WIDTH: f32 = 208.0;

pub(super) const ACCENT_CONTROL_WIDTH: f32 =
    BRANDS.len() as f32 * SWATCH_SIZE + BRANDS.len() as f32 * SPACING.sm + DEFAULT_CHIP_WIDTH;

/// How much room a hint needs before the row is worth splitting in two. Below
/// this the title, hint and control stack, because a hint squeezed narrower
/// than this wraps and stops being a hint.
pub(super) const ROW_TEXT_MIN_WIDTH: f32 = 240.0;

/// One settings row: a title and a short hint on the left, its control on the
/// right.
///
/// The control gets a fixed-width column so the row cannot depend on how wide
/// the widget happens to measure. Below a small window width the pair stacks
/// instead of squeezing the hint into a column too narrow to read.
pub(super) fn setting_row<R>(
    ui: &mut egui::Ui,
    title: &str,
    hint: &str,
    control_width: f32,
    control: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let height = 46.0;
    if ui.available_width() < control_width + ROW_TEXT_MIN_WIDTH {
        ui.label(components::semibold(title).size(TYPE.body));
        ui.add_space(SPACING.xs);
        components::caption(ui, hint);
        ui.add_space(SPACING.sm);
        return control(ui);
    }

    let full = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(full, height), egui::Sense::hover());
    let text_width = (full - control_width - SPACING.lg).max(0.0);

    let mut label_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(egui::Rect::from_min_size(
                rect.min,
                egui::vec2(text_width, height),
            ))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    label_ui.label(components::semibold(title).size(TYPE.body));
    label_ui.add_space(SPACING.xs);
    components::caption(&mut label_ui, hint);

    let mut control_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(egui::Rect::from_min_size(
                egui::pos2(rect.max.x - control_width, rect.min.y),
                egui::vec2(control_width, height),
            ))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    control(&mut control_ui)
}

/// A two-way switch with an icon and a word per side.
///
/// Selected is drawn the same way a selected navigation row is: a pale accent
/// wash with accent ink. The track is a well, so the control reads as one
/// object rather than two buttons that happen to be adjacent.
pub(super) fn theme_segmented(ui: &mut egui::Ui, selected: theme::Theme) -> Option<theme::Theme> {
    let palette = theme::current(ui.ctx());
    let options = [
        (theme::Theme::Light, Icon::SUN, theme::Theme::Light.label()),
        (theme::Theme::Dark, Icon::MOON, theme::Theme::Dark.label()),
    ];
    let height = 40.0;
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(THEME_CONTROL_WIDTH, height),
        egui::Sense::hover(),
    );
    let inset = 3.0;
    let gap = 2.0;
    let segment_width = (rect.width() - inset * 2.0 - gap) / 2.0;
    let mut choice = None;

    if ui.is_rect_visible(rect) {
        ui.painter().rect(
            rect,
            egui::CornerRadius::same(RADII.pill as u8),
            palette.surface_alt,
            egui::Stroke::new(1.0_f32, palette.border),
            egui::StrokeKind::Inside,
        );
    }

    for (index, (side, icon, label)) in options.into_iter().enumerate() {
        let segment = egui::Rect::from_min_size(
            egui::pos2(
                rect.min.x + inset + index as f32 * (segment_width + gap),
                rect.min.y + inset,
            ),
            egui::vec2(segment_width, height - inset * 2.0),
        );
        let response = ui.interact(
            segment,
            ui.id().with(("appearance_theme", index)),
            egui::Sense::click(),
        );
        if response.clicked() {
            choice = Some(side);
        }

        if ui.is_rect_visible(segment) {
            let active = side == selected;
            if active || response.hovered() {
                ui.painter().rect(
                    segment,
                    egui::CornerRadius::same(RADII.pill as u8),
                    if active {
                        palette.accent_soft
                    } else {
                        palette.surface
                    },
                    egui::Stroke::new(
                        1.0_f32,
                        if active {
                            palette.accent_ink()
                        } else {
                            palette.border
                        },
                    ),
                    egui::StrokeKind::Inside,
                );
            }

            let color = if active {
                palette.accent_ink()
            } else {
                palette.text
            };
            let galley = ui.painter().layout_no_wrap(
                label.to_owned(),
                egui::FontId::new(TYPE.small, theme::semibold()),
                color.into(),
            );
            let glyph = 16.0;
            let glyph_gap = SPACING.xs + 2.0;
            let content_width = glyph + glyph_gap + galley.size().x;
            let start = segment.center().x - content_width * 0.5;

            icon.image(glyph, color.into()).paint_at(
                ui,
                egui::Rect::from_center_size(
                    egui::pos2(start + glyph * 0.5, segment.center().y),
                    egui::Vec2::splat(glyph),
                ),
            );
            ui.painter().galley(
                egui::pos2(
                    start + glyph + glyph_gap,
                    segment.center().y - galley.size().y * 0.5,
                ),
                galley,
                color.into(),
            );
        }

        let _ = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    }

    choice
}

/// Four brand accents, plus a way back to the theme's own accent.
///
/// `Some(None)` means the default was chosen, `Some(Some(accent))` a brand, and
/// `None` that nothing was clicked. The caller owns the state write so the
/// comparison after the call still sees the old value.
pub(super) fn accent_picker(ui: &mut egui::Ui, selected: Option<Rgb>) -> Option<Option<Rgb>> {
    let mut choice = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = SPACING.sm;
        for brand in BRANDS {
            let active = selected == Some(brand.accent);
            if accent_swatch(ui, brand.accent, brand.name, active).clicked() {
                choice = Some(Some(brand.accent));
            }
        }
        if default_chip(ui, selected.is_none()).clicked() {
            choice = Some(None);
        }
    });
    choice
}

pub(super) fn accent_swatch(
    ui: &mut egui::Ui,
    accent: Rgb,
    label: &str,
    selected: bool,
) -> egui::Response {
    let palette = theme::current(ui.ctx());
    let (rect, response) =
        ui.allocate_exact_size(egui::Vec2::splat(SWATCH_SIZE), egui::Sense::click());

    if ui.is_rect_visible(rect) {
        let center = rect.center();
        let outer = SWATCH_SIZE * 0.5;
        if selected {
            ui.painter().circle_stroke(
                center,
                outer - 3.0,
                egui::Stroke::new(2.0_f32, palette.accent_ink()),
            );
        } else if response.hovered() {
            ui.painter().circle_stroke(
                center,
                outer - 3.0,
                egui::Stroke::new(1.0_f32, palette.border_strong),
            );
        }
        ui.painter()
            .circle_filled(center, outer - 7.0, egui::Color32::from(accent));

        if selected {
            Icon::CHECK.image(14.0, accent.best_ink().into()).paint_at(
                ui,
                egui::Rect::from_center_size(center, egui::Vec2::splat(14.0)),
            );
        }
    }

    response
        .on_hover_text(label)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The quiet option at the end of the swatch row: drop the brand override.
pub(super) fn default_chip(ui: &mut egui::Ui, selected: bool) -> egui::Response {
    let palette = theme::current(ui.ctx());
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(DEFAULT_CHIP_WIDTH, 32.0), egui::Sense::click());

    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let fill: egui::Color32 = if selected {
            palette.accent_soft.into()
        } else if hovered {
            palette.surface_alt.into()
        } else {
            egui::Color32::TRANSPARENT
        };
        let stroke = if selected {
            egui::Stroke::new(1.0_f32, palette.accent_ink())
        } else if hovered {
            egui::Stroke::new(1.0_f32, palette.border_strong)
        } else {
            egui::Stroke::NONE
        };
        ui.painter().rect(
            rect,
            egui::CornerRadius::same(RADII.pill as u8),
            fill,
            stroke,
            egui::StrokeKind::Inside,
        );

        let color = if selected {
            palette.accent_ink()
        } else {
            palette.text_muted
        };
        let galley = ui.painter().layout_no_wrap(
            "Default".to_owned(),
            egui::FontId::new(TYPE.small, theme::semibold()),
            color.into(),
        );
        ui.painter().galley(
            egui::pos2(
                rect.center().x - galley.size().x * 0.5,
                rect.center().y - galley.size().y * 0.5,
            ),
            galley,
            color.into(),
        );
    }

    response
        .on_hover_text("Use the theme's own accent")
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The phone scanner: one switch, the address to type, and a QR code that
/// carries all of it.
///
/// The listener is off by default. Turning it on is the whole opt-in: nothing
/// binds a port until this card says so, and turning it off stops the worker in
/// the same frame the shell next syncs.
pub(super) fn scanner_card(ui: &mut egui::Ui, state: &mut State) {
    let config = state.settings.scanner.clone();
    let status = state.scanner_status.clone();
    let palette = theme::current(ui.ctx());

    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        components::caption(
            ui,
            "Scan barcodes with an Android phone on the same Wi-Fi. Scans land in the \
             scan field exactly as a USB scanner's would.",
        );
        ui.add_space(SPACING.md);

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.sm;
            if config.enabled {
                if actions::secondary_compact(ui, "Turn off").clicked() {
                    state.settings.scanner.enabled = false;
                    state.settings_dirty = true;
                }
            } else if actions::secondary_compact(ui, "Turn on").clicked() {
                state.settings.scanner.enabled = true;
                state.settings_dirty = true;
            }

            let (text, color) = match &status {
                crate::scan::ScanStatus::Off => {
                    ("Off. Nothing is listening.".to_owned(), palette.text_muted)
                }
                crate::scan::ScanStatus::Listening { port } => {
                    (format!("Listening on port {port}."), palette.success)
                }
                crate::scan::ScanStatus::Error(error) => (error.clone(), palette.danger),
            };
            ui.label(egui::RichText::new(text).size(TYPE.small).color(color));
        });

        ui.add_space(SPACING.xs);
        components::caption(
            ui,
            "Only devices with this till's pairing code can send a scan. Keep the till on \
             the shop network; this listener is not for the internet.",
        );
    });

    if !config.enabled {
        return;
    }

    ui.add_space(SPACING.lg);
    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        components::caption(
            ui,
            "Open Stride Scan on the phone and point it at this code. One till can take \
             scans from as many phones as the shop pairs.",
        );
        ui.add_space(SPACING.md);

        let Some(address) = crate::scan::local_ip() else {
            components::empty_state_card(
                ui,
                "No network address",
                "Connect this till to the shop Wi-Fi, then open Settings again.",
                140.0,
            );
            return;
        };
        let host = address.to_string();
        let endpoint = config.endpoint(&host);

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.lg;
            pairing_qr(ui, &config.pair_uri(&host));
            ui.vertical(|ui| {
                info_row(ui, "Address", &endpoint);
                info_row(ui, "Port", &config.port.to_string());
                let shown_token = crate::scan::spaced_token(&config.token);
                info_row(ui, "Code", &shown_token);
                ui.add_space(SPACING.sm);
                if actions::secondary_compact(ui, "Copy code").clicked() {
                    ui.ctx().copy_text(shown_token);
                }
                ui.add_space(SPACING.xs);
                components::caption(
                    ui,
                    "If the camera cannot read the code, type the address, port and this \
                     ten-character code into the app.",
                );
            });
        });

        if config.enabled {
            ui.add_space(SPACING.md);
            #[cfg(target_os = "linux")]
            firewall_note(ui, state, config.port);
        }
    });
}

/// Till-to-till sync: one switch, this till's identity, and the list of peers
/// it exchanges with.
///
/// Every till keeps its own database and sells offline. Pairing two tills is a
/// choice about who reads whose history, not a hierarchy: the only asymmetry is
/// the catalog owner, because two tills editing products is two catalogs, and a
/// shop wants one.
pub(super) fn sync_card(ui: &mut egui::Ui, state: &mut State) {
    let config = state.settings.sync.clone();
    let status = state.sync_status.clone();
    let palette = theme::current(ui.ctx());

    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        components::caption(
            ui,
            "Sales travel between tills on the shop's Wi-Fi. Each till keeps \
             selling on its own database, and everything it records shows up \
             on the others the next time they talk.",
        );
        ui.add_space(SPACING.md);

        // The status can be a long error. Let it wrap to a second line instead
        // of pushing the card past the page's right edge.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.sm;
            if config.enabled {
                if actions::secondary_compact(ui, "Turn off").clicked() {
                    state.set_sync_enabled(false);
                }
            } else if actions::secondary_compact(ui, "Turn on").clicked() {
                state.set_sync_enabled(true);
            }
            let (text, color) = sync_status_text(&status, &palette);
            ui.label(egui::RichText::new(text).size(TYPE.small).color(color));
        });

        ui.add_space(SPACING.xs);
        components::caption(
            ui,
            "A till that cannot reach the others keeps working. Sales are \
             exchanged later, in any order, without duplicating.",
        );
    });

    if !config.enabled {
        return;
    }

    ui.add_space(SPACING.lg);
    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        components::caption(
            ui,
            "What other tills call this one. The name is only a label; the \
             database keeps its own identity, so renaming it does not lose \
             history or a peer's place in the feed.",
        );
        ui.add_space(SPACING.md);

        let dirty = state.sync_name_draft.trim() != config.device_name.trim();
        components::field_label(ui, "Till name");
        ui.add_space(SPACING.xs);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.sm;
            text_field(ui, &mut state.sync_name_draft, "Till", 320.0);
            if dirty && actions::primary(ui, "Save").clicked() {
                state.save_sync_device_name();
            }
        });
        ui.add_space(SPACING.xs);
        components::caption(ui, if dirty { "Not saved yet." } else { "Saved." });

        ui.add_space(SPACING.md);
        let mut owner = config.catalog_owner;
        if ui
            .checkbox(&mut owner, "This till authors the product catalog")
            .changed()
        {
            state.set_catalog_owner(owner);
        }
        ui.add_space(SPACING.xs);
        components::caption(
            ui,
            "Exactly one till should claim this. Replicas replace their products \
             and categories with the owner's the next time they sync; a till \
             that does not claim it never publishes a catalog.",
        );
    });

    ui.add_space(SPACING.lg);
    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        components::caption(
            ui,
            "Another till pulls this till's sales with the address, port and \
             code below. Give them only to a till you trust: the code also \
             reads every sale kept here.",
        );
        ui.add_space(SPACING.md);

        let Some(address) = crate::scan::local_ip() else {
            components::empty_state_card(
                ui,
                "No network address",
                "Connect this till to the shop Wi-Fi, then open Settings again.",
                140.0,
            );
            return;
        };
        let host = address.to_string();
        let shown_token = crate::scan::spaced_token(&config.sync_token);
        // `info_row` already lays its value against the card's right edge, so
        // stacking the rows keeps the address, port and code inside the page.
        // Three of them side by side made each row claim the whole width and
        // pushed the port out of the viewport.
        info_row(ui, "Address", &host);
        info_row(ui, "Port", &state.settings.scanner.port.to_string());
        info_row(ui, "Code", &shown_token);
        ui.add_space(SPACING.sm);
        if actions::secondary_compact(ui, "Copy code").clicked() {
            ui.ctx().copy_text(shown_token);
        }
    });

    ui.add_space(SPACING.lg);
    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.sm;
            components::section_title(ui, "Other tills");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if actions::secondary_compact(ui, "Sync now").clicked()
                    && !state.sync_peers.is_empty()
                {
                    state.sync_now_requested = true;
                }
            });
        });
        ui.add_space(SPACING.sm);

        if state.sync_peers.is_empty() {
            components::caption(
                ui,
                "No tills paired yet. Enter another till's address and code below.",
            );
        } else {
            // Copy so the row can ask to remove a peer without holding a borrow
            // on the list the removal rewrites.
            let peers = state.sync_peers.clone();
            for peer in &peers {
                let mut remove = false;
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(components::semibold(peer.display_name()).size(TYPE.body));
                        let (detail, color) = match (&peer.last_error, &peer.last_seen_at) {
                            (Some(error), _) => (error.clone(), palette.danger),
                            (None, Some(seen)) => (format!("Last read {seen}"), palette.text_muted),
                            (None, None) => ("Not contacted yet.".to_owned(), palette.text_muted),
                        };
                        components::caption(ui, &peer.address());
                        ui.label(egui::RichText::new(detail).size(TYPE.small).color(color));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if actions::quiet_compact(ui, "Remove").clicked() {
                            remove = true;
                        }
                    });
                });
                if remove {
                    state.remove_sync_peer(&peer.id);
                }
                ui.add_space(SPACING.sm);
            }
        }

        ui.add_space(SPACING.sm);
        components::field_label(ui, "Pair another till");
        ui.add_space(SPACING.xs);
        // The fields used fixed widths. On a 945 px window the two 220 px
        // columns and their gap were wider than the card, so the address and
        // pairing-code fields ran past the page edge. Derive the widths from
        // the card instead; the two rows always add up to its inner width.
        let gap = SPACING.sm;
        let full = ui.available_width();
        let half = ((full - gap) / 2.0).max(0.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            text_field(ui, &mut state.sync_peer_draft.name, "Name (optional)", half);
            text_field(ui, &mut state.sync_peer_draft.host, "Address", half);
        });
        ui.add_space(SPACING.sm);
        let port_width = 120.0_f32.min((full - gap).max(0.0));
        let token_width = (full - port_width - gap).max(0.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            text_field(ui, &mut state.sync_peer_draft.port, "Port", port_width);
            text_field(
                ui,
                &mut state.sync_peer_draft.token,
                "Pairing code",
                token_width,
            );
        });
        ui.add_space(SPACING.sm);
        if actions::primary(ui, "Pair").clicked() {
            state.add_sync_peer();
        }
        ui.add_space(SPACING.xs);
        components::caption(
            ui,
            "Both tills must be on the same network and have sync turned on.",
        );

        if let Some(error) = state.sync_peer_error.clone() {
            ui.add_space(SPACING.xs);
            ui.label(
                egui::RichText::new(error)
                    .size(TYPE.small)
                    .color(palette.danger),
            );
        }
    });
}

/// One line about what sync is doing right now.
pub(super) fn sync_status_text(
    status: &crate::sync::SyncStatus,
    palette: &ui_kit::tokens::Palette,
) -> (String, egui::Color32) {
    if !status.enabled {
        return (
            "Off. Nothing is being exchanged.".to_owned(),
            palette.text_muted.into(),
        );
    }
    if let Some(error) = &status.error {
        return (error.clone(), palette.danger.into());
    }
    if status.syncing {
        return (
            format!("Talking to {} till(s)…", status.peers),
            palette.text_muted.into(),
        );
    }
    match status.last_synced {
        Some(last) => {
            let seconds = last.elapsed().as_secs();
            let when = if seconds < 60 {
                format!("{seconds} s ago")
            } else if seconds < 3600 {
                format!("{} min ago", seconds / 60)
            } else {
                format!("{} h ago", seconds / 3600)
            };
            (format!("Last synced {when}."), palette.success.into())
        }
        None if status.peers == 0 => (
            "No other tills paired yet.".to_owned(),
            palette.text_muted.into(),
        ),
        None => (
            "Waiting for the next pass.".to_owned(),
            palette.text_muted.into(),
        ),
    }
}

/// Whether the computer's firewall will let a phone reach the listening port.
///
/// This is the one part of pairing the app cannot finish on its own: reading a
/// firewall's rules and changing them both need the administrator password, so
/// the till only says which firewall is running and offers the operating
/// system's own permission dialog through `pkexec` — the same dialog the user
/// would see installing a printer. When that dialog cannot appear, the exact
/// command is printed and can be copied.
#[cfg(target_os = "linux")]
pub(super) fn firewall_note(ui: &mut egui::Ui, state: &mut State, port: u16) {
    use crate::firewall::Snapshot;

    let palette = theme::current(ui.ctx());

    let (text, color) = match &state.firewall {
        Snapshot::Unprobed => (
            "Checking whether this computer's firewall lets phones in…".to_owned(),
            palette.text_muted,
        ),
        Snapshot::Clear => (
            "No host firewall is blocking phones on this network.".to_owned(),
            palette.text_muted,
        ),
        Snapshot::Allowed(backend) => (
            format!("Port {port} was allowed in {}.", backend.label()),
            palette.success,
        ),
        Snapshot::Unknown(backend) => (
            format!(
                "{} is running, so phones reach the till only after port {port} is \
                 allowed.",
                backend.label()
            ),
            palette.text_muted,
        ),
    };
    ui.label(egui::RichText::new(text).size(TYPE.small).color(color));

    if state.firewall.wants_allow() {
        ui.add_space(SPACING.sm);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.sm;
            ui.add_enabled_ui(!state.firewall_busy, |ui| {
                if actions::secondary_compact(ui, "Allow port").clicked() {
                    state.firewall_allow_requested = true;
                }
            });
            if state.firewall_busy {
                ui.label(
                    egui::RichText::new("Waiting for permission…")
                        .size(TYPE.small)
                        .color(palette.text_muted),
                );
            } else {
                components::caption(ui, "Asks for the administrator password once.");
            }
        });
    }

    let Some(error) = state.firewall_error.clone() else {
        return;
    };

    ui.add_space(SPACING.sm);
    ui.label(
        egui::RichText::new(error)
            .size(TYPE.small)
            .color(palette.danger),
    );

    if let Some(command) = state.firewall.command(port) {
        ui.add_space(SPACING.xs);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.sm;
            components::caption(ui, &format!("Run this instead: {command}"));
            if actions::quiet_compact(ui, "Copy").clicked() {
                ui.ctx().copy_text(command);
            }
        });
    }
}

/// Draw the pairing URI as a QR code.
///
/// Painted by hand instead of behind an image feature: `qrcode`'s default
/// features pull in encoders this app never uses, and a QR code is a grid of
/// squares either way. Two modules of quiet zone so a phone camera has a
/// margin, and fixed black on white so it scans in either theme.
pub(super) fn pairing_qr(ui: &mut egui::Ui, data: &str) {
    let Ok(code) = qrcode::QrCode::new(data.as_bytes()) else {
        components::caption(
            ui,
            "Could not draw the pairing code. Type the details instead.",
        );
        return;
    };

    const SIDE: f32 = 176.0;
    const QUIET: usize = 2;

    let (rect, _) = ui.allocate_exact_size(egui::vec2(SIDE, SIDE), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        egui::CornerRadius::same(RADII.sm as u8),
        egui::Color32::WHITE,
    );

    let width = code.width();
    let modules = (width + QUIET * 2) as f32;
    let cell = SIDE / modules;
    let colors = code.to_colors();
    for y in 0..width {
        for x in 0..width {
            if colors[y * width + x] != qrcode::Color::Dark {
                continue;
            }
            let min = rect.min + egui::vec2((x + QUIET) as f32 * cell, (y + QUIET) as f32 * cell);
            painter.rect_filled(
                egui::Rect::from_min_size(min, egui::vec2(cell, cell)),
                egui::CornerRadius::ZERO,
                egui::Color32::BLACK,
            );
        }
    }
}

/// The shop's name, as it prints at the top of every receipt.
///
/// The field edits a draft and an explicit Save commits it. Binding it straight
/// to `settings` would rewrite `settings.json` on every keystroke, because the
/// shell persists whenever the flag is set: a crash mid-word would leave half a
/// shop name as the letterhead of every receipt.
pub(super) fn shop_card(ui: &mut egui::Ui, state: &mut State) {
    let dirty = state.shop_draft.trim() != state.settings.shop_name.trim();

    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        components::caption(
            ui,
            "Prints at the top of every receipt. The sample under Receipt updates as you type.",
        );
        ui.add_space(SPACING.md);

        components::field_label(ui, "Shop name");
        ui.add_space(SPACING.xs);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.sm;
            text_field(ui, &mut state.shop_draft, crate::APP_NAME, 320.0);
            // A Save that is always there invites a press that does nothing.
            // The action appears with the change, and the caption below says
            // whether the name on the receipts is the one in the field.
            if dirty && actions::primary(ui, "Save").clicked() {
                state.save_shop_name();
            }
        });

        ui.add_space(SPACING.xs);
        components::caption(ui, shop_caption(state));
    });
}

/// What the card says under the field.
///
/// Derived from the field rather than remembered in a flag: a "Saved."
/// confirmation held in state can outlive the text it described — the operator
/// types on, and the card keeps congratulating a name that is no longer there.
pub(super) fn shop_caption(state: &State) -> &'static str {
    let saved = state.settings.shop_name.trim();
    if state.shop_draft.trim() != saved {
        "Not saved yet."
    } else if saved.is_empty() {
        "Leave it blank to print the app name."
    } else {
        "Saved."
    }
}

/// The two numbers that describe money the till handles but did not earn: the
/// tax it collects on the shop's behalf, and the change it opens with.
///
/// `0` tax means no tax — some small stores are not VAT-registered. Prices on
/// the till are tax-inclusive: the shelf price is what the customer pays, and
/// the receipt backs the tax out underneath the total. Changing the rate does
/// not alter past sales; each sale records the tax it collected.
///
/// The float is the cash in the drawer before the first sale. Zero is right for
/// a shop that opens an empty drawer; a shop that starts with change sets it
/// once, so the evening count is compared against takings plus the float instead
/// of reading its own float as a surplus every single day.
pub(super) fn money_card(ui: &mut egui::Ui, state: &mut State) {
    let tax_dirty = state.tax_draft.trim() != state.settings.tax_rate_percent.to_string();
    let saved_float = state::plain_amount(state.settings.cash_float_minor, state.currency);
    let float_dirty = state.float_draft.trim() != saved_float;
    let palette = theme::current(ui.ctx());

    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        components::caption(
            ui,
            "The tax rate the till charges, as a whole percentage. Zero means no \
             tax. Prices are tax-inclusive: the receipt breaks the tax out \
             underneath the total, and past sales keep the rate they were sold \
             at.",
        );
        ui.add_space(SPACING.md);

        components::field_label(ui, "Tax rate (%)");
        ui.add_space(SPACING.xs);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.sm;
            text_field(ui, &mut state.tax_draft, "0", 120.0);
            if tax_dirty && actions::primary(ui, "Save").clicked() {
                state.save_tax_rate();
            }
        });

        ui.add_space(SPACING.xs);
        components::caption(ui, tax_caption(state));

        ui.add_space(SPACING.md);
        let rule = ui.available_rect_before_wrap();
        ui.painter().hline(
            rule.x_range(),
            rule.top(),
            egui::Stroke::new(1.0_f32, palette.border),
        );
        ui.add_space(SPACING.md);

        components::caption(
            ui,
            "The change in the drawer before the first sale. It is added to the \
             day's takings when the drawer is counted, so the float never reads \
             as a surplus.",
        );
        ui.add_space(SPACING.md);

        components::field_label(ui, "Change float");
        ui.add_space(SPACING.xs);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.sm;
            text_field(ui, &mut state.float_draft, "0", 120.0);
            if float_dirty && actions::primary(ui, "Save").clicked() {
                state.save_cash_float();
            }
        });

        ui.add_space(SPACING.xs);
        let caption = if float_dirty {
            "Not saved yet.".to_owned()
        } else {
            format!(
                "Counted in {} at close.",
                state.money(state.settings.cash_float_minor)
            )
        };
        components::caption(ui, &caption);
    });
}

/// What the card says under the tax field.
pub(super) fn tax_caption(state: &State) -> &'static str {
    let rate = state.settings.tax_rate_percent;
    if state.tax_draft.trim() != rate.to_string() {
        "Not saved yet."
    } else if rate == 0 {
        "No tax charged."
    } else {
        "Saved."
    }
}

/// The terminal's identity, in the same card language as the rest of the app.
pub(super) fn terminal_card(ui: &mut egui::Ui, state: &mut State) {
    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        info_row(ui, "Terminal", crate::APP_NAME);
        info_row(
            ui,
            "Database schema",
            &format!("v{}", state.store.user_version().unwrap_or(-1)),
        );
        info_row(
            ui,
            "Currency",
            &format!("{} ({})", state.currency.name(), state.currency.code()),
        );

        ui.add_space(SPACING.sm);
        // Currency is a property of the amounts already stored, so it can only
        // change while there are none. Re-pricing a recorded history is not a
        // setting; it is a correction, and corrections are reversing entries.
        if crate::db::has_sales(&state.store) {
            components::caption(
                ui,
                "Locked: this till has sales. Past totals are never re-priced.",
            );
        } else {
            components::caption(ui, "No sales yet, so the currency can still change.");
            ui.add_space(SPACING.sm);
            // The currency is the owner's: it decides what every stored number
            // means, and a manager borrowing that for a minute is not a thing.
            if state.permits(crate::auth::Permission::ChangeCurrency) {
                if actions::secondary_compact(ui, "Change currency").clicked() {
                    // A failed attempt from last time is not about this one.
                    state.currency_error = None;
                    state.currency_picker = true;
                }
            } else {
                components::caption(ui, "Only the owner can change it.");
            }
        }
    });
}

/// The seeded business types the app has category suggestions for, in the
/// order they appear in the picker. Any value not here is treated as "Other"
/// and yields no seeded categories — the shop uses its own categories instead.
pub(super) const BUSINESS_TYPES: [&str; 3] = ["Supermarket", "Pharmacy", "Stationery"];

/// Which kind of shop this till is. The choice only affects which category
/// names the product form offers as suggestions: it never inserts categories
/// automatically, and the shop can rename or add to them at any time.
///
/// A dropdown instead of a free-text field, so the operator cannot type a
/// case variant the seed lookup misses (e.g. "Supermarket" vs "supermarket").
/// "Other…" opens a custom text field for types the app does not seed.
pub(super) fn business_type_card(ui: &mut egui::Ui, state: &mut State) {
    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        components::caption(
            ui,
            "Picks which category names the product form offers as suggestions. \
             It never inserts anything: the shop's own categories are the ones \
             it has actually used, and any name can be added from the form.",
        );
        ui.add_space(SPACING.md);

        components::field_label(ui, "Business type");
        ui.add_space(SPACING.xs);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.sm;
            egui::ComboBox::from_id_salt("pos_business_type")
                .width(220.0)
                .selected_text(if state.business_draft.trim().is_empty() {
                    "Pick…"
                } else {
                    &state.business_draft
                })
                .show_ui(ui, |ui| {
                    ui.set_min_width(200.0);
                    for kind in BUSINESS_TYPES {
                        let active = state.business_draft.trim().eq_ignore_ascii_case(kind);
                        if ui.selectable_label(active, kind).clicked() {
                            state.business_draft = kind.to_ascii_lowercase();
                            state.business_showing_other = false;
                            ui.close();
                        }
                    }
                    if ui
                        .selectable_label(state.business_showing_other, "Other…")
                        .clicked()
                    {
                        state.business_showing_other = true;
                        ui.close();
                    }
                });
        });

        if state.business_showing_other {
            ui.add_space(SPACING.xs);
            ui.horizontal(|ui| {
                ui.label("Custom type:");
                ui.add_space(SPACING.xs);
                text_field(ui, &mut state.business_draft, "e.g. boutique", 220.0);
            });
            components::caption(
                ui,
                "No seeded category suggestions for a custom type; the shop's \
                 own categories are used instead.",
            );
        }

        let chosen = state.business_draft.trim();
        let dirty = !chosen.is_empty() && state.settings.business_type.as_deref() != Some(chosen);
        ui.add_space(SPACING.xs);
        if dirty && actions::primary(ui, "Save").clicked() {
            state.save_business_type();
        }

        ui.add_space(SPACING.xs);
        if dirty {
            components::caption(ui, "Not saved yet.");
        } else if let Some(ref current) = state.settings.business_type {
            components::caption(ui, &format!("Saved: {current}."));
        } else {
            components::caption(ui, "No business type set. Category suggestions are off.");
        }
    });
}

/// The owner's copy of the books.
///
/// A button, not a dialog: a backup is the thing an owner postpones, so the
/// distance from "I should do that" to "done" has to be a press and a line
/// saying where the file went. The one extra press is a confirmation inline in
/// the card — the same two-step the product rows use — because a backup is the
/// one action on this screen that writes a second copy of everything.
/// Failures are shown in the same line, because a backup that quietly did not
/// happen is worse than none.
pub(super) fn backup_card(ui: &mut egui::Ui, state: &mut State) {
    let folder = state.paths.data.join("backups");
    let palette = theme::current(ui.ctx());

    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        components::caption(
            ui,
            "A copy of the database, written beside it. One is taken every day by itself, and the newest 10 are kept.",
        );
        ui.add_space(SPACING.md);

        if state.backup_confirm {
            components::caption(ui, "Write a copy of the database now?");
            ui.add_space(SPACING.xs);
            actions::end(ui, |ui| {
                if actions::primary(ui, "Back up").clicked() {
                    state.backup_confirm = false;
                    match crate::db::backup(&state.store, &folder) {
                        Ok(path) => {
                            state.file_path = Some(path.clone());
                            state.file_notice = Some(format!("Backed up to {}", path.display()));
                        }
                        Err(err) => {
                            state.file_path = None;
                            state.file_notice = Some(format!("Backup failed: {err:#}"));
                        }
                    }
                }
                if actions::secondary(ui, "Cancel").clicked() {
                    state.backup_confirm = false;
                }
            });
        } else if actions::secondary(ui, "Back up now").clicked() {
            state.backup_confirm = true;
        }

        ui.add_space(SPACING.xs);
        components::caption(ui, &format!("Folder: {}", folder.display()));
        if let Some(error) = state.auto_backup_error.clone() {
            ui.add_space(SPACING.xs);
            ui.label(
                egui::RichText::new(format!("The automatic backup did not happen: {error}"))
                    .size(TYPE.small)
                    .color(palette.danger),
            );
        }
        if state.file_notice.is_some() {
            ui.add_space(SPACING.xs);
            file_feedback(ui, state);
        }
    });
}

pub(super) fn info_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        components::field_label(ui, label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(components::semibold(value).size(TYPE.body));
        });
    });
    ui.add_space(SPACING.xs);
}

/// The receipt's optional lines, edited as a draft and committed by Save.
///
/// Same rule as the shop name: binding the fields straight to `settings` would
/// rewrite `settings.json` on every keystroke, so a crash mid-word could leave
/// half an address on every receipt.
pub(super) fn receipt_details_card(ui: &mut egui::Ui, state: &mut State) {
    let dirty = state.receipt_draft.normalised() != state.settings.receipt;

    components::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        components::caption(
            ui,
            "Optional lines printed around the sale. Blank fields are left off the paper; \
             an empty closing line still prints \"Thank you.\"",
        );
        ui.add_space(SPACING.md);

        egui::Grid::new("receipt_details_fields")
            .num_columns(2)
            .spacing([SPACING.md, SPACING.sm])
            .show(ui, |ui| {
                receipt_field(
                    ui,
                    "Address",
                    &mut state.receipt_draft.address,
                    "Street, town",
                    360.0,
                );
                receipt_field(
                    ui,
                    "Phone",
                    &mut state.receipt_draft.phone,
                    "+256 700 000",
                    220.0,
                );
                receipt_field(
                    ui,
                    "Tax / VAT",
                    &mut state.receipt_draft.tax_number,
                    "TIN 1000123456",
                    220.0,
                );
                receipt_field(
                    ui,
                    "Return policy",
                    &mut state.receipt_draft.return_policy,
                    "Exchanges within 7 days with this receipt.",
                    420.0,
                );
                receipt_field(
                    ui,
                    "Closing line",
                    &mut state.receipt_draft.footer,
                    "Thank you for shopping with us.",
                    320.0,
                );
            });

        ui.add_space(SPACING.sm);
        ui.checkbox(
            &mut state.receipt_draft.show_cashier,
            "Print the cashier's name on the receipt",
        );

        ui.add_space(SPACING.sm);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.sm;
            if dirty && actions::primary(ui, "Save").clicked() {
                state.save_receipt_details();
            }
            components::caption(ui, receipt_caption(state));
        });
    });
}

pub(super) fn receipt_field(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut String,
    hint: &str,
    width: f32,
) {
    components::field_label(ui, label);
    text_field(ui, value, hint, width);
    ui.end_row();
}

/// What the card says about the fields, derived from the draft rather than a
/// remembered flag so a "Saved." cannot outlive the text it described.
pub(super) fn receipt_caption(state: &State) -> &'static str {
    if state.receipt_draft.normalised() != state.settings.receipt {
        "Not saved yet."
    } else {
        "Saved. Changes show in the sample below."
    }
}

/// The receipt section: the fields that shape the paper, then the sample those
/// fields produce.
pub(super) fn receipt_preview(ui: &mut egui::Ui, state: &mut State) {
    receipt_details_card(ui, state);
    ui.add_space(SPACING.lg);

    components::section_title(ui, "Sample");
    receipt_sample(ui, state);
}

/// A sample receipt: the one place the receipt face and its 13 px column width
/// can be judged before a printer exists.
///
/// Reads the drafts, not the saved values: the point of the preview is to answer
/// "what will this look like?" for the name and lines currently being typed,
/// before Save makes them real.
pub(super) fn receipt_sample(ui: &mut egui::Ui, state: &State) {
    let typed = state.shop_draft.trim();
    let shop = if typed.is_empty() {
        crate::APP_NAME
    } else {
        typed
    };
    // A preview of a name that has not been saved is still a preview, but it
    // must not pass for the letterhead: the till prints what was saved.
    let unsaved = typed != state.settings.shop_name.trim();
    let paper = crate::receipt::Paper::from_receipt(shop, &state.receipt_draft);

    components::card(ui, |ui| {
        ui.set_width(300.0);
        if unsaved {
            components::caption(ui, "Shop name not saved yet: save it under Terminal.");
            ui.add_space(SPACING.xs);
        }
        crate::receipt::body(ui, &crate::receipt::sample(), &paper, state.currency, true);
        ui.add_space(SPACING.sm);
        components::caption(ui, "Printer and paper width arrive with the receipt flow.");
    });
}
