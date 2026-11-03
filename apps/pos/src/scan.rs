//! Scans from a phone, on the shop's local network.
//!
//! The desktop already understands a USB scanner because a scanner is a
//! keyboard: it types a barcode and presses Enter. A phone cannot type into a
//! window, so this module runs a tiny listener instead. The companion app sends
//! one JSON request; [`State::receive_scan`] routes it through the same scan
//! path the keyboard uses.
//!
//! The listener is off by default. While it is on it binds `0.0.0.0`, because a
//! phone on the LAN must reach it, and it accepts only requests that carry the
//! pairing token from Settings. It is not a cloud service and must never be
//! exposed to the internet.
//!
//! [`State::receive_scan`]: crate::state::State::receive_scan

pub mod protocol;
mod server;

use std::fmt::Write as _;
use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::sync::OnceLock;
use std::sync::mpsc::{self, Receiver, Sender};

use serde::{Deserialize, Serialize};

pub use protocol::ScanAction;

/// The port the companion app expects until the till is told otherwise.
pub const DEFAULT_PORT: u16 = 47_831;

/// The characters a pairing code can contain: Crockford's base 32, which leaves
/// out I, L, O and U. A code is read off a screen and typed on a phone, so the
/// letters that look like 1 and 0 are simply not in it.
const TOKEN_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Ten characters is 50 bits of OS randomness. That is far more than an attacker
/// on the shop Wi-Fi can work through, and few enough to type without reading it
/// out twice.
const TOKEN_LEN: usize = 10;

/// The terminal's side of the phone scanner, stored inside `settings.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScanConfig {
    /// The listener does not bind anything until this is true.
    pub enabled: bool,
    pub port: u16,
    /// Upper-case, canonical (see [`normalize_token`]). Empty on a settings file
    /// written before this feature existed; [`ScanConfig::ensure_token`] fills it
    /// on first launch.
    pub token: String,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: DEFAULT_PORT,
            token: String::new(),
        }
    }
}

impl ScanConfig {
    /// Give a new or hand-broken settings file a usable token. Returns whether
    /// the token changed, so the caller can persist it.
    pub fn ensure_token(&mut self) -> bool {
        if valid_token(&self.token) {
            return false;
        }
        self.token = new_token();
        true
    }

    /// What the companion app reads from the pairing QR code.
    pub fn pair_uri(&self, host: &str) -> String {
        format!(
            "stridepos://pair?host={}&port={}&token={}",
            encode_component(host),
            self.port,
            encode_component(&self.token)
        )
    }

    pub fn endpoint(&self, host: &str) -> String {
        format!("http://{}:{}", display_host(host), self.port)
    }
}

/// One scan that reached the desktop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanEvent {
    pub barcode: String,
    pub action: ScanAction,
}

/// What the listener is doing, for the Settings card and the status bar.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ScanStatus {
    #[default]
    Off,
    Listening {
        port: u16,
    },
    Error(String),
}

/// The long-lived side of phone scanning.
///
/// The listener runs for the life of the till process while enabled, not per
/// scan: starting a thread for every barcode would add latency to the one action
/// a queue cannot afford to make slow.
pub struct Service {
    tx: Sender<ScanEvent>,
    rx: Receiver<ScanEvent>,
    listener: Option<server::Listener>,
    /// The configuration the listener was started with. `None` before the first
    /// frame; equal configs do not restart a healthy listener.
    config: Option<ScanConfig>,
    status: ScanStatus,
}

impl Default for Service {
    fn default() -> Self {
        Self::new()
    }
}

impl Service {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx,
            listener: None,
            config: None,
            status: ScanStatus::Off,
        }
    }

    /// Bring the listener in line with Settings. Cheap when nothing changed.
    pub fn sync(&mut self, config: &ScanConfig) {
        if self.config.as_ref() == Some(config) {
            return;
        }

        self.stop_listener();
        self.config = Some(config.clone());

        if !config.enabled {
            self.status = ScanStatus::Off;
            return;
        }
        if config.port == 0 {
            self.status = ScanStatus::Error("Choose a port between 1 and 65535.".to_owned());
            return;
        }
        if !valid_token(&config.token) {
            self.status = ScanStatus::Error(
                "The pairing token is missing. Turn the listener off and on again.".to_owned(),
            );
            return;
        }

        match server::Listener::start(config.port, config.token.clone(), self.tx.clone()) {
            Ok(listener) => {
                let port = listener.port();
                self.listener = Some(listener);
                self.status = ScanStatus::Listening { port };
            }
            Err(error) => {
                self.status = ScanStatus::Error(error);
            }
        }
    }

    /// Take every scan that arrived since the last frame.
    pub fn drain(&mut self) -> Vec<ScanEvent> {
        self.rx.try_iter().collect()
    }

    pub fn status(&self) -> &ScanStatus {
        &self.status
    }

    fn stop_listener(&mut self) {
        if let Some(listener) = self.listener.take() {
            listener.stop();
        }
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.stop_listener();
    }
}

/// The machine's address on the shop network, as a phone should type it.
///
/// Connecting a UDP socket chooses a route but sends no packet, so this asks the
/// OS which interface it would use without touching the network. The result is
/// cached: a till's address rarely changes while the app is open, and Settings
/// draws every frame.
pub fn local_ip() -> Option<IpAddr> {
    static ADDRESS: OnceLock<Option<IpAddr>> = OnceLock::new();
    *ADDRESS.get_or_init(|| {
        for target in ["8.8.8.8:80", "1.1.1.1:80"] {
            let Ok(socket) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) else {
                continue;
            };
            if socket.connect(target).is_err() {
                continue;
            }
            let Ok(local) = socket.local_addr() else {
                continue;
            };
            let IpAddr::V4(address) = local.ip() else {
                continue;
            };
            if !address.is_loopback() && !address.is_unspecified() {
                return Some(IpAddr::V4(address));
            }
        }
        None
    })
}

/// A fresh pairing token: ten characters from [`TOKEN_ALPHABET`].
pub fn new_token() -> String {
    let mut bytes = [0u8; TOKEN_LEN];
    if getrandom::fill(&mut bytes).is_err() {
        // `getrandom` failing means the OS has no entropy. Refusing to start the
        // listener is the safe response, so an empty token is invalid by design.
        return String::new();
    }
    let mut token = String::with_capacity(TOKEN_LEN);
    for byte in bytes {
        // 256 is a multiple of 32, so the low five bits are uniform.
        token.push(TOKEN_ALPHABET[(byte & 31) as usize] as char);
    }
    token
}

/// Read a pairing code the way a person types it, and return the canonical form.
///
/// Dashes and spaces are separators, not secrets: `7qk4-m2x9ab` and `7QK4M2X9AB`
/// are the same code. `I`/`L` and `O` are folded into `1` and `0`, because those
/// are the letters the alphabet deliberately omits.
pub fn normalize_token(raw: &str) -> Option<String> {
    let mut token = String::with_capacity(TOKEN_LEN);
    for byte in raw.trim().bytes() {
        if byte == b'-' || byte == b' ' {
            continue;
        }
        let byte = match byte.to_ascii_uppercase() {
            b'I' | b'L' => b'1',
            b'O' => b'0',
            other => other,
        };
        if !TOKEN_ALPHABET.contains(&byte) {
            return None;
        }
        token.push(byte as char);
    }
    (token.len() == TOKEN_LEN).then_some(token)
}

/// The code as Settings shows it: two groups of five, which is what makes a
/// ten-character code easy to read aloud and type.
pub fn spaced_token(token: &str) -> String {
    match token.split_at_checked(TOKEN_LEN / 2) {
        Some((left, right)) => format!("{left}-{right}"),
        None => token.to_owned(),
    }
}

fn valid_token(token: &str) -> bool {
    normalize_token(token).is_some_and(|normal| normal == token)
}

fn display_host(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_owned()
    }
}

fn encode_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_token_is_ten_characters_that_are_hard_to_misread() {
        let token = new_token();
        assert_eq!(token.len(), TOKEN_LEN);
        assert!(token.bytes().all(|byte| TOKEN_ALPHABET.contains(&byte)));
        assert!(!token.bytes().any(|byte| b"ILOU".contains(&byte)));
    }

    #[test]
    fn a_token_can_be_typed_with_dashes_or_in_lower_case() {
        let canonical = "7QK4M2X9AB";
        for typed in ["7QK4M2X9AB", "7qk4m2x9ab", "7qk4-m2x9ab", " 7QK4 M2X9AB "] {
            assert_eq!(
                normalize_token(typed).as_deref(),
                Some(canonical),
                "{typed}"
            );
        }
    }

    #[test]
    fn letters_that_look_like_digits_are_folded_not_rejected() {
        assert_eq!(normalize_token("7OK4M2X9AB").as_deref(), Some("70K4M2X9AB"));
        assert_eq!(normalize_token("7IK4M2X9AB").as_deref(), Some("71K4M2X9AB"));
    }

    #[test]
    fn a_token_that_is_the_wrong_length_or_has_stray_characters_is_refused() {
        for typed in ["7QK4M2X9A", "7QK4M2X9ABC", "", "7QK4M2X9A!", "7QK4M2X9AU"] {
            assert!(normalize_token(typed).is_none(), "{typed}");
        }
    }

    #[test]
    fn the_displayed_code_is_two_groups_of_five() {
        assert_eq!(spaced_token("7QK4M2X9AB"), "7QK4M-2X9AB");
        assert_eq!(spaced_token(""), "");
    }

    #[test]
    fn ensure_token_replaces_an_empty_or_broken_value() {
        let mut config = ScanConfig::default();
        assert!(config.ensure_token());
        assert!(valid_token(&config.token));
        assert!(!config.ensure_token());

        config.token = "not-a-token".to_owned();
        assert!(config.ensure_token());
        assert!(valid_token(&config.token));
    }

    #[test]
    fn the_pair_uri_carries_everything_the_phone_needs() {
        let config = ScanConfig {
            enabled: true,
            port: 47_831,
            token: "7QK4M2X9AB".to_owned(),
        };
        assert_eq!(
            config.pair_uri("192.168.1.20"),
            "stridepos://pair?host=192.168.1.20&port=47831&token=7QK4M2X9AB"
        );
        assert_eq!(config.endpoint("192.168.1.20"), "http://192.168.1.20:47831");
    }
}
