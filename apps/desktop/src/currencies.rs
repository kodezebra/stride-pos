//! The countries a till can be set up for.
//!
//! A list, not detection: a terminal shipped to Kampala must not guess its
//! country from a clock, a locale or an IP address, and a shop near a border is
//! regularly not in the country its network suggests. The operator picks, once.
//!
//! East Africa first, then the neighbours a shop here might actually trade with.
//! A row stores the country, not a code: ISO 4217 supplies the currency, so the
//! picker cannot offer a currency the money crate does not know.

use money::{Country, Currency};

/// One row in the picker.
pub struct Choice {
    /// Short enough for a list. ISO country names ("Tanzania, United Republic
    /// of") are for customs forms, not for a cashier.
    pub label: &'static str,
    /// The ISO country. The currency is looked up rather than hard-coded, so a
    /// country whose currency changes cannot drift from the data.
    pub country: Country,
}

impl Choice {
    /// The tender this choice would set, or `None` if ISO has no regular
    /// currency for the country.
    pub fn currency(&self) -> Option<Currency> {
        money::tender_for(self.country)
    }
}

/// East African Community members.
pub const EAST_AFRICA: &[Choice] = &[
    Choice {
        label: "Uganda",
        country: Country::UG,
    },
    Choice {
        label: "Kenya",
        country: Country::KE,
    },
    Choice {
        label: "Tanzania",
        country: Country::TZ,
    },
    Choice {
        label: "Rwanda",
        country: Country::RW,
    },
    Choice {
        label: "Burundi",
        country: Country::BI,
    },
    Choice {
        label: "South Sudan",
        country: Country::SS,
    },
    Choice {
        label: "DR Congo",
        country: Country::CD,
    },
    Choice {
        label: "Somalia",
        country: Country::SO,
    },
];

/// Countries next door that a shop in the region may price in.
pub const NEIGHBOURS: &[Choice] = &[
    Choice {
        label: "Ethiopia",
        country: Country::ET,
    },
    Choice {
        label: "Eritrea",
        country: Country::ER,
    },
    Choice {
        label: "Djibouti",
        country: Country::DJ,
    },
    Choice {
        label: "Sudan",
        country: Country::SD,
    },
    Choice {
        label: "Zambia",
        country: Country::ZM,
    },
    Choice {
        label: "Malawi",
        country: Country::MW,
    },
    Choice {
        label: "Mozambique",
        country: Country::MZ,
    },
    Choice {
        label: "Zimbabwe",
        country: Country::ZW,
    },
    Choice {
        label: "Madagascar",
        country: Country::MG,
    },
    Choice {
        label: "Mauritius",
        country: Country::MU,
    },
    Choice {
        label: "Seychelles",
        country: Country::SC,
    },
    Choice {
        label: "Comoros",
        country: Country::KM,
    },
];

/// The picker's groups, in display order.
pub const GROUPS: &[(&str, &[Choice])] = &[
    ("East African Community", EAST_AFRICA),
    ("Neighbours", NEIGHBOURS),
];

/// The smallest coin the shop can hand back, in minor units.
///
/// Cash rounding is a shop's decision, but the step it rounds to is not: nobody
/// in Kampala can tender 340 shillings when the smallest coin is 100. These are
/// the smallest coins in circulation for the currencies this picker offers, and
/// anything not listed rounds to its own minor unit — which is the same as not
/// rounding at all.
///
/// A shop that wants a different step (a stall that never sees a 50 shilling
/// coin, a supermarket that rounds to 500) needs a setting of its own. Until
/// there is one, this is the default the payment screen applies and shows.
pub fn cash_step(currency: Currency) -> i64 {
    match currency.code() {
        // 100 shillings, the smallest Ugandan coin.
        "UGX" => 100,
        // 1 shilling: the 50 cent coin no longer circulates.
        "KES" => 100,
        // 50 shillings — two decimal places, so 5000 minor units.
        "TZS" => 5_000,
        _ => 1,
    }
}

/// The common tender denominations for a currency, in minor units.
///
/// The payment modal rounds the bill up to the biggest of these to offer the
/// cashier one-tap amounts, so the list is the note ladder rather than a menu:
/// a cashier whose customer handed cash other than the exact amount should not
/// have to type into the number field.
pub fn denominations(currency: Currency) -> &'static [i64] {
    match currency.code() {
        // Uganda (UGX): 500 coin, 1k, 2k, 5k, 10k, 20k, 50k notes
        "UGX" => &[500, 1_000, 2_000, 5_000, 10_000, 20_000, 50_000],
        // Kenya (KES, scale 2): 50, 100, 200, 500, 1,000 shillings
        "KES" => &[5_000, 10_000, 20_000, 50_000, 100_000],
        // Tanzania (TZS, scale 2): 500, 1,000, 2,000, 5,000, 10,000 shillings
        "TZS" => &[50_000, 100_000, 200_000, 500_000, 1_000_000],
        // Rwanda (RWF, scale 0): 500, 1,000, 2,000, 5,000 francs
        "RWF" => &[500, 1_000, 2_000, 5_000],
        // Burundi (BIF, scale 0): 500, 1,000, 2,000, 5,000, 10,000 francs
        "BIF" => &[500, 1_000, 2_000, 5_000, 10_000],
        // South Sudan (SSP, scale 2): 10, 20, 50, 100, 500, 1,000 pounds
        "SSP" => &[1_000, 2_000, 5_000, 10_000, 50_000, 100_000],
        // DR Congo (CDF, scale 2): 100, 200, 500, 1,000, 5,000, 10,000, 20,000 francs
        "CDF" => &[
            10_000, 20_000, 50_000, 100_000, 500_000, 1_000_000, 2_000_000,
        ],
        // US Dollar (USD, scale 2): $1, $5, $10, $20, $50, $100
        "USD" => &[100, 500, 1_000, 2_000, 5_000, 10_000],
        // Euro (EUR, scale 2): €5, €10, €20, €50, €100
        "EUR" => &[500, 1_000, 2_000, 5_000, 10_000],
        // British Pound (GBP, scale 2): £5, £10, £20, £50
        "GBP" => &[500, 1_000, 2_000, 5_000],
        _ => match currency.exponent().unwrap_or(0) {
            0 => &[500, 1_000, 2_000, 5_000, 10_000, 20_000, 50_000],
            _ => &[100, 500, 1_000, 2_000, 5_000, 10_000],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_choice_resolves_to_its_own_currency() {
        let mut codes = Vec::new();
        for (_, choices) in GROUPS {
            for choice in *choices {
                let currency = choice
                    .currency()
                    .unwrap_or_else(|| panic!("{} has no regular currency", choice.label));
                assert!(
                    !codes.contains(&currency),
                    "{} repeats {}",
                    choice.label,
                    currency.code()
                );
                codes.push(currency);
            }
        }
    }

    /// The step is in minor units, so a currency with two decimal places needs
    /// a hundred times the number a currency with none does for the same coin.
    /// Getting that wrong rounds Kenya to the nearest 1/100 of a shilling.
    #[test]
    fn cash_steps_are_whole_coins() {
        assert_eq!(cash_step(Currency::UGX), 100, "the 100 shilling coin");
        assert_eq!(cash_step(Currency::KES), 100, "one shilling, in cents");
        assert_eq!(cash_step(Currency::TZS), 5_000, "fifty shillings, in cents");
        assert_eq!(cash_step(Currency::USD), 1, "no coin is bigger than a cent");

        for (_, choices) in GROUPS {
            for choice in *choices {
                let currency = choice.currency().expect("tender");
                assert!(
                    cash_step(currency) >= 1,
                    "{} cannot round to nothing",
                    currency.code()
                );
            }
        }
    }

    #[test]
    fn denominations_are_positive_and_sorted() {
        for currency in [
            Currency::UGX,
            Currency::KES,
            Currency::TZS,
            Currency::USD,
            Currency::EUR,
            Currency::GBP,
        ] {
            let denoms = denominations(currency);
            assert!(!denoms.is_empty());
            for window in denoms.windows(2) {
                assert!(window[0] < window[1]);
                assert!(window[0] > 0);
            }
        }
    }
}
