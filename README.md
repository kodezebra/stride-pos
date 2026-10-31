# Stride POS

A till for small shops. It runs on the shop's own computer and keeps working
with no internet, no account, and no cloud — your sales stay in one file on that
machine.

## What it does

**Sell** — search the product grid, tap items into the cart, change quantities,
and watch the total. The Pay button is there, but taking payment is not
connected yet.

**Dashboard** — takings, number of sales, and the latest transactions for today,
the last 7 days, the last 30 days, or all time.

**Products** — the catalogue of what you sell. Still being built.

**Sales** — every completed sale in the chosen range, newest first.

**Settings** — light or dark appearance, a preview of the receipt layout, and
places for tax rules and terminal sync that are not filled in yet.

## Keyboard

| Key | Does |
|---|---|
| `F1` … `F5` | switch between the five sections |
| `Ctrl+P` | command palette — jump anywhere by name |
| `Ctrl+B` | show or hide the sidebar |
| `Ctrl+T` | light / dark |

## Your data

Everything lives in one file on this computer:

```
~/.local/share/pos/pos.db
```

To back up the shop, close the app and copy that file. To start over with an
empty till, delete it. Nothing is sent anywhere.

## Status

This is an early build. You can build up a cart on the Sell screen, but a sale
cannot be completed yet. The app says so on screen rather than pretending
otherwise:

- payments are not connected — Pay explains this
- the product grid shows sample items until a catalogue can be imported
- receipts can be previewed but not printed
- each till runs alone; there is no sync between terminals yet

## Running it

Today it is started from a terminal, on a Linux desktop:

```bash
cargo run -p pos
```

There is no installer yet. If you are working on the code, see
[DEVELOPMENT.md](DEVELOPMENT.md).
