package com.stride.scan

import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone

/**
 * The till's wire clock.
 *
 * The till writes timestamps as UTC with milliseconds, for example
 * `2026-11-10T09:41:02.123Z`. The phone keeps its own clock but applies the
 * offset learned from `/v1/ping`, so a sale rung up during a blackout carries a
 * time the server recognises as plausible. The till still clamps anything wild.
 */
object Clock {
    private const val WITH_MILLIS = "yyyy-MM-dd'T'HH:mm:ss.SSS'Z'"
    private const val WHOLE_SECONDS = "yyyy-MM-dd'T'HH:mm:ss'Z'"
    private val PATTERNS = arrayOf(WITH_MILLIS, WHOLE_SECONDS)

    fun parse(raw: String?): Long? {
        val text = raw?.trim().orEmpty()
        if (text.isEmpty()) return null
        for (pattern in PATTERNS) {
            val parsed = runCatching { formatter(pattern).parse(text) }.getOrNull()
            if (parsed != null) return parsed.time
        }
        return null
    }

    fun format(epochMillis: Long): String = formatter(WITH_MILLIS).format(Date(epochMillis))

    private fun formatter(pattern: String) = SimpleDateFormat(pattern, Locale.US).apply {
        timeZone = TimeZone.getTimeZone("UTC")
        isLenient = false
    }
}
