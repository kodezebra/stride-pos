package com.stride.scan

/**
 * One line in the phone's cart.
 *
 * `productId` is the till's integer id. A manual item has none, and a product
 * the till does not track (a service, a bag) has an id but produces no stock
 * movement. `tracked` is copied from the cached catalogue at the moment the
 * line is added, so a later catalogue refresh cannot change what this sale
 * meant.
 */
data class CartLine(
    val uid: String,
    val productId: Long?,
    val name: String,
    val unitMinor: Long,
    val qty: Int,
    val tracked: Boolean,
) {
    val totalMinor: Long get() = unitMinor * qty
}

/**
 * The cash sale a phone rings up while the desktop is unavailable.
 *
 * It is deliberately plain Kotlin: no Android type and no database. The till's
 * checkout does the same arithmetic: the total is the sum of the lines, cash
 * rounds the amount due to the smallest coin, and tax is the portion included
 * in the total, never an extra.
 */
class Cart {
    private val lines = ArrayList<CartLine>()

    val items: List<CartLine> get() = lines.toList()

    val isEmpty: Boolean get() = lines.isEmpty()

    fun add(
        productId: Long?,
        name: String,
        unitMinor: Long,
        tracked: Boolean,
        newUid: () -> String = UuidV7::generate,
    ) {
        val index = lines.indexOfFirst { matches(it, productId, name, unitMinor) }
        if (index >= 0) {
            val line = lines[index]
            lines[index] = line.copy(qty = line.qty + 1)
        } else {
            lines.add(
                CartLine(
                    uid = newUid(),
                    productId = productId,
                    name = name.trim(),
                    unitMinor = unitMinor,
                    qty = 1,
                    tracked = tracked,
                ),
            )
        }
    }

    fun change(uid: String, delta: Int) {
        val index = lines.indexOfFirst { it.uid == uid }
        if (index < 0) return
        val next = lines[index].qty + delta
        if (next <= 0) lines.removeAt(index) else lines[index] = lines[index].copy(qty = next)
    }

    fun clear() = lines.clear()

    fun totalMinor(): Long = lines.sumOf { it.totalMinor }

    /**
     * Cash rounding, half away from zero, exactly like the till.
     *
     * A 340 shilling basket against a 100 shilling coin rounds down to 300, so
     * the change is `-40`. The customer pays a coin the shop can actually give.
     */
    fun roundingMinor(cashStepMinor: Long): Long {
        if (cashStepMinor <= 1) return 0
        val total = totalMinor()
        val rounded = ((total + cashStepMinor / 2) / cashStepMinor) * cashStepMinor
        return rounded - total
    }

    /**
     * The tax already inside the total, using the till's integer arithmetic.
     */
    fun taxMinor(taxRatePercent: Int): Long {
        if (taxRatePercent <= 0) return 0
        val total = totalMinor()
        val preTax = total * 100 / (100 + taxRatePercent)
        return total - preTax
    }

    fun dueMinor(cashStepMinor: Long): Long = totalMinor() + roundingMinor(cashStepMinor)

    private fun matches(line: CartLine, productId: Long?, name: String, unitMinor: Long): Boolean =
        if (productId != null) {
            line.productId == productId
        } else {
            line.productId == null &&
                line.unitMinor == unitMinor &&
                line.name.equals(name.trim(), ignoreCase = true)
        }
}
