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
    let modules = modules::all();
    let mut commands = Vec::new();

    for &index in modules::rail_order(role) {
        let module = &modules[index];
        commands.push(
            Command::new(
                format!("Go to {}", module.title()),
                move |state: &mut State| {
                    modules::open(state, index);
                },
            )
            .hint(module.hotkey().name()),
        );
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
            .take(4)
            .map(|command| command.label)
            .collect();
        assert_eq!(
            cashier,
            vec![
                "Go to Sell",
                "Go to Sales",
                "Go to Inventory",
                "Go to Dashboard"
            ]
        );

        let cashier = all(Role::Cashier);
        assert!(
            !cashier
                .iter()
                .any(|command| command.label == "Go to Settings")
        );

        let manager = all(Role::Manager);
        let settings = manager
            .iter()
            .find(|command| command.label == "Go to Settings")
            .expect("a manager can open Settings");
        assert_eq!(settings.hint, "F8");
        let purchases = manager
            .iter()
            .find(|command| command.label == "Go to Purchases")
            .expect("a manager can open Purchases");
        assert_eq!(purchases.hint, "F7");
    }
}
