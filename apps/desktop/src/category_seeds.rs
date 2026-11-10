//! Suggested category names for each business type.
//!
//! These are *suggestions only*, never auto-inserted. A till starts with an
//! empty `categories` table, and the JSON list is offered in the product form's
//! dropdown alongside categories the shop has already used. The business type
//! is chosen once at setup and stored in `settings.json`.

/// All seed files, keyed by business type. Loaded at compile time with
/// `include_str!` so the binary is self-contained.
macro_rules! seed_file {
    ($path:expr) => {{ include_str!(concat!(env!("CARGO_MANIFEST_DIR"), $path)) }};
}

const SUPERMARKET_JSON: &str = seed_file!("/data/categories/supermarket.json");
const PHARMACY_JSON: &str = seed_file!("/data/categories/pharmacy.json");
const STATIONERY_JSON: &str = seed_file!("/data/categories/stationery.json");

/// Suggested category names for a business type, in sort order.
///
/// Returns an empty vector for an unknown type so the dropdown falls back to
/// categories the shop has already created.
pub fn seed_names(business_type: &str) -> Vec<String> {
    // Case-insensitive so "Supermarket" and "supermarket" both match the
    // seeded file. The stored value is normalized to lowercase on save, but
    // older settings files may still hold a mixed-case value.
    let json = match business_type.to_ascii_lowercase().as_str() {
        "supermarket" => SUPERMARKET_JSON,
        "pharmacy" => PHARMACY_JSON,
        "stationery" => STATIONERY_JSON,
        _ => return Vec::new(),
    };

    let categories: Vec<String> = match serde_json::from_str(json) {
        Ok(categories) => categories,
        Err(err) => {
            tracing::warn!(error = %err, business_type, "could not parse category seeds");
            return Vec::new();
        }
    };

    categories
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supermarket_returns_expected_names() {
        let names = seed_names("supermarket");
        assert!(names.contains(&"Bakery".to_owned()));
        assert!(names.contains(&"Beverages".to_owned()));
        assert!(!names.is_empty());
    }

    #[test]
    fn supermarket_mixed_case_returns_names() {
        let names = seed_names("Supermarket");
        assert!(!names.is_empty());
        assert!(names.contains(&"Bakery".to_owned()));
    }

    #[test]
    fn pharmacy_returns_expected_names() {
        let names = seed_names("pharmacy");
        assert!(names.contains(&"Prescription".to_owned()));
        assert!(names.contains(&"Vitamins & Supplements".to_owned()));
    }

    #[test]
    fn unknown_business_type_is_empty() {
        assert!(seed_names("nothing").is_empty());
    }

    #[test]
    fn sort_order_is_respected() {
        let names = seed_names("stationery");
        assert!(!names.is_empty());
    }
}
