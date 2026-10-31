# Stride POS

A till for small shops. It runs on the shop's own computer and keeps working
with no internet, no account, and no cloud — your sales stay in one file on that
machine.

## What it does

**Sell** — search the product grid, tap items into the cart, change quantities,
and watch the total. Pay takes the cash, shows the change, and writes the sale:
the receipt lines, the payment, and the drawer's takings in one go.

The total is exact. Where the smallest coin is worth more than one unit of the
currency (UGX and KES round to 100, TZS to 5000), the cash due is rounded to the
nearest coin and the difference is stored on the sale, so the books add up to
what the drawer actually holds.

**Dashboard** — takings, number of sales, and the latest transactions for today,
the last 7 days, the last 30 days, or all time.

**Products** — the catalogue of what you sell: add a product, edit its price or
category, give it a barcode. Everything here becomes a key on the Sell screen.

**Sales** — every completed sale in the chosen range, newest first.

**Settings** — light or dark appearance, a preview of the receipt layout, and
places for tax rules and terminal sync that are not filled in yet.

## Keyboard

| Key | Does |
|---|---|
| `F1` … `F5` | switch between the five sections |
| `Ctrl+Enter` | take payment for the cart |
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

This is an early build. You can sell: build a cart, take the cash, and the sale
is recorded and shows up on the Dashboard and in Sales.

- payments are cash only — mobile money and cards come later
- the catalogue is entered by hand; importing a supplier's list comes later
- receipts can be previewed but not printed
- there is no staff sign-in yet, so the drawer has no name on it
- each till runs alone; there is no sync between terminals yet

## Running it

Today it is started from a terminal, on a Linux desktop:

```bash
cargo run -p pos
```

There is no installer yet. If you are working on the code, see
[DEVELOPMENT.md](DEVELOPMENT.md).
