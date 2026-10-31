//! The command palette's contents.
//!
//! One list, built from the same module indices the rail uses. A command is an
//! ordinary closure over [`State`], which is the whole point: anything a widget
//! can do, the palette can do, without a second code path to keep in sync.

use ui_shell::regions::Command;

use crate::modules;
use crate::state::State;

pub fn all() -> Vec<Command<State>> {
    vec![
        Command::new("Go to Sell", |state: &mut State| {
            state.select(modules::SELL)
        })
        .hint("F1"),
        Command::new("Go to Dashboard", |state: &mut State| {
            state.select(modules::DASHBOARD)
        })
        .hint("F2"),
        Command::new("Go to Products", |state: &mut State| {
            state.select(modules::PRODUCTS)
        })
        .hint("F3"),
        Command::new("Go to Sales", |state: &mut State| {
            state.select(modules::SALES)
        })
        .hint("F4"),
        Command::new("Go to Settings", |state: &mut State| {
            state.select(modules::SETTINGS)
        })
        .hint("F5"),
        Command::new("Toggle sidebar", State::toggle_sidebar).hint("Ctrl+B"),
        Command::new("Toggle theme", State::toggle_theme).hint("Ctrl+T"),
        Command::new("Clear cart", State::clear_cart),
    ]
}
