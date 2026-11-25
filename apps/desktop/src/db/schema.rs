//! The schema, as a list of forward-only migrations.
//!
//! Append a migration with a higher version; never edit one that has shipped,
//! because some till somewhere already applied it.

use storage::Migration;

/// Owner stamp in the SQLite header. Arbitrary, permanent, and never derived
/// from the app's name — a rename must not orphan a till's database.
pub const APPLICATION_ID: i64 = 1_836_624_513;

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "create_sales",
        sql: r#"
            CREATE TABLE IF NOT EXISTS sales (
                id          INTEGER PRIMARY KEY,
                total_cents INTEGER NOT NULL,
                created_at  TEXT    NOT NULL DEFAULT (datetime('now'))
            );
        "#,
    },
    // The till's currency is a property of the install, not of a settings file:
    // it must travel with the database it prices. `meta` is the general
    // key/value table for that kind of fact.
    //
    // `total_cents` named a unit, not an amount: the same integer is whole
    // shillings in Kampala and cents in Nairobi. `total_minor` says what the
    // column actually holds.
    Migration {
        version: 2,
        name: "currency_and_meta",
        sql: r#"
            ALTER TABLE sales RENAME COLUMN total_cents TO total_minor;

            CREATE TABLE IF NOT EXISTS meta (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
        "#,
    },
    // The catalog. `category` is free text rather than a second table: a shop
    // with forty products needs a grouping, not a taxonomy, and a category row
    // nothing references is one more thing to keep consistent.
    //
    // The price is an integer count of minor units in the till's currency — the
    // same rule as `sales.total_minor`. A price is not rounded to the cash step
    // when it is stored: the exact price is what the receipt and the ledger
    // record, and only the cash tender rounds.
    Migration {
        version: 3,
        name: "create_products",
        sql: r#"
            CREATE TABLE IF NOT EXISTS products (
                id          INTEGER PRIMARY KEY,
                name        TEXT    NOT NULL CHECK (length(name) > 0),
                price_minor INTEGER NOT NULL CHECK (price_minor >= 0),
                category    TEXT    NOT NULL CHECK (length(category) > 0),
                barcode     TEXT,
                created_at  TEXT    NOT NULL DEFAULT (datetime('now'))
            );

            CREATE UNIQUE INDEX IF NOT EXISTS products_barcode
                ON products (barcode) WHERE barcode IS NOT NULL;

            CREATE INDEX IF NOT EXISTS products_name ON products (name);
        "#,
    },
    // What was sold, and how it was paid for.
    //
    // A line copies the name and price it was sold at: the catalog is editable,
    // and a receipt that changes when a product is renamed or repriced is not a
    // receipt. `product_id` is only a link back for reporting, and it is
    // `SET NULL` rather than `CASCADE` so deleting a product never deletes the
    // history of selling it.
    //
    // `sales.rounding_minor` is the cash adjustment, signed: the customer paid
    // `total_minor + rounding_minor`, because 340 shillings cannot be tendered
    // with a 100 shilling coin. The exact total stays exact; only the tender
    // rounds.
    //
    // `payments` is a table rather than three columns on the sale because a
    // sale can be paid in parts, and because it is append-only like `sales`.
    // The method is text, not a CHECK list: mobile money is a new row, not a
    // migration that rebuilds the table.
    Migration {
        version: 4,
        name: "sale_lines_and_payments",
        sql: r#"
            ALTER TABLE sales ADD COLUMN rounding_minor INTEGER NOT NULL DEFAULT 0;

            CREATE TABLE IF NOT EXISTS sale_lines (
                id          INTEGER PRIMARY KEY,
                sale_id     INTEGER NOT NULL REFERENCES sales(id),
                product_id  INTEGER REFERENCES products(id) ON DELETE SET NULL,
                name        TEXT    NOT NULL CHECK (length(name) > 0),
                qty         INTEGER NOT NULL CHECK (qty > 0),
                unit_minor  INTEGER NOT NULL CHECK (unit_minor >= 0),
                total_minor INTEGER NOT NULL CHECK (total_minor >= 0)
            );

            CREATE INDEX IF NOT EXISTS sale_lines_sale ON sale_lines (sale_id);

            CREATE TABLE IF NOT EXISTS payments (
                id             INTEGER PRIMARY KEY,
                sale_id        INTEGER NOT NULL REFERENCES sales(id),
                method         TEXT    NOT NULL CHECK (length(method) > 0),
                amount_minor   INTEGER NOT NULL CHECK (amount_minor >= 0),
                tendered_minor INTEGER NOT NULL CHECK (tendered_minor >= 0),
                change_minor   INTEGER NOT NULL CHECK (change_minor >= 0)
            );

            CREATE INDEX IF NOT EXISTS payments_sale ON payments (sale_id);
        "#,
    },
    // Who works here, and who took each sale.
    //
    // The id is a UUIDv7 rather than a rowid because this table is the first one
    // that will be shared between tills: a person is the same person at the
    // counter and in the back office, and merging two installs must not need a
    // renumbering. `sales.user_id` is nullable on purpose — sales written before
    // this migration, and any sale whose cashier row is removed, keep their
    // amounts and lose only the name.
    //
    // `credential` holds a PHC string, never a secret, and `kind` is which
    // keyboard the lock screen shows. Neither is exposed to the UI.
    //
    // `login_attempts` is local wall-clock policy: it must never sync, because a
    // lockout is about a person standing at *this* counter, and a peer's clock
    // has no business deciding when this till opens.
    Migration {
        version: 5,
        name: "staff_and_sale_owner",
        sql: r#"
            CREATE TABLE IF NOT EXISTS users (
                id         TEXT PRIMARY KEY,
                name       TEXT NOT NULL CHECK (length(trim(name)) > 0),
                role       TEXT NOT NULL CHECK (role IN ('cashier', 'manager', 'owner')),
                credential TEXT NOT NULL,
                kind       TEXT NOT NULL CHECK (kind IN ('pin', 'password')),
                active     INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0, 1)),
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE UNIQUE INDEX IF NOT EXISTS users_name ON users (name COLLATE NOCASE);

            -- One owner per install: the recovery code and the currency belong
            -- to exactly one person.
            CREATE UNIQUE INDEX IF NOT EXISTS users_one_owner
                ON users (role) WHERE role = 'owner';

            ALTER TABLE sales ADD COLUMN user_id TEXT REFERENCES users(id);

            CREATE TABLE IF NOT EXISTS login_attempts (
                user_id      TEXT PRIMARY KEY REFERENCES users(id),
                failures     INTEGER NOT NULL DEFAULT 0 CHECK (failures >= 0),
                locked_until INTEGER
            );
        "#,
    },
    // Who the shop buys from.
    //
    // A supplier is a party, not a catalog row: the id is a UUIDv7 for the same
    // reason the staff ids are, because the same supplier is the same supplier
    // in the back office and on another till. Sync metadata is still absent
    // until the sync layer lands; the id is the part that would be expensive to
    // change later.
    //
    // Suppliers are archived rather than deleted. A purchase order will point
    // at one, and "who did we buy this from" must survive the shop changing
    // hands or a rep moving on.
    Migration {
        version: 6,
        name: "create_suppliers",
        sql: r#"
            CREATE TABLE IF NOT EXISTS suppliers (
                id           TEXT PRIMARY KEY,
                name         TEXT NOT NULL CHECK (length(trim(name)) > 0),
                contact_name TEXT,
                phone        TEXT,
                email        TEXT,
                address      TEXT,
                notes        TEXT,
                archived     INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
                created_at   TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE UNIQUE INDEX IF NOT EXISTS suppliers_name
                ON suppliers (name COLLATE NOCASE);
        "#,
    },
    // Stock, as a ledger rather than a number.
    //
    // `stock_movements` is append-only: every receive, sale, return, waste and
    // adjustment is a signed `delta`, and what the shop has is the sum. That is
    // the only shape that survives corrections — an "on hand" column has to be
    // repaired by hand the first time somebody mistypes a count, while a wrong
    // movement can be answered with another one that says why.
    //
    // Tracking is opt-in per product. A catalog imported from a spreadsheet has
    // no counts, and inventing zero for every row would claim the shop is out
    // of everything. `tracked` marks the rows that have been counted.
    //
    // `low_stock` is a per-product warning line, not a reorder policy: units are
    // whole things on a shelf, and a shop that sells crates and single sweets
    // needs two different lines. Five is a default, not a truth.
    //
    // Deleting a product deletes its movements. This is inventory, not the
    // ledger: there is no valuation or cost to preserve, and a receipt keeps the
    // name and price of what was sold. A purchasing module that needs history
    // past a deleted catalog row will have to carry its own snapshot.
    Migration {
        version: 7,
        name: "create_stock_ledger",
        sql: r#"
            ALTER TABLE products ADD COLUMN tracked INTEGER NOT NULL DEFAULT 0
                CHECK (tracked IN (0, 1));
            ALTER TABLE products ADD COLUMN low_stock INTEGER NOT NULL DEFAULT 5
                CHECK (low_stock >= 0);

            CREATE TABLE IF NOT EXISTS stock_movements (
                id         INTEGER PRIMARY KEY,
                product_id INTEGER NOT NULL REFERENCES products(id) ON DELETE CASCADE,
                delta      INTEGER NOT NULL CHECK (delta <> 0),
                kind       TEXT NOT NULL CHECK
                    (kind IN ('opening', 'sale', 'receive', 'return', 'waste', 'adjust')),
                note       TEXT,
                sale_id    INTEGER REFERENCES sales(id),
                user_id    TEXT REFERENCES users(id),
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE INDEX IF NOT EXISTS stock_movements_product
                ON stock_movements (product_id, id);
            CREATE INDEX IF NOT EXISTS stock_movements_sale
                ON stock_movements (sale_id) WHERE sale_id IS NOT NULL;
        "#,
    },
    // What the sale collected for the tax authority. The rate is a terminal
    // setting that can change, so the amount is written onto the sale itself:
    // a receipt reprinted after the rate moves must show the same tax it did
    // the day the sale happened. `0` means no tax — some small stores are not
    // VAT-registered, and `DEFAULT 0` keeps old rows honest without a rewrite.
    Migration {
        version: 8,
        name: "sale_tax",
        sql: r#"
            ALTER TABLE sales ADD COLUMN tax_minor INTEGER NOT NULL DEFAULT 0
                CHECK (tax_minor >= 0);
        "#,
    },
    // Categories: a proper table so the product form can offer a dropdown and
    // renaming a shelf cascades to every product on it. `category_id` is added
    // to `products` and the old free-text `category` column stays as the
    // denormalized name printed on receipts.
    Migration {
        version: 9,
        name: "create_categories",
        sql: r#"
            CREATE TABLE IF NOT EXISTS categories (
                id          INTEGER PRIMARY KEY,
                name        TEXT    NOT NULL CHECK (length(trim(name)) > 0),
                sort_order  INTEGER NOT NULL DEFAULT 0,
                color       TEXT
            );

            CREATE UNIQUE INDEX IF NOT EXISTS categories_name
                ON categories (name COLLATE NOCASE);

            ALTER TABLE products ADD COLUMN category_id INTEGER
                REFERENCES categories(id) ON DELETE SET NULL;

            CREATE INDEX IF NOT EXISTS products_category
                ON products (category_id);
        "#,
    },
    // The bones of till-to-till replication.
    //
    // A rowid is only meaningful inside the database that assigned it, so every
    // row that travels gets a `uid` beside its integer key. Rows written before
    // this migration keep `uid = NULL`: they are history, and there is no honest
    // way to invent an identity for them. Every new row is a UUIDv7, which sorts
    // by creation time without being a clock the peers have to agree on.
    //
    // `change_seq` is the local feed's cursor, not a timestamp. It is bumped once
    // per transaction that creates a replicable sale, so a peer can ask "what
    // happened after 41?" and a restart or a clock change cannot make it skip or
    // repeat a row. `origin_device` says which install wrote it; `received_at`
    // is set only when a peer's sale arrives here.
    //
    // A product is archived rather than deleted, because a sale line and a stock
    // movement both point at it. The barcode index therefore covers only live
    // products: an archived barcode can be reused, and the operator is not told
    // "taken" by a product they cannot see.
    Migration {
        version: 10,
        name: "sync_foundation",
        sql: r#"
            INSERT INTO meta (key, value) VALUES ('device_id', '')
                ON CONFLICT(key) DO NOTHING;
            INSERT INTO meta (key, value) VALUES ('change_seq', '0')
                ON CONFLICT(key) DO NOTHING;
            INSERT INTO meta (key, value) VALUES ('catalog_version', '1')
                ON CONFLICT(key) DO NOTHING;

            ALTER TABLE sales ADD COLUMN uid TEXT;
            ALTER TABLE sales ADD COLUMN origin_device TEXT;
            ALTER TABLE sales ADD COLUMN is_offline INTEGER NOT NULL DEFAULT 0
                CHECK (is_offline IN (0, 1));
            ALTER TABLE sales ADD COLUMN clock_skew INTEGER NOT NULL DEFAULT 0;
            ALTER TABLE sales ADD COLUMN received_at TEXT;
            ALTER TABLE sales ADD COLUMN change_seq INTEGER;

            ALTER TABLE sale_lines ADD COLUMN uid TEXT;
            ALTER TABLE payments ADD COLUMN uid TEXT;
            ALTER TABLE stock_movements ADD COLUMN uid TEXT;

            ALTER TABLE products ADD COLUMN archived INTEGER NOT NULL DEFAULT 0
                CHECK (archived IN (0, 1));

            CREATE UNIQUE INDEX IF NOT EXISTS sales_uid
                ON sales (uid) WHERE uid IS NOT NULL;
            CREATE UNIQUE INDEX IF NOT EXISTS sale_lines_uid
                ON sale_lines (uid) WHERE uid IS NOT NULL;
            CREATE UNIQUE INDEX IF NOT EXISTS payments_uid
                ON payments (uid) WHERE uid IS NOT NULL;
            CREATE UNIQUE INDEX IF NOT EXISTS stock_movements_uid
                ON stock_movements (uid) WHERE uid IS NOT NULL;

            CREATE INDEX IF NOT EXISTS sales_change_seq
                ON sales (change_seq) WHERE change_seq IS NOT NULL;
            CREATE INDEX IF NOT EXISTS products_archived
                ON products (archived);

            DROP INDEX IF EXISTS products_barcode;
            CREATE UNIQUE INDEX IF NOT EXISTS products_barcode
                ON products (barcode) WHERE barcode IS NOT NULL AND archived = 0;

            CREATE TABLE IF NOT EXISTS sync_peers (
                id                   TEXT PRIMARY KEY,
                name                 TEXT NOT NULL DEFAULT '',
                kind                 TEXT NOT NULL CHECK (kind IN ('till', 'phone')),
                host                 TEXT,
                port                 INTEGER,
                token                TEXT NOT NULL,
                last_pulled_seq      INTEGER NOT NULL DEFAULT 0,
                last_catalog_version INTEGER NOT NULL DEFAULT 0,
                last_seen_at         TEXT,
                created_at           TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS sync_rejects (
                uid           TEXT PRIMARY KEY,
                origin_device TEXT,
                reason        TEXT NOT NULL,
                payload       TEXT NOT NULL,
                rejected_at   TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TRIGGER IF NOT EXISTS products_catalog_version_insert
            AFTER INSERT ON products
            BEGIN
                UPDATE meta SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
                WHERE key = 'catalog_version';
            END;

            CREATE TRIGGER IF NOT EXISTS products_catalog_version_update
            AFTER UPDATE ON products
            BEGIN
                UPDATE meta SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
                WHERE key = 'catalog_version';
            END;

            CREATE TRIGGER IF NOT EXISTS categories_catalog_version_insert
            AFTER INSERT ON categories
            BEGIN
                UPDATE meta SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
                WHERE key = 'catalog_version';
            END;

            CREATE TRIGGER IF NOT EXISTS categories_catalog_version_update
            AFTER UPDATE ON categories
            BEGIN
                UPDATE meta SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
                WHERE key = 'catalog_version';
            END;

            CREATE TRIGGER IF NOT EXISTS users_catalog_version_insert
            AFTER INSERT ON users
            BEGIN
                UPDATE meta SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
                WHERE key = 'catalog_version';
            END;

            CREATE TRIGGER IF NOT EXISTS users_catalog_version_update
            AFTER UPDATE ON users
            BEGIN
                UPDATE meta SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
                WHERE key = 'catalog_version';
            END;
        "#,
    },
    // A peer that refused a pull leaves a reason the owner can read. Kept out of
    // migration 10 because that migration had already shipped to tills in the
    // field; editing an applied migration is the one thing this table forbids.
    Migration {
        version: 11,
        name: "sync_peer_errors",
        sql: r#"
            ALTER TABLE sync_peers ADD COLUMN last_error TEXT;
        "#,
    },
    // What the shop set out to buy, and what came back on the trip.
    //
    // A purchase is not an inventory movement: it records intent (a list for
    // the market) and the receiving screen is the only place that writes stock.
    // That split is what lets a planned list exist without claiming the goods
    // are already on the shelf.
    //
    // `status` has two values, not a lifecycle. A purchase is either still a
    // plan or it is closed. This migration also allowed a plan to be counted
    // against without being closed, which is what migration 15 repairs; nothing
    // since opens that door, so receiving is one act, not a half state.
    // `unit_cost_minor` is nullable because a market trip often has no clean
    // per-unit price and a missing cost must never block receiving.
    //
    // Each line snapshots the product name for the same reason a sale line
    // does: the catalog can be archived or renamed, and the purchase must keep
    // reading as what was bought. `purchase_line_id` on the ledger is the link
    // back from a movement to the trip that caused it.
    //
    // `products.cost_minor` is the last price paid, kept denormalized so the
    // next purchase can prefill it and the catalog can show a margin without a
    // join through history. `target_stock` is what "Add low-stock items"
    // aims for; NULL falls back to twice the warning line.
    //
    // `sale_lines.cost_minor` is a snapshot taken at checkout. Profit has to be
    // historical: recomputing it from today's cost would rewrite last month's
    // margin every time a supplier changes their price.
    Migration {
        version: 12,
        name: "purchases",
        sql: r#"
            CREATE TABLE IF NOT EXISTS purchases (
                id          INTEGER PRIMARY KEY,
                uid         TEXT,
                supplier_id TEXT REFERENCES suppliers(id),
                status      TEXT NOT NULL DEFAULT 'planned'
                            CHECK (status IN ('planned', 'received')),
                note        TEXT,
                received_at TEXT,
                created_at  TEXT NOT NULL DEFAULT (datetime('now')),
                user_id     TEXT REFERENCES users(id)
            );

            CREATE INDEX IF NOT EXISTS purchases_status
                ON purchases (status, id DESC);

            CREATE TABLE IF NOT EXISTS purchase_lines (
                id              INTEGER PRIMARY KEY,
                uid             TEXT,
                purchase_id     INTEGER NOT NULL
                                REFERENCES purchases(id) ON DELETE CASCADE,
                product_id      INTEGER REFERENCES products(id) ON DELETE SET NULL,
                name_snapshot   TEXT    NOT NULL CHECK (length(name_snapshot) > 0),
                qty_ordered     INTEGER NOT NULL CHECK (qty_ordered > 0),
                qty_received    INTEGER NOT NULL DEFAULT 0
                                CHECK (qty_received >= 0),
                unit_cost_minor INTEGER CHECK (unit_cost_minor >= 0),
                position        INTEGER NOT NULL DEFAULT 0
            );

            CREATE INDEX IF NOT EXISTS purchase_lines_purchase
                ON purchase_lines (purchase_id, position);

            ALTER TABLE stock_movements ADD COLUMN purchase_line_id INTEGER
                REFERENCES purchase_lines(id);

            ALTER TABLE products ADD COLUMN cost_minor INTEGER
                CHECK (cost_minor >= 0);
            ALTER TABLE products ADD COLUMN target_stock INTEGER
                CHECK (target_stock >= 0);

            ALTER TABLE sale_lines ADD COLUMN cost_minor INTEGER
                CHECK (cost_minor >= 0);
        "#,
    },
    // The drawer: one row per stretch of time the till is accountable for cash.
    //
    // A shift is not a rota and not a person. It opens by itself on the first
    // sale of the day and closes when the cash is counted, so the operator is
    // never asked to start work before serving a customer. Who rang up each
    // sale is already on `sales.user_id`; this table only knows about money.
    //
    // `float_minor` is the change that was in the drawer before the first sale.
    // It is copied from settings at open rather than read at close, because the
    // count must be compared against what was actually there that day, not
    // against whatever the setting says months later.
    //
    // The partial unique index is the rule "one open drawer", enforced where it
    // cannot be forgotten: a second insert while one is open fails. `uid` is the
    // identity a sale carries, so a sale that reaches another till does not
    // point at that till's row 7.
    Migration {
        version: 13,
        name: "cash_drawer",
        sql: r#"
            CREATE TABLE IF NOT EXISTS shifts (
                id            INTEGER PRIMARY KEY,
                uid           TEXT    NOT NULL,
                opened_at     TEXT    NOT NULL DEFAULT (datetime('now')),
                closed_at     TEXT,
                opened_by     TEXT REFERENCES users(id),
                closed_by     TEXT REFERENCES users(id),
                float_minor   INTEGER NOT NULL DEFAULT 0 CHECK (float_minor >= 0),
                counted_minor INTEGER CHECK (counted_minor >= 0)
            );

            CREATE UNIQUE INDEX IF NOT EXISTS shifts_uid ON shifts (uid);
            CREATE UNIQUE INDEX IF NOT EXISTS shifts_one_open
                ON shifts ((closed_at IS NULL)) WHERE closed_at IS NULL;
            CREATE INDEX IF NOT EXISTS shifts_closed ON shifts (closed_at);

            ALTER TABLE sales ADD COLUMN shift_uid TEXT;
            CREATE INDEX IF NOT EXISTS sales_shift
                ON sales (shift_uid) WHERE shift_uid IS NOT NULL;
        "#,
    },
    // A receipt number the customer can quote, and the till can find.
    //
    // The row id was a local counter: two tills syncing to each other both have
    // a sale 42, and the slip in the customer's hand has to be one number. The
    // number is written when the sale is, and never recomputed — a receipt that
    // renumbers itself when a peer's sale arrives is not a receipt.
    //
    // Nullable, and left null on every row already written: those slips were
    // printed with `#42` on them, and the archive has to keep saying `#42`. New
    // sales carry `<device>-<day>-<seq>`.
    Migration {
        version: 14,
        name: "receipt_numbers",
        sql: r#"
            ALTER TABLE sales ADD COLUMN receipt_no TEXT;
            CREATE UNIQUE INDEX IF NOT EXISTS sales_receipt_no
                ON sales (origin_device, receipt_no) WHERE receipt_no IS NOT NULL;
        "#,
    },
    // Close the half-received trips an earlier build left open.
    //
    // Migration 12 let a plan sit at `planned` while lines were counted against
    // it, so a tills-in-the-field database can still hold "3 of 5 bought" with
    // no way to finish: receiving again would add the first three to the shelf a
    // second time. A trip is one act now, so those rows are what they always
    // were in fact — a finished trip that came home short — and that is what
    // this records. `received_at` comes from the last movement that actually
    // arrived, falling back to the day the list was written, because the
    // receiving happened whenever the stock moved, not now.
    //
    // The two of five that never came home are not invented as a new list: the
    // shortfall reads on the closed trip, and the next trip is the operator's to
    // plan. No ledger row is written or removed here — only the status that made
    // a second receive possible.
    Migration {
        version: 15,
        name: "close_half_received_purchases",
        sql: r#"
            UPDATE purchases
               SET status = 'received',
                   received_at = COALESCE(
                       (SELECT MAX(m.created_at)
                          FROM stock_movements m
                          JOIN purchase_lines l ON l.id = m.purchase_line_id
                         WHERE l.purchase_id = purchases.id),
                       created_at
                   )
             WHERE status = 'planned'
               AND EXISTS (
                       SELECT 1 FROM purchase_lines l
                        WHERE l.purchase_id = purchases.id AND l.qty_received > 0
                   );
        "#,
    },
    // A product sold loose and in a sealed pack.
    //
    // The base unit — the smallest thing that can leave the shelf — stays the
    // row. `pack_size` is how many base units one sealed pack holds, and the
    // pack is priced and barcoded separately on the same row. A product that
    // only ever sells as a pack uses `pack_size = 1` and `unit_label = 'box'`;
    // there is no second catalog row for the same goods.
    //
    // Cost is now the truth per base unit, in thousandths of a minor unit
    // (`cost_milli`). A pack of 100 costing 1,850 costs 18.5 per tablet, and an
    // integer per-base column would round that away. `cost_minor` stays for
    // older builds but nothing here reads it again; the backfill keeps a
    // database that has been through this migration able to answer margin.
    //
    // Sale lines snapshot the factor and label, like the name and price, so a
    // receipt reprinted after the pack size changes still reads as the sale
    // that happened. `unit_factor` is how many base units one line unit held:
    // one tablet is 1, a pack of 100 is 100.
    //
    // The purchase line carries its own `pack_size` rather than inheriting one:
    // the shop may buy loose tablets one week and a sealed box the next, and
    // the receiving screen is the only place that knows which it did.
    //
    // A pack barcode needs its own partial unique index. The existing index is
    // on `products.barcode`; a second index cannot stop a pack code colliding
    // with a loose code, so the write path and the catalog import check across
    // both columns before either index has to speak.
    Migration {
        version: 16,
        name: "packs_and_units",
        sql: r#"
            ALTER TABLE products ADD COLUMN unit_label TEXT NOT NULL DEFAULT 'unit'
                CHECK (length(trim(unit_label)) > 0);
            ALTER TABLE products ADD COLUMN pack_size INTEGER NOT NULL DEFAULT 1
                CHECK (pack_size >= 1);
            ALTER TABLE products ADD COLUMN pack_price_minor INTEGER
                CHECK (pack_price_minor >= 0);
            ALTER TABLE products ADD COLUMN pack_barcode TEXT;
            ALTER TABLE products ADD COLUMN cost_milli INTEGER
                CHECK (cost_milli >= 0);

            UPDATE products
               SET cost_milli = cost_minor * 1000
             WHERE cost_minor IS NOT NULL;

            CREATE UNIQUE INDEX IF NOT EXISTS products_pack_barcode
                ON products (pack_barcode)
                WHERE pack_barcode IS NOT NULL AND archived = 0;

            ALTER TABLE sale_lines ADD COLUMN unit_factor INTEGER NOT NULL DEFAULT 1
                CHECK (unit_factor >= 1);
            ALTER TABLE sale_lines ADD COLUMN unit_label TEXT;
            ALTER TABLE sale_lines ADD COLUMN cost_milli INTEGER
                CHECK (cost_milli >= 0);

            UPDATE sale_lines
               SET cost_milli = cost_minor * 1000
             WHERE cost_minor IS NOT NULL;

            ALTER TABLE purchase_lines ADD COLUMN pack_size INTEGER NOT NULL DEFAULT 1
                CHECK (pack_size >= 1);
        "#,
    },
    Migration {
        version: 17,
        name: "product_expiry",
        sql: r#"
            ALTER TABLE products ADD COLUMN perishable INTEGER NOT NULL DEFAULT 0
                CHECK (perishable IN (0, 1));
            ALTER TABLE products ADD COLUMN shelf_life_days INTEGER
                CHECK (shelf_life_days IS NULL OR shelf_life_days > 0);

            ALTER TABLE purchase_lines ADD COLUMN expires_on TEXT;
            ALTER TABLE stock_movements ADD COLUMN expires_on TEXT;

            CREATE INDEX IF NOT EXISTS stock_movements_expiry
                ON stock_movements (product_id, expires_on)
                WHERE expires_on IS NOT NULL;
        "#,
    },
    // Quantity breaks: the price of one loose unit once the line is long.
    //
    // A break belongs to the product and starts at the quantity it names. The
    // shelf price stays `products.price_minor` and is the rung below the first
    // break, which is why a break cannot start at one unit: a rung at one would
    // be a second name for the shelf price.
    //
    // Packs are left out on purpose and have no rows here. `pack_price_minor`
    // is already the bulk deal, priced for the whole carton; a break on a pack
    // line would stack a second discount on the first until the box price
    // meant nothing. The till only consults these rows for loose lines.
    Migration {
        version: 18,
        name: "price_breaks",
        sql: r#"
            CREATE TABLE IF NOT EXISTS price_breaks (
                product_id  INTEGER NOT NULL REFERENCES products (id) ON DELETE CASCADE,
                min_qty     INTEGER NOT NULL CHECK (min_qty >= 2),
                price_minor INTEGER NOT NULL CHECK (price_minor >= 0),
                PRIMARY KEY (product_id, min_qty)
            );
        "#,
    },
    // The shop's regular customers, and the rate they buy at.
    //
    // A customer is a pricing term, not a loyalty record: the till needs a name
    // to show the cashier, and a rate to take off the basket. The rate is basis
    // points rather than percent because five percent is 500 and two and a half
    // is 250; an integer percent would round the shop's own terms.
    //
    // `archived` mirrors products: a customer who stops coming is hidden, not
    // deleted, because sales written under them still point here. The partial
    // unique index keeps two live customers from sharing a name while letting
    // the archived one keep it.
    //
    // The table is catalog, not a till's own list: it syncs to replicas with
    // products, and the triggers below keep the catalog version honest.
    Migration {
        version: 19,
        name: "customers",
        sql: r#"
            CREATE TABLE IF NOT EXISTS customers (
                id          INTEGER PRIMARY KEY,
                name        TEXT    NOT NULL CHECK (length(trim(name)) > 0),
                phone       TEXT,
                discount_bp INTEGER NOT NULL DEFAULT 0
                    CHECK (discount_bp >= 0 AND discount_bp <= 10000),
                archived    INTEGER NOT NULL DEFAULT 0
                    CHECK (archived IN (0, 1)),
                created_at  TEXT    NOT NULL DEFAULT (datetime('now'))
            );

            CREATE UNIQUE INDEX IF NOT EXISTS customers_name
                ON customers (name COLLATE NOCASE) WHERE archived = 0;
            CREATE INDEX IF NOT EXISTS customers_archived ON customers (archived);

            CREATE TRIGGER IF NOT EXISTS customers_catalog_version_insert
            AFTER INSERT ON customers
            BEGIN
                UPDATE meta SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
                WHERE key = 'catalog_version';
            END;

            CREATE TRIGGER IF NOT EXISTS customers_catalog_version_update
            AFTER UPDATE ON customers
            BEGIN
                UPDATE meta SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
                WHERE key = 'catalog_version';
            END;
        "#,
    },
    // A basket-level discount, recorded as money and as the rate that made it.
    //
    // `total_minor` stops being the exact sum of the lines and becomes what the
    // customer was charged: the line sum less `discount_minor`. The lines keep
    // their gross totals, so item revenue in a report stays the price on the
    // shelf and the discount is a separate figure the books subtract once.
    // Storing both halves is what lets a receipt reconcile and a peer verify.
    //
    // `discount_bp` is the rate as applied, kept for reporting and for the
    // receipt's "Discount (5%)"; it is zero on every sale written before this
    // migration, and on a wholesale sale where the customer's rate is
    // suppressed. `price_mode` says which of those two a sale was, because a
    // wholesale basket is not a retail basket with no discount.
    //
    // `customer_name` is a snapshot, like a line's name: a receipt reprinted
    // after a rename must still say who was standing there. `customer_id` is
    // only a link for reporting, and it is `SET NULL` so removing a customer
    // never rewrites a sale.
    Migration {
        version: 20,
        name: "sale_discount",
        sql: r#"
            ALTER TABLE sales ADD COLUMN discount_minor INTEGER NOT NULL DEFAULT 0
                CHECK (discount_minor >= 0);
            ALTER TABLE sales ADD COLUMN discount_bp INTEGER NOT NULL DEFAULT 0
                CHECK (discount_bp >= 0 AND discount_bp <= 10000);
            ALTER TABLE sales ADD COLUMN price_mode TEXT NOT NULL DEFAULT 'retail'
                CHECK (price_mode IN ('retail', 'wholesale'));
            ALTER TABLE sales ADD COLUMN customer_id INTEGER
                REFERENCES customers (id) ON DELETE SET NULL;
            ALTER TABLE sales ADD COLUMN customer_name TEXT;

            CREATE INDEX IF NOT EXISTS sales_customer
                ON sales (customer_id) WHERE customer_id IS NOT NULL;
        "#,
    },
    // A till that took a mobile-money tender can record the transaction id the
    // customer's phone shows. It is the only handle a shop has when a payment
    // is later disputed, and it costs nothing on a cash sale to leave it NULL.
    // The till never verifies the id; it is a note on the sale, not a receipt
    // from the network.
    Migration {
        version: 21,
        name: "payment_reference",
        sql: r#"
            ALTER TABLE payments ADD COLUMN reference TEXT;
        "#,
    },
    // A sale can be recorded "on credit": the customer takes the goods now and
    // pays later. When `is_on_credit` is set, `record_sale` allows zero tenders
    // and records the sale with `paid_minor` equal to whatever cash was actually
    // received (which may be zero). The outstanding balance is tracked as
    // `total_minor - paid_minor`.
    Migration {
        version: 22,
        name: "sale_on_credit",
        sql: r#"
            ALTER TABLE sales ADD COLUMN is_on_credit INTEGER NOT NULL DEFAULT 0
                CHECK (is_on_credit IN (0, 1));
        "#,
    },
    // A payment taken later belongs to the shift and the cashier who took it,
    // not to the sale being paid off. Without its own attribution the money
    // would land in the drawer that has already been counted and closed.
    Migration {
        version: 23,
        name: "payment_attribution",
        sql: r#"
            ALTER TABLE payments ADD COLUMN shift_uid TEXT;
            ALTER TABLE payments ADD COLUMN user_id TEXT;
            -- SQLite rejects a non-constant default in ADD COLUMN, so the
            -- timestamp is nullable here and written by the insert itself.
            ALTER TABLE payments ADD COLUMN created_at TEXT;
            UPDATE payments
               SET shift_uid = (SELECT shift_uid FROM sales WHERE sales.id = payments.sale_id),
                   user_id   = (SELECT user_id FROM sales WHERE sales.id = payments.sale_id),
                   created_at = COALESCE(
                       (SELECT created_at FROM sales WHERE sales.id = payments.sale_id),
                       datetime('now')
                   );
            CREATE INDEX IF NOT EXISTS payments_shift ON payments(shift_uid)
                WHERE shift_uid IS NOT NULL;
        "#,
    },
];
