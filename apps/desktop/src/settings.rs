//! The terminal's configuration, persisted in `settings.json`.
//!
//! Distinct from [`crate::prefs::Prefs`]: this is the shop's configuration, that
//! is one person's window habits. A shared till may be reopened by anyone, but
//! the theme belongs to the terminal.

use serde::{Deserialize, Serialize};

use ui_kit::theme::Theme;
use ui_kit::tokens::Rgb;

use crate::scan::ScanConfig;
use crate::sync::SyncConfig;

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// `"light"` or `"dark"`. `None` means "never chosen", which resolves to the
    /// default theme rather than pinning a value the user did not pick.
    pub theme: Option<String>,
    /// The one brand accent, as `#RRGGBB`. `None` keeps the palette's built-in
    /// emerald. An unreadable or mistyped value is ignored rather than guessed.
    pub brand_accent: Option<String>,
    /// What prints at the top of every receipt. Empty means "not set yet" and
    /// falls back to the app name, so a fresh till still prints something rather
    /// than a blank letterhead.
    pub shop_name: String,
    /// The extra lines a shop puts on its paper: address, contact, tax number,
    /// returns policy and the closing line.
    pub receipt: ReceiptDetails,
    /// The phone scanner. Off until the shop turns it on; the token is generated
    /// on first launch so pairing never waits for a "generate" button.
    pub scanner: ScanConfig,
    /// Tills on the shop's own network. Off until the shop turns it on; the
    /// token is generated on first launch for the same reason.
    pub sync: SyncConfig,
    /// Tax rate as a whole percentage: `16` means 16% VAT, `0` means none.
    /// Prices on the till are tax-inclusive: the shelf price is what the
    /// customer pays, and the receipt breaks the tax out underneath the total.
    /// Default `0` because some small stores are not VAT-registered.
    pub tax_rate_percent: i64,
    /// The shop's business type: `supermarket`, `pharmacy`, `stationery`, or
    /// `None` when not yet chosen. It selects which category seed list the
    /// product form offers as suggestions; it never auto-inserts anything.
    pub business_type: Option<String>,
}

/// The receipt's optional content, kept apart from the shop name because these
/// are the fields a shop fills in once and then forgets.
///
/// Every field may be blank; blank simply means the line is not printed. The
/// footer is the exception: an unset footer still closes the slip with
/// "Thank you.", because a receipt that ends mid-air reads as unfinished.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReceiptDetails {
    pub address: String,
    pub phone: String,
    /// VAT, TIN or any local tax identifier. The label is generic because the
    /// app does not know which one the shop uses.
    pub tax_number: String,
    pub return_policy: String,
    /// The line under the total. Blank falls back to "Thank you."
    pub footer: String,
    /// Old settings files predate this field, so its default is on rather than
    /// the derived `false`: hiding the cashier must be a choice, not an upgrade.
    pub show_cashier: bool,
}

impl Default for ReceiptDetails {
    fn default() -> Self {
        Self {
            address: String::new(),
            phone: String::new(),
            tax_number: String::new(),
            return_policy: String::new(),
            footer: String::new(),
            show_cashier: true,
        }
    }
}

impl ReceiptDetails {
    /// The fields as they should be stored: no stray spaces, and a footer that
    /// is only spaces counts as unset.
    pub fn normalised(&self) -> Self {
        Self {
            address: self.address.trim().to_owned(),
            phone: self.phone.trim().to_owned(),
            tax_number: self.tax_number.trim().to_owned(),
            return_policy: self.return_policy.trim().to_owned(),
            footer: self.footer.trim().to_owned(),
            show_cashier: self.show_cashier,
        }
    }

    /// The line that closes the receipt. A shop that leaves it blank still gets
    /// a finished slip rather than a total followed by nothing.
    pub fn footer_text(&self) -> &str {
        let footer = self.footer.trim();
        if footer.is_empty() {
            "Thank you."
        } else {
            footer
        }
    }
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

    /// The brand accent to derive the palette from. An unknown value is ignored:
    /// a typo must leave the till usable rather than fail to paint.
    pub fn brand_accent(&self) -> Option<Rgb> {
        self.brand_accent.as_deref().and_then(Rgb::parse_hex)
    }

    pub fn set_brand_accent(&mut self, accent: Option<Rgb>) {
        self.brand_accent = accent.map(|accent| format!("#{:06x}", accent.to_hex()));
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
    fn brand_accent_round_trips_and_rejects_a_typo() {
        let mut settings = Settings::default();
        assert_eq!(settings.brand_accent(), None);

        let accent = Rgb::hex(0x6d_28_d9);
        settings.set_brand_accent(Some(accent));
        assert_eq!(settings.brand_accent.as_deref(), Some("#6d28d9"));
        assert_eq!(settings.brand_accent(), Some(accent));

        settings.set_brand_accent(None);
        assert_eq!(settings.brand_accent, None);

        let typo = Settings {
            brand_accent: Some("#6d28d".to_owned()),
            ..Settings::default()
        };
        assert_eq!(typo.brand_accent(), None);
    }

    #[test]
    fn a_settings_file_without_a_brand_accent_keeps_the_product_default() {
        let loaded: Settings = serde_json::from_str("{}").expect("empty settings load");
        assert_eq!(loaded.brand_accent(), None);
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

    /// A tax rate survives the file: a till that charges VAT does not have to
    /// re-enter it on every launch, and an old file with no tax field still
    /// opens at zero.
    #[test]
    fn the_tax_rate_survives_the_file() {
        let path = std::env::temp_dir().join(format!(
            "stride-pos-tax-settings-{}-{}.json",
            std::process::id(),
            line!()
        ));
        let settings = Settings {
            tax_rate_percent: 16,
            ..Settings::default()
        };
        assert_eq!(settings.tax_rate_percent, 16);

        config::save(&path, &settings).expect("save");
        let loaded: Settings = config::load(&path).expect("load").expect("present");
        std::fs::remove_file(&path).ok();

        assert_eq!(loaded.tax_rate_percent, 16);
    }

    /// An old settings file has no tax field: zero means no tax, and a shop that
    /// is not VAT-registered is not surprised by a rate it never set.
    #[test]
    fn a_settings_file_without_a_tax_rate_is_zero_tax() {
        let loaded: Settings = serde_json::from_str("{}").expect("empty settings load");
        assert_eq!(loaded.tax_rate_percent, 0);
    }

    /// Old `settings.json` files have no `receipt` object at all. They must keep
    /// printing the cashier and the default footer, not silently lose both.
    #[test]
    fn a_settings_file_written_before_receipt_details_keeps_its_defaults() {
        let loaded: Settings = serde_json::from_str("{}").expect("empty settings load");

        assert_eq!(loaded.receipt, ReceiptDetails::default());
        assert!(loaded.receipt.show_cashier);
        assert_eq!(loaded.receipt.footer_text(), "Thank you.");
    }

    #[test]
    fn receipt_details_survive_the_file() {
        let path = std::env::temp_dir().join(format!(
            "stride-pos-receipt-settings-{}-{}.json",
            std::process::id(),
            line!()
        ));
        let settings = Settings {
            receipt: ReceiptDetails {
                address: "12 Kampala Road".to_owned(),
                phone: "+256 700 123 456".to_owned(),
                tax_number: "TIN 1000123456".to_owned(),
                return_policy: "Exchanges within 7 days with this receipt.".to_owned(),
                footer: "Thank you for shopping with us.".to_owned(),
                show_cashier: false,
            },
            ..Settings::default()
        };

        config::save(&path, &settings).expect("save");
        let loaded: Settings = config::load(&path).expect("load").expect("present");
        std::fs::remove_file(&path).ok();

        assert_eq!(loaded.receipt, settings.receipt);
        assert!(!loaded.receipt.show_cashier);
        assert_eq!(
            loaded.receipt.footer_text(),
            "Thank you for shopping with us."
        );
    }

    /// A footer of spaces is not a footer: the receipt still needs a closing
    /// line, and saving should not turn the field into invisible content.
    #[test]
    fn a_blank_footer_falls_back_and_trims_otherwise() {
        let mut details = ReceiptDetails {
            footer: "   ".to_owned(),
            ..ReceiptDetails::default()
        };
        assert_eq!(details.footer_text(), "Thank you.");

        details.footer = "  Please come again.  ".to_owned();
        assert_eq!(details.normalised().footer, "Please come again.");
        assert_eq!(details.footer_text(), "Please come again.");
    }
}
