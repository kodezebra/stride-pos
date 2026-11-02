//! The terminal's configuration, persisted in `settings.json`.
//!
//! Distinct from [`crate::prefs::Prefs`]: this is the shop's configuration, that
//! is one person's window habits. A shared till may be reopened by anyone, but
//! the theme belongs to the terminal.

use serde::{Deserialize, Serialize};

use ui_kit::theme::Theme;

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// `"light"` or `"dark"`. `None` means "never chosen", which resolves to the
    /// default theme rather than pinning a value the user did not pick.
    pub theme: Option<String>,
    /// What prints at the top of every receipt. Empty means "not set yet" and
    /// falls back to the app name, so a fresh till still prints something rather
    /// than a blank letterhead.
    pub shop_name: String,
}

impl Settings {
    /// The theme to draw with. An unknown value falls back to the default; a
    /// hand-edited file must not stop a till from opening.
    pub fn theme(&self) -> Theme {
        self.theme.as_deref().map(Theme::parse).unwrap_or_default()
    }

    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = Some(theme.name().to_owned());
    }

    /// The merchant as a receipt should print it.
    ///
    /// One resolver because the receipt, the preview and the clipboard all need
    /// the same answer: a till whose on-screen slip names the shop and whose
    /// pasted slip names the app is worse than either alone. A name that is only
    /// spaces counts as unset, so a stray keystroke cannot erase the header.
    pub fn shop_name(&self) -> &str {
        let trimmed = self.shop_name.trim();
        if trimmed.is_empty() {
            crate::APP_NAME
        } else {
            trimmed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_unknown_theme_falls_back_to_light() {
        assert_eq!(Settings::default().theme(), Theme::Light);
        let settings = Settings {
            theme: Some("chartreuse".to_owned()),
            ..Settings::default()
        };
        assert_eq!(settings.theme(), Theme::Light);
    }

    #[test]
    fn dark_round_trips_through_the_file_spelling() {
        let mut settings = Settings::default();
        settings.set_theme(Theme::Dark);
        assert_eq!(settings.theme.as_deref(), Some("dark"));
        assert_eq!(settings.theme(), Theme::Dark);
    }

    #[test]
    fn a_blank_shop_name_prints_the_app_name() {
        assert_eq!(Settings::default().shop_name(), crate::APP_NAME);

        let blank = Settings {
            shop_name: "   \n".to_owned(),
            ..Settings::default()
        };
        assert_eq!(blank.shop_name(), crate::APP_NAME);
    }

    #[test]
    fn the_shop_name_is_trimmed_before_it_prints() {
        let settings = Settings {
            shop_name: "  Nakawa Grocers \n".to_owned(),
            ..Settings::default()
        };
        assert_eq!(settings.shop_name(), "Nakawa Grocers");
    }

    /// `settings.json` is the only place the name lives, so the round-trip is
    /// the whole feature: a name that saves but does not load would print the
    /// app name again after the next launch, and the owner would have no reason
    /// to suspect the file rather than the field.
    #[test]
    fn the_shop_name_survives_the_file() {
        let path = std::env::temp_dir().join(format!(
            "stride-pos-settings-{}-{}.json",
            std::process::id(),
            line!()
        ));
        let settings = Settings {
            shop_name: "Nakawa Grocers".to_owned(),
            ..Settings::default()
        };

        config::save(&path, &settings).expect("save");
        let loaded: Settings = config::load(&path).expect("load").expect("present");
        std::fs::remove_file(&path).ok();

        assert_eq!(loaded.shop_name(), "Nakawa Grocers");
        assert_eq!(loaded.theme(), Theme::Light);
    }
}
