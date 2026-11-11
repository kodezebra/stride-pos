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
cargo run -p desktop              # debug
cargo run --release -p desktop    # optimized, single stripped binary
cargo build --offline             # must always work: every dep is cached
```

Needs a display (X11 or Wayland).

## Layout

```
apps/desktop/src/
  main.rs       wiring only: open db → migrate → settings → state → shell
  db.rs         schema, migrations, and the SQL behind the screens
  state.rs      everything the UI can change, in one struct
  modules.rs    Sell / Dashboard / Products / Sales / Settings
  checkout.rs   the payment modal and its change panel
  auth.rs       roles, credentials, lockout, lock screen, approvals, staff card
  commands.rs   the Ctrl+P palette
  prefs.rs      active module + sidebar visibility
  scan.rs       phone-scanner service: config, token, pairing URI, lifecycle
  scan/         protocol validation and the LAN HTTP listener
  firewall.rs   is the listener's port reachable, and the privileged step that opens it
  settings.rs   terminal + scanner settings (tax and printer later)
  shell.rs      rail, sidebar, workspace, status bar, the modals
crates/         reserved for product-internal crates
```

`apps/scanner/` is the Android companion: Kotlin, CameraX for the preview, and
ML Kit for on-device barcode decoding. It has its own Gradle build and no Rust
dependency.

## Data

| Path | Contents |
|---|---|
| `~/.local/share/pos/pos.db` | SQLite, WAL, forward-only migrations |
| `~/.local/share/pos/logs/pos.log` | log, also mirrored to stderr |
| `~/.config/pos/settings.json` | appearance, receipt, scanner settings |
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

## Phone scanner

The till side lives in `apps/desktop/src/scan.rs` (config, token, pairing URI,
service lifecycle), `scan/protocol.rs` (request validation), and `scan/server.rs`
(the LAN listener). `shell.rs` owns one `Service` for the life of the process;
`modules.rs` draws the Settings card, and `state.rs` routes a delivered scan
through `receive_scan()` into the same path a USB keyboard scanner uses.

The listener binds `0.0.0.0:<port>` only while **Settings → Scanner** says it is
on. The default port is `47831`. The worker stays up across screens and scans;
starting a thread per barcode would add latency to the one action a queue cannot
afford to make slow.

The protocol is deliberately small:

```
POST /v1/scan
X-Till-Token: <10-character pairing code>
{"barcode":"6001234567890","action":"sell"}
```

`action` is `sell` (the default) or `add`. `add` opens the product editor with
the barcode prefilled and needs `Permission::ChangePrice`. `POST /v1/ping` with
the same header checks reachability. The body is capped at 512 bytes, a barcode
at 128 characters, and unknown JSON fields are rejected.

`State::receive_scan()` drops a scan while the till is locked, in first-run
currency setup, during payment or approval, or with any editor/modal open: a
phone must not put an item in a cart the cashier cannot see. The Android app
suppresses duplicate reads of the same barcode for 1.5 seconds.

Pairing is the QR code in Settings, painted from
`stridepos://pair?host=<host>&port=<port>&token=<token>`, with the address, port
and code shown underneath for manual entry. The code is ten characters from
Crockford's base 32, drawn from the OS random source on first launch and kept in
`settings.json`; the QR is fixed black-on-white so it scans in either theme.

A pairing code is compared after normalising case, dashes and spaces, so
`7qk4-m2x9ab` and `7QK4M2X9AB` are the same code; ten characters is 50 bits,
which is enough that guessing one over the shop Wi-Fi is not the weak point.

A listener can be healthy and still unreachable: the host firewall drops the
packet before the app sees it, while `127.0.0.1` keeps answering because
loopback never crosses the policed interface. `firewall.rs` therefore detects a
running firewall (`systemctl is-active firewalld`, and the `ENABLED` flag plus
the unit for ufw) and the Settings card says which one this till uses. It does
not read that firewall's rules: firewalld gates `--query-port` behind
`org.fedoraproject.FirewallD1.config.info`, so probing would raise a password
dialog just because Settings was opened. Opening the port is a privileged change
too, so it never happens on its own: the **Allow port** button runs `pkexec
firewall-cmd --add-port=<port>/tcp --permanent` (or `pkexec ufw allow
<port>/tcp`) after the OS asks for the administrator password, and the exact
command is printed when that dialog cannot appear. The probe and the privileged
step run off the frame thread because both can wait on the OS.

The card reports `Allowed` only for a port this run of the till opened, because
that is the only thing it can know without asking for a password.

`firewall.rs` is Linux-only because the till ships for Linux. Windows would need
the same card with a UAC-elevated `netsh advfirewall` rule scoped to the
executable; Windows already prompts on first listen, so the detection there is
about a prompt the operator dismissed rather than one that never appeared.

This is a LAN feature, not a cloud service. It binds to the local network,
requires the token on every request, validates the payload before it reaches the
cart, and uses cleartext HTTP because the traffic never leaves the shop. Do not
port-forward it or put the till on a network the shop does not trust. The token
is visible on the Settings screen and in the QR code, so anyone who can see that
screen can pair a phone.

## Android companion

`apps/scanner/` is a Kotlin app built with CameraX and ML Kit. The camera preview
and barcode decoding stay on the phone; ML Kit's model is bundled in the APK, so
scanning needs no network. The app scans pairing QR codes in setup mode and
product barcodes in scan mode. It posts to the till over HTTP and shows `Sell`
and `Add` actions.

Build it with JDK 17+ (the system JRE alone is not enough) and Android SDK 36:

```bash
cd apps/scanner
./gradlew :app:assembleDebug
```

The debug APK is `app/build/outputs/apk/debug/app-debug.apk`. A release build
uses the debug signing key unless all four of these are set:

| Variable | Meaning |
|---|---|
| `STRIDE_SCAN_KEYSTORE` | path to the keystore |
| `STRIDE_SCAN_STORE_PASSWORD` | keystore password |
| `STRIDE_SCAN_KEY_ALIAS` | key alias |
| `STRIDE_SCAN_KEY_PASSWORD` | key password |

`STRIDE_SCAN_VERSION` and `STRIDE_SCAN_VERSION_CODE` set the APK's version; the
release workflow derives them from the `pos-v*` tag. See
[apps/scanner/README.md](apps/scanner/README.md) for phone setup and
troubleshooting.

CI (`.github/workflows/ci.yml`) runs Rust formatting, clippy and tests, then an
Android lint and debug build. The release workflow
(`.github/workflows/release.yml`) runs on a `pos-v*` tag, refuses a tag that is
not an ancestor of `main`, builds the Linux desktop tarball and the release APK,
checks the APK signature, and attaches both plus `SHA256SUMS.txt` to the GitHub
release. `stride-libs` is public, so both workflows clone the revision
`Cargo.toml` pins without any credential. Optional signing
secrets are `STRIDE_SCAN_KEYSTORE_BASE64`, `STRIDE_SCAN_STORE_PASSWORD`,
`STRIDE_SCAN_KEY_ALIAS`, and `STRIDE_SCAN_KEY_PASSWORD`; without them the APK is
signed with the debug key and is still installable by sideloading.

## Release compatibility

A Linux binary keeps the glibc of the machine that links it. Build the `.deb`,
`.rpm` and tarball on a newer distro than the shop runs and the loader stops at
start-up with `version 'GLIBC_2.39' not found`, because it resolves symbols
against the shop's `libc.so.6`. Windows has the same failure with the MSVC C
runtime: without the VC++ Redistributable the exe wants `vcruntime140.dll`,
which is not part of Windows.

So the rule is: **the oldest supported distro builds the artifact,** and the
gate refuses anything that would not load there.

| What | How |
|---|---|
| Linux packages | Built in `ubuntu:22.04` (glibc 2.35), which covers Debian 12+ and Ubuntu 22.04+. Never the host, never a `-latest` runner. |
| Local build | `scripts/docker-build-linux-release.sh 0.1.0` — same image, works from any host distro. |
| Gate | `scripts/check-glibc-baseline.sh 2.35 <binary>` fails if any dependency starts requiring a newer glibc. Runs in `ci.yml` and `release.yml`. |
| Declared floor | The `.deb` carries `Depends: libc6 (>= 2.35)`, the `.rpm` `Requires: glibc >= 2.35`. |
| Windows | `RUSTFLAGS=-C target-feature=+crt-static` for `x86_64-pc-windows-msvc`; the release job checks with `dumpbin /dependents` that no VC++ runtime is imported. |
| Rust | Nothing is pinned. CI installs the current stable inside the build image, and the gate below fails the release if a compiler upgrade raises the floor. No `rust-toolchain.toml`: rustup matches that file by name, so it would make every developer download whatever version it names. |

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

Settings sections: Terminal, Receipt, Scanner, Staff and Sync are real; Tax is a
stub.
