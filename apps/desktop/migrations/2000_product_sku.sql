-- A code the shop owns, beside the barcode the manufacturer printed.
--
-- A barcode is not a reorder key: many goods arrive without one, and the same
-- packet can carry different codes from one supplier to the next. The SKU is
-- the shop's own name for the row, typed once and kept through repricing, and
-- it is what an "update from CSV" file matches on.
--
-- NULL on every existing row: nobody has typed a code yet, and a blank code
-- must not read as a duplicate. The partial index keeps two live products from
-- sharing one while letting an archived row keep its old code for reuse.
ALTER TABLE products ADD COLUMN sku TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS products_sku
    ON products (sku COLLATE NOCASE)
   WHERE sku IS NOT NULL AND archived = 0;
