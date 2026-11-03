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

/// 128 bits of OS randomness. A till is not a bank, but a token that can be
/// guessed would let anyone on the café Wi-Fi put items in the basket.
const TOKEN_BYTES: usize = 16;

/// The terminal's side of the phone scanner, stored inside `settings.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScanConfig {
    /// The listener does not bind anything until this is true.
    pub enabled: bool,
    pub port: u16,
    /// Lowercase hex. Empty on a settings file written before this feature
    /// existed; [`ScanConfig::ensure_token`] fills it on first launch.
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

/// A fresh pairing token: 32 lowercase hex characters.
pub fn new_token() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    if getrandom::fill(&mut bytes).is_err() {
        // `getrandom` failing means the OS has no entropy. Refusing to start the
        // listener is the safe response, so an empty token is invalid by design.
        return String::new();
    }
    let mut token = String::with_capacity(TOKEN_BYTES * 2);
    for byte in bytes {
        let _ = write!(token, "{byte:02x}");
    }
    token
}

fn valid_token(token: &str) -> bool {
    token.len() == TOKEN_BYTES * 2 && token.bytes().all(|byte| byte.is_ascii_hexdigit())
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
    fn a_new_token_is_32_lowercase_hex_characters() {
        let token = new_token();
        assert_eq!(token.len(), 32);
        assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(!token.bytes().any(|byte| byte.is_ascii_uppercase()));
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
            token: "0123456789abcdef0123456789abcdef".to_owned(),
        };
        assert_eq!(
            config.pair_uri("192.168.1.20"),
            "stridepos://pair?host=192.168.1.20&port=47831&token=0123456789abcdef0123456789abcdef"
        );
        assert_eq!(config.endpoint("192.168.1.20"), "http://192.168.1.20:47831");
    }
}
