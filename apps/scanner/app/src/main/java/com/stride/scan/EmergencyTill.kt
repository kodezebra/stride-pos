package com.stride.scan

/**
 * Who the cash sale is attributed to.
 *
 * The till's staff directory is read-only on this phone and carries no PIN, so
 * the id is only a label on the receipt. A phone with no cached staff still
 * works: the sale is recorded as unassigned rather than refused.
 */
data class Cashier(val id: String?, val name: String) {
    companion object {
        val unassigned = Cashier(null, "Unassigned")
    }
}

/**
 * The blackout till: a cash cart and the money arithmetic the desktop uses.
 *
 * It holds no Android view and no network code. The activity renders it; this
 * class decides what a sale means. That split keeps the arithmetic testable and
 * makes it obvious that nothing here can reach a server mid-sale.
 */
class EmergencyTill(private val db: TillDb) {
    val cart = Cart()

    var settings: CatalogSettings = db.catalogSettings()
        private set

    var clockOffsetMs: Long = db.clockOffsetMs()
        private set

    var cashier: Cashier = Cashier.unassigned

    /** Re-read what the last catalogue snapshot and ping taught the phone. */
    fun refresh() {
        settings = db.catalogSettings()
        clockOffsetMs = db.clockOffsetMs()
    }

    fun addProduct(product: CachedProduct) {
        cart.add(
            productId = product.id,
            name = product.name,
            unitMinor = product.priceMinor,
            tracked = product.tracked,
        )
    }

    fun addManual(name: String, unitMinor: Long) {
        cart.add(productId = null, name = name, unitMinor = unitMinor, tracked = false)
    }

    fun totalMinor(): Long = cart.totalMinor()

    fun roundingMinor(): Long = cart.roundingMinor(settings.cashStepMinor)

    fun taxMinor(): Long = cart.taxMinor(settings.taxRatePercent)

    fun dueMinor(): Long = cart.dueMinor(settings.cashStepMinor)

    fun occurredAt(now: Long = System.currentTimeMillis()): String =
        Clock.format(now + clockOffsetMs)

    /**
     * Build the sale that will be queued.
     *
     * A blank tender means the customer paid the exact rounded amount. The
     * till rechecks every total, so a bug here is refused rather than posted as
     * a bad sale.
     */
    fun checkout(tenderedMinor: Long?, now: Long = System.currentTimeMillis()): SaleDraft {
        require(!cart.isEmpty) { "a sale needs at least one item" }
        val due = dueMinor()
        val cash = tenderedMinor ?: due
        require(cash >= due) { "cash is less than the amount due" }

        return SalePayload.build(
            originDevice = db.originDevice(),
            occurredAt = occurredAt(now),
            userId = cashier.id,
            lines = cart.items,
            roundingMinor = roundingMinor(),
            taxMinor = taxMinor(),
            tenderedMinor = cash,
        )
    }
}
