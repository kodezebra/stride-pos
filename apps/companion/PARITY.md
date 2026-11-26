# Desktop parity map

The companion's visual language is a port of the desktop's. Keep these values aligned when
either side changes.

| Companion file | Desktop source |
|---|---|
| `src/lib/theme/tokens.css` | `stride-libs/crates/ui-kit/src/tokens.rs` (spacing, radii, TYPE), `apps/desktop/src/theme.rs` (LIGHT/DARK palettes) |
| `src/app.css` (`@font-face`) | `stride-libs/crates/ui-kit/fonts/*.ttf` (copied into `static/fonts/`) |
| `src/lib/components/StatusStrip.svelte` | `ui-shell/src/regions.rs` `status()`, `size::STATUS = 36px`, `TYPE.status = 13.5px` |
| `src/lib/components/BottomNav.svelte` | `ui-shell/src/regions.rs` `rail()` — rows ≥44px; on phone the rail becomes a bottom nav |
| `src/lib/components/CartSheet.svelte` | desktop Sell screen cart column; `TYPE.total = 32px`, `RADII.md = 10px` |

Desktop constants that must not drift:

- Spacing `4 / 8 / 12 / 16 / 24`
- Radii `6 / 10 / 14 / 999`
- Type `12 / 13.5 / 15 / 15 / 20 / 28 / 32 / 13`
- Fonts: Open Sans Regular/Semibold, JetBrains Mono Regular

Companion-specific layout decisions (not in the desktop):

- Below 600px the rail becomes a bottom nav and the category sidebar moves into a search sheet.
- Touch targets: 48px rows, 56px primary buttons, 16px page gutters.
- Input font-size is 16px to stop mobile WebViews zooming on focus.
