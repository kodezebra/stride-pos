//! The command palette's contents.
//!
//! One list, built from the same module indices the rail uses. A command is an
//! ordinary closure over [`State`], which is the whole point: anything a widget
//! can do, the palette can do, without a second code path to keep in sync.
//!
//! Module commands go through [`modules::open`] rather than `State::select`, so
//! the palette cannot walk past a permission the rail enforces.
//!
//! The module list is the same role-filtered list the rail draws, so a cashier
//! is not offered a destination they cannot open. The palette is rebuilt when
//! the signed-in role changes, not every frame.

use ui_shell::regions::Command;

use crate::auth::Role;
use crate::modules;
use crate::state::State;

pub fn all(role: Role) -> Vec<Command<State>> {
    let mut commands = Vec::new();

    for &view in modules::views(role) {
        let tabs = modules::tabs(view, role);

        // A view with one page is reached by its own name. A view with several
        // is reached page by page, because "Go to Shop" would leave the person
        // to guess which page they wanted.
        if tabs.is_empty() {
            let index = modules::landing(view);
            commands.push(
                Command::new(
                    format!("Go to {}", view.label()),
                    move |state: &mut State| {
                        modules::open(state, index);
                    },
                )
                .hint(view.hotkey().name()),
            );
            continue;
        }

        for (offset, &index) in tabs.iter().enumerate() {
            let mut command = Command::new(
                format!("Go to {}", modules::tab_label(index)),
                move |state: &mut State| {
                    modules::open(state, index);
                },
            );
            // The key opens the view, and the view lands on its first tab, so
            // only that tab can claim it. Two rows promising F2 would be a lie
            // about one of them.
            if offset == 0 {
                command = command.hint(view.hotkey().name());
            }
            commands.push(command);
        }
    }

    commands.extend([
        Command::new("Toggle sidebar", State::toggle_sidebar).hint("Ctrl+B"),
        Command::new("Toggle theme", State::toggle_theme).hint("Ctrl+T"),
        Command::new("Take payment", State::start_payment).hint("F12 or Ctrl+Enter"),
        Command::new("Clear cart", State::clear_cart),
        // Locking has to be reachable without the mouse: a cashier who walks
        // away should not have to find the right key first.
        Command::new("Lock till", |state: &mut State| {
            if state.permits(crate::auth::Permission::Lock) {
                state.lock();
            }
        })
        .hint("Ctrl+L"),
    ]);

    commands
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_palette_offers_only_the_rail_the_role_can_use() {
        let cashier: Vec<String> = all(Role::Cashier)
            .into_iter()
            .take(2)
            .map(|command| command.label)
            .collect();
        assert_eq!(cashier, vec!["Go to Sell", "Go to Today"]);

        let cashier = all(Role::Cashier);
        assert!(
            !cashier.iter().any(|command| command.label == "Go to Setup"),
            "a cashier has no Shop to set up"
        );
        assert!(
            !cashier
                .iter()
                .any(|command| command.label == "Go to Trends"),
            "trends read the shop's whole history; a cashier reads the day"
        );

        let manager = all(Role::Manager);
        let setup = manager
            .iter()
            .find(|command| command.label == "Go to Setup")
            .expect("a manager can open Setup");
        assert_eq!(setup.hint, "", "Setup is a tab, not a key");
        let catalog = manager
            .iter()
            .find(|command| command.label == "Go to Catalog")
            .expect("a manager can open the catalog");
        assert_eq!(catalog.hint, "F3", "F3 opens Shop, and Shop lands on it");
    }
}
