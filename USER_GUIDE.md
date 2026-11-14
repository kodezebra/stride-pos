# Stride POS — User Guide

This guide is for the people who run the till: the owner, the manager, and the
cashiers. It covers day-to-day use — ringing up sales, taking cash, checking the
day's takings — and the setup jobs the owner does once.

If you are installing or building from source, see [README.md](README.md) and
[DEVELOPMENT.md](DEVELOPMENT.md) instead.

If you are training a new cashier, use [TRAINING_MANUAL.md](TRAINING_MANUAL.md).
It teaches this material in four lessons, with practice data, scenarios, and a
competency checklist.

Everything in this guide reflects the current build. Where a feature is planned
but not present, it says so rather than guessing.

---

## 1. What this till is

Stride POS is a cash till for a small shop. It runs on the shop's own computer
and keeps working with no internet, no account and no cloud. Your sales live in
one file on that machine.

**What it does today**

- Signs in the person taking the money (owner, manager, cashier)
- Sells from a product grid and records the sale, the lines, the payment
- Rounds cash to the smallest circulating coin (UGX/KES to 100, TZS to 5000) and
  stores the difference so the books match the drawer
- Tracks stock per product (opt-in), with an opening count, automatic subtraction
  on sale, and a full movement history
- Keeps a supplier directory (add, edit, archive)
- Plans a restocking trip, receives what actually arrived, and records the unit
  cost of each line
- Shows a dashboard: takings, sales count, busiest hour, top items, per-person
  share, and profit where costs are known
- Exports products and sales to CSV; imports products from CSV
- Pairs an Android phone on the same Wi-Fi as a barcode scanner
- Backs up the database to a folder of the 10 newest copies

**What it does not do yet**

- Payments are **cash only** — no mobile money, no cards
- Receipts can be previewed and copied as text, but not printed
- Void, refund and discount have roles defined but **no screens**
- Inventory counts stock, not cost — there is no full stock valuation yet
- Profit appears only for sale lines that had a known cost; it is not a complete
  set of accounts
- Purchases are local to this till and do not sync between terminals
- Tax and Sync settings are placeholders
- One till, one file — no multi-terminal sync

---

## 2. The screen at a glance

```
┌─────┬──────────────────┬─────────────────────────────────┐
│ rail│ sidebar          │ workspace                       │
│ F1–8│ (module-specific)│ (the module you opened)         │
├─────┴──────────────────┴─────────────────────────────────┤
│ status bar: local only · shortcuts · version · name/role │
└──────────────────────────────────────────────────────────┘
```

**The rail (left edge) — the eight sections:**

| Key | Section | What it is |
|---|---|---|
| F1 | Sell | the till screen: search/scan, cart, payment |
| F2 | Sales | every completed sale, newest first |
| F3 | Inventory | what is on the shelf, per tracked product |
| F4 | Dashboard | takings and best sellers for today / 7 / 30 days / all time |
| F5 | Products | the catalogue: add, edit, import, export |
| F6 | Suppliers | who the shop buys from |
| F7 | Restock | plan a restocking trip, then receive what arrived |
| F8 | Settings | appearance, receipt preview, scanner, staff, backup |

**The status bar (bottom)** shows: "local only", shortcut hints, the app version,
your name and role (or "no cashier" if nobody is signed in), and the current cart
("3 item(s) - 12,000"). There is also a sun/moon button for light/dark.

**Sidebar shortcut** — `Ctrl+B` shows or hides the sidebar. `Ctrl+P` opens the
command palette (type a few letters of any destination: "dash", "sales", "lock"…).

### Keyboard shortcuts

| Key | Does |
|---|---|
| `F1` … `F8` | switch between the eight sections |
| `Ctrl+Enter` or `F12` | take payment for the cart |
| `Ctrl+L` | lock the till now |
| `Ctrl+P` | command palette |
| `Ctrl+B` | show / hide the sidebar |
| `Ctrl+T` | light / dark |

Inside forms and modals, `Enter` saves and `Escape` cancels. In the payment
modal, `Enter` completes the sale and `Escape` returns to the cart. After a sale,
`Enter` or `Space` starts the next sale.

---

## 3. Who can do what

Three roles. The till asks for the person, not a username.

| | Cashier | Manager | Owner |
|---|:---:|:---:|:---:|
| Sell and take payment | ✓ | ✓ | ✓ |
| Lock the till | ✓ | ✓ | ✓ |
| See stock levels | ✓ | ✓ | ✓ |
| See **own** sales and dashboard | ✓ | ✓ | ✓ |
| See the **whole shop** sales and dashboard | — | ✓ | ✓ |
| Void, refund, discount, change a price | — | ✓* | ✓* |
| Open the drawer without a sale | — | ✓* | ✓* |
| Edit settings, manage suppliers, adjust stock, plan and receive purchases | — | ✓ | ✓ |
| Import/export CSV, back up the database | — | ✓ | ✓ |
| Change the catalogue (add/edit/delete products) | — | ✓ | ✓ |
| Add staff, switch accounts off, reset PINs, change currency | — | — | ✓ |

\* These permissions exist for the role matrix, but there are no screens for
them yet.

**Cashiers sign in with a six-digit PIN; the owner uses a password of at least
eight characters.** The till *makes* the PIN itself and shows it once when the
account is created — write it down before leaving the Staff page. After that only
a hash is kept; a forgotten PIN is replaced, never recovered.

**The one-minute approval.** When a cashier hits something above their role, the
till raises "A manager is needed" instead of a hard "no". A manager (or the
owner) picks their name, enters their PIN or password, and the till allows that
one action for one minute. The sale stays in the cashier's name. Two permissions
can never be lent out this way: **managing staff** and **changing the currency** —
those always need the owner.

**Lockout.** Five wrong tries lock the account: 30 seconds, doubling with each
further failure, capped at fifteen minutes. Signing in and resetting a PIN clear
the count.

**Auto-lock.** The till locks itself after five idle minutes (fifteen for the
owner). Any input resets the timer. `Ctrl+L` locks it now. The cart survives a
lock — it is in memory, not written until you complete a sale.

---

## 4. First day: setting up a new till, end to end

Do this once, in order. Budget about twenty minutes.

1. **Launch the app** (`cargo run -p desktop`, or the installed release). The very
   first screen is the lock screen in "Set up this till" mode — not the normal
   sign-in list.
2. **Create the owner account.** Enter the owner's name (e.g. "Amina"), a
   password of at least eight characters, and the same password again. Press
   **Create the owner**. Length is what makes passwords hard to guess —
   12+ characters beats any symbol rules.
3. **Write down the recovery code.** The till shows a one-time 26-character code
   (`XXXXX-XXXXX-…`). This is the *only* way back in if the owner forgets the
   password, and it works once, then is replaced automatically. Keep it away from
   the till itself. Press **I have written it down**.
4. **Choose the currency.** A picker lists the East African Community countries
   first, then neighbours. Pick the one the shop trades in. The caption says how
   cash behaves: UGX/KES round to the nearest 100, TZS to the nearest 5000. Until
   a currency is chosen, nothing is priced. **The currency locks after the first
   sale — past totals are never re-priced.**
5. **Set the shop name** (Settings → Terminal → "Prints at the top of every
   receipt"). Type it and press **Save**. This name appears on receipts and in
   the Receipt preview.
6. **Add staff** (Settings → Staff): one **Cashier** record for the counter (the
   person who actually works it). The owner account stays as the back office.
   For each person added, the till generates a six-digit PIN and shows it once —
   write it down before navigating away.
7. **Add products.** Either one at a time (Products → **New product**), or import
   the shop's spreadsheet: Products → **More → Save a template**, fill in the
   template, then **Import CSV**. See §6.10.
8. **Check the receipt layout** (Settings → Receipt): the preview shows your shop
   name and sample lines. Any change to the shop name appears here immediately.
9. **Back up.** Settings → Terminal → **Back up now**. The copy lands in
   `~/.local/share/pos/backups`; the newest 10 are kept.
10. **(Optional) Pair the Android phone** as a barcode scanner. Settings →
    Scanner → **Turn on**, then scan the QR with the Stride Scan app. See §6.15.

The owner is signed in from step 2. A cashier signs in from then on using the
account created in step 6.

---

## 5. The end-to-end flows

### 5.1 A typical sale, step by step

This is the whole counter flow, from idle till to done.

1. Cashier signs in: picks their name, types the PIN, press **Sign in**.
2. Sell screen (F1) opens with the product grid, search field focused.
3. Items go into the cart three ways, mixed freely:
   - scan a barcode (phone or USB scanner) — exact barcode wins over search;
   - type part of a name or barcode in **Scan or search** — the grid narrows as
     you type;
   - tap a tile in the grid.
4. Same product twice merges into one line at the next quantity. Use the **− / +**
   stepper on a line to change quantity; going below 1 removes the line.
5. The right-hand pane shows **Subtotal**, **Tax** (literal "not configured"),
   and **Total**. If rounding will apply, it says "Rounds to *N* in cash".
6. Press **Pay** (or `Ctrl+Enter` / `F12`). The payment modal opens with the
   amounts: subtotal, the rounding line only if it moved the figure, and the
   **To pay** amount in big type.
7. Enter the cash received (empty means exact), or tap a quick-tender chip — the
   first chip is always **Exact**; the others round the bill up to the biggest
   circulating notes. The screen shows **Short by …** in red or **Change …** in
   green as you type.
8. Press **Complete sale** (or `Enter`). The till:
   - records the sale (`#123`), the lines (name and price snapshotted), and the
     cash payment, in one transaction;
   - subtracts tracked stock for the items sold;
   - clears the cart and puts the cursor back in **Scan or search** for the next
     customer. There is no confirmation panel to dismiss: the change you read in
     step 7 is what you hand over, and the receipt is kept on the sale.
9. Open `F2` **Sales** → the sale whenever you need the slip again — lines,
   amount paid, method, change. **Copy receipt** puts it on the clipboard as
   text.
10. Questions mid-sale? The cart stays put — even if the till auto-locks after
    five idle minutes, nothing is lost.

**Worked example.** A customer buys:

- Africa Tea Leaves 100 g — 3,000
- Royco Mchuzi Mix 70 g — 2,200
- Unga Mill Maize Flour 1 kg — 4,500

Subtotal = **9,700**. UGX rounds to the nearest 100, and 9,700 is already a
hundred, so the customer pays 9,700 and the receipt shows no rounding line.
If the subtotal were **9,660**, the customer would pay **9,700** in cash; the
receipt would show *Subtotal 9,660*, *Rounded up 40*, *Total 9,700*. The books
always match the drawer, not the price list — see §7.3.

### 5.2 A day at the counter, open to close

**Morning**

1. Sign in, check F4 **Dashboard** with range **Today** for yesterday's context,
   confirm stock warnings under F3 **Inventory** (the **Low** and **Out**
   filters), and make sure the backup ran (Settings → Terminal → the backup
   folder holds the last 10).
2. Unlock and open. The status bar shows "local only" — expected.

**During the day** — ring sales (§5.1). If something needs a manager, ask for the
one-minute approval rather than handing over PINs.

**Close of day (manager or owner)**

1. F4 **Dashboard**, range **Last 7 days** if you want the week in view.
2. Read the KPI cards and the **By person** table (who took what).
3. F2 **Sales** → **Export CSV** to a folder you keep (see §8.10).
4. Settings → Terminal → **Back up now**.
5. `Ctrl+L` to lock.

### 5.3 A new cashier, end to end

1. Owner: Settings → Staff → **Add a person**: name, role **Cashier**, **Add**.
2. The till shows the six-digit PIN. Owner writes it down and gives it to the
   cashier directly (never over chat).
3. Cashier signs in on the till: name → PIN → **Sign in**.
4. Cashier can sell, take payment, lock. Everything else raises "A manager is
   needed".
5. Cashier leaves the shop: owner presses **Switch off** on that account. Their
   past sales keep their name. Signing the *current* account off locks the till
   immediately.

### 5.4 A restocking trip, end to end

The shop does not wait for a supplier to deliver. The owner goes to town with a
list, buys what is available, and counts it in when they get back. **Restock**
(F7) records both halves of that trip, on one page.

**Before leaving**

1. Press F7 **Restock** → **New list**.
2. Name the supplier if there is one; it is optional. Add a note such as
   "Saturday market".
3. Press **Add low stock** to fill the list with everything at or under its
   warning line. The quantity suggested aims at the product's *Keep in stock*
   level, after counting what is already on the shelf and already on its way.
4. Adjust any quantity, or search for another product and add it. A product can
   be added even if it is not low — the list is a plan, not a rule.
5. Leave the cost empty if you do not know it yet. **Save** reads the list back
   as **Planned**; `← Restock` goes back without saving.

**At the market**

Print the list or copy it down; the plan is already saved on the till and needs
no network. Buy what is there. If something is unavailable, you will count it at
zero when you receive.

**When the goods come back**

1. F7 **Restock** → open the planned list.
2. Each row starts at the planned quantity. Change it to what actually arrived.
   A line you did not find is set to **0**: it stays on the record as not bought.
   Enter the unit cost if you know it — this becomes the product's last known
   cost, and profit on future sales uses it.
3. If you bought something not on the list, add it from the search at the bottom
   of the page with its quantity and cost.
4. Press **Received**. Each line writes one stock movement, and the shelf count
   goes up.

Receiving is one act. A list reads **Planned** until the trip comes home, then
**Received**; there is no half-received state to come back to, so the same goods
cannot be counted in twice. A trip that came home short says so — "3 of 5 bought".
The shortfall is not turned into a new list for you: plan the next trip from
**Add low stock**. A planned list can still be edited or deleted; a received one
opens read-only, because a stock movement points at its lines.

**If a product is not tracked yet:** track it first (F3 **Inventory** → **Track
stock**), because a purchase line needs a shelf count to move. Counting it at the
same time as the trip is fine — enter what is on the shelf now, then receive the
new goods.

### 5.5 Money doesn't add up at close, end to end

1. F4 **Dashboard** (shop view) → **Payments taken** card lists takings by method.
2. Compare with the physical cash in the drawer.
3. F2 **Sales** → range **Today** → compare line count and per-sale totals.
   Click any row to open the receipt and see exactly what the customer paid and
   received as change.
4. If a cashier's numbers are consistently off, the **By person** table shows
   their share of takings.
5. If the drawer is short but the sales list is complete, that is a cash-handling
   issue, not a till issue — the till records what was tendered and what change
   was given; it does not count the drawer.
6. Unusual patterns (many small sales, one big one) are visible in **Top items**
   and **Busiest hour**.
7. Export the day's sales (F2 → **Export CSV**) and keep the file with the
   paper records.

---

## 6. Common scenarios

### 6.1 "No product matches …" when scanning

- The barcode may not be in the catalogue. F5 **Products** → search the code. If
  absent, **New product** with the scanner focused in the **Barcode** field — or
  scan from the phone with the **Add** action, which pre-fills the barcode (needs
  a manager or the owner for permission).
- Partial scan? Increase the search text or clear it and rescan.
- Duplicate barcode on save → another product already uses it; fix in Products.

### 6.2 The cart is wrong

- Same product twice merges automatically — check the line's quantity, not line
  count.
- Use **− / +** to adjust. Below 1 removes the line. There is no undo: re-add
  the right product.
- Closing the payment modal (Escape) does not lose the cart.
- Locking the till (Ctrl+L) does not lose the cart either.
- `Ctrl+P` → **Clear cart** empties the cart from anywhere.

### 6.3 The customer pays with a note larger than the bill

- Tap the quick-tender chip for that note. The change shows live. If the entered
  amount is still short of the bill, tapping a chip **adds** to it
  (500 + 1,000 = 1,500); once it covers the bill, the next tap replaces it.
  The **Exact** chip always gives you the exact amount.
- Or type the received amount and read the green **Change** figure.

### 6.4 Cash is rounded — why?

Some coins no longer circulate. UGX and KES round to the nearest 100, TZS to the
nearest 5000 minor units. The till rounds the *cash* due only; the sale records
the exact price-list total plus the rounding difference separately. Half goes
away from zero (1,350 → 1,400; 1,340 → 1,300). A cashier cannot override
this — it is how the drawer physically balances.

### 6.5 A cashier forgot their PIN

- Owner: Settings → Staff → find the person → **Reset PIN** → confirm. The new
  PIN shows once. Any previous lockout is cleared.
- The old PIN is unusable immediately.

### 6.6 An employee left

- Settings → Staff → **Switch off**. They disappear from the sign-in list but
  stay on past receipts. Reversible with **Switch on**.

### 6.7 The owner forgot the password

- At the lock screen press **Forgot the owner password?**
- Enter the **recovery code** you wrote down at setup, a new password, and press
  **Recover**.
- A new recovery code is shown once — write it down. The old code is dead.
- If the recovery code is also lost, there is no back door: the data file can be
  copied but the credentials cannot be reset without it. Plan accordingly.

### 6.8 Five wrong tries and the account is locked

- Wait. The wait starts at 30 seconds and doubles per further failure, capped at
  15 minutes. The screen shows the countdown.
- Owner can clear it faster by resetting that person's PIN (cashier/manager) —
  or, for the owner's own account, by using the recovery flow.

### 6.9 Adding or editing a product

- F5 **Products** → **New product**. Name required; price required; category
  defaults to **General** if blank; barcode optional.
- **Pick…** beside Category lists known categories plus the starter tree.
- Edit via the pencil icon on the row; delete via the trash icon — deletion is
  permanent and takes the product's stock history with it.
- Prices are stored exactly as entered; cash rounding happens only at payment.
- Editing a product never changes old receipts — sale lines copy the name and
  price at the time of sale.

### 6.10 Importing a product list

1. F5 **Products** → **More** → **Save a template** → pick a folder. The file
   `products-template.csv` is written with two example rows.
2. Fill the template in your spreadsheet (name, category, price, barcode
   optional; headings honoured case-insensitively). Prices are read in this
   till's currency.
3. F5 → **More** → **Import CSV** → pick the file. Rows that cannot be read
   are skipped and counted, never half-imported. Duplicate barcodes are skipped.
4. "Imported N product(s), skipped M." appears as a notice. The first skipped
   row's reason is shown.
5. Excel tip: a barcode starting with 0 loses its leading zero unless the column
   is formatted as text. The till does not pad them.
6. Files exported earlier can be imported straight back — the export uses the
   same columns as the import.

### 6.11 Exporting sales or products

- F5 **Products** → **More** → **Export CSV** (visible rows only) or **Export
  all** (whole catalogue).
- F2 **Sales** → **Export CSV** (same range and same scope as the screen — a
  cashier exports only their own takings).
- Files are named `products-…-YYYYMMDD-HHMMSS.csv` / `sales-…-YYYYMMDD-HHMMSS.csv`
  and land in the folder you pick. Location buttons: Home, Documents, Downloads,
  Desktop, Till exports (next to the database).
- Amounts are plain decimals in the till's currency — `1200` UGX, `12.50` KES —
  with no symbol or thousand separators, so spreadsheets read them cleanly.
- Sales export columns: `sale_id, when, cashier, total, rounding, paid, method`.
- **Copy path** copies the file location to the clipboard.

### 6.12 Backing up and restoring

- Back up: Settings → Terminal → **Back up now**. Confirm. The file is
  `pos-<stamp>.db` in `~/.local/share/pos/backups`; the newest 10 are kept.
- Or, at rest: quit the app and copy `~/.local/share/pos/pos.db`.
- Restore: quit, replace `pos.db` with the backup, relaunch.
- Deleting `pos.db` starts the till from scratch — including the staff list.

### 6.13 Stock tracking, adjustments and history

- Stock is the **sum of movements** — it is not edited directly.
- A sale subtracts its quantities automatically.
- Receiving adds; customer returns add; waste removes.
- **Adjustment** is for "I counted the shelf and it says 37, the till says 41":
  enter what you counted, the till records the difference. Zero difference keeps
  the form open and tells you they match.
- Every movement lands in the product's **History**: kind, date, who, note,
  signed delta. Newest 200 shown.
- Low-stock warning level is set once, when the product is first tracked (the
  "Warn at or below" field). Adjusting stock or viewing history does not change it.
- Sales never fail because stock ran out — the count just goes negative, which
  is your reminder to receive stock.
- Receiving a purchase is the usual way stock comes back in. It writes one
  movement per line, so a trip appears in the product's history exactly once.
- **Keep in stock** is the level a restock aims for. Leave it empty to use twice
  the warning line; the suggested quantity subtracts both what is on the shelf
  and what is already planned on a purchase.

### 6.14 Changing the shop name / currency / theme

- **Shop name**: Settings → Terminal. Type, **Save**. Shown on receipts and the
  preview.
- **Currency**: Settings → Terminal → **Change currency** — only while the till
  has *no sales yet*, and owner only. After the first sale the button is a
  locked caption: past totals are never re-priced.
- **Theme**: Settings → Terminal → Light / Dark, or `Ctrl+T` anywhere. Light is
  the default (a till sits under shop lighting).

### 6.15 Pairing the Android phone scanner

1. Till: Settings → Scanner → **Turn on**. Status goes green: "Listening on port
   47831."
2. The phone and the till must be on the **same Wi-Fi**. Do not port-forward the
   listener or put the till on an untrusted network.
3. On the phone, open the **Stride Scan** app; point its camera at the QR code in
   Settings → Scanner. Or type the **Address**, **Port** and **Code** shown
   underneath (ten characters, e.g. `7QK4M-2X9AB`; the dash is optional and
   lower case is accepted).
4. A scan arrives in Sell exactly like a USB scanner. In the phone app you can
   choose **Sell** (add to cart) or **Add** (open the new-product form with the
   barcode filled in — needs manager/owner).
5. The listener is off until you turn it on, and scans are ignored while the till
   is locked or any dialog is open.
6. The code is shown on that Settings screen — anyone who can see it can pair a
   phone. Treat the screen accordingly.

### 6.16 Manager approving something on the spot

- Cashier does the action (e.g. opens F2 for the shop view, presses something
  gated). Till shows **"A manager is needed"** (or "The owner is needed" for
  staff/currency).
- Manager or owner picks their name, types PIN/password, presses **Approve**.
- The action is allowed for **one minute**, and the cashier's name stays on the
  sale.
- Failed approval shows why (wrong PIN, or "Too many tries…").

### 6.17 What a manager cannot approve for a cashier

**Manage users** (add staff, PIN resets, switching accounts) and **change the
currency**. Those two always need the owner — the approval dialog says so and
offers no credential field for them.

### 6.18 Closing the till at night

- `Ctrl+L`. The cart survives; whoever opens next signs in cleanly.
- The app itself can simply be quit. Nothing unsaved is at risk except the cart —
  and it is held in memory, so finish or abandon it knowingly.

---

## 7. Understanding the basics

### 7.1 Signing in is picking a name

Staff know each other. The sign-in screen lists the active accounts; you pick who
you are and type your secret. There is no username to forget and no typing for
the counter. PINs are generated by the till, never invented, and stored only as
Argon2id hashes. Five wrong tries → doubling lockout, capped at 15 minutes.

### 7.2 Roles and permissions in one line each

- **Cashier** — sells, takes cash, locks, sees own numbers and stock.
- **Manager** — everything a cashier does, plus settings, suppliers, stock
  movements, catalogue changes, CSV, backups, and the whole shop's numbers.
- **Owner** — the back office: staff and currency on top of the manager's set.

Every nav path (rail, palette, Ctrl+P) goes through the same permission gate, so
"can't" really means can't — a hidden screen cannot be reached by a shortcut
either.

### 7.3 How a sale is recorded — and why revenue ≠ the price list

Each completed sale writes three tables in one transaction:

- `sales` — one row: exact `total_minor`, signed `rounding_minor`, tender, change.
- `sale_lines` — what sold, with **name and price copied at the time** (so later
  edits never rewrite history). `product_id` is a link for reports only, and
  goes NULL if the product is deleted.
- `payments` — one row per tender (always `cash` today).

Revenue everywhere in the app means **paid = total + rounding** — the money that
actually entered the drawer — not the sum of price-list amounts. That is why
today's Dashboard can differ from "what the shelf should have earned".

### 7.4 Stock is a ledger

Tracked products have a movement ledger (+opening, −sale, +received, +return,
−waste, ±adjustment). On-hand = sum of the ledger. Nothing else writes to stock,
so "What the ledger says is on the shelf" is always derivable. A purchase plan is
not stock; only receiving its lines writes movements. Deleting a product deletes
its ledger — stock history is operational, not financial.

### 7.5 Your data lives in one file

`~/.local/share/pos/pos.db` — SQLite, WAL mode, forward-only migrations.
`~/.config/pos/settings.json` holds appearance, receipt and scanner settings;
`prefs.json` holds the last module and sidebar state. Backups (10 newest) in
`~/.local/share/pos/backups`. Logs in `~/.local/share/pos/logs/pos.log`.

Nothing is sent anywhere. There is no account, no telemetry, no cloud.

---

## 8. Analytics: reading the Dashboard and Sales

This chapter is the answer to "how do I know how the shop is doing?".

### 8.1 The two screens that report

- **F4 Dashboard** — the picture: KPIs, the payments and profit cards, the chart,
  the tables.
- **F2 Sales** — the evidence: the transaction list behind the picture, CSV
  export, and a tappable receipt behind every row.

Both share the same sidebar range selector and the same scope rule: **managers
and the owner see the whole shop; a cashier sees only their own sales** — unless
a manager approved the shop view for one minute.

### 8.2 The range selector

| Range | Compared against |
|---|---|
| Today | yesterday |
| Last 7 days | the previous 7 days |
| Last 30 days | the previous 30 days |
| All time | nothing — caption "All recorded sales" |

The selection is shared between Dashboard and Sales; switching modules keeps it.

### 8.3 The KPI cards, one by one

**Revenue** — sum of `total + rounding` over the range. Shows a trend vs the
comparison window: `+12%`-style. `new` if the previous window was zero; `—` if
both windows were empty or the range is All time. 0% shows as `—`.

**Sales** — count of sales. Same trend format, in units.

**Items sold** — sum of line quantities. Same trend format, in units.

**Busiest hour** — the hour with the most sales (`HH:00`), tie-broken by revenue.
Sub-line: "N sale(s)". `—` / "no sales" when the range is empty.

Use Revenue vs Sales together: if revenue rises but sales don't, tickets are
bigger (upsells or price change). If sales rise but revenue falls, the basket
shrank (discounting, wrong items, freebies).

### 8.4 The Payments card

Lists takings by method — today always **Cash · N sale(s)** — with a **Total**
row. Amounts are after rounding (what landed in the drawer), never the tendered
note value or the price list. There is no float/drawer cash-up figure; that is a
physical count of the drawer, not a database number. Empty state: "No money taken
yet in this range."

### 8.5 The Profit card

Below Payments. It counts only the sale lines whose product had a cost at the
time of the sale — a cost comes from receiving a purchase with a unit price.
Revenue with a known cost, the cost of those items, and the profit between them
are shown, plus the margin as a percentage.

The caption says how much of the range it speaks for: "Every sale line in this
range had a cost" or "12 of 40 sale lines had a cost; the rest are not counted."
A sale made before the product ever had a cost is left out of both sides rather
than counted as pure profit. Empty state: "No costs recorded yet."

### 8.6 The "Sales by day" chart

- Bars for each day, oldest on the left, violet bars (deliberately not the accent
  blue so money reads differently from everything else).
- Today/7-day/30-day ranges are **calendar-filled** — days with no sales still
  show as zero bars, so a quiet Sunday looks empty, not missing.
- **All time** shows only selling days — the caption reads "The latest 30 selling
  days".
- Axis grid rounded to 1/2/2.5/5×10ⁿ steps, four steps, with the month span
  (e.g. "26 Oct – 1 Nov") top-right. ≤ 8 days show weekday names; more show
  day-of-month numbers.
- Hover a bar: slot wash + callout with weekday, day and month, sale count and
  amount.

Read the shape, not the pixels: a rising sawtooth with a weekly peak suggests
weekly pay-day buying; a flat line with one spike suggests an event or a data
entry error.

### 8.7 "By person" (shop view only)

Sales count and takings per cashier, revenue descending. This is the fairness and
coaching table — a second till without this report is a worse till. Cashiers
never see it (the section is simply absent).

### 8.8 "Top items"

Five best sellers ranked by **revenue**, grouped by line name — so an item that
was renamed or deleted still groups correctly under its sale-time name. Columns:
Item / Sold (qty) / Revenue. This is your reorder list, your shelf-layout list,
and your "why is this always out of stock" list.

### 8.9 "Latest sales" — and the sale detail

The five most recent sales in the range, tappable. The detail is the receipt:
shop name, sale number, time, cashier, lines with qty × unit, rounding lines if
any, total, amount paid, method, and change. **Copy receipt** puts it on the
clipboard as text — useful for WhatsApp confirmations or dispute handling. F2
lists the same rows for the whole range (max 100).

### 8.10 Sales export for spreadsheets

Columns: `sale_id, when, cashier, total, rounding, paid, method`. Amounts are
plain decimals, no symbols. `paid = total + rounding` (the drawer figure); the
`total`/`rounding` columns let you reconstruct either. Scope follows the screen:
a cashier exports only their own sales.

### 8.11 What the figures do NOT tell you

- They do not count the drawer. A short drawer with complete sales is a cash
  problem, not a till problem.
- They have partial cost data: profit counts only sale lines whose product had a
  known cost at the time. It is not a full set of accounts.
- They do not track voids/refunds/discounts — those flows don't exist yet.
- "All time" trend deltas are meaningless by definition; read the absolute.
- Busiest hour is by *count* of sales, not basket size.

### 8.12 A weekly rhythm for the owner

1. Monday: F4, **Last 7 days**, shop view. Note Revenue trend, Sales trend.
2. Check **Busiest hour** — is someone rostered?
3. Check **Top items** vs F3 **Inventory** low-stock list — reorder what sells.
4. Check **By person** — who needs coaching, who ran out of change.
5. F2 → **Export CSV**, file it with the week's paper.
6. Settings → Terminal → **Back up now** if the till has not backed up today.
7. Saturday: same, plus a physical drawer count against **Payments taken**.

---

## 9. Reference

### 9.1 Keyboard

See §2 table. In dialogs: `Enter` = default action, `Escape` = close. Payment
modal: `Enter` completes, `Escape` back to cart, after sale `Enter`/`Space` = new
sale.

### 9.2 Command palette (Ctrl+P)

Go to Sell · Go to Dashboard · Go to Products · Go to Sales · Go to Settings ·
Go to Suppliers · Go to Inventory · Go to Restock · Toggle sidebar · Toggle
theme · Take payment · Clear cart · Lock till.

### 9.3 Files and folders

| Path | Holds |
|---|---|
| `~/.local/share/pos/pos.db` | everything: products, sales, staff hashes, ledger |
| `~/.local/share/pos/backups/` | last 10 `pos-<stamp>.db` copies |
| `~/.local/share/pos/logs/pos.log` | log, mirrored to stderr |
| `~/.config/pos/settings.json` | theme, receipt, scanner state, shop name |
| `~/.config/pos/prefs.json` | last module, sidebar |
| `~/.local/share/pos/exports/` | default target for "Till exports" |

### 9.4 CSV formats

**Import products** — headings (case-insensitive, optional): `name`, `product_name`,
`category`, `category_name`, `price`, `retail_price`, `retail_price_ugx`, `barcode`.
Plain column order `name,category,price,barcode` works with no headings. Prices
in the till's currency. Blank/invalid rows skipped, never half-imported.

**Export products** — `name,category,price,barcode`, same order.

**Export sales** — `sale_id,when,cashier,total,rounding,paid,method`.

### 9.5 The scanner protocol (for the technically curious)

LAN-only HTTP, off by default, token required on every request
(`X-Till-Token: <10-character pairing code>`), 512-byte body cap, barcode ≤ 128 printable
chars, unknown JSON fields rejected. `POST /v1/scan` with
`{"barcode":"…","action":"sell"|"add"}`; `POST /v1/ping` for reachability. No
internet, ever — do not port-forward it.

---

## 10. Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| "No products yet" on the grid | empty catalogue | Products → New product, or import CSV |
| Scan does nothing | scanner (phone) off, locked till, or dialog open | Settings → Scanner → Turn on; close dialogs; unlock |
| "A product already uses that barcode" | duplicate barcode | F5 → search that barcode → edit the right product |
| "A manager is needed" (or owner) | cashier action too high | Ask for the one-minute approval |
| Sign-in rejects correct PIN | lockout active | wait for countdown; owner can reset PIN |
| Recovery code not accepted | mistyped or already used | codes are single-use; a used one is replaced — use the latest |
| Owner can't change currency | till has sales | by design — totals can never be re-priced |
| Dashboard shows only my sales | signed in as cashier | manager approval or sign in as manager/owner |
| Barcode imports lose leading zeros | Excel auto-format | pre-format the column as text in the spreadsheet |
| Export greyed / nothing to export | Sales range empty, or nothing visible | widen the range; check search/category filter |
| Receipt shows app name instead of shop | shop name not saved | Settings → Terminal → type → Save |
| Backup failed | disk full, permissions, db in use | check space; enter backup after closing other instances |
| Scanner won't pair | different Wi-Fi, or code seen on screen | same network; verify the code in Settings |
| Phone says it cannot reach the till | the computer's firewall drops port 47831 | Settings → Scanner names the firewall this till uses. Press **Allow port** and give the administrator password once; if that fails, run `sudo firewall-cmd --add-port=47831/tcp` (Fedora) or `sudo ufw allow 47831/tcp` (Ubuntu) |

---

## 11. Known limits of this build

- Cash only; no mobile money or card tender UI (`payments.method` is `cash`)
- No printed receipts — copy the confirmation slip as text instead
- Void / refund / discount / drawer-open are permissioned but have no screens
- Inventory counts units, not value; a purchase can record a unit cost, but there
  is no full stock valuation
- Tax is not configured anywhere; cart prints "not configured"
- No held/parked carts; abandoning a sale means emptying the cart
- No multi-terminal sync; one till per database
- The companion phone app scans into the till only — it is not a second till
- The recovery code is the only credential reset path for the owner

*Questions not answered here: the in-app screen is always the authority — it says
exactly what a button will do before you press it.*
