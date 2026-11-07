//! The request the companion app sends to `/v1/scan`.
//!
//! A fixed shape with `deny_unknown_fields` is intentional: this listener is
//! reachable from the shop network, and "parse whatever JSON arrives" is how a
//! small endpoint becomes somebody else's API. Every field that can reach the
//! cart is validated here, before the state sees it.

use std::fmt;

use serde::Deserialize;

use super::ScanEvent;

/// The largest request body the listener reads. The useful payload is a barcode
/// and a two-word action; anything larger is not from our app.
pub const MAX_BODY_BYTES: usize = 512;
const MAX_BARCODE_CHARS: usize = 128;

/// What the phone asks the till to do with a barcode.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ScanAction {
    /// Put the item in the cart. The default, because that is why a phone is
    /// pointed at a shelf.
    #[default]
    Sell,
    /// Open the product editor with this barcode filled in.
    Add,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireScan {
    barcode: String,
    #[serde(default)]
    action: ScanAction,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    TooLarge,
    InvalidJson,
    EmptyBarcode,
    BarcodeTooLong,
    BarcodeNotPrintable,
}

impl Error {
    pub fn message(&self) -> &'static str {
        match self {
            Self::TooLarge => "the scan is too large",
            Self::InvalidJson => "expected a JSON object with a barcode",
            Self::EmptyBarcode => "the barcode is empty",
            Self::BarcodeTooLong => "the barcode is too long",
            Self::BarcodeNotPrintable => "the barcode contains characters a barcode cannot hold",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for Error {}

pub fn parse(body: &[u8]) -> Result<ScanEvent, Error> {
    if body.len() > MAX_BODY_BYTES {
        return Err(Error::TooLarge);
    }

    let wire: WireScan = serde_json::from_slice(body).map_err(|_| Error::InvalidJson)?;
    let barcode = wire.barcode.trim();

    if barcode.is_empty() {
        return Err(Error::EmptyBarcode);
    }
    if barcode.chars().count() > MAX_BARCODE_CHARS {
        return Err(Error::BarcodeTooLong);
    }
    // A barcode is printed data, not a note. Keeping it printable also means a
    // scan cannot put a newline or a terminal escape into the search field.
    if !barcode
        .chars()
        .all(|character| character == ' ' || character.is_ascii_graphic())
    {
        return Err(Error::BarcodeNotPrintable);
    }

    Ok(ScanEvent {
        barcode: barcode.to_owned(),
        action: wire.action,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_actionless_scan_sells() {
        let event = parse(br#"{"barcode":"4006381333931"}"#).unwrap();
        assert_eq!(event.barcode, "4006381333931");
        assert_eq!(event.action, ScanAction::Sell);
    }

    #[test]
    fn add_is_recognised_and_the_barcode_is_trimmed() {
        let event = parse(br#"{"barcode":"  ABC-123  ","action":"add"}"#).unwrap();
        assert_eq!(event.barcode, "ABC-123");
        assert_eq!(event.action, ScanAction::Add);
    }

    #[test]
    fn an_empty_barcode_is_rejected() {
        assert_eq!(
            parse(br#"{"barcode":"   "}"#).unwrap_err(),
            Error::EmptyBarcode
        );
    }

    #[test]
    fn control_characters_and_newlines_are_rejected() {
        assert_eq!(
            parse(br#"{"barcode":"123\n456"}"#).unwrap_err(),
            Error::BarcodeNotPrintable
        );
    }

    #[test]
    fn unknown_fields_are_rejected() {
        assert_eq!(
            parse(br#"{"barcode":"123","price":1}"#).unwrap_err(),
            Error::InvalidJson
        );
    }

    #[test]
    fn unknown_actions_are_rejected() {
        assert_eq!(
            parse(br#"{"barcode":"123","action":"delete"}"#).unwrap_err(),
            Error::InvalidJson
        );
    }

    #[test]
    fn a_body_over_the_limit_is_rejected_before_json_parsing() {
        let body = vec![b'x'; MAX_BODY_BYTES + 1];
        assert_eq!(parse(&body).unwrap_err(), Error::TooLarge);
    }

    #[test]
    fn a_barcode_over_the_limit_is_rejected() {
        let body = format!(r#"{{"barcode":"{}"}}"#, "1".repeat(MAX_BARCODE_CHARS + 1));
        assert_eq!(parse(body.as_bytes()).unwrap_err(), Error::BarcodeTooLong);
    }

    #[test]
    fn malformed_json_is_rejected() {
        assert_eq!(parse(br#"{"barcode":"#).unwrap_err(), Error::InvalidJson);
    }
}
