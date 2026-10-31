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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_unknown_theme_falls_back_to_light() {
        assert_eq!(Settings::default().theme(), Theme::Light);
        let settings = Settings {
            theme: Some("chartreuse".to_owned()),
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
}
