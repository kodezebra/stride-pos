# Stride Companion

A Tauri 2 + SvelteKit companion till. It keeps selling when the main till is off or the LAN is
down: scan barcodes, build a cart, take cash or record a mobile-money reference, put a sale on a
customer's account, deduct stock, and queue everything for reconciliation.

**Scope:** cashier-only. There is no manager role, no approvals, no product/customer editing,
no refunds and no reports on this device. Limits are set on the main till and delivered in the
catalog snapshot.

## Layout

```
src/                       SvelteKit SPA (one codebase, three breakpoints)
  routes/+page.svelte      Ready screen
  routes/sell/             Camera-first scan + cart sheet
  routes/checkout/         Cash / mobile money / credit
  routes/queue/            Pending sales + sync
  lib/api.ts               Tauri invoke adapter + browser mock
  lib/stores/              Cart and session (Svelte 5 runes)
  lib/theme/tokens.css     Ported desktop tokens
  lib/components/          Status strip, bottom nav, cart sheet
src-tauri/                 Tauri shell + Rust core (SQLite, LAN protocol)
  src/db.rs                Own application_id, migrations, local sales outbox
  src/protocol.rs          Desktop wire shapes (catalog, ping, push)
  src/commands.rs          get_status/get_catalog/get_sales/record_sale/pair/sync_now
  migrations/001_init.sql  Companion schema (catalog cache + sales + lines)
  Cargo.lock               Committed; CI builds --locked
static/fonts/              Desktop font files + OFL licences
```

## Rust core

The Tauri backend owns all persistence and network access. It uses `storage` and `lan-http`
from `stride-libs` at the same pinned rev as the desktop. Sales are stored locally first with a
UUIDv7 id and origin device, so a dead desktop or OS kill cannot lose them.

`sync_now` pushes only what the till's ingest accepts: single-cash-tender offline sales.
Mobile-money and credit sales are **held** locally with a reason, never silently dropped and
never sent to an endpoint that would reject them. Held sales are shown on the Queue screen for
confirmation on the main till.

Verify the core:

```sh
cd src-tauri
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
```

## Test in a browser first (fastest)

The frontend runs standalone. `isTauri()` is false in a browser, so `lib/api.ts` serves an
in-memory catalog with 10 products, 3 customers, 2 staff and spending caps.

```sh
cd apps/companion
bun run dev
# open http://localhost:1420
```

Use the browser's device toolbar (e.g. 390 × 844) to check the phone layout. In browser:

- `Sell` → type `9876543210001` and Enter, or press **Simulate scan**.
- Drag/expand the cart, then **Take payment**.
- `Checkout` → enter a cash tender on the keypad, or switch to mobile money and add a reference.
- `Queue` → the sale is `● Queued`; press **Sync now** → it becomes `✓ Synced`.
- Record a mobile-money or credit sale, then **Sync now** → it becomes `⚠ Held` with the reason.
- `/pair` demonstrates the pairing form (the browser mock accepts any 10-character code).

The camera area is a placeholder in the browser; real barcode decoding arrives with the Tauri
Android build.

```sh
bun run check    # svelte-check
bun run build    # static SPA into build/
```

## Test in Waydroid later

Waydroid needs a real APK — it cannot open the Vite dev server. It is useful for touch layout
and lifecycle checks, but the camera and real-device performance need a phone.

Prerequisites: Android SDK + NDK, JDK 17, `ANDROID_HOME`, `NDK_HOME`, and the four Rust targets:

```sh
rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android
bun run tauri android init
bun run tauri android build --apk
waydroid app install <path-to-the-apk-the-build-prints>
```

Until the Tauri commands exist, the APK will show the mock catalog — same screens, real WebView.

## Parity

See `PARITY.md` for the mapping from Svelte styles to the desktop's `ui-kit` tokens.
