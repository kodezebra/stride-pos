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
  auth.rs       roles, credentials, lockout, lock screen, approvals, staff card
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

`sales.user_id` names who took the money (`LEFT JOIN users`, so sales from before
staff existed read `NULL` and show `-`).

## Staff

`auth.rs` holds the roles, the credential rules, the lockout, the lock screen,
the approval modal and the Staff card; `db.rs` holds the tables.

| Role | Permits |
|---|---|
| Cashier | sell, take payment, lock |
| Manager | cashier, plus void, refund, discount, change a price, open the drawer without a sale, edit settings |
| Owner | manager, plus manage staff and change the currency |

`Role` derives `Ord`, so "manager or above" is `role >= Role::Manager` and the
whole matrix is one `permits()` match. Every screen goes through
`state.permits(permission)`; a module the person cannot open goes through
`state.open()`, which either permits it or raises `ask_override()`.

`Permission::approvable()` is false for `ManageUsers` and `ChangeCurrency`:
those two are never lent to a manager, and the approval modal says so instead of
offering a PIN that would not work. An approval lasts one minute (`ELEVATION`)
and does not change `state.session` — the sale stays in the cashier's name.

Credentials are Argon2id PHC strings (m=19 MiB, t=2, p=1) in `users.credential`,
never in the clear. A PIN is exactly six ASCII digits and is **generated** by the
till (`auth::generate_pin`), not typed by the owner: an invented PIN is a
birthday or 123456, and it is the one secret in a shop nobody ever changes. The
digits are shown once, on a card above the staff list, and only the hash is kept.
The owner's password is at least eight characters with no composition rules,
because length is the only thing that makes a guess expensive.
`CredentialKind` follows the role, so `add_staff` never asks which one to make.

Five wrong tries lock the account (`FREE_ATTEMPTS`): 30 seconds, doubling per
further failure, capped at 15 minutes. The state lives in `login_attempts` and is
local wall-clock policy — it must never sync, or a peer could unlock an account.
A successful sign-in and a PIN reset both clear it.

Signing in is picking a name from a list, not typing a username: staff know each
other, and a username is friction for the person at the counter.

The till locks itself after `Role::idle_timeout()` (5 minutes cashier/manager,
15 owner) and `Ctrl+L` locks it immediately. The lock screen is a full screen,
not a modal, and paints `palette.bg` itself — the root `Ui` is bare, and every
other screen gets its background from the panels around it.

The first launch creates the owner and shows a one-time recovery code (128 bits,
Crockford base32, grouped by five). Its hash lives in `meta` under
`auth::RECOVERY_KEY` and rotates when used; there is no other way back in.

Settings sections: Terminal, Receipt, Staff and Sync are real; Tax is a stub.
