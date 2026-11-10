package com.stride.scan

import android.content.ContentValues
import android.content.Context
import android.database.Cursor
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper
import org.json.JSONObject

data class CachedProduct(
    val id: Long,
    val name: String,
    val priceMinor: Long,
    val barcode: String?,
    val category: String,
    val tracked: Boolean,
)

data class StaffMember(val id: String, val name: String, val role: String)

data class CatalogSettings(
    val currency: String,
    val exponent: Int,
    val cashStepMinor: Long,
    val taxRatePercent: Int,
)

data class PendingSale(val uid: String, val payload: String)

data class RejectedSale(val uid: String, val reason: String, val occurredAt: String)

/**
 * The phone's own database: a cached catalogue and the cash outbox.
 *
 * Nothing here is authoritative. The catalogue is a copy of the last snapshot
 * the till served, and the outbox is the phone's record of what it sold while
 * the desktop was away. The outbox is therefore never dropped on upgrade:
 * those rows are money.
 */
class TillDb(context: Context) : SQLiteOpenHelper(context, NAME, null, VERSION) {
    override fun onCreate(db: SQLiteDatabase) {
        db.execSQL("CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
        db.execSQL(
            """CREATE TABLE products (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                price_minor INTEGER NOT NULL,
                barcode TEXT,
                category TEXT NOT NULL DEFAULT '',
                tracked INTEGER NOT NULL DEFAULT 0,
                archived INTEGER NOT NULL DEFAULT 0
            )""",
        )
        db.execSQL("CREATE INDEX products_barcode ON products (barcode)")
        db.execSQL(
            "CREATE TABLE staff (id TEXT PRIMARY KEY, name TEXT NOT NULL, role TEXT NOT NULL DEFAULT '')",
        )
        db.execSQL(
            """CREATE TABLE offline_sales (
                uid TEXT PRIMARY KEY,
                payload TEXT NOT NULL,
                occurred_at TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                attempts INTEGER NOT NULL DEFAULT 0,
                last_error TEXT,
                created_at INTEGER NOT NULL
            )""",
        )
        db.execSQL("CREATE INDEX offline_sales_status ON offline_sales (status, created_at)")
    }

    override fun onUpgrade(db: SQLiteDatabase, oldVersion: Int, newVersion: Int) {
        // Deliberately empty today. A future schema change must migrate
        // offline_sales in place, never drop it: those rows are cash. Ship the
        // migration with the version bump that needs it.
    }

    /**
     * Replace the cached catalogue with a snapshot from the till.
     *
     * Products are inserted including archived ones, so a product that is
     * switched off at the till cannot be sold here even if its barcode was
     * cached before. Sales already queued keep their own copy of the name and
     * price, so this sweep cannot change what they meant.
     */
    fun replaceCatalog(snapshot: JSONObject) {
        val db = writableDatabase
        db.beginTransaction()
        try {
            db.execSQL("DELETE FROM products")
            val products = snapshot.optJSONArray("products")
            if (products != null) {
                for (index in 0 until products.length()) {
                    val item = products.optJSONObject(index) ?: continue
                    val id = item.optLong("id", 0)
                    if (id <= 0) continue
                    db.execSQL(
                        "INSERT INTO products (id, name, price_minor, barcode, category, tracked, archived) " +
                            "VALUES (?, ?, ?, ?, ?, ?, ?)",
                        arrayOf<Any?>(
                            id,
                            item.optString("name", ""),
                            item.optLong("price_minor", 0),
                            if (item.isNull("barcode")) {
                                null
                            } else {
                                item.optString("barcode").takeIf { it.isNotBlank() }
                            },
                            item.optString("category", ""),
                            item.optInt("tracked", 0),
                            item.optInt("archived", 0),
                        ),
                    )
                }
            }

            db.execSQL("DELETE FROM staff")
            val staff = snapshot.optJSONArray("staff")
            if (staff != null) {
                for (index in 0 until staff.length()) {
                    val item = staff.optJSONObject(index) ?: continue
                    val id = item.optString("id", "")
                    if (id.isBlank()) continue
                    db.execSQL(
                        "INSERT OR REPLACE INTO staff (id, name, role) VALUES (?, ?, ?)",
                        arrayOf(id, item.optString("name", ""), item.optString("role", "")),
                    )
                }
            }

            putMeta(db, KEY_CATALOG_VERSION, snapshot.optLong("version", 0).toString())
            putMeta(db, KEY_CATALOG_CURRENCY, snapshot.optString("currency", ""))
            putMeta(db, KEY_CATALOG_EXPONENT, snapshot.optInt("currency_exponent", 0).toString())
            putMeta(db, KEY_CATALOG_CASH_STEP, snapshot.optLong("cash_step_minor", 1).toString())
            putMeta(db, KEY_CATALOG_TAX_RATE, snapshot.optInt("tax_rate_percent", 0).toString())
            putMeta(db, KEY_LAST_CATALOG_MS, System.currentTimeMillis().toString())
            db.setTransactionSuccessful()
        } finally {
            db.endTransaction()
        }
    }

    fun catalogSettings(): CatalogSettings {
        val values = HashMap<String, String>()
        readableDatabase.rawQuery("SELECT key, value FROM meta", null).use { cursor ->
            while (cursor.moveToNext()) values[cursor.getString(0)] = cursor.getString(1)
        }
        return CatalogSettings(
            currency = values[KEY_CATALOG_CURRENCY].orEmpty(),
            exponent = values[KEY_CATALOG_EXPONENT]?.toIntOrNull()?.coerceIn(0, 3) ?: 0,
            cashStepMinor = values[KEY_CATALOG_CASH_STEP]?.toLongOrNull()?.coerceAtLeast(1) ?: 1,
            taxRatePercent = values[KEY_CATALOG_TAX_RATE]?.toIntOrNull()?.coerceIn(0, 100) ?: 0,
        )
    }

    fun lastCatalogAt(): Long = meta(KEY_LAST_CATALOG_MS)?.toLongOrNull() ?: 0L

    fun searchProducts(query: String, limit: Int = 12): List<CachedProduct> {
        val term = query.trim()
        if (term.isEmpty()) return emptyList()
        val like = "%${escapeLike(term)}%"
        return readableDatabase.rawQuery(
            "SELECT id, name, price_minor, barcode, category, tracked FROM products " +
                "WHERE archived = 0 AND (name LIKE ? ESCAPE '\\' OR barcode = ?) " +
                "ORDER BY name COLLATE NOCASE LIMIT ?",
            arrayOf(like, term, limit.toString()),
        ).use { cursor ->
            buildList {
                while (cursor.moveToNext()) add(product(cursor))
            }
        }
    }

    fun productByBarcode(barcode: String): CachedProduct? {
        val code = barcode.trim()
        if (code.isEmpty()) return null
        return readableDatabase.rawQuery(
            "SELECT id, name, price_minor, barcode, category, tracked FROM products " +
                "WHERE archived = 0 AND barcode = ? LIMIT 1",
            arrayOf(code),
        ).use { cursor -> if (cursor.moveToFirst()) product(cursor) else null }
    }

    fun staff(): List<StaffMember> =
        readableDatabase.rawQuery(
            "SELECT id, name, role FROM staff ORDER BY name COLLATE NOCASE",
            null,
        ).use { cursor ->
            buildList {
                while (cursor.moveToNext()) {
                    add(StaffMember(cursor.getString(0), cursor.getString(1), cursor.getString(2)))
                }
            }
        }

    fun originDevice(): String {
        val saved = meta(KEY_ORIGIN_DEVICE)?.takeIf { it.isNotBlank() }
        if (saved != null) return saved
        val created = "phone-" + UuidV7.generate()
        putMeta(KEY_ORIGIN_DEVICE, created)
        return created
    }

    fun clockOffsetMs(): Long = meta(KEY_CLOCK_OFFSET_MS)?.toLongOrNull() ?: 0L

    fun putClockOffset(offsetMs: Long) = putMeta(KEY_CLOCK_OFFSET_MS, offsetMs.toString())

    fun insertOfflineSale(uid: String, payload: String, occurredAt: String) {
        writableDatabase.insertOrThrow(
            "offline_sales",
            null,
            ContentValues().apply {
                put("uid", uid)
                put("payload", payload)
                put("occurred_at", occurredAt)
                put("status", "pending")
                put("created_at", System.currentTimeMillis())
            },
        )
    }

    fun pendingSales(limit: Int): List<PendingSale> =
        readableDatabase.rawQuery(
            "SELECT uid, payload FROM offline_sales WHERE status = 'pending' " +
                "ORDER BY created_at, rowid LIMIT ?",
            arrayOf(limit.toString()),
        ).use { cursor ->
            buildList {
                while (cursor.moveToNext()) add(PendingSale(cursor.getString(0), cursor.getString(1)))
            }
        }

    fun pendingCount(): Int = count("status = 'pending'")

    fun rejectedCount(): Int = count("status = 'rejected'")

    fun markAccepted(uids: Collection<String>) {
        if (uids.isEmpty()) return
        val db = writableDatabase
        db.beginTransaction()
        try {
            for (uid in uids) db.delete("offline_sales", "uid = ?", arrayOf(uid))
            db.setTransactionSuccessful()
        } finally {
            db.endTransaction()
        }
    }

    fun markRejected(uid: String, reason: String) {
        writableDatabase.execSQL(
            "UPDATE offline_sales SET status = 'rejected', attempts = attempts + 1, last_error = ? " +
                "WHERE uid = ?",
            arrayOf(reason, uid),
        )
    }

    fun markAttemptFailed(uids: Collection<String>, error: String) {
        if (uids.isEmpty()) return
        val db = writableDatabase
        db.beginTransaction()
        try {
            for (uid in uids) {
                db.execSQL(
                    "UPDATE offline_sales SET attempts = attempts + 1, last_error = ? WHERE uid = ?",
                    arrayOf(error, uid),
                )
            }
            db.setTransactionSuccessful()
        } finally {
            db.endTransaction()
        }
    }

    fun rejectedSales(limit: Int): List<RejectedSale> =
        readableDatabase.rawQuery(
            "SELECT uid, COALESCE(last_error, ''), occurred_at FROM offline_sales " +
                "WHERE status = 'rejected' ORDER BY created_at DESC LIMIT ?",
            arrayOf(limit.toString()),
        ).use { cursor ->
            buildList {
                while (cursor.moveToNext()) {
                    add(RejectedSale(cursor.getString(0), cursor.getString(1), cursor.getString(2)))
                }
            }
        }

    fun retryRejected() {
        writableDatabase.execSQL(
            "UPDATE offline_sales SET status = 'pending', attempts = 0, last_error = NULL " +
                "WHERE status = 'rejected'",
        )
    }

    fun meta(key: String): String? =
        readableDatabase.rawQuery("SELECT value FROM meta WHERE key = ?", arrayOf(key)).use { cursor ->
            if (cursor.moveToFirst()) cursor.getString(0) else null
        }

    fun putMeta(key: String, value: String) = putMeta(writableDatabase, key, value)

    private fun putMeta(db: SQLiteDatabase, key: String, value: String) {
        db.execSQL(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?, ?)",
            arrayOf(key, value),
        )
    }

    private fun count(where: String): Int =
        readableDatabase.rawQuery("SELECT COUNT(*) FROM offline_sales WHERE $where", null).use { cursor ->
            if (cursor.moveToFirst()) cursor.getInt(0) else 0
        }

    private fun product(cursor: Cursor) = CachedProduct(
        id = cursor.getLong(0),
        name = cursor.getString(1),
        priceMinor = cursor.getLong(2),
        barcode = cursor.getString(3),
        category = cursor.getString(4),
        tracked = cursor.getInt(5) != 0,
    )

    private fun escapeLike(value: String): String = buildString(value.length) {
        for (character in value) {
            if (character == '\\' || character == '%' || character == '_') append('\\')
            append(character)
        }
    }

    private companion object {
        const val NAME = "stride-scan.db"
        const val VERSION = 1
        const val KEY_CATALOG_VERSION = "catalog.version"
        const val KEY_CATALOG_CURRENCY = "catalog.currency"
        const val KEY_CATALOG_EXPONENT = "catalog.exponent"
        const val KEY_CATALOG_CASH_STEP = "catalog.cash_step"
        const val KEY_CATALOG_TAX_RATE = "catalog.tax_rate"
        const val KEY_LAST_CATALOG_MS = "catalog.fetched_at"
        const val KEY_ORIGIN_DEVICE = "device.origin"
        const val KEY_CLOCK_OFFSET_MS = "clock.offset_ms"
    }
}
