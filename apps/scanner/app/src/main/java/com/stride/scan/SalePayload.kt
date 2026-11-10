package com.stride.scan

import java.util.Locale

/**
 * A line exactly as it will travel to the till.
 *
 * The name and price are copied from the cart, not looked up again at send
 * time: the cached catalogue may have been refreshed since the sale, and a
 * receipt must say what the customer was actually charged.
 */
data class DraftLine(
    val uid: String,
    val productId: Long?,
    val name: String,
    val qty: Int,
    val unitMinor: Long,
) {
    val totalMinor: Long get() = unitMinor * qty
}

data class DraftPayment(
    val uid: String,
    val amountMinor: Long,
    val tenderedMinor: Long,
    val changeMinor: Long,
)

data class DraftMovement(
    val uid: String,
    val productId: Long,
    val qty: Int,
)

/**
 * A complete offline sale, before it is encoded.
 *
 * The till validates this shape again. The phone's job is to send something
 * internally consistent: a total that matches the lines, one cash payment for
 * the rounded amount due, and one stock movement per tracked product.
 */
data class SaleDraft(
    val uid: String,
    val originDevice: String,
    val occurredAt: String,
    val userId: String?,
    val totalMinor: Long,
    val roundingMinor: Long,
    val taxMinor: Long,
    val lines: List<DraftLine>,
    val payment: DraftPayment,
    val movements: List<DraftMovement>,
    val isOffline: Int = 1,
)

/**
 * Turns a cart into the money the till expects.
 *
 * The tender must cover the rounded amount due. A tendered amount equal to the
 * due gives zero change, which is the common case and the one the UI uses when
 * the operator leaves the cash field empty.
 */
object SalePayload {
    fun build(
        originDevice: String,
        occurredAt: String,
        userId: String?,
        lines: List<CartLine>,
        roundingMinor: Long,
        taxMinor: Long,
        tenderedMinor: Long,
        newUid: () -> String = UuidV7::generate,
    ): SaleDraft {
        require(lines.isNotEmpty()) { "a sale needs at least one line" }

        val draftLines = lines.map {
            DraftLine(
                uid = newUid(),
                productId = it.productId,
                name = it.name,
                qty = it.qty,
                unitMinor = it.unitMinor,
            )
        }
        val total = draftLines.sumOf { it.totalMinor }
        require(taxMinor >= 0 && taxMinor <= total) { "tax must be between zero and the total" }

        val due = total + roundingMinor
        require(due >= 0) { "the amount due cannot be negative" }
        require(tenderedMinor >= due) { "cash is less than the amount due" }

        // One movement per tracked product, even when the cart reached the
        // same product through two lines. The till rejects a duplicate and
        // recomputes the delta from the lines anyway.
        val quantities = LinkedHashMap<Long, Int>()
        for (line in lines) {
            val productId = line.productId ?: continue
            if (!line.tracked) continue
            quantities[productId] = (quantities[productId] ?: 0) + line.qty
        }
        val movements = quantities.map { (productId, qty) ->
            DraftMovement(uid = newUid(), productId = productId, qty = qty)
        }

        return SaleDraft(
            uid = newUid(),
            originDevice = originDevice,
            occurredAt = occurredAt,
            userId = userId?.takeIf { it.isNotBlank() },
            totalMinor = total,
            roundingMinor = roundingMinor,
            taxMinor = taxMinor,
            lines = draftLines,
            payment = DraftPayment(
                uid = newUid(),
                amountMinor = due,
                tenderedMinor = tenderedMinor,
                changeMinor = tenderedMinor - due,
            ),
            movements = movements,
        )
    }
}

/**
 * A small, hand-written encoder for the sale payload.
 *
 * `org.json` is stubbed in local JVM tests, and the payload is the one thing
 * that must be byte-shape-correct before a sale can be queued. Keeping the
 * encoder here means the test suite can prove it without a device.
 */
object SaleJson {
    fun encode(draft: SaleDraft): String = buildString(512) {
        append('{')
        append("\"uid\":").append(Json.string(draft.uid))
        append(",\"origin_device\":").append(Json.string(draft.originDevice))
        append(",\"is_offline\":").append(draft.isOffline)
        append(",\"occurred_at\":").append(Json.string(draft.occurredAt))
        draft.userId?.let { append(",\"user_id\":").append(Json.string(it)) }
        append(",\"total_minor\":").append(draft.totalMinor)
        append(",\"rounding_minor\":").append(draft.roundingMinor)
        append(",\"tax_minor\":").append(draft.taxMinor)

        append(",\"lines\":[")
        draft.lines.forEachIndexed { index, line ->
            if (index > 0) append(',')
            append('{')
            append("\"uid\":").append(Json.string(line.uid))
            append(",\"name\":").append(Json.string(line.name))
            append(",\"qty\":").append(line.qty)
            append(",\"unit_minor\":").append(line.unitMinor)
            append(",\"total_minor\":").append(line.totalMinor)
            line.productId?.let { append(",\"product_id\":").append(it) }
            append('}')
        }
        append(']')

        append(",\"payments\":[{")
        append("\"uid\":").append(Json.string(draft.payment.uid))
        append(",\"method\":\"cash\"")
        append(",\"amount_minor\":").append(draft.payment.amountMinor)
        append(",\"tendered_minor\":").append(draft.payment.tenderedMinor)
        append(",\"change_minor\":").append(draft.payment.changeMinor)
        append("}]")

        append(",\"movements\":[")
        draft.movements.forEachIndexed { index, movement ->
            if (index > 0) append(',')
            append('{')
            append("\"uid\":").append(Json.string(movement.uid))
            append(",\"product_id\":").append(movement.productId)
            append(",\"delta\":").append(-movement.qty)
            append(",\"kind\":\"sale\"")
            append('}')
        }
        append(']')

        append('}')
    }
}

internal object Json {
    fun string(value: String): String = buildString(value.length + 2) {
        append('"')
        for (character in value) {
            when (character) {
                '"' -> append("\\\"")
                '\\' -> append("\\\\")
                '\n' -> append("\\n")
                '\r' -> append("\\r")
                '\t' -> append("\\t")
                '\b' -> append("\\b")
                '\u000C' -> append("\\f")
                else ->
                    if (character < ' ') {
                        append(String.format(Locale.US, "\\u%04x", character.code))
                    } else {
                        append(character)
                    }
            }
        }
        append('"')
    }
}
