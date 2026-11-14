# Stride POS — Training Manual

**For:** owners, managers, and cashiers.
**Goal:** after this manual, a new person can add a product, record a delivery, keep stock honest, and take cash at the counter without help.

This is a teaching document. It is deliberately repetitive: every lesson has steps, a check, practice, and the mistakes to avoid.

> **The one rule:** the screen is the authority. If this manual and the screen disagree, the screen is right. Read the caption under a button before you press it.

---

## How to use this manual

- **If you are the trainer:** read Part 0, then follow the session plan in Part 6. Do not read the manual to the trainee. Let them drive the till.
- **If you are learning alone:** read Part 0, then do Lessons 1–4 in order. Do every practice step at the till. Do not skip to selling.
- **If you only have ten minutes:** go to the Quick Reference at the end. It is one page.
- **If a new cashier starts tomorrow and you cannot train the full course:** teach the Quick Reference sale card today, then teach Lessons 1–3 in the first quiet hour. A cashier can sell without knowing inventory, but the shop cannot trust stock without someone learning it.

**Train on a test till if you can.** Practice sales are real sales: this build has no void or refund screen, so they cannot be removed from the Dashboard. Use a spare computer or a copy of the data file. If you must train on the live till, train before opening and tell the owner the day's figures include practice.

---

## Part 0 — Before you start

### 0.1 What this till is

Stride POS is a cash till for a small shop. It runs on the shop's computer and keeps working with no internet, no account, and no cloud. Every sale is stored in one file on that computer.

It does five jobs, and this manual teaches them in the order goods move through the shop:

1. **Products** — name and price what you sell.
2. **Suppliers** — record who you buy from.
3. **Inventory** — record what is on the shelf.
4. **Restock** — plan a restocking trip and receive what arrived.
5. **Sell** — take the money.

That order is intentional. You cannot sell what is not in Products. You cannot receive goods without a product to receive them into. You cannot trust a stock count you never started. Suppliers sit beside that chain: the delivery note needs a name. Learn the order once; after that, nearly all your time is in Lesson 4.

### 0.2 The eight screens

| Key | Screen | What it is for |
|---|---|---|
| `F1` | **Sell** | The counter: search, scan, cart, payment |
| `F2` | **Sales** | Every completed sale, newest first |
| `F3` | **Inventory** | What the stock record says is on the shelf |
| `F4` | **Dashboard** | Takings, number of sales, best sellers, busiest hour |
| `F5` | **Products** | The catalogue: add, edit, import, export |
| `F6` | **Suppliers** | Who the shop buys from |
| `F7` | **Restock** | Plan a restocking trip, then receive what arrived |
| `F8` | **Settings** | Shop name, staff, scanner, receipt preview, backup |

F7 is the shop's shopping list: it plans what to buy and records what came back. The bottom bar shows your name, your role, and the current cart.

### 0.3 Signing in and locking

1. The lock screen lists the active people. **Pick your name.**
2. Type your **six-digit PIN** (the owner types a password instead).
3. Press **Sign in**.

The till locks itself after **five idle minutes** (fifteen for the owner). Press `Ctrl+L` to lock it now. **The cart survives a lock.** Nothing in the cart is lost.

Five wrong tries lock the account for 30 seconds. Each further failure doubles the wait, up to 15 minutes. The screen shows the countdown.

### 0.4 Who can do what

| Task | Cashier | Manager | Owner |
|---|:---:|:---:|:---:|
| Sell and take cash | Yes | Yes | Yes |
| Lock the till | Yes | Yes | Yes |
| See own sales and stock | Yes | Yes | Yes |
| See the whole shop's sales | No | Yes | Yes |
| Add or edit products | No | Yes | Yes |
| Manage suppliers | No | Yes | Yes |
| Record stock movements | No | Yes | Yes |
| Import/export, backup, settings | No | Yes | Yes |
| Add staff, reset a PIN, change currency | No | No | Yes |

**The one-minute approval.** When a cashier needs something above their role, the till says **"A manager is needed."** A manager or the owner picks their name, enters their PIN or password, and presses **Approve**. That one action is allowed for one minute. The sale stays in the cashier's name.

Two things can never be approved for a cashier: **managing staff** and **changing the currency**. Those always need the owner.

### 0.5 Keys you will use

| Key | Does |
|---|---|
| `F1`–`F8` | Jump to a screen |
| `Ctrl+Enter` or `F12` | Take payment |
| `Ctrl+L` | Lock the till |
| `Ctrl+P` | Command palette — type "dash", "sales", "lock" |
| `Ctrl+B` | Show or hide the sidebar |
| `Ctrl+T` | Light or dark |

In forms and dialogs: `Enter` saves, `Escape` cancels. In the payment dialog: `Enter` completes the sale, `Escape` goes back to the cart.

### 0.6 If the till is brand new

Do this once, with the owner present. About twenty minutes.

1. **Create the owner account** (name, password of at least eight characters).
2. **Write down the recovery code.** It is shown once. It is the only way back in if the owner forgets the password. Keep it away from the till.
3. **Choose the currency.** This locks after the first sale, so choose carefully.
4. **Set the shop name:** Settings → Terminal → type the name → **Save**. It prints on the receipt preview.
5. **Add staff:** Settings → Staff → **Add a person**. The till makes a six-digit PIN and shows it once. Write it down before leaving the page.
6. **Add products** (Lesson 1).
7. **Take a backup:** Settings → Terminal → **Back up now**.

### 0.7 The practice shop

Use these exact products and suppliers for every lesson and scenario. The numbers are chosen so the answers come out cleanly. Type them exactly.

Prices in the table are written with thousands separators so they are easy to read. When you type a price into the till, type the digits only: `3000`, not `3,000`.

**Products**

| Name | Category | Price (UGX) | Barcode | Opening stock | Warn at |
|---|---|---:|---|---:|---:|
| Africa Tea Leaves 100 g | Groceries | 3,000 | 6001234500011 | 20 | 5 |
| Royco Mchuzi Mix 70 g | Groceries | 2,200 | 6001234500028 | 15 | 5 |
| Unga Mill Maize Flour 1 kg | Groceries | 4,500 | 6001234500035 | 30 | 10 |
| Sugar 500 g | Groceries | 3,350 | 6001234500042 | 12 | 4 |
| Sunlight Bar Soap 175 g | Household | 1,800 | 6001234500059 | 24 | 6 |
| Colgate Toothpaste 100 ml | Household | 5,200 | 6001234500066 | 8 | 3 |

**Suppliers**

| Name | Contact | Phone | Notes |
|---|---|---|---|
| Acme Wholesale | Amina | +256 700 111 222 | Delivers Monday and Thursday |
| Lakeside Distributors | John | +256 700 333 444 | Soap and household goods |

**Use a fake shop name for training**, for example "Practice Shop". Change it to the real name only when the till goes live.

**A note on currency:** every example uses UGX, which rounds cash to the nearest 100. If your shop uses KES, the figures are the same because KES also rounds to 100. If your shop uses TZS, cash rounds to the nearest 5,000 and every cash total in these examples changes. Do the steps; read the total on the screen rather than copying the number from this page.

---

## Lesson 1 — Products: what you sell

**You will learn:** how to add one product, change a price, load a list from a spreadsheet, and delete a product safely.

**Before you start:** sign in as manager or owner. The currency must be set.

### 1.1 Add one product

1. Press `F5` to open **Products**.
2. Press **New product**.
3. Type the **Name** — for example, `Africa Tea Leaves 100 g`.
4. Type the **Price**. Type only the number: `3000`, not `UGX 3,000`.
5. Leave **Category** as `General`, or type a shelf name such as `Groceries`. **Pick…** shows categories that already exist.
6. Type the **Barcode** if the item has one. It is optional, but it must be unique.
7. Press **Save**.

**Check:** the product appears in the Products list. Press `F1` — a tile for it is now on the Sell grid.

### 1.2 Change a price, edit, or delete

- **Edit:** press the pencil icon on the row. Change what you need. Press **Save**.
- **Delete:** press the trash icon, then the check mark. Deletion is permanent and takes that product's stock history with it.

Two facts to remember:

- **Changing a price does not change old receipts.** Every sale copies the name and price at the moment of sale. History stays history.
- **Deleting a product is not "hiding" it.** If you only want it off the grid, there is no hide button in this build. Deleting loses its stock record. Ask the owner first.

### 1.3 Load many products from a spreadsheet

1. `F5` → **More** → **Import CSV**. A file chooser opens.
2. In the chooser, press **Save a template**, then choose a folder. The till writes `products-template.csv`.
3. Open that file in a spreadsheet. Fill one product per row: `name,category,price,barcode`.
4. Save it as CSV.
5. `F5` → **More** → **Import CSV** again, and pick your filled file.
6. Read the notice: **"Imported N products, skipped M."** If nothing at all could be imported, the same box shows the first reason. If only some rows were skipped, the notice gives the counts — open the file and check the rows yourself.

Bad rows are skipped, never half-imported. Duplicate barcodes are skipped as well.

**Excel warning:** a barcode that starts with `0` will lose the zero unless you format that column as text first. The till does not put the zero back.

### 1.4 Export products

- **More → Export CSV** exports the rows you can see.
- **More → Export all** exports the whole catalogue.

Both produce `products-…-YYYYMMDD-HHMMSS.csv`. Amounts are plain numbers with no currency symbol, so a spreadsheet reads them cleanly.

### 1.5 Practice

1. Add the first three products from the practice table by hand.
2. Import the last three from a CSV you build yourself.
3. Search for `tea` and confirm one result.
4. Change Sugar 500 g from 3,350 to 3,500.
5. Change it back to 3,350.
6. Export all products and open the file.

**Common mistakes**

| Mistake | What happens | What to do |
|---|---|---|
| Typing `3,000` with a comma | The price is rejected or read wrongly | Type `3000` |
| Reusing a barcode | "Another product already uses that barcode" | Search that barcode and edit the right product |
| Deleting instead of editing | Stock history is gone | Restore by re-adding, and re-count stock |
| Leaving the category blank | It becomes `General` | Fine, or type a shelf name |

---

## Lesson 2 — Suppliers: who you buy from

**You will learn:** add a supplier, find one, edit one, and archive one without losing the record.

**Before you start:** sign in as manager or owner.

### 2.1 Add a supplier

1. Press `F6` to open **Suppliers**.
2. Press **New supplier**.
3. Type the **Name** — this is the only required field.
4. Fill in what you know: **Contact**, **Phone**, **Email**, **Address**, **Notes** (delivery days, terms, the rep's name).
5. Press **Save**.

**Check:** the supplier appears in the list. Type two letters of the name in the search box and confirm it filters.

### 2.2 Edit or archive

- **Edit:** pencil icon on the row.
- **Archive:** archive icon on the row, then confirm with the check mark. Archived suppliers leave the working list but stay for the record. Tick **Show archived** to see them.
- **Restore:** tick **Show archived**, find the supplier, and press the restore icon.

Archiving is the correct way to retire a supplier. Do not delete — there is no delete for suppliers, by design.

### 2.3 What this screen does not do

Suppliers are a **directory**. A purchase can name one, but it does not have to: most market trips have no delivery note. The supplier name is for your records; it does not send an order, receive goods, or move stock. Stock changes when you receive a purchase (Lesson 3A) or record a movement yourself (Lesson 3). If there is an invoice, write its number in the purchase note.

### 2.4 Practice

1. Add Acme Wholesale and Lakeside Distributors from the practice table.
2. Search for `lake` and confirm one result.
3. Edit Acme's notes to `Delivers Monday and Thursday`.
4. Archive Lakeside. Confirm it leaves the main list.
5. Restore Lakeside.

---

## Lesson 3 — Inventory: what is on the shelf

**You will learn:** start tracking a product, record a delivery, fix a wrong count, record waste and returns, and read the history.

**Before you start:** sign in as manager or owner. The products must exist.

### 3.1 How stock works here

Stock is not a number you type and overwrite. It is the **sum of movements**:

- Opening count adds the starting quantity.
- A sale subtracts automatically.
- Received adds.
- Customer return adds.
- Waste subtracts.
- Adjustment corrects to a counted number.

Because stock is a ledger, you can always answer "why is the count what it is?" by opening the product's **History**.

### 3.2 Start tracking a product

1. Press `F3` to open **Inventory**.
2. Press **Track stock**.
3. Choose the product.
4. Enter **what is on the shelf right now** — the opening count.
5. Set **Warn at or below**. For tea with 20 on the shelf, `5` is sensible.
6. Save.

The product now appears in Inventory with its on-hand count. Every future sale of that product subtracts automatically.

**What the warning level means:** the **Low** filter shows products at or below this number, but above zero; **Out** shows zero or negative. The warning does not stop a sale. It is a reminder to reorder.

### 3.3 Record a movement

1. `F3` → find the product → **Move stock**.
2. Choose **What happened**.

| Choice | Use it when | Effect |
|---|---|---|
| **Received** | A supplier delivery arrives | Adds the quantity |
| **Customer return** | A customer brings goods back | Adds the quantity |
| **Waste** | Spoiled, broken, or unsellable | Subtracts the quantity |
| **Adjustment** | A physical count differs | Sets stock to the counted number |

3. Type the **Quantity**. For an adjustment, type the **counted quantity**, not the difference.
4. Type a **Note** — invoice number, delivery date, or reason.
5. Press **Record**.

**Adjustment example:** the till says 12 sugar, you count 9. Choose **Adjustment**, type `9`, and record. The till stores a difference of −3. If you type the number the till already has, it tells you they match and keeps the form open.

### 3.4 History and filters

- **History** shows kind, date, who recorded it, note, and the signed change. The newest 200 are shown.
- **All / Low / Out** filter the list.
- Sales are written to the history automatically. You never record a sale by hand.

### 3.5 Three rules to say out loud

1. **Never type a stock number to "make it right"** unless you have physically counted the shelf. Use **Adjustment** with the counted number.
2. **Negative stock is allowed.** If the till says −1, the shop can still sell. The negative is your reminder to receive stock or fix the count.
3. **Deleting a product deletes its stock history.** The stock record is operational, not a financial archive.

### 3.6 Practice

1. Track all six products with the opening counts and warning levels in the table.
2. Receive a delivery: 24 tea, note `INV-104`.
3. Check tea is now 44.
4. Count sugar: the shelf has 9, the till says 12. Record an **Adjustment** of 9.
5. Record **Waste** of 2 toothpaste.
6. Record a **Customer return** of 1 soap.
7. Open tea's **History** and read the last three lines.

---

## Lesson 3A — Restock: the restocking trip

**You will learn:** build a shopping list from low stock, adjust it before leaving, receive only what you actually bought, and record the unit cost.

**Before you start:** the products you plan to buy must be tracked (Lesson 3).

### 3A.1 Plan before you leave

1. Press `F7` to open **Restock**.
2. Press **New list**.
3. Type the supplier's name if there is one. It is optional — a market trip often has no supplier or delivery note.
4. Press **Add low stock**. Every tracked product at or under its warning line appears with a suggested quantity.
5. Change any quantity. The suggestion aims at the product's **Keep in stock** level after counting what is already on the shelf and on other planned purchases.
6. Add anything else with the product search. A product does **not** have to be low to go on the list.
7. Leave cost empty if you do not know it. Press **Save**.

The list now reads **Planned**. Nothing has moved in stock — a plan is intent, not goods.

### 3A.2 Receive what came back

1. `F7` → open the list → the page opens ready to count in.
2. Each row starts at the planned quantity. Type what actually arrived.
3. Enter the **unit cost** if you know it. This becomes the product's last known cost, and profit on future sales uses it.
4. Set a line you did not find to **0**. It stays on the record as not bought.
5. Bought something extra? Add it from the search at the bottom of the page with its quantity and cost.
6. Press **Received**.

Every received line writes exactly one stock movement, so the product's History shows the trip once. The shelf count goes up by what you received.

### 3A.3 Reading the status

| Status | Means | What you can do |
|---|---|---|
| **Planned** | Nothing received yet | Edit quantities, delete the whole list |
| **Received** | The trip came home and is recorded | Read it; it is locked |

There is no **Part received** state. Receiving is one act, so the same goods cannot be counted in twice. A trip that came home short says "3 of 5 bought" — the shortfall is not turned into a new list for you; plan the next trip from **Add low stock**.

### 3A.4 Three rules to say out loud

1. **A plan is not stock.** Only receiving writes a movement.
2. **Receive what is real.** Change the quantity to what is in front of you, not what the list hoped for.
3. **Cost is optional but valuable.** Without it the till can still count stock; with it, the Dashboard can show profit.

### 3A.5 Practice

1. Make tea low: sell enough, or adjust it down to 2.
2. `F7` → **New list** → **Add low stock**. Check tea appears with a sensible quantity.
3. Add soap manually even though it is not low.
4. Save. Confirm the list is **Planned** and tea's on-hand count did not change.
5. Open it, receive 10 tea at cost `800`. Set soap to **0**.
6. Confirm tea's stock rose by 10, the trip says **Received**, History shows one received line, and soap's count did not move.

---

## Lesson 4 — Sell: taking money

**You will learn:** ring a sale, take cash, give change, understand rounding, fix a cart, and start the next sale.

**Before you start:** sign in. Products must exist. Payment is **cash only** in this build.

### 4.1 A sale in seven steps

1. Press `F1` to open **Sell**.
2. Add items: scan a barcode, type in **Scan or search**, or tap a tile.
3. Check the cart on the right. The total is at the bottom.
4. Press **Pay** (or `Ctrl+Enter` / `F12`).
5. Type the cash received, or tap a quick-tender chip.
6. Press **Complete sale** (or `Enter`).
7. Hand over the change. The cart is empty and the cursor is back in **Scan or search** — scan the next customer's first item.

### 4.2 Putting items in the cart

All three ways can be mixed in one sale:

- **Scan** with a USB scanner or the paired phone. An exact barcode wins over the search text.
- **Search** by part of a name or barcode. The grid narrows as you type.
- **Tap** a product tile.

**Same product twice** merges into one line at quantity 2. Check the quantity, not the number of lines.

**Change quantity:** use **−** and **+** on the line. Going below 1 removes the line. There is no undo; re-add the right product.

**Empty the whole cart:** `Ctrl+P` → **Clear cart**.

### 4.3 Taking cash

The cart shows **Subtotal**, **Tax** (it says "not configured" in this build — ignore it), and **Total**. The payment dialog shows **Subtotal**, any rounding line, and **To pay** in large type.

- Leave the cash field empty to mean **exact amount**.
- Or type what the customer handed over.
- Or tap a quick chip. The first chip is always **Exact**. The others round the bill up to a circulating note.

As you type, the screen shows **Short by …** in red or **Change …** in green.

**Chip behaviour:** a chip is never smaller than the bill, so one tap always covers it. If you have already typed part of the cash and the amount is still short, tapping a chip **adds** to what is there. Once the amount covers the bill, the next tap **replaces** it.

### 4.4 Why the total sometimes changes by a few shillings

Some coins no longer circulate. The till rounds the **cash due** only:

- UGX and KES round to the nearest **100**.
- TZS rounds to the nearest **5,000**.

The sale still records the exact price-list total and stores the rounding difference separately, so the books match the drawer. Half goes away from zero: 1,350 becomes 1,400; 1,340 becomes 1,300. A cashier cannot override this.

**Example:** tea 3,000 + sugar 3,350 + Royco 2,200 = **8,550**. The customer pays **8,600**. The receipt shows `Subtotal 8,550`, `Rounded up 50`, `Total (UGX) 8,600`. Give 1,400 change from 10,000.

### 4.5 After the sale

Pressing **Complete sale** writes the sale, takes the stock out, empties the cart, and puts the cursor back in **Scan or search**. There is no confirmation panel and nothing to dismiss — the next customer's first item can be scanned immediately.

- The **Change** you read before completing is what you hand over.
- The slip is kept on the sale: `F2` **Sales** → open the sale, which shows lines, amount paid, method and change. **Copy receipt** puts it on the clipboard as text, for a WhatsApp confirmation or a dispute.
- Receipts cannot be printed in this build. Say so plainly and offer the copied text.

**If the queue is long:** the cart is already empty after a sale, so `Enter` on the scanner's next barcode starts the next sale. No keypress is needed to "start a new sale".

### 4.6 Fixing mistakes

Fix the cart **before** you press **Complete sale**. After that:

- There is no void, refund, or discount screen in this build.
- Do not invent a fake sale to balance the drawer. It makes the reports wrong.
- Write the problem in the paper record, keep the receipt copy, and tell the owner or manager.

### 4.7 Locking mid-sale

`Ctrl+L` locks the till. The cart stays exactly as it was. The next person signs in and continues from the same cart, so lock only when you are coming straight back.

### 4.8 Practice: three sales

Ring these in order and check every expected result.

**Sale A — exact cash**

| Item | Qty | Line |
|---|---:|---:|
| Africa Tea Leaves 100 g | 2 | 6,000 |
| Royco Mchuzi Mix 70 g | 1 | 2,200 |
| Sunlight Bar Soap 175 g | 1 | 1,800 |

Total **10,000**. Pay 10,000. **Change 0.**

**Sale B — change and rounding**

| Item | Qty | Line |
|---|---:|---:|
| Africa Tea Leaves 100 g | 1 | 3,000 |
| Sugar 500 g | 1 | 3,350 |
| Royco Mchuzi Mix 70 g | 1 | 2,200 |

Subtotal **8,550** → cash total **8,600**. Pay 10,000. **Change 1,400.** Receipt shows **Rounded up 50**.

**Sale C — quantity**

| Item | Qty | Line |
|---|---:|---:|
| Unga Mill Maize Flour 1 kg | 2 | 9,000 |
| Colgate Toothpaste 100 ml | 1 | 5,200 |

Total **14,200**. Pay 20,000. **Change 5,800.**

**Check the stock afterwards:**

| Product | If you did the Lesson 3 practice | If you only entered the opening counts |
|---|---:|---:|
| Tea | 41 | 17 |
| Royco | 13 | 13 |
| Soap | 24 | 23 |
| Sugar | 8 | 11 |
| Flour | 28 | 28 |
| Toothpaste | 5 | 7 |

The rule is simpler than the table: **each product falls by exactly what you sold.** The difference between the two columns is the delivery, waste, return, and correction you recorded in Lesson 3.

### 4.9 Optional: the phone scanner

Only if the shop uses it.

1. Till: Settings → Scanner → **Turn on**.
2. Phone and till must be on the **same Wi-Fi**.
3. Open the **Stride Scan** app and scan the QR code on the Settings screen.
4. In the app, choose **Sell** to add to the cart, or **Add** to open a new product with the barcode filled in.

The listener is off until you turn it on. Scans are ignored while the till is locked or a dialog is open. Do not put the till on an untrusted network or forward the port.

---

## Part 5 — Scenario workbook

Use these after the four lessons. Read the situation, do the task, then check the answer. The trainer should let the trainee make the mistake and recover.

Run them on a fresh practice till when you can. If a product or supplier already exists from the lesson practice, use a new name or skip that step — the point is the action, not the exact record.

### Products

**S1 — The first product**

*Situation:* A supplier delivers tea, but the till has never heard of it.

*Do:* Add `Africa Tea Leaves 100 g`, category `Groceries`, price `3000`, barcode `6001234500011`. Then press `F1` and find it on the grid.

*Check:* The product is in the list and has a tile on Sell.

**S2 — The supplier raises a price**

*Situation:* Sugar moves from 3,350 to 3,500. The earlier receipts must not change.

*Do:* Open the Sale B receipt from Lesson 4 — it contains sugar at 3,350. Edit Sugar 500 g to 3,500 and save. Open `F2` and open Sale B again. Change the price back afterwards.

*Check:* The Sell tile shows 3,500. The old sale still shows 3,350.

**S3 — A spreadsheet with one bad row**

*Situation:* You have 3 new products in CSV. One row has `abc` in the price column. Use these rows:

```
Vanilla Essence 50 ml,Groceries,4500,6001234500073
Omo Washing Powder 500 g,Household,6200,6001234500080
Bad Row,Groceries,abc,6001234500097
```

*Do:* `F5` → **More** → **Import CSV**.

*Check:* The notice says **"Imported 2 products, skipped 1."** The good rows are complete. The notice does not name the bad row, so check the file if the number surprises you.

**S4 — The barcode is already taken**

*Situation:* You try to add a second product with barcode `6001234500011`.

*Do:* Save it and read the error.

*Check:* "Another product already uses that barcode." Search the barcode and edit the existing product instead.

### Suppliers

**S5 — A new supplier on delivery day**

*Situation:* A new wholesaler arrives. You only know the name and phone.

*Do:* Add the supplier with the name and phone, leave the rest blank.

*Check:* It saves with only the name required and is findable by search.

**S6 — A supplier stops supplying**

*Situation:* Lakeside Distributors no longer delivers, but you want the record.

*Do:* Archive Lakeside, confirm, tick **Show archived**, then restore it.

*Check:* It leaves the working list, then comes back when you show archived. Nothing is deleted.

### Inventory

**S7 — Start tracking**

*Situation:* Tea has 20 on the shelf. You want a warning at 5.

*Do:* **Track stock** → tea → opening count 20 → warn at 5.

*Check:* Inventory shows the count you entered.

**S8 — A delivery arrives**

*Situation:* Acme delivers 24 tea, invoice `INV-104`.

*Do:* **Move stock** → **Received** → 24 → note `INV-104`.

*Check:* Tea rises by 24. The history shows +24, your name, and the note.

**S9 — The shelf and the till disagree**

*Situation:* The till says 12 of a product; you count 9.

*Do:* **Move stock** → **Adjustment** → counted quantity 9.

*Check:* The count becomes 9 and the history shows −3. If you type the number the till already has, it says they match and does not record.

**S10 — Spoiled goods**

*Situation:* Two toothpaste tubes are crushed and cannot be sold.

*Do:* **Move stock** → **Waste** → 2 → note `Crushed in delivery`.

*Check:* Toothpaste drops by 2.

**S11 — A customer returns goods**

*Situation:* A customer brings back one bar of soap, still sellable.

*Do:* **Move stock** → **Customer return** → 1.

*Check:* Soap rises by 1. Do not record a sale.

### Selling

**S12 — Exact cash**

*Situation:* A customer buys 2 tea, 1 Royco, 1 soap and hands over 10,000.

*Do:* Ring Sale A.

*Check:* Total 10,000, change 0, and the screen is ready for the next scan.

**S13 — Change and rounding**

*Situation:* A customer buys tea, sugar, and Royco and pays with 10,000.

*Do:* Ring Sale B.

*Check:* Subtotal 8,550; cash total 8,600; change 1,400; the sale detail in `F2` shows **Rounded up 50**.

**S14 — The barcode will not scan**

*Situation:* The scanner beeps, but the till says "No product matches".

*Do:* Type part of the name in **Scan or search**. If the item truly is not in the catalogue, ask a manager to add it with the barcode.

*Check:* You can sell it by search, and the catalogue now holds the barcode.

**S15 — The wrong item is in the cart**

*Situation:* You tapped toothpaste instead of soap, and the customer also wants only one tea, not two.

*Do:* Use **−** on tea to go to 1. Remove toothpaste with **−** below 1, then add soap.

*Check:* The lines and total match the customer's order. Re-add if you remove the wrong line.

**S16 — Two customers, one till**

*Situation:* The first sale is paid; the next customer is already waiting.

*Do:* Scan the next customer's first item. The cart cleared itself; there is no **New sale** button to press and nothing to close.

*Check:* The cart is empty and the search field is ready.

**S17 — The cashier needs the shop view**

*Situation:* A cashier opens `F2` and sees only their own sales.

*Do:* Ask a manager to approve. The manager picks their name, enters their PIN, and presses **Approve**.

*Check:* The shop view opens for one minute. The sale still belongs to the cashier.

### Mixed and end of day

**S18 — Close of day**

*Situation:* The shop is closing.

*Do:* `F4` → **Today** → read Revenue and the **Payments taken** card. `F2` → **Export CSV**. Settings → Terminal → **Back up now**. `Ctrl+L`.

*Check:* The cash in the drawer matches the Payments card. The export file exists. The backup is in the folder.

**S19 — A new cashier starts**

*Situation:* A new person joins the counter tomorrow.

*Do:* Owner: Settings → Staff → **Add a person** → name → role **Cashier** → **Add**. Write down the six-digit PIN shown once. Have them sign in and ring a test sale.

*Check:* They can sell, take payment, and lock. Product changes and stock movements ask for approval. Switch the account off when they leave.

**S20 — The drawer is short**

*Situation:* The drawer has 2,000 less than the till says.

*Do:* Compare the **Payments taken** card with the physical cash. Open `F2` → **Today** and check the sale count and totals. Open suspicious rows to see paid and change. Export the day.

*Check:* If every sale is recorded, the till is right and the difference is a cash-handling problem. Do not invent a sale to hide it. Report it to the owner.

---

## Part 6 — Trainer's session plan

Two hours, one trainee, one till. Add 30 minutes for a second trainee so each person drives. The times are a guide: slow down rather than skip a practice step, and split the session if the trainee is new to computers.

| Time | What | Who drives |
|---|---|---|
| 0:00–0:10 | Welcome. Show one complete sale, start to finish, without explaining every click. Say: "By the end, you will do this alone." | Trainer |
| 0:10–0:35 | Lesson 1 — Products. Add 3 by hand, import 3, edit a price. | Trainee |
| 0:35–0:50 | Lesson 2 — Suppliers. Add 2, search, archive, restore. | Trainee |
| 0:50–1:10 | Lesson 3 — Inventory. Track all 6, receive, adjust, waste, return. | Trainee |
| 1:10–1:35 | Lesson 4 — Sell. Three practice sales, including rounding and change. | Trainee |
| 1:35–1:50 | Scenario workbook. Pick six: one products, one suppliers, two inventory, two selling. | Trainee |
| 1:50–2:00 | Recap, questions, sign-off. | Both |

**How to teach each step**

1. **Say** what you are about to do.
2. **Show** it once, slowly, narrating the label you press.
3. **Let them do it** with the manual closed. Say nothing for the first ten seconds.
4. **Watch** where the hand hesitates. That is the real lesson.
5. **Correct with a question**, not a command: "What does the caption under that button say?"
6. **Repeat** the step once more, immediately.

**Rules for the trainer**

- Never take the mouse or keyboard. Point at the screen instead.
- One trainee, one till. Watching is not learning.
- Praise the check, not the speed: "You checked the change — good."
- If they make a mistake, let them finish and read the result. The till explains itself.
- Stop for five minutes of quiet every 45 minutes. Novices stop absorbing before they stop nodding.
- End every lesson with the trainee saying the rule back in their own words: "Stock is the sum of movements." "Changing a price does not change old receipts."

**What to test, not what to say**

Do not ask "Do you understand?" Ask them to do it. A trainee who can ring Sale B with correct change and explain the 50-shilling rounding is ready for the counter.

---

## Part 7 — Competency checklist

Tick each item only when the trainee has done it alone, without help. Sign at the bottom.

**Sign-in and safety**

- [ ] Signs in with their own name and PIN
- [ ] Locks the till with `Ctrl+L` and signs back in
- [ ] Explains what the one-minute approval is
- [ ] Knows the cart survives a lock

**Products**

- [ ] Adds a product with name, price, category, and barcode
- [ ] Edits a price
- [ ] Explains why old receipts do not change
- [ ] Imports a CSV and reads the "imported/skipped" message
- [ ] Knows that deleting a product deletes its stock history

**Suppliers**

- [ ] Adds a supplier with only a name
- [ ] Finds a supplier by search
- [ ] Archives and restores a supplier
- [ ] Knows a purchase can name a supplier, but the supplier is only a record

**Inventory**

- [ ] Starts tracking with an opening count and warning level
- [ ] Records a **Received** movement with a note
- [ ] Records an **Adjustment** using the counted quantity
- [ ] Records **Waste** and **Customer return**
- [ ] Reads a product's history and explains a negative count

**Selling**

- [ ] Rings a sale by search, by tap, and by scan (if the shop has a scanner)
- [ ] Changes quantity and removes a line
- [ ] Takes exact cash
- [ ] Takes a larger note and gives the right change
- [ ] Explains rounding in one sentence
- [ ] Completes a sale and starts a new one
- [ ] Copies a receipt and knows printed receipts are not available

**End of day**

- [ ] Reads Revenue, Sales, and Payments taken on the Dashboard
- [ ] Exports sales to CSV
- [ ] Takes a backup
- [ ] Compares the drawer with the Payments card and reports a difference

**Sign-off**

| | Name | Signature | Date |
|---|---|---|---|
| Trainee | | | |
| Trainer | | | |

---

## Part 8 — Quick reference (print this)

```
SIGN IN      pick your name → 6-digit PIN → Sign in
LOCK         Ctrl+L          (cart survives)
SCREENS      F1 Sell  F2 Sales  F3 Inventory  F4 Dashboard
             F5 Products  F6 Suppliers  F7 Restock  F8 Settings

ONE SALE     F1 → scan / search / tap → Pay (Ctrl+Enter)
             → cash received or Exact chip → Complete sale
             → give change → scan the next customer's first item
FIX CART     − / + on the line; below 1 removes it
             Ctrl+P → Clear cart
CHANGE       green = give back; red = still short
             the slip stays on the sale: F2 → open it → Copy receipt

ROUNDING     UGX/KES nearest 100 · TZS nearest 5,000
             cash only; the exact total is still recorded

STOCK        Track stock → count → warn level → keep-in-stock level
             Move stock → Received / Customer return / Waste / Adjustment
             Adjustment = type what you COUNTED, not the difference
             Sale subtracts by itself. Negative is allowed.

PURCHASES    F7 → New list → Add low stock → Save (Planned)
             open it → type what arrived → cost if known → Received
             0 = not bought; an extra buy is added from the search
             Only receiving moves stock. Received trips are locked.

SUPPLIERS    F6 → New supplier (name only is enough)
             Archive to retire; restore to bring back

END OF DAY   F4 Today → compare Payments with the drawer
             F2 Export CSV → Settings → Back up now → Ctrl+L

IF STUCK     Read the caption under the button.
             "A manager is needed" → ask for one-minute approval.
             Never invent a sale to fix a number.
```

---

## Part 9 — Troubleshooting

| What you see | Likely cause | What to do |
|---|---|---|
| "No products yet" on Sell | Empty catalogue | `F5` → **New product**, or import a CSV |
| Scan does nothing | Phone scanner off, till locked, or a dialog open | Settings → Scanner → **Turn on**; close dialogs; unlock |
| "No product matches" | Barcode not in the catalogue | Search by name; add the product with the barcode |
| "Another product already uses that barcode" | Duplicate barcode | Search the barcode and edit the existing product |
| "A manager is needed" | Cashier action above their role | Ask for the one-minute approval |
| Correct PIN rejected | Lockout is active | Wait for the countdown; the owner can reset the PIN |
| Dashboard shows only my sales | Signed in as cashier | Manager approval, or sign in as manager/owner |
| Barcode lost its leading zero | Excel formatted it as a number | Format the column as text and re-import |
| Export greyed out | Empty range or empty list | Widen the range; clear the search/filter |
| Receipt shows the app name | Shop name not saved | Settings → Terminal → type the name → **Save** |
| Backup failed | Disk full, permissions, or the till is open elsewhere | Check space; close other copies; try again |
| Phone cannot reach the till | Different Wi-Fi or firewall | Same network; Settings → Scanner → **Allow port** |

**Owner locked out:** at the lock screen press **Forgot the owner password?**, enter the recovery code written down at setup, and set a new password. A new recovery code appears once. Codes are single-use. If the code is lost too, there is no back door.

---

## Part 10 — Words we use

| Word | Plain meaning |
|---|---|
| **Product** | One thing you sell |
| **Catalogue** | All your products together |
| **Category** | A shelf name, such as Groceries or Household |
| **Barcode** | The number under the stripes; it must be unique |
| **Stock** | How many are on the shelf, according to the till |
| **Movement** | One change to stock: received, sold, returned, wasted, adjusted |
| **Opening count** | The quantity on the shelf when tracking starts |
| **Warn level** | The number at or below which the till shows **Low** |
| **Keep in stock** | The level a restock aims for; empty means twice the warn level |
| **Purchase** | A shopping list for a restocking trip |
| **Incoming** | Quantity planned but not yet received |
| **Receive** | Count what actually arrived; this is what moves stock |
| **Unit cost** | What one item cost you; used for profit on later sales |
| **Profit** | Revenue minus cost, for the sale lines whose cost is known |
| **Adjustment** | A correction after counting the shelf by hand |
| **Rounding** | The small cash difference caused by coins no longer circulating |
| **Approval** | A manager allowing one action for one minute |
| **Receipt** | The record of one sale; it can be copied, not printed |

---

## Part 11 — What this build cannot do (say this honestly)

Teach these limits so nobody hunts for a button that is not there.

- **Cash only.** No mobile money, no cards.
- **No printed receipts.** Copy the receipt as text instead.
- **No void, refund, or discount screens.** The permissions exist, the screens do not.
- **No full stock valuation.** Purchases record a unit cost, and the Dashboard shows profit only for sale lines that had a known cost.
- **No supplier delivery workflow.** Purchases are a shopping list for the owner's trip; there is no sent order, no invoice, no payment to a supplier.
- **No tax.** The cart says "not configured".
- **No held carts.** Finishing or clearing the cart is the only way out.
- **One till, one file.** No sync between terminals; purchases stay on this till.
- **The phone app is a scanner, not a second till.**
- **The recovery code is the only owner password reset.**

When a customer asks for something the till cannot do, say what it *can* do: "I can copy the receipt to you" or "The owner can correct that in the record." Never fake a transaction to make the screen look right.
