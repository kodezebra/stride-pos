//! The trading day: the shift, the drawer, and the roll from one day to the next.
//!
//! A shop day is a cash float, a running drawer, and a close. The timer that
//! rolls the day over belongs here because a day is the only thing it changes.

use super::*;

impl State {
    /// The drawer a sale is about to land in, opened if the day has not started.
    ///
    /// A failure is logged and the sale goes through unattributed. Bookkeeping
    /// must never refuse a sale: a gap in a count is recoverable, a customer
    /// turned away is not.
    pub(super) fn ensure_shift(&mut self) -> Option<String> {
        if let Some(drawer) = self.drawer.as_ref()
            && drawer.shift.is_open()
        {
            return Some(drawer.shift.uid.clone());
        }

        let user = self.session.as_ref().map(|session| session.user_id.clone());
        match crate::db::ensure_open_shift(
            &self.store,
            self.settings.cash_float_minor,
            user.as_deref(),
        ) {
            Ok(shift) => {
                let uid = shift.uid.clone();
                self.refresh_drawer();
                Some(uid)
            }
            Err(error) => {
                tracing::error!(error = %error, "opening the drawer failed");
                None
            }
        }
    }

    /// Re-read the drawer after a sale or a close, so the card shows the money
    /// that is actually in it.
    pub fn refresh_drawer(&mut self) {
        self.drawer = crate::db::drawer(&self.store);
    }

    /// Count the drawer and close the day.
    ///
    /// One number, one tap: the operator types what is in the drawer and the
    /// till works out what should have been there. A count that does not parse
    /// leaves the field and the card open rather than closing on a guess, and a
    /// second tap cannot overwrite a count that was already made.
    pub fn close_day(&mut self) {
        let Some(drawer) = self.drawer.as_ref() else {
            return;
        };
        if !drawer.shift.is_open() {
            return;
        }

        let counted = match Money::parse(self.drawer_count.trim(), self.currency) {
            Ok(amount) => amount.minor(),
            Err(_) => {
                self.drawer_error = Some("Type the cash you counted.".to_owned());
                return;
            }
        };

        let uid = drawer.shift.uid.clone();
        let user = self.session.as_ref().map(|session| session.user_id.clone());
        match crate::db::close_shift(&self.store, &uid, counted, user.as_deref()) {
            Ok(()) => {
                self.drawer_error = None;
                self.drawer_count.clear();
                self.refresh_drawer();
            }
            Err(error) => {
                tracing::error!(error = %error, "closing the drawer failed");
                self.drawer_error = Some(error.to_string());
            }
        }
    }

    /// Called once a frame. Locks the till after a stretch with no input.
    ///
    /// Idleness is read from input events rather than from a timer started at
    /// sign-in: a mouse that moves is a person, and a screen nobody has touched
    /// is not. The clock only runs while someone is signed in.
    pub fn tick(&mut self, ctx: &egui::Context) {
        if ctx.input(|input| !input.events.is_empty()) {
            self.last_activity = Instant::now();
        }
        let Some(session) = &self.session else {
            return;
        };
        if self.last_activity.elapsed() >= session.role.idle_timeout() {
            self.lock();
        }
    }
}
