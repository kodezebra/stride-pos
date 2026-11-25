//! Catalog rows: products, the categories they sit in, and the editor's field
//! bundle with its validation.
//!
//! `ProductFields` is a struct rather than eight arguments because two fields
//! are optional money and two are optional barcodes; a swapped pair would still
//! compile. `CleanProduct` is what survives validation.

use super::*;

/// Every live product, by name.
///
/// The list is small enough to keep in memory and filter there, which is why
/// the search box does not run a query per keystroke. A catalog big enough to
/// change that is a catalog that needs the query, and that is the moment to
/// move the filter into SQL.
///
/// Archived products are excluded here and still reachable by the sync snapshot,
/// which reads the table directly: the shop hid them, a peer still has to learn
/// that they are gone.
pub fn products(store: &storage::Store) -> Vec<Product> {
    let mut statement = match store.conn().prepare(
        "SELECT id, name, price_minor, category, category_id, barcode,
                unit_label, pack_size, pack_price_minor, pack_barcode, cost_milli
         FROM products WHERE archived = 0 ORDER BY name COLLATE NOCASE",
    ) {
        Ok(statement) => statement,
        Err(err) => {
            tracing::warn!(error = %err, "products prepare failed");
            return Vec::new();
        }
    };

    let rows = statement.query_map([], |row| {
        Ok(Product {
            id: row.get(0)?,
            name: row.get(1)?,
            price_minor: row.get(2)?,
            category: row.get(3)?,
            category_id: row.get(4)?,
            barcode: row.get(5)?,
            unit_label: row.get(6)?,
            pack_size: row.get(7)?,
            pack_price_minor: row.get(8)?,
            pack_barcode: row.get(9)?,
            cost_milli: row.get(10)?,
            tiers: Vec::new(),
        })
    });

    let mut products: Vec<Product> = match rows {
        Ok(rows) => rows.filter_map(Result::ok).collect(),
        Err(err) => {
            tracing::warn!(error = %err, "products query failed");
            Vec::new()
        }
    };

    let mut breaks = breaks_by_product(store);
    for product in &mut products {
        product.tiers = breaks.remove(&product.id).unwrap_or_default();
    }
    products
}

/// Every product's price breaks, grouped by product.
///
/// One statement for the whole catalog rather than one per product: the list is
/// reloaded whenever the catalog changes, and a query per row would be the only
/// thing on that screen costing a statement per product.
fn breaks_by_product(store: &storage::Store) -> std::collections::HashMap<i64, Vec<PriceTier>> {
    let mut grouped: std::collections::HashMap<i64, Vec<PriceTier>> =
        std::collections::HashMap::new();

    let mut statement = match store.conn().prepare(
        "SELECT product_id, min_qty, price_minor FROM price_breaks
         ORDER BY product_id, min_qty",
    ) {
        Ok(statement) => statement,
        Err(err) => {
            tracing::warn!(error = %err, "price breaks prepare failed");
            return grouped;
        }
    };

    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            PriceTier {
                min_qty: row.get(1)?,
                price_minor: row.get(2)?,
            },
        ))
    });

    match rows {
        Ok(rows) => {
            for (product_id, tier) in rows.filter_map(Result::ok) {
                grouped.entry(product_id).or_default().push(tier);
            }
        }
        Err(err) => tracing::warn!(error = %err, "price breaks query failed"),
    }

    grouped
}

/// The shelves the sidebars show, in the shop's order, with a live count.
///
/// The `categories` table is the source of the name and the order; the count
/// comes from the denormalized name on each live product, so a product with no
/// `category_id` still lands on the shelf its name says. Names that only exist
/// on products — an older import, or a database from before the table — are
/// appended rather than dropped. Hiding them would strand those products behind
/// a search box.
pub fn categories(store: &storage::Store) -> Vec<CategorySummary> {
    let mut shelves = Vec::new();

    let table = store.conn().prepare(
        "SELECT c.name, c.sort_order,
                (SELECT COUNT(*) FROM products p
                  WHERE p.archived = 0 AND p.category = c.name COLLATE NOCASE)
           FROM categories c
          ORDER BY c.sort_order, c.name COLLATE NOCASE",
    );
    match table {
        Ok(mut statement) => match statement.query_map([], |row| {
            Ok(CategorySummary {
                name: row.get(0)?,
                sort_order: row.get(1)?,
                count: row.get::<_, i64>(2)? as usize,
            })
        }) {
            Ok(rows) => shelves.extend(rows.filter_map(Result::ok)),
            Err(err) => tracing::warn!(error = %err, "categories query failed"),
        },
        Err(err) => tracing::warn!(error = %err, "categories prepare failed"),
    }

    let extra = store.conn().prepare(
        "SELECT p.category, COUNT(*)
           FROM products p
          WHERE p.archived = 0
            AND NOT EXISTS (SELECT 1 FROM categories c
                             WHERE c.name = p.category COLLATE NOCASE)
          GROUP BY p.category COLLATE NOCASE
          ORDER BY p.category COLLATE NOCASE",
    );
    match extra {
        Ok(mut statement) => match statement.query_map([], |row| {
            Ok(CategorySummary {
                name: row.get(0)?,
                sort_order: 0,
                count: row.get::<_, i64>(1)? as usize,
            })
        }) {
            Ok(rows) => shelves.extend(rows.filter_map(Result::ok)),
            Err(err) => tracing::warn!(error = %err, "product-only categories query failed"),
        },
        Err(err) => tracing::warn!(error = %err, "product-only categories prepare failed"),
    }

    shelves
}

/// Every row in the `categories` table, by name.
///
/// Used by the product form's dropdown: these are the categories the shop has
/// actually created, not the seed list the app invented.
pub fn categories_list(store: &storage::Store) -> Vec<Category> {
    let mut statement = match store
        .conn()
        .prepare("SELECT id, name, sort_order, color FROM categories ORDER BY sort_order, name COLLATE NOCASE")
    {
        Ok(statement) => statement,
        Err(err) => {
            tracing::warn!(error = %err, "categories_list prepare failed");
            return Vec::new();
        }
    };

    let rows = statement.query_map([], |row| {
        Ok(Category {
            id: row.get(0)?,
            name: row.get(1)?,
            sort_order: row.get(2)?,
            color: row.get(3)?,
        })
    });

    match rows {
        Ok(rows) => rows.filter_map(Result::ok).collect(),
        Err(err) => {
            tracing::warn!(error = %err, "categories_list query failed");
            Vec::new()
        }
    }
}

/// Look up a category by name, inserting it when it does not exist.
///
/// Returns the category id. Case-insensitive lookup because "Vitamins" and
/// "vitamins" are the same shelf, and a second product typed by hand should not
/// create a duplicate.
pub fn find_or_insert_category(store: &storage::Store, name: &str) -> anyhow::Result<i64> {
    insert_category(store, name, 0)
}

/// The same lookup, with the place a new shelf takes in the sidebar.
///
/// A shelf the shop already has keeps its order: coming back through the form
/// must not reshuffle the sidebars. The order is only written on the row that is
/// actually new, which is how the suggested list can arrive pre-ordered.
pub fn insert_category(store: &storage::Store, name: &str, sort_order: i64) -> anyhow::Result<i64> {
    let name = name.trim();
    anyhow::ensure!(!name.is_empty(), "a category needs a name");

    // Try to find an existing one (case-insensitive).
    if let Ok(found) = store.conn().query_row(
        "SELECT id FROM categories WHERE name = ?1 COLLATE NOCASE",
        [name],
        |row| row.get::<_, i64>(0),
    ) {
        return Ok(found);
    }

    // Insert it.
    store.conn().execute(
        "INSERT INTO categories (name, sort_order) VALUES (?1, ?2)",
        (name, sort_order),
    )?;
    Ok(store.conn().last_insert_rowid())
}

/// Add a product and return its id.
///
/// `category_id` links to the `categories` table when the product was saved
/// with a known shelf; `None` when it was saved with free text that was never
/// added to the categories table.
pub fn insert_product(store: &storage::Store, fields: &ProductFields<'_>) -> anyhow::Result<i64> {
    let product = fields.clean()?;
    product.ensure_barcodes_free(store, None)?;

    let transaction = store.conn().unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO products
             (name, price_minor, category, category_id, barcode,
              unit_label, pack_size, pack_price_minor, pack_barcode)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            product.name,
            fields.price_minor,
            product.category,
            fields.category_id,
            product.barcode,
            product.unit_label,
            product.pack_size,
            product.pack_price_minor,
            product.pack_barcode,
        ],
    )?;
    let id = transaction.last_insert_rowid();
    write_tiers(&transaction, id, &product.tiers)?;
    transaction.commit()?;
    Ok(id)
}

/// Replace a product's editable fields.
///
/// Returns an error when the row is gone: the list on screen is a cache, and
/// "Saved" over a product that no longer exists is a lie the operator would
/// only discover on the next restart.
///
/// A product that has moved stock cannot change what its base unit is called:
/// "tablet" becoming "bottle" would silently reinterpret every count already in
/// the ledger. The pack size is free to change — the movement is in base units,
/// and each sale and purchase line snapshotted the factor it used — but the
/// thing the base unit names is not.
pub fn update_product(
    store: &storage::Store,
    id: i64,
    fields: &ProductFields<'_>,
) -> anyhow::Result<()> {
    let product = fields.clean()?;
    product.ensure_barcodes_free(store, Some(id))?;

    let existing = store
        .conn()
        .query_row(
            "SELECT unit_label FROM products WHERE id = ?1",
            [id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(existing_label) = existing else {
        anyhow::bail!("product {id} is no longer in the catalog");
    };

    if existing_label != product.unit_label {
        let moved: bool = store.conn().query_row(
            "SELECT EXISTS(SELECT 1 FROM stock_movements WHERE product_id = ?1)",
            [id],
            |row| row.get(0),
        )?;
        anyhow::ensure!(
            !moved,
            "this product has stock history, so its unit cannot be renamed. \
             Archive it and add the new unit as a product."
        );
    }

    let transaction = store.conn().unchecked_transaction()?;
    let changed = transaction.execute(
        "UPDATE products
         SET name = ?2, price_minor = ?3, category = ?4, category_id = ?5,
             barcode = ?6, unit_label = ?7, pack_size = ?8,
             pack_price_minor = ?9, pack_barcode = ?10
         WHERE id = ?1",
        rusqlite::params![
            id,
            product.name,
            fields.price_minor,
            product.category,
            fields.category_id,
            product.barcode,
            product.unit_label,
            product.pack_size,
            product.pack_price_minor,
            product.pack_barcode,
        ],
    )?;
    anyhow::ensure!(changed == 1, "product {id} is no longer in the catalog");
    write_tiers(&transaction, id, &product.tiers)?;
    transaction.commit()?;
    Ok(())
}

/// Archive a product.
///
/// The row stays: sale lines and stock movements point at it, and those are the
/// record of what happened. "Delete" is what the operator asked for, so the
/// screen says removed and the catalog list stops showing it; the database
/// keeps the identity a report or a peer needs.
pub fn delete_product(store: &storage::Store, id: i64) -> anyhow::Result<()> {
    store.conn().execute(
        "UPDATE products SET archived = 1 WHERE id = ?1 AND archived = 0",
        [id],
    )?;
    Ok(())
}

/// Whether a barcode already belongs to a product other than `except`.
///
/// Checked before the write so the operator gets "that barcode is taken"
/// instead of a constraint error. Both columns are searched: a pack code that
/// matches another row's loose code is the same scanning bug, and each column's
/// unique index is blind to the other. The unique indexes still enforce each
/// column on their own.
pub fn barcode_taken(store: &storage::Store, barcode: &str, except: Option<i64>) -> bool {
    store
        .conn()
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM products
                 WHERE (barcode = ?1 OR pack_barcode = ?1)
                   AND id IS NOT ?2 AND archived = 0
             )",
            (barcode, except),
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(true)
}

/// The editable facts of a product, as the form and the importer collect them.
///
/// A struct rather than eight positional arguments: two fields are optional
/// money and two are optional barcodes, and a call site that swapped them would
/// still compile.
pub struct ProductFields<'a> {
    pub name: &'a str,
    pub price_minor: i64,
    pub category: &'a str,
    pub category_id: Option<i64>,
    pub barcode: Option<&'a str>,
    pub unit_label: &'a str,
    pub pack_size: i64,
    pub pack_price_minor: Option<i64>,
    pub pack_barcode: Option<&'a str>,
    /// The product's loose-unit price breaks. Empty when the shop prices one
    /// way, and never used for a pack, whose price is already the bulk deal.
    pub tiers: &'a [PriceTier],
}

/// A product after trimming, defaulting and the pack-of-one rule.
pub(super) struct CleanProduct {
    name: String,
    category: String,
    unit_label: String,
    pack_size: i64,
    pack_price_minor: Option<i64>,
    barcode: Option<String>,
    pack_barcode: Option<String>,
    tiers: Vec<PriceTier>,
}

impl ProductFields<'_> {
    fn clean(&self) -> anyhow::Result<CleanProduct> {
        let name = self.name.trim();
        anyhow::ensure!(!name.is_empty(), "a product needs a name");
        anyhow::ensure!(self.price_minor >= 0, "a price cannot be negative");

        let category = self.category.trim();
        let category = if category.is_empty() {
            "General"
        } else {
            category
        };

        let unit_label = self.unit_label.trim();
        let unit_label = if unit_label.is_empty() {
            "unit"
        } else {
            unit_label
        };

        anyhow::ensure!(self.pack_size >= 1, "a pack holds at least one unit");
        anyhow::ensure!(
            self.pack_price_minor.is_none_or(|price| price >= 0),
            "a pack price cannot be negative"
        );

        let barcode = self
            .barcode
            .map(str::trim)
            .filter(|barcode| !barcode.is_empty())
            .map(str::to_owned);

        // A pack of one is the base unit. Keeping a second price and code for
        // it would be a second name for the same thing.
        let (pack_price_minor, pack_barcode) = if self.pack_size <= 1 {
            (None, None)
        } else {
            (
                self.pack_price_minor,
                self.pack_barcode
                    .map(str::trim)
                    .filter(|barcode| !barcode.is_empty())
                    .map(str::to_owned),
            )
        };

        anyhow::ensure!(
            barcode.is_none() || barcode != pack_barcode,
            "the loose and pack barcodes cannot be the same"
        );

        let tiers = clean_tiers(self.price_minor, self.tiers)?;

        Ok(CleanProduct {
            name: name.to_owned(),
            category: category.to_owned(),
            unit_label: unit_label.to_owned(),
            pack_size: self.pack_size,
            pack_price_minor,
            barcode,
            pack_barcode,
            tiers,
        })
    }
}

/// Sort and check a product's price breaks.
///
/// The ladder starts at the shelf price, so a break cannot start at one unit
/// and cannot cost more than the rung below it: a customer who buys more and
/// pays more is a typo, not a price. A rung at the same price as the one below
/// is allowed — a shop sometimes wants the till to say "10+ price" without
/// changing the amount — and so is the same price on every rung.
pub(crate) fn clean_tiers(base_minor: i64, tiers: &[PriceTier]) -> anyhow::Result<Vec<PriceTier>> {
    let mut sorted = tiers.to_vec();
    sorted.sort_by_key(|tier| tier.min_qty);

    let mut floor_minor = base_minor;
    let mut previous_qty = 1;
    for tier in &sorted {
        anyhow::ensure!(tier.min_qty >= 2, "a price break starts at two units");
        anyhow::ensure!(
            tier.min_qty > previous_qty,
            "two price breaks start at {previous_qty} units"
        );
        anyhow::ensure!(tier.price_minor >= 0, "a price break cannot be negative");
        let floor = if previous_qty == 1 {
            "shelf price".to_owned()
        } else {
            format!("{previous_qty}-unit price")
        };
        anyhow::ensure!(
            tier.price_minor <= floor_minor,
            "the {}-unit price cannot be above the {floor}",
            tier.min_qty
        );
        previous_qty = tier.min_qty;
        floor_minor = tier.price_minor;
    }

    Ok(sorted)
}

/// Replace a product's price breaks. Callers pass rungs that came through
/// [`clean_tiers`], so this only writes them.
fn write_tiers(
    conn: &rusqlite::Connection,
    product_id: i64,
    tiers: &[PriceTier],
) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM price_breaks WHERE product_id = ?1",
        [product_id],
    )?;
    for tier in tiers {
        conn.execute(
            "INSERT INTO price_breaks (product_id, min_qty, price_minor)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![product_id, tier.min_qty, tier.price_minor],
        )?;
    }
    Ok(())
}

impl CleanProduct {
    /// Both codes are checked against both columns: a pack code that collides
    /// with a loose code on another row scans the wrong thing, and the two
    /// unique indexes cannot see each other. Only the codes that survive
    /// `clean` are checked, so a pack code dropped by a pack of one cannot
    /// block the save.
    fn ensure_barcodes_free(
        &self,
        store: &storage::Store,
        except: Option<i64>,
    ) -> anyhow::Result<()> {
        for barcode in [self.barcode.as_deref(), self.pack_barcode.as_deref()]
            .into_iter()
            .flatten()
        {
            anyhow::ensure!(
                !barcode_taken(store, barcode, except),
                "another product already uses the barcode {barcode}"
            );
        }
        Ok(())
    }
}
