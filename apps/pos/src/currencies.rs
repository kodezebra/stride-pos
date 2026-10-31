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
}
