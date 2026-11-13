//! Stride POS palettes — the app owns its own brand colors.
//!
//! `ui-kit` provides the mechanism (`Theme`, `Palette`, `install`); this
//! module provides the values. A school till, a clinic desk, and this coffee
//! & boutique POS all look different, and none of that belongs in the shared
//! lib.

use ui_kit::tokens::{Palette, Rgb};

/// Dark theme — Scandinavian FinTech.
///
/// Slate-obsidian shell, muted slate surfaces, ember-orange primary actions.
/// Cool, calm, hyper-legible for low-light hours; the ember gives the one
/// action that matters a warm focal point without tinting the chrome.
pub const DARK: Palette = Palette {
    bg: Rgb::hex(0x0f_17_2a),
    surface: Rgb::hex(0x1e_29_3b),
    surface_alt: Rgb::hex(0x33_41_55),
    text: Rgb::hex(0xf8_fa_fc),
    text_muted: Rgb::hex(0x94_a3_b8),
    accent: Rgb::hex(0xff_6b_00),
    accent_text: Rgb::hex(0x0f_17_2a),
    accent_soft: Rgb::hex(0x46_35_30),
    border: Rgb::hex(0x33_41_55),
    border_strong: Rgb::hex(0x64_74_8b),
    danger: Rgb::hex(0xf8_71_71),
    success: Rgb::hex(0x34_d3_99),
    warning: Rgb::hex(0xf5_9e_0b),
};

/// Light theme — Artisanal Coffee & Boutique Retail.
///
/// Warm cashmere-cream canvas, deep warm-charcoal shell, burnt-terracotta
/// primary. Organic and low-blue-light, easy on the eyes in bright retail, and
/// nothing like corporate POS software. The default: a till under shop lighting
/// next to a window, where a dark screen reads as a dead terminal to a customer.
pub const LIGHT: Palette = Palette {
    bg: Rgb::hex(0xf5_f5_f4),
    surface: Rgb::hex(0xff_ff_ff),
    surface_alt: Rgb::hex(0xe7_e5_e4),
    text: Rgb::hex(0x1c_19_17),
    text_muted: Rgb::hex(0x6e_66_5d),
    accent: Rgb::hex(0xc2_41_0c),
    accent_text: Rgb::hex(0xff_ff_ff),
    accent_soft: Rgb::hex(0xf6_e4_dd),
    border: Rgb::hex(0xd6_d3_ce),
    border_strong: Rgb::hex(0x7a_71_68),
    danger: Rgb::hex(0xc0_39_2b),
    success: Rgb::hex(0x4d_7c_0f),
    warning: Rgb::hex(0xb4_53_09),
};

/// The palette for the given theme side of the switch.
pub fn palette_for(theme: ui_kit::theme::Theme) -> (&'static Palette, bool) {
    match theme {
        ui_kit::theme::Theme::Light => (&LIGHT, false),
        ui_kit::theme::Theme::Dark => (&DARK, true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ui_kit::tokens::BRANDS;

    /// Status and body text sit on the canvas or a card. Both are checked so a
    /// vivid role cannot pass on the page and disappear inside a card.
    #[test]
    fn static_text_roles_clear_aa_on_canvas_and_surface() {
        for (name, palette) in [("light", LIGHT), ("dark", DARK)] {
            for (role, color) in [
                ("text", palette.text),
                ("muted", palette.text_muted),
                ("danger", palette.danger),
                ("success", palette.success),
                ("warning", palette.warning),
            ] {
                for (surface, background) in [("canvas", palette.bg), ("surface", palette.surface)]
                {
                    assert!(
                        color.contrast_ratio(background) >= 4.5,
                        "{name} {role} on {surface} is below 4.5:1"
                    );
                }
            }
        }
    }

    /// `border_strong` is the visible edge of inputs and focusable controls.
    /// WCAG treats it as a non-text boundary, so 3:1 is the bar.
    #[test]
    fn the_strong_border_is_visible_on_both_surfaces() {
        for (name, palette) in [("light", LIGHT), ("dark", DARK)] {
            for (surface, background) in [("canvas", palette.bg), ("surface", palette.surface)] {
                assert!(
                    palette.border_strong.contrast_ratio(background) >= 3.0,
                    "{name} strong border on {surface} is below 3:1"
                );
            }
        }
    }

    /// The shipped palette, and every brand override derived from it, must keep
    /// a selected row and a primary button readable.
    #[test]
    fn every_accent_keeps_text_readable() {
        for (name, palette, dark) in [("light", LIGHT, false), ("dark", DARK, true)] {
            let mut palettes = vec![palette, palette.with_accent(palette.accent, dark)];
            palettes.extend(
                BRANDS
                    .iter()
                    .map(|brand| palette.with_accent(brand.accent, dark)),
            );

            for palette in palettes {
                assert!(
                    palette.accent_text.contrast_ratio(palette.accent) >= 4.49,
                    "{name} accent text on the brand fill is below 4.5:1"
                );
                for background in [
                    palette.bg,
                    palette.surface,
                    palette.surface_alt,
                    palette.accent_soft,
                ] {
                    assert!(
                        palette.accent_ink().contrast_ratio(background) >= 4.49,
                        "{name} accent ink is below 4.5:1"
                    );
                }
            }
        }
    }

    /// `Default` has to mean something more than "the first swatch". If the
    /// product accent duplicates a brand, the reset action and that swatch
    /// produce the same screen and the control lies about what was chosen.
    #[test]
    fn the_default_accent_is_not_a_brand_swatch() {
        for (name, palette) in [("light", LIGHT), ("dark", DARK)] {
            assert!(
                BRANDS.iter().all(|brand| brand.accent != palette.accent),
                "{name} default accent duplicates a brand swatch"
            );
        }
    }
}
