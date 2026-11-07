# Stride POS

A till for small shops. It runs on the shop's own computer and keeps working
with no internet, no account, and no cloud — your sales stay in one file on that
machine.

## What it does

**Sign in** — the till opens on a list of the people who work here: pick your
name and type your PIN. Nobody reaches the takings without one, and every sale
is signed by the person who took the money.

**Sell** — search the product grid, tap items into the cart, change quantities,
and watch the total. Pay takes the cash, shows the change, and writes the sale:
the receipt lines, the payment, and the drawer's takings in one go.

The total is exact. Where the smallest coin is worth more than one unit of the
currency (UGX and KES round to 100, TZS to 5000), the cash due is rounded to the
nearest coin and the difference is stored on the sale, so the books add up to
what the drawer actually holds.

**Phone scanner** — when the till and an Android phone are on the same Wi-Fi,
the phone can scan a barcode straight into the till. Open **Settings → Scanner**,
turn the listener on, and scan the pairing code with the companion app. A scan
marked **Sell** puts the item in the cart; **Add** opens a new product with the
barcode already filled in. The listener is off until you turn it on, and it
never leaves the shop's network.

**Dashboard** — takings, number of sales, and the latest transactions for today,
the last 7 days, the last 30 days, or all time.

**Products** — the catalogue of what you sell: add a product, edit its price or
category, give it a barcode. Everything here becomes a key on the Sell screen.

**Suppliers** — who the shop buys from: name, contact, phone, email, address and
notes. Search by any of them, and archive a supplier you no longer use without
losing the record. Purchases and stock are not part of this screen yet.

**Inventory** — what the ledger says is on the shelf. Tracking is opt-in per
product: choose a product, count it, and set the level that counts as low. From
then on every sale of that product subtracts automatically. Receiving, customer
returns, waste and a physical count are recorded as movements, and each product
keeps its own history. Stock is always the sum of those movements; deleting a
product deletes its history, because the ledger is operational and not a
financial record. Cashiers can see stock; a manager or the owner records a
movement.

**Sales** — every completed sale in the chosen range, newest first.

**Settings** — light or dark appearance, a preview of the receipt layout, the
staff list, the Android phone scanner, and places for tax rules and terminal
sync that are not filled in yet.

## Who can do what

Three roles, and a till that asks for the person rather than a username.

| | Cashier | Manager | Owner |
|---|---|---|---|
| Sell and take payment | yes | yes | yes |
| Lock the till | yes | yes | yes |
| Void, refund, discount, change a price, open the drawer without a sale, edit settings, manage suppliers, change stock | — | yes | yes |
| Add staff, switch an account off, reset a PIN, change the currency | — | — | yes |

The owner account is the back office. An owner who also works the counter should
have a second record as a cashier or manager: that is the account the shop sees
on the receipts, and it is the one handed over at the end of a shift without
giving out the owner's password.

A cashier who needs something above their role asks for it on the spot: the till
asks a manager (or the owner) to approve that one action for a minute, and the
sale stays in the cashier's name. Nobody has to log out and back in.

Cashiers and managers sign in with a six-digit PIN; the owner uses a password of
at least eight characters. The till makes the PIN itself and shows it once when
the account is added, so write it down before leaving the Staff page — after
that only a hash is kept, and a forgotten PIN is replaced rather than recovered.
Five wrong tries lock the account for thirty seconds, doubling with each further
try up to fifteen minutes. The till locks itself after five idle minutes
(fifteen for the owner), and `Ctrl+L` locks it now.

## Keyboard

| Key | Does |
|---|---|
| `F1` … `F7` | switch between the seven sections |
| `Ctrl+Enter` | take payment for the cart |
| `Ctrl+L` | lock the till |
| `Ctrl+P` | command palette — jump anywhere by name |
| `Ctrl+B` | show or hide the sidebar |
| `Ctrl+T` | light / dark |

## Your data

Everything lives in one file on this computer:

```
~/.local/share/pos/pos.db
```

To back up the shop, close the app and copy that file. To start over with an
empty till, delete it — that takes the staff list with it, and the next launch
sets the till up from scratch. Nothing is sent anywhere.

PINs and passwords are stored in that file as hashes, never in the clear. The
first launch shows a one-time recovery code: write it down and keep it away from
the till. It is the only way back in if the owner's password is lost, and it
stops working the moment it has been used once.

## Status

This is an early build. You can sign in, sell, lock the till, keep a supplier
directory, and track stock: build a cart, take the cash, and the sale is recorded
with the name of whoever took it, on the Dashboard and in Sales.

- payments are cash only — mobile money and cards come later
- the catalogue can be typed in or imported from a CSV
  (`name, category, price, barcode`); stock counts are not imported
- receipts can be previewed but not printed
- void, refund and discount have permissions but no screen yet
- inventory tracks counts, not cost: purchase orders and valuation come later
- each till runs alone; there is no sync between terminals yet
- the phone scanner is a companion Android app, not a mobile till of its own;
  it needs the till and the phone on the same Wi-Fi

For the people running the till — setup, day-to-day selling, common scenarios,
and how to read the dashboard — see [USER_GUIDE.md](USER_GUIDE.md).
To train someone new, use [TRAINING_MANUAL.md](TRAINING_MANUAL.md): the same
material as a four-lesson course with practice scenarios and a sign-off sheet.

## Running it

Today it is started from a terminal, on a Linux desktop:

```bash
cargo run -p desktop
```

There is no installer yet. A release build is published as a Linux tarball on
the GitHub releases page, with the companion phone app beside it as
`stride-scan-*.apk`. If you are working on the code, see
[DEVELOPMENT.md](DEVELOPMENT.md).
