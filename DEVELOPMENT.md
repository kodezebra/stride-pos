# Stride POS — development

For what the app *is*, read [README.md](README.md). This file is for people
building it: how to run it, where the code and data live, and where shared code
comes from.

One of three repos:

| Repo | Holds |
|---|---|
| `stride-libs` | shared crates: `ui-kit`, `ui-shell`, `storage`, `config`, `app-boot`, `money` |
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
  checkout.rs   the payment modal and its change panel
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

The shared crates come from `stride-libs` at a pinned `rev` in
`Cargo.toml`. Do not edit that rev by hand: the procedure (and the rule that
`Cargo.lock` lands in the same commit) is in `../libs/AGENTS.md`.

## State

Sell's grid reads the `products` table, which Products writes; there is no
sample data. Dashboard and Sales read the `sales` table, which stays empty until
someone pays.

A sale writes three tables in one transaction:

| Table | Holds |
|---|---|
| `sales` | one row per sale: exact `total_minor`, signed `rounding_minor`, tender and change |
| `sale_lines` | what was sold, with the name and unit price copied at the time |
| `payments` | one row per tender; append-only, `method` is free text |

`sale_lines` copies the name and price on purpose: editing or deleting a product
must not rewrite yesterday's receipt. `product_id` is a link for reporting only,
and it goes `NULL` when the catalog row disappears.

`paid = total + rounding`, and both `totals()` and the Sales list sum
`total_minor + rounding_minor`, so revenue matches the drawer rather than the
price list. The cash step per currency is in `currencies.rs` (`cash_step`), and
`Money::round_to_step` does the arithmetic — it rounds half away from zero, so
1340 UGX is 1300, not 1400.

Settings sections are stubs.
