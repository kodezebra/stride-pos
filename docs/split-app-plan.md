# Splitting the app

The app is 31k lines in nine files. This plan breaks it up without changing
behaviour: same screens, same database, same tests.

**The invariant:** tests go from 259 passing to 259 passing. If the count moves,
a move went wrong.

**Commit rules for this work**

- One move per commit. `cargo test` green after every one.
- Never change behaviour in a move. A bug found on the way is noted, fixed in
  its own commit, never "while I'm here".
- `cargo fmt` in the same commit as the move, so diffs stay readable.
- Work on a local branch; `main` moves only by merge.

---

## Step 1 — split `modules.rs` (7,445 lines)

Into a directory module. Private helpers move with their screen; the spine and
the shared widgets stay reachable from `mod.rs`, so no call site outside the
module changes.

| File | Holds | Lines |
|---|---|---|
| `modules/mod.rs` | constants, `View`, `Module`, `PageLeft`, `action_bar`, `all`/`views`/`tabs`/`open` | ~400 |
| `modules/widgets.rs` | `modal`, `amount_row`, `text_field`, `secret_field`, `prefixed_field`, table rows, action icons, `locked_card` | ~470 |
| `modules/sell.rs` | `Sell`, tiles, rows, cart, view toggle, quantity modal | ~1,000 |
| `modules/day.rs` | `Dashboard`, KPI cards, chart, tables, date helpers | ~1,050 |
| `modules/catalog.rs` | `Products`, stock, categories, export, product editor, import | ~1,300 |
| `modules/tests.rs` | the module-table, rail-order and permission tests | ~500 |
| `modules/purchases.rs` | `Purchases`, the purchase pages | ~700 |
| `modules/suppliers.rs` | `Suppliers`, supplier editor, track, stock move, history | ~550 |
| `modules/sales.rs` | `Sales`, sale detail, the drawer card | ~800 |
| `modules/settings.rs` | `Settings`, every card, receipt preview | ~1,150 |

- [x] **1.1** Baseline: 259 tests, clippy clean, on a branch
- [x] **1.2** Add this plan
- [x] **1.3** `modules.rs` → `modules/mod.rs` (pure rename, no content change)
- [x] **1.4** Extract `widgets.rs`
- [x] **1.5** Extract `settings.rs`
- [x] **1.6** Extract `suppliers.rs`
- [x] **1.7** Extract `purchases.rs`
- [x] **1.8** Extract `sales.rs`
- [x] **1.9** Extract `catalog.rs`
- [x] **1.10** Extract `day.rs`
- [x] **1.11** Extract `sell.rs`
- [x] **1.12** Move the module tests to `modules/tests.rs`
- [x] **1.13** Final verification: 259 tests, clippy, release build

## Step 2 — split `state.rs` (4,683 lines)

A pure move: `impl State` blocks go to `state/{cart,catalog,suppliers,purchasing,staff,sync,day,prefs}.rs`.
Fields do **not** move — `state.cart` keeps working, so no call site changes.

- [x] **2.0** `state.rs` → `state/mod.rs` (pure rename, no content change)
- [x] **2.1** Extract `state/cart.rs`
- [x] **2.2** Extract `state/catalog.rs`
- [x] **2.3** Extract `state/suppliers.rs`
- [x] **2.4** Extract `state/purchasing.rs`
- [x] **2.5** Extract `state/staff.rs`
- [x] **2.6** Extract `state/sync.rs`
- [x] **2.7** Extract `state/day.rs`
- [x] **2.8** Extract `state/prefs.rs`
- [x] **2.9** Move the state tests to `state/tests.rs`
- [x] **2.10** Final verification: 259 tests, clippy, release build

## Step 3 — split `db.rs` (8,773 lines)

A pure move, like step 2: `db.rs` becomes a directory and every file is
re-exported from `db/mod.rs`, so all 154 `db::foo(...)` call sites stay as they
are. The rows types move to `db/types.rs`; everything else goes beside the
queries that read it, one table group per file.

No service layer. The seam that matters is already the module boundary: each
screen talks to `db` and nothing else does. Wrapping that in structs would add a
second name for every function without removing a dependency.

- [x] **3.0** `db.rs` → `db/mod.rs` (pure rename, no content change)
- [x] **3.1** Extract `db/types.rs` (the row types and their impls)
- [x] **3.2** Extract `db/schema.rs` (`APPLICATION_ID`, `MIGRATIONS`)
- [x] **3.3** Extract `db/staff.rs` (`users`, credentials, attempt log)
- [x] **3.4** Extract `db/sync.rs` (device id, counters, peers, snapshots)
- [x] **3.5** Extract `db/catalog.rs` (products, categories, `ProductFields`)
- [x] **3.6** Extract `db/suppliers.rs`
- [x] **3.7** Extract `db/stock.rs` (movements, expiry buckets, date math)
- [x] **3.8** Extract `db/purchasing.rs` (orders, receiving)
- [x] **3.9** Extract `db/sales.rs` (a sale, shifts, drawers, reports)
- [x] **3.10** Extract `db/backup.rs`
- [x] **3.11** Move the db tests to `db/tests.rs`
- [x] **3.12** Final verification: 259 tests, clippy, release build

`db/mod.rs` ends at 53 lines: the module doc, one `mod` per file, and the
re-exports that keep the call sites honest.
