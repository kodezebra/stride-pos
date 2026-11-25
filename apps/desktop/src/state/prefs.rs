//! Preferences: what the shop looks like, what it charges, and where files go.
//!
//! These settings all end the same way — a write, then a visible change on the
//! next frame — and most of them are edited from one screen of cards. The
//! `Paths` struct lives here because it is the same kind of choice.

use super::*;
/// The two places the till keeps its own files.
///
/// Grouped because they are only ever known together, at boot: the database
/// lives in `data` and `settings.json` in `config`, and both come from
/// `app_boot`'s directories. `Store` deliberately does not remember where it was
/// opened from, so backups and exports ask the state for this.
pub struct Paths {
    pub data: PathBuf,
    pub settings: PathBuf,
}

impl State {
    /// Format an amount in this till's currency: `12345` → `"Ksh 123.45"`.
    ///
    /// The currency is the one fact every screen needs and none of them own, so
    /// it lives here and the screens ask the state rather than passing it down.
    pub fn money(&self, minor: i64) -> String {
        Money::new(minor, self.currency).format()
    }

    /// Record a currency choice. On a failed write the picker stays open with
    /// the reason, because selling in an unrecorded currency is worse than
    /// blocking the till for one more attempt.
    pub fn set_currency(&mut self, currency: Currency) {
        match crate::db::set_currency(&self.store, currency) {
            Ok(()) => {
                self.currency = currency;
                self.needs_currency = false;
                self.currency_picker = false;
                self.currency_error = None;
            }
            Err(error) => {
                tracing::error!(error = %error, "storing the currency failed");
                self.currency_error = Some(format!("Could not save the currency: {error}"));
            }
        }
    }

    /// Switch appearance. The shell installs it on the next frame; screens never
    /// touch global visuals.
    pub fn set_theme(&mut self, theme: Theme) {
        if self.theme != theme {
            self.theme = theme;
            self.settings.set_theme(theme);
            self.theme_dirty = true;
            self.settings_dirty = true;
        }
    }

    /// Switch the one brand accent. The shell derives the palette on the next
    /// frame; screens never touch global visuals.
    pub fn set_brand_accent(&mut self, accent: Option<Rgb>) {
        if self.brand_accent != accent {
            self.brand_accent = accent;
            self.settings.set_brand_accent(accent);
            self.theme_dirty = true;
            self.settings_dirty = true;
        }
    }

    /// Flip light/dark. One place decides, so the icon button, the palette
    /// command and Ctrl+T cannot drift apart.
    pub fn toggle_theme(&mut self) {
        self.set_theme(self.theme.flipped());
    }

    /// Commit the shop name typed in Settings.
    ///
    /// Deliberately not bound straight to `settings.shop_name`: the shell writes
    /// the file whenever the flag is set, so a field wired to the settings would
    /// rewrite `settings.json` on every keystroke, and a crash mid-word would
    /// leave half a shop name as the letterhead of every receipt.
    pub fn save_shop_name(&mut self) {
        let name = self.shop_draft.trim().to_owned();
        self.shop_draft.clone_from(&name);
        if self.settings.shop_name != name {
            self.settings.shop_name = name;
            self.settings_dirty = true;
        }
    }

    /// Commit the optional receipt fields typed in Settings.
    ///
    /// They are saved together because they are read as one block on paper: an
    /// address saved without the phone number beside it is a half-edited
    /// letterhead, and the owner would have to notice which field they last
    /// touched.
    pub fn save_receipt_details(&mut self) {
        let details = self.receipt_draft.normalised();
        self.receipt_draft = details.clone();
        if self.settings.receipt != details {
            self.settings.receipt = details;
            self.settings_dirty = true;
        }
    }

    /// Commit the tax rate typed in Settings. The draft is parsed as a whole
    /// percentage; anything that does not parse is ignored rather than written
    /// to disk, so a stray keystroke cannot invent a rate the shop does not pay.
    pub fn save_tax_rate(&mut self) {
        let parsed: Option<i64> = self.tax_draft.trim().parse().ok();
        let rate = parsed.unwrap_or(0).max(0);
        self.tax_draft = rate.to_string();
        if self.settings.tax_rate_percent != rate {
            self.settings.tax_rate_percent = rate;
            self.settings_dirty = true;
        }
    }

    /// Commit the change float typed in Settings.
    ///
    /// It is the cash that sits in the drawer before the first sale, so the
    /// evening count is compared against takings plus this rather than against
    /// takings alone. A figure that does not parse is ignored rather than
    /// written: the float is a statement about real money.
    pub fn save_cash_float(&mut self) {
        let Ok(amount) = Money::parse(self.float_draft.trim(), self.currency) else {
            return;
        };
        let minor = amount.minor().max(0);
        self.float_draft = plain_amount(minor, self.currency);
        if self.settings.cash_float_minor != minor {
            self.settings.cash_float_minor = minor;
            self.settings_dirty = true;
        }
    }

    /// Commit the business type chosen in Settings. The value must match one of
    /// the seed files; anything else is ignored. The choice only affects which
    /// category names the product form offers as suggestions — it never inserts
    /// categories automatically.
    ///
    /// Stored lowercased so "Supermarket" and "supermarket" both match the
    /// seeded category list.
    pub fn save_business_type(&mut self) {
        let chosen = self.business_draft.trim().to_ascii_lowercase();
        if chosen.is_empty() {
            self.business_draft = String::new();
            if self.settings.business_type.is_some() {
                self.settings.business_type = None;
                self.settings_dirty = true;
                self.category_seed_notice = None;
            }
            return;
        }
        if self.settings.business_type.as_deref() != Some(&chosen) {
            self.settings.business_type = Some(chosen.clone());
            self.business_draft = chosen;
            self.settings_dirty = true;
            // The old type's result no longer describes this one.
            self.category_seed_notice = None;
        }
    }

    /// Add the suggested categories for the saved business type to the
    /// `categories` table. Nothing calls this but the explicit button in
    /// Settings: choosing a type only changes product-form suggestions.
    ///
    /// Existing shelves are matched case-insensitively and left untouched, so
    /// pressing the button twice is harmless. Returns the number of new rows.
    pub fn apply_suggested_categories(&mut self) -> anyhow::Result<usize> {
        let Some(business_type) = self.settings.business_type.clone() else {
            return Ok(0);
        };
        let seeds = crate::category_seeds::seed_names(&business_type);
        let existing = crate::db::categories_list(&self.store);
        let missing: Vec<String> = seeds
            .into_iter()
            .filter(|seed| {
                !existing
                    .iter()
                    .any(|row| row.name.eq_ignore_ascii_case(seed))
            })
            .collect();

        // The seed file is already in the order the shop expects on its
        // shelves, so write that order onto the new rows. A shelf that already
        // exists keeps its own place; `insert_category` leaves it alone.
        let added = missing.len();
        for (order, name) in missing.into_iter().enumerate() {
            crate::db::insert_category(&self.store, &name, order as i64)?;
        }
        Ok(added)
    }

    pub fn sidebar_open(&self, module: usize) -> bool {
        self.sidebar_open.get(module).copied().unwrap_or(true)
    }

    pub fn set_sidebar_open(&mut self, module: usize, open: bool) {
        if module >= self.sidebar_open.len() {
            self.sidebar_open.resize(module + 1, true);
        }
        if self.sidebar_open[module] != open {
            self.sidebar_open[module] = open;
            self.prefs_dirty = true;
        }
    }

    pub fn toggle_sidebar(&mut self) {
        let open = self.sidebar_open(self.module);
        self.set_sidebar_open(self.module, !open);
    }
}
