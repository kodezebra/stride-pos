//! The command palette's contents.
//!
//! One list, built from the same module indices the rail uses. A command is an
//! ordinary closure over [`State`], which is the whole point: anything a widget
//! can do, the palette can do, without a second code path to keep in sync.
//!
//! Module commands go through [`modules::open`] rather than `State::select`, so
//! the palette cannot walk past a permission the rail enforces.

use ui_shell::regions::Command;

use crate::modules;
use crate::state::State;

pub fn all() -> Vec<Command<State>> {
    vec![
        Command::new("Go to Sell", |state: &mut State| {
            modules::open(state, modules::SELL)
        })
        .hint("F1"),
        Command::new("Go to Dashboard", |state: &mut State| {
            modules::open(state, modules::DASHBOARD)
        })
        .hint("F2"),
        Command::new("Go to Products", |state: &mut State| {
            modules::open(state, modules::PRODUCTS)
        })
        .hint("F3"),
        Command::new("Go to Sales", |state: &mut State| {
            modules::open(state, modules::SALES)
        })
        .hint("F4"),
        Command::new("Go to Settings", |state: &mut State| {
            modules::open(state, modules::SETTINGS)
        })
        .hint("F5"),
        Command::new("Go to Inventory", |state: &mut State| {
            modules::open(state, modules::INVENTORY)
        })
        .hint("F6"),
        Command::new("Go to Suppliers", |state: &mut State| {
            modules::open(state, modules::SUPPLIERS)
        })
        .hint("F7"),
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
    ]
}
