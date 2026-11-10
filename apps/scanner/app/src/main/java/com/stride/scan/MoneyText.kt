package com.stride.scan

import kotlin.math.abs

/**
 * Reading and writing the shop's money.
 *
 * Prices come from the till as minor units: 1,200 shillings is `1200` in UGX
 * and 12.50 is `1250` in KES. The exponent, not a hard-coded 2, decides where
 * the decimal point goes, so the same screen works in both currencies.
 *
 * Parsing is strict on purpose: a manual price must not quietly become ten
 * times larger because of a stray decimal place.
 */
object MoneyText {
    fun parse(raw: String, exponent: Int): Long? {
        val places = exponent.coerceIn(0, 3)
        val cleaned = raw.trim()
            .replace(",", "")
            .replace(" ", "")
            .replace("_", "")
            .removePrefix("+")
        if (cleaned.isEmpty()) return null
        if (cleaned.startsWith("-")) return null
        if (cleaned.count { it == '.' } > 1) return null

        val dot = cleaned.indexOf('.')
        val wholeText = if (dot >= 0) cleaned.substring(0, dot) else cleaned
        val fractionText = if (dot >= 0) cleaned.substring(dot + 1) else ""
        if (wholeText.isEmpty() && fractionText.isEmpty()) return null
        if (wholeText.any { !it.isDigit() } || fractionText.any { !it.isDigit() }) return null
        if (fractionText.length > places) return null

        val whole = if (wholeText.isEmpty()) 0L else wholeText.toLongOrNull() ?: return null
        val scale = pow10(places)
        val fraction = if (places == 0) {
            0L
        } else {
            fractionText.padEnd(places, '0').ifEmpty { "0" }.toLongOrNull() ?: return null
        }
        return whole * scale + fraction
    }

    fun format(minor: Long, exponent: Int): String {
        val places = exponent.coerceIn(0, 3)
        val scale = pow10(places)
        val sign = if (minor < 0) "-" else ""
        val absolute = abs(minor)
        val whole = absolute / scale
        val fraction = absolute % scale
        val grouped = whole.toString().reversed().chunked(3).joinToString(",").reversed()
        return if (places == 0) {
            sign + grouped
        } else {
            sign + grouped + "." + fraction.toString().padStart(places, '0')
        }
    }

    private fun pow10(places: Int): Long {
        var value = 1L
        repeat(places) { value *= 10 }
        return value
    }
}
