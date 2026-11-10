package com.stride.scan

import java.security.SecureRandom

/**
 * A time-ordered id the till accepts as a sale, line, payment or movement uid.
 *
 * The first 48 bits are the millisecond clock, so ids sort in the order they
 * were made; the rest is random. Nothing here touches Android, which is what
 * lets the JVM tests check the shape without a device.
 *
 * A UUIDv7 is not a proof of identity: the till trusts the pairing token, not
 * the id. The id exists so a retry can be recognised as the same sale.
 */
object UuidV7 {
    private val random = SecureRandom()

    fun generate(nowMillis: Long = System.currentTimeMillis()): String {
        val tail = ByteArray(10)
        random.nextBytes(tail)
        return build(nowMillis, tail)
    }

    /** Deterministic tail, for tests only. */
    internal fun generate(nowMillis: Long, tail: ByteArray): String = build(nowMillis, tail)

    private fun build(nowMillis: Long, tail: ByteArray): String {
        require(tail.size >= 10) { "a v7 uuid needs ten random bytes" }

        val bytes = ByteArray(16)
        var time = nowMillis
        for (index in 5 downTo 0) {
            bytes[index] = (time and 0xFF).toByte()
            time = time ushr 8
        }
        for (index in 0 until 10) bytes[6 + index] = tail[index]

        // Version 7 in the high nibble of byte 6, RFC 4122 variant in byte 8.
        bytes[6] = ((bytes[6].toInt() and 0x0F) or 0x70).toByte()
        bytes[8] = ((bytes[8].toInt() and 0x3F) or 0x80).toByte()

        return hex(bytes)
    }

    private fun hex(bytes: ByteArray): String {
        val text = StringBuilder(36)
        for (index in bytes.indices) {
            if (index == 4 || index == 6 || index == 8 || index == 10) text.append('-')
            val value = bytes[index].toInt() and 0xFF
            text.append(HEX[value ushr 4]).append(HEX[value and 0x0F])
        }
        return text.toString()
    }

    private const val HEX = "0123456789abcdef"
}
