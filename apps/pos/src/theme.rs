//! Stride POS palettes — the app owns its own brand colors.
//!
//! `ui-kit` provides the mechanism (`Theme`, `Palette`, `install`); this
//! module provides the values. A school till, a clinic desk, and this coffee
//! & boutique POS all look different, and none of that belongs in the shared
//! lib.

use ui_kit::tokens::{Palette, Rgb};

/// Dark theme — Scandinavian FinTech.
///
/// Slate-obsidian shell, muted slate surfaces, electric-mint primary actions.
/// Cool, calm, hyper-legible for low-light hours; the mint stays visually
/// sharp without the optical intensity of saturated orange or blue.
pub const DARK: Palette = Palette {
    bg: Rgb::hex(0x0f_17_2a),
    surface: Rgb::hex(0x1e_29_3b),
    surface_alt: Rgb::hex(0x33_41_55),
    text: Rgb::hex(0xf8_fa_fc),
    text_muted: Rgb::hex(0x94_a3_b8),
    accent: Rgb::hex(0x14_b8_a6),
    accent_text: Rgb::hex(0x0f_17_2a),
    accent_soft: Rgb::hex(0x16_4e_4b),
    border: Rgb::hex(0x33_41_55),
    border_strong: Rgb::hex(0x64_74_8b),
    danger: Rgb::hex(0xef_44_44),
    success: Rgb::hex(0x34_d3_99),
    warning: Rgb::hex(0xf5_9e_0b),
};

/// Light theme — Artisanal Coffee & Boutique Retail.
///
/// Warm cashmere-cream canvas, deep warm-charcoal shell, deep-emerald primary.
/// Organic and low-blue-light, easy on the eyes in bright retail, and nothing
/// like corporate POS software. The default: a till under shop lighting next
/// to a window, where a dark screen reads as a dead terminal to a customer.
pub const LIGHT: Palette = Palette {
    bg: Rgb::hex(0xf5_f5_f4),
    surface: Rgb::hex(0xff_ff_ff),
    surface_alt: Rgb::hex(0xe7_e5_e4),
    text: Rgb::hex(0x1c_19_17),
    text_muted: Rgb::hex(0x6e_66_5d),
    accent: Rgb::hex(0x04_78_57),
    accent_text: Rgb::hex(0xff_ff_ff),
    accent_soft: Rgb::hex(0xdc_fce7),
    border: Rgb::hex(0xd6_d3_ce),
    border_strong: Rgb::hex(0xb9_b4_ac),
    danger: Rgb::hex(0xc0_39_2b),
    success: Rgb::hex(0x84cc16),
    warning: Rgb::hex(0xd9_77_06),
};

/// The palette for the given theme side of the switch.
pub fn palette_for(theme: ui_kit::theme::Theme) -> (&'static Palette, bool) {
    match theme {
        ui_kit::theme::Theme::Light => (&LIGHT, false),
        ui_kit::theme::Theme::Dark => (&DARK, true),
    }
}
