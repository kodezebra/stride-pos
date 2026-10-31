# Stride POS — development

For what the app *is*, read [README.md](README.md). This file is for people
building it: how to run it, where the code and data live, and where shared code
comes from.

One of three repos:

| Repo | Holds |
|---|---|
| `stride-libs` | shared crates: `ui-kit`, `ui-shell`, `storage`, `config`, `app-boot` |
| `stride-pos` (this one) | the point-of-sale desktop app |
| `stride-school` | school administration |

## Run

```bash
cargo run -p pos              # debug
cargo run --release -p pos    # optimized, single stripped binary
cargo build --offline         # must always work: every dep is cached
```

Needs a display (X11 or Wayland).

## Layout

```
apps/pos/src/
  main.rs       wiring only: open db → migrate → settings → state → shell
  db.rs         schema, migrations, and the SQL behind the screens
  state.rs      everything the UI can change, in one struct
  modules.rs    Sell / Dashboard / Products / Sales / Settings
  commands.rs   the Ctrl+P palette
  prefs.rs      active module + sidebar visibility
  settings.rs   terminal settings (appearance; tax and printer later)
  shell.rs      rail, sidebar, workspace, status bar, the modals
crates/         reserved for product-internal crates
```

## Data

| Path | Contents |
|---|---|
| `~/.local/share/pos/pos.db` | SQLite, WAL, forward-only migrations |
| `~/.local/share/pos/logs/pos.log` | log, also mirrored to stderr |
| `~/.config/pos/settings.json` | appearance / terminal settings |
| `~/.config/pos/prefs.json` | active module, sidebar visibility |

Delete `pos.db` to start from an empty till; migrations re-run on next launch.
`RUST_LOG=debug` for verbose logging.

## Shared code

The five shared crates come from `stride-libs` at a pinned `rev` in
`Cargo.toml`. Do not edit that rev by hand: the procedure (and the rule that
`Cargo.lock` lands in the same commit) is in `../libs/AGENTS.md`.

## State

Sell's grid reads the `products` table, which Products writes; there is no
sample data. Pay says a sale cannot be completed rather than faking one.
Dashboard and Sales read the `sales` table, which stays empty until the sale
flow writes to it. Settings sections are stubs.
