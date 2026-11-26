CREATE TABLE meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE products (
  id               INTEGER PRIMARY KEY,
  name             TEXT NOT NULL,
  price_minor      INTEGER NOT NULL,
  barcode          TEXT,
  sku              TEXT,
  category         TEXT NOT NULL,
  category_id      INTEGER,
  tracked          INTEGER NOT NULL DEFAULT 0,
  archived         INTEGER NOT NULL DEFAULT 0,
  unit_label       TEXT NOT NULL DEFAULT 'unit',
  pack_size        INTEGER NOT NULL DEFAULT 1,
  pack_price_minor INTEGER,
  pack_barcode     TEXT
);
CREATE UNIQUE INDEX products_barcode      ON products(barcode)      WHERE barcode IS NOT NULL;
CREATE UNIQUE INDEX products_pack_barcode ON products(pack_barcode) WHERE pack_barcode IS NOT NULL;

CREATE TABLE categories (
  id         INTEGER PRIMARY KEY,
  name       TEXT NOT NULL,
  sort_order INTEGER NOT NULL DEFAULT 0,
  color      TEXT
);

CREATE TABLE staff (
  id   TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  role TEXT NOT NULL
);

CREATE TABLE customers (
  id          INTEGER PRIMARY KEY,
  name        TEXT NOT NULL,
  phone       TEXT,
  discount_bp INTEGER NOT NULL DEFAULT 0,
  archived    INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE sales (
  uid            TEXT PRIMARY KEY,
  origin_device  TEXT NOT NULL,
  occurred_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  total_minor    INTEGER NOT NULL,
  discount_minor INTEGER NOT NULL DEFAULT 0,
  discount_bp    INTEGER NOT NULL DEFAULT 0,
  price_mode     TEXT NOT NULL DEFAULT 'retail',
  customer_id    INTEGER,
  customer_name  TEXT,
  cashier        TEXT NOT NULL,
  tender         TEXT NOT NULL,
  reference      TEXT,
  is_on_credit   INTEGER NOT NULL DEFAULT 0,
  status         TEXT NOT NULL DEFAULT 'pending',
  unverified     INTEGER NOT NULL DEFAULT 0,
  held_reason    TEXT,
  attempts       INTEGER NOT NULL DEFAULT 0,
  last_error     TEXT,
  synced_at      TEXT,
  receipt_no     TEXT
);
CREATE INDEX sales_status ON sales(status);

CREATE TABLE sale_lines (
  uid         TEXT PRIMARY KEY,
  sale_uid    TEXT NOT NULL REFERENCES sales(uid) ON DELETE CASCADE,
  product_id  INTEGER,
  name        TEXT NOT NULL,
  qty         INTEGER NOT NULL,
  unit_minor  INTEGER NOT NULL,
  total_minor INTEGER NOT NULL,
  unit_factor INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX sale_lines_sale ON sale_lines(sale_uid);
