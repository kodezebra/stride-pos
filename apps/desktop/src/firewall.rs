//! Whether the computer's firewall will let a phone reach the scanner.
//!
//! The listener can be healthy and still be unreachable: the host firewall
//! drops the packet before the app ever sees it. Testing on the till itself
//! hides this, because `127.0.0.1` never crosses the interface the firewall
//! polices. A till that says "listening" while the phone says "cannot reach"
//! is therefore the normal shape of the problem, not a contradiction.
//!
//! This module answers two questions for the Settings card: is something
//! filtering inbound traffic, and — when the operator presses the button — how
//! to allow exactly the scanner's port. The privileged step goes through
//! `pkexec`, so the OS asks for the administrator password in its own dialog;
//! the command is printed as a fallback for machines where that dialog cannot
//! appear.
//!
//! It deliberately does *not* inspect firewalld's rules on its own. Reading
//! them (`firewall-cmd --query-port`, `--list-ports`) is gated by the polkit
//! action `org.fedoraproject.FirewallD1.config.info`, which asks for the
//! administrator password — and a password dialog appearing because Settings
//! was opened is not a trade anyone would accept. So a running firewall is
//! reported as "running, port not known", and the card offers the one action
//! that can settle it. `Allowed` therefore means "this card allowed it just
//! now", never "the rules were read".
//!
//! Linux only for now, because that is what the till ships. On Windows the
//! same card would need a backend that triggers UAC (`ShellExecuteW` with
//! `runas`) and adds an inbound rule scoped to the executable with
//! `netsh advfirewall`, and Windows already prompts on first listen, so the
//! detection there is about a dismissed prompt rather than a missing one.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

/// How the operating system is filtering the scanner's port.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Snapshot {
    /// Not asked, or the scanner is off.
    #[default]
    Unprobed,
    /// No host firewall was found.
    Clear,
    /// The port was allowed from this card, during this run of the till.
    Allowed(Backend),
    /// A firewall is running. Whether it already allows the port cannot be
    /// read without the administrator password, so the till does not read it.
    Unknown(Backend),
}

/// Which firewall the probe found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Firewalld,
    Ufw,
}

impl Backend {
    pub fn label(self) -> &'static str {
        match self {
            Self::Firewalld => "firewalld",
            Self::Ufw => "ufw",
        }
    }
}

impl Snapshot {
    /// Whether the card should offer to open the port.
    pub fn wants_allow(&self) -> bool {
        matches!(self, Self::Unknown(_))
    }

    /// The command that does the same thing as the button, for machines where
    /// `pkexec` is missing or the dialog cannot be shown.
    pub fn command(&self, port: u16) -> Option<String> {
        match self {
            Self::Unknown(Backend::Firewalld) => Some(format!(
                "sudo firewall-cmd --add-port={port}/tcp --permanent && sudo firewall-cmd --reload"
            )),
            Self::Unknown(Backend::Ufw) => Some(format!("sudo ufw allow {port}/tcp")),
            _ => None,
        }
    }
}

/// What the Settings card reads each frame.
#[derive(Clone, Debug, Default)]
pub struct Status {
    pub snapshot: Snapshot,
    pub busy: bool,
    pub error: Option<String>,
}

enum Report {
    Probed {
        port: u16,
        snapshot: Snapshot,
    },
    /// `Ok(Some(backend))` means the rule was added and reloaded, `Ok(None)`
    /// means there was no firewall to change.
    Allowed {
        port: u16,
        result: Result<Option<Backend>, String>,
    },
}

/// Runs the probe and the privileged step off the UI thread.
///
/// `pkexec` blocks until the operator answers its dialog, which can be a
/// minute; the frame loop must not wait for it.
pub struct Service {
    tx: Sender<Report>,
    rx: Receiver<Report>,
    /// The port the current snapshot describes. `None` while the scanner is off.
    port: Option<u16>,
    workers: usize,
    snapshot: Snapshot,
    error: Option<String>,
}

impl Service {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx,
            port: None,
            workers: 0,
            snapshot: Snapshot::Unprobed,
            error: None,
        }
    }

    /// Probe again when the listener is turned on or moved to another port.
    pub fn sync(&mut self, port: Option<u16>) {
        let port = port.filter(|port| *port != 0);
        if self.port == port {
            return;
        }

        self.port = port;
        self.snapshot = Snapshot::Unprobed;
        self.error = None;
        if let Some(port) = port {
            self.probe(port);
        }
    }

    /// Ask the OS to allow the port. The dialog is the operator's to answer.
    pub fn allow(&mut self) {
        let Some(port) = self.port else {
            return;
        };
        if self.workers > 0 {
            return;
        }

        self.error = None;
        self.workers += 1;
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = platform::allow(port);
            let _ = tx.send(Report::Allowed { port, result });
        });
    }

    /// Apply whatever the workers finished since the last frame.
    pub fn poll(&mut self) {
        for report in self.rx.try_iter() {
            self.workers = self.workers.saturating_sub(1);
            match report {
                Report::Probed { port, snapshot } => {
                    if self.port == Some(port) {
                        self.snapshot = snapshot;
                    }
                }
                Report::Allowed { port, result } => {
                    if self.port != Some(port) {
                        continue;
                    }
                    match result {
                        Ok(Some(backend)) => {
                            self.snapshot = Snapshot::Allowed(backend);
                            self.error = None;
                        }
                        Ok(None) => {
                            self.snapshot = Snapshot::Clear;
                            self.error = None;
                        }
                        // The rules are unchanged, so the card keeps saying
                        // that the port is not known to be open.
                        Err(error) => self.error = Some(error),
                    }
                }
            }
        }
    }

    pub fn status(&self) -> Status {
        Status {
            snapshot: self.snapshot.clone(),
            busy: self.workers > 0,
            error: self.error.clone(),
        }
    }

    fn probe(&mut self, port: u16) {
        self.workers += 1;
        let tx = self.tx.clone();
        thread::spawn(move || {
            let snapshot = platform::probe(port);
            let _ = tx.send(Report::Probed { port, snapshot });
        });
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::net::IpAddr;
    use std::process::{Command, Output};

    use super::{Backend, Snapshot};

    /// Presence only. Reading whether the port is already open needs the
    /// administrator password, and a passive probe must never ask for one.
    pub(super) fn probe(_port: u16) -> Snapshot {
        if firewalld_present() {
            Snapshot::Unknown(Backend::Firewalld)
        } else if ufw_enabled() {
            Snapshot::Unknown(Backend::Ufw)
        } else {
            Snapshot::Clear
        }
    }

    pub(super) fn allow(port: u16) -> Result<Option<Backend>, String> {
        if firewalld_present() {
            let zone = firewalld_zone().unwrap_or_else(|| "public".to_owned());
            elevate(
                "firewall-cmd",
                &[
                    "--permanent".to_owned(),
                    format!("--zone={zone}"),
                    format!("--add-port={port}/tcp"),
                ],
            )?;
            // The permanent change only takes effect on reload. polkit caches
            // the answer for a few minutes, so this is usually the same prompt.
            elevate("firewall-cmd", &["--reload".to_owned()])?;
            return Ok(Some(Backend::Firewalld));
        }

        if ufw_enabled() {
            elevate("ufw", &["allow".to_owned(), format!("{port}/tcp")])?;
            return Ok(Some(Backend::Ufw));
        }

        Ok(None)
    }

    fn elevate(program: &str, args: &[String]) -> Result<(), String> {
        use std::io::Read;
        use std::process::Stdio;
        use std::thread;
        use std::time::{Duration, Instant};

        let mut child = Command::new("pkexec")
            .arg(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    "pkexec is not installed, so the till cannot ask for permission. \
                     Copy the command below into a terminal."
                        .to_owned()
                } else {
                    format!("Could not ask for permission: {error}")
                }
            })?;

        // The password dialog belongs to a person. Give them time to answer,
        // then stop waiting rather than leave the card spinning forever.
        let deadline = Instant::now() + Duration::from_secs(120);
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(100)),
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("The password prompt was not answered. Copy the command \
                                below into a terminal."
                        .to_owned());
                }
                Err(error) => return Err(format!("Could not ask for permission: {error}")),
            }
        };

        let mut stderr = String::new();
        if let Some(mut pipe) = child.stderr.take() {
            let _ = pipe.read_to_string(&mut stderr);
        }
        if status.success() {
            return Ok(());
        }

        let stderr = stderr.trim();
        if !stderr.is_empty() {
            return Err(stderr.to_owned());
        }
        Err(match status.code() {
            Some(126) => "Permission was dismissed.".to_owned(),
            Some(127) => "Permission was refused.".to_owned(),
            Some(code) => format!("The firewall command failed with exit code {code}."),
            None => "The firewall command was stopped.".to_owned(),
        })
    }

    /// The daemon is filtering traffic if its unit is active, even when its
    /// D-Bus interface stops answering — a wedged firewalld is still a
    /// firewalld, and calling that "no firewall" would send the operator
    /// looking in the wrong place.
    fn firewalld_present() -> bool {
        unit_active("firewalld")
    }

    /// The zone the Wi-Fi interface is in, falling back to the default zone.
    ///
    /// NetworkManager can put a connection in any zone; adding the port to the
    /// wrong one is a silent no-op, so ask about the interface we actually use.
    /// Both queries are allowed without authentication (`…FirewallD1.info`).
    fn firewalld_zone() -> Option<String> {
        if let Some(iface) = crate::scan::local_ip().and_then(interface_of) {
            let flag = format!("--get-zone-of-interface={iface}");
            let zone = command("firewall-cmd", &[&flag])
                .filter(|output| output.status.success())
                .map(|output| text(&output.stdout));
            if let Some(zone) = zone.filter(|zone| !zone.is_empty() && zone != "no zone") {
                return Some(zone);
            }
        }

        command("firewall-cmd", &["--get-default-zone"])
            .filter(|output| output.status.success())
            .map(|output| text(&output.stdout))
            .filter(|zone| !zone.is_empty())
    }

    fn ufw_enabled() -> bool {
        let Ok(config) = std::fs::read_to_string("/etc/ufw/ufw.conf") else {
            return false;
        };
        config.lines().any(|line| line.trim() == "ENABLED=yes") && unit_active("ufw")
    }

    fn unit_active(unit: &str) -> bool {
        matches!(
            Command::new("systemctl")
                .args(["is-active", "--quiet", unit])
                .output(),
            Ok(output) if output.status.success()
        )
    }

    fn interface_of(ip: IpAddr) -> Option<String> {
        let output = command("ip", &["-o", "-4", "addr", "show"])?;
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .find_map(|line| parse_addr_line(line, ip))
    }

    /// One `ip -o -4 addr show` line:
    /// `2: wlp4s0    inet 192.168.100.128/24 brd 192.168.100.255 scope global …`
    fn parse_addr_line(line: &str, ip: IpAddr) -> Option<String> {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 4 || fields[2] != "inet" {
            return None;
        }
        let address: IpAddr = fields[3].split('/').next()?.parse().ok()?;
        (address == ip).then(|| fields[1].trim_end_matches(':').to_owned())
    }

    fn command(program: &str, args: &[&str]) -> Option<Output> {
        Command::new(program).args(args).output().ok()
    }

    fn text(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).trim().to_owned()
    }

    #[cfg(test)]
    mod tests {
        use std::net::Ipv4Addr;

        use super::*;

        #[test]
        fn the_interface_holding_an_address_is_found() {
            let line = "2: wlp4s0    inet 192.168.100.128/24 brd 192.168.100.255 \
                        scope global dynamic noprefixroute wlp4s0";
            let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 100, 128));
            assert_eq!(parse_addr_line(line, ip).as_deref(), Some("wlp4s0"));
        }

        #[test]
        fn another_address_on_the_same_line_does_not_match() {
            let line = "3: docker0    inet 172.17.0.1/16 brd 172.17.255.255 scope global docker0";
            let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 100, 128));
            assert!(parse_addr_line(line, ip).is_none());
        }

        #[test]
        fn a_line_that_is_not_an_address_is_ignored() {
            let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 100, 128));
            assert!(parse_addr_line("", ip).is_none());
            assert!(parse_addr_line("1: lo    inet", ip).is_none());
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod platform {
    use super::{Backend, Snapshot};

    /// The till ships for Linux. A Windows backend is described at the top of
    /// this module; until it exists, claiming anything about the firewall would
    /// be a guess, and the card is only drawn on Linux.
    pub(super) fn probe(_port: u16) -> Snapshot {
        Snapshot::Unprobed
    }

    pub(super) fn allow(_port: u16) -> Result<Option<Backend>, String> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_firewall_whose_rules_are_unknown_wants_the_allow_button() {
        assert!(Snapshot::Unknown(Backend::Firewalld).wants_allow());
        assert!(Snapshot::Unknown(Backend::Ufw).wants_allow());
        assert!(!Snapshot::Allowed(Backend::Firewalld).wants_allow());
        assert!(!Snapshot::Clear.wants_allow());
        assert!(!Snapshot::Unprobed.wants_allow());
    }

    #[test]
    fn the_fallback_command_matches_the_firewall() {
        assert_eq!(
            Snapshot::Unknown(Backend::Firewalld)
                .command(47_831)
                .as_deref(),
            Some(
                "sudo firewall-cmd --add-port=47831/tcp --permanent && sudo firewall-cmd --reload"
            )
        );
        assert_eq!(
            Snapshot::Unknown(Backend::Ufw).command(47_831).as_deref(),
            Some("sudo ufw allow 47831/tcp")
        );
        assert_eq!(Snapshot::Allowed(Backend::Ufw).command(47_831), None);
        assert_eq!(Snapshot::Clear.command(47_831), None);
    }
}
