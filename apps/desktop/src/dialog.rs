//! The computer's own file chooser.
//!
//! Import and export need a path from the operator. The till asks the computer
//! for it, in the window every other program on the machine opens: the
//! operator's own folders, their own shortcuts, the places they already know
//! how to find. [`crate::picker`] is still here, and is what a machine with no
//! chooser to show gets instead — it always works, but nobody has to learn it.
//!
//! A native dialog blocks the thread that opened it, and a till whose window
//! freezes for as long as the operator browses looks broken. The question is
//! asked on a worker thread and the answer arrives through a channel, the same
//! shape as the firewall probe in `firewall.rs`; `shell.rs` looks for it once a
//! frame and keeps frames coming while the question is open.
//!
//! Nothing here is drawn. The dialog is the system's window, and the till draws
//! nothing at all while it waits.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use crate::picker::Purpose;

/// How quickly a dialog can come back and still be believed.
///
/// A person cannot read a file dialog, decide and cancel it in a fifth of a
/// second. A system call that fails to open one returns at once. Anything
/// faster than this never opened, and the till offers the picker it can always
/// draw instead of leaving the operator with a button that did nothing.
pub const INSTANT: Duration = Duration::from_millis(200);

/// One question, open until the operator answers it.
///
/// Only one at a time: a second dialog would land behind the first and leave
/// the operator with two windows and no idea which one the till is waiting for.
pub struct Chooser {
    tx: Sender<Answer>,
    rx: Receiver<Answer>,
    /// The question a dialog is answering. `None` when none is open.
    asking: Option<Purpose>,
}

/// What came back from a dialog.
pub struct Answer {
    pub purpose: Purpose,
    /// `None` when the operator cancelled.
    pub path: Option<PathBuf>,
    /// How long the question was open; see [`INSTANT`].
    pub open_for: Duration,
}

impl Chooser {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx,
            asking: None,
        }
    }

    /// Whether a dialog is open.
    pub fn busy(&self) -> bool {
        self.asking.is_some()
    }

    /// Ask the computer for a path. `false` means a dialog is already open.
    ///
    /// The caller checks [`available`] first: this asks, it does not decide
    /// whether there is anything to ask.
    pub fn ask(&mut self, purpose: Purpose, start: PathBuf) -> bool {
        self.ask_with(purpose, start, show)
    }

    /// The same, with the dialog itself passed in, so a test can answer without
    /// opening a window.
    fn ask_with(
        &mut self,
        purpose: Purpose,
        start: PathBuf,
        show: impl FnOnce(Purpose, PathBuf) -> Answer + Send + 'static,
    ) -> bool {
        if self.asking.is_some() {
            return false;
        }
        self.asking = Some(purpose);
        let tx = self.tx.clone();
        thread::spawn(move || {
            let answer = show(purpose, start);
            // The till may have closed while the operator was browsing; a
            // send nobody is listening for is not an error worth reporting.
            let _ = tx.send(answer);
        });
        true
    }

    /// The answer to the question that just closed, if there is one.
    pub fn poll(&mut self) -> Option<Answer> {
        let answer = self.rx.try_recv().ok()?;
        self.asking = None;
        Some(answer)
    }
}

/// Whether this computer has a file chooser to show.
///
/// Windows always does: the common dialog is part of the system. On Linux the
/// dialog belongs to the desktop portal — the same window every other program
/// opens — which the session bus starts on demand and which needs a portal
/// backend installed beside the desktop; `zenity` is the fallback when there is
/// none. A till image can have neither, and a button that opens nothing is
/// worse than the till's own picker, so the pieces are checked instead of
/// assumed. The check errs towards "yes": a chooser that then fails to open is
/// caught by [`INSTANT`].
#[cfg(windows)]
pub fn available() -> bool {
    true
}

#[cfg(target_os = "linux")]
pub fn available() -> bool {
    chooser_available(
        session_bus(),
        portal_installed(),
        command_installed("zenity"),
    )
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn available() -> bool {
    false
}

/// The decision, with the three facts it rests on passed in so a test can hold
/// them still.
#[cfg(target_os = "linux")]
fn chooser_available(session_bus: bool, portal: bool, zenity: bool) -> bool {
    (session_bus && portal) || zenity
}

/// Whether this session has a bus to reach the portal on.
///
/// Usually advertised in the environment. A systemd session may leave it
/// implicit and put the socket at `$XDG_RUNTIME_DIR/bus`, which is where the
/// portal looks when the variable is absent.
#[cfg(target_os = "linux")]
fn session_bus() -> bool {
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some() {
        return true;
    }
    std::env::var_os("XDG_RUNTIME_DIR")
        .is_some_and(|dir| std::path::Path::new(&dir).join("bus").exists())
}

/// Whether a portal backend is installed.
///
/// `xdg-desktop-portal` is one half; the other is a backend per desktop
/// (`xdg-desktop-portal-gtk`, `-kde`, …) and it is the backend that owns the
/// chooser. Their `.portal` descriptors are what the front end reads, so one of
/// those is the cheapest honest sign that a chooser exists.
#[cfg(target_os = "linux")]
fn portal_installed() -> bool {
    let Ok(entries) = std::fs::read_dir("/usr/share/xdg-desktop-portal/portals") else {
        return false;
    };
    entries
        .flatten()
        .any(|entry| entry.path().extension().is_some_and(|ext| ext == "portal"))
}

/// Whether a program is on `PATH`, for the `zenity` fallback.
#[cfg(target_os = "linux")]
fn command_installed(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
}

/// Open the system dialog and wait for it.
///
/// Runs on the worker thread, so blocking is the point rather than a problem.
fn show(purpose: Purpose, start: PathBuf) -> Answer {
    let asked_at = Instant::now();
    let dialog = rfd::FileDialog::new()
        .set_title(title(purpose))
        .set_directory(&start);
    let path = match purpose {
        // Only a CSV reads back correctly, so nothing else is offered.
        Purpose::Import => dialog.add_filter("CSV", &["csv"]).pick_file(),
        // An export names the file itself, stamped with when the books were
        // taken; the operator chooses where it lands.
        Purpose::Export(_) => dialog.pick_folder(),
    };
    Answer {
        purpose,
        path,
        open_for: asked_at.elapsed(),
    }
}

fn title(purpose: Purpose) -> &'static str {
    match purpose {
        Purpose::Import => "Import products from CSV",
        Purpose::Export(kind) => kind.heading(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::picker::ExportKind;

    /// Stands in for the system dialog: answers at once, from a thread, without
    /// a window.
    fn answered(purpose: Purpose, start: PathBuf) -> Answer {
        Answer {
            purpose,
            path: Some(start),
            open_for: Duration::ZERO,
        }
    }

    /// The worker needs a moment; a test that only polled once would race it.
    fn wait(chooser: &mut Chooser) -> Answer {
        for _ in 0..100 {
            if let Some(answer) = chooser.poll() {
                return answer;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the worker never answered");
    }

    #[test]
    fn an_answer_arrives_through_the_channel() {
        let mut chooser = Chooser::new();
        let folder = PathBuf::from("/tmp/opencode/export");

        assert!(chooser.ask_with(Purpose::Export(ExportKind::Sales), folder.clone(), answered));
        assert!(chooser.busy());

        let answer = wait(&mut chooser);
        assert_eq!(answer.purpose, Purpose::Export(ExportKind::Sales));
        assert_eq!(answer.path, Some(folder));
        assert!(!chooser.busy());
    }

    #[test]
    fn one_question_waits_for_the_other() {
        let mut chooser = Chooser::new();
        let folder = PathBuf::from("/tmp/opencode");

        assert!(chooser.ask_with(Purpose::Import, folder.clone(), answered));
        assert!(!chooser.ask_with(Purpose::Import, folder, answered));
        wait(&mut chooser);
    }

    /// A failed dialog is indistinguishable from a cancel by its result alone;
    /// the clock is what tells them apart.
    #[test]
    fn a_dialog_that_closes_at_once_says_so() {
        let mut chooser = Chooser::new();

        chooser.ask_with(
            Purpose::Import,
            PathBuf::from("/tmp/opencode"),
            |purpose, _| Answer {
                purpose,
                path: None,
                open_for: Duration::from_micros(80),
            },
        );

        let answer = wait(&mut chooser);
        assert_eq!(answer.path, None);
        assert!(answer.open_for < INSTANT);
    }

    #[cfg(target_os = "linux")]
    mod linux {
        use super::super::chooser_available;

        #[test]
        fn a_bare_image_has_no_chooser() {
            assert!(!chooser_available(false, false, false));
        }

        #[test]
        fn a_portal_needs_a_session_bus() {
            assert!(!chooser_available(false, true, false));
        }

        #[test]
        fn a_portal_on_a_session_bus_is_enough() {
            assert!(chooser_available(true, true, false));
        }

        #[test]
        fn zenity_alone_is_enough() {
            assert!(chooser_available(false, false, true));
        }
    }
}
