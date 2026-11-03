package com.stride.scan

import android.content.SharedPreferences
import android.net.Uri

/**
 * Where the till is and how to prove the phone belongs to it.
 *
 * The code is not a password the operator types every day: it is copied once,
 * from the till's QR code, and then kept in this app's private preferences. It is
 * still ten characters, so typing it by hand is a fallback rather than a chore.
 * [endpoint] brackets a raw IPv6 address so `URL` can parse it.
 */
data class Pairing(
    val host: String,
    val port: Int,
    val token: String,
) {
    val endpoint: String
        get() {
            val shown = if (host.contains(':') && !host.startsWith("[")) "[$host]" else host
            return "http://$shown:$port"
        }

    fun save(prefs: SharedPreferences) {
        prefs.edit()
            .putString(KEY_HOST, host)
            .putInt(KEY_PORT, port)
            .putString(KEY_TOKEN, token)
            .apply()
    }

    companion object {
        private const val KEY_HOST = "till.host"
        private const val KEY_PORT = "till.port"
        private const val KEY_TOKEN = "till.token"

        /** Crockford's base 32: no I, L, O or U, so nothing looks like 1 or 0. */
        private const val TOKEN_LENGTH = 10
        private val TOKEN = Regex("[0-9A-HJKMNP-TV-Z]{$TOKEN_LENGTH}")

        /**
         * Read a code the way a person types it and return the canonical form.
         *
         * Dashes and spaces are separators, and `I`/`L`/`O` fold into `1`/`0`.
         * Returns null for anything that is not a pairing code.
         */
        fun normalize(raw: String): String? {
            val cleaned = buildString(TOKEN_LENGTH) {
                for (character in raw.trim()) {
                    if (character == '-' || character == ' ') continue
                    val upper = character.uppercaseChar()
                    append(
                        when (upper) {
                            'I', 'L' -> '1'
                            'O' -> '0'
                            else -> upper
                        },
                    )
                }
            }
            return cleaned.takeIf { TOKEN.matches(it) }
        }

        /** The code as the till shows it: two groups of five. */
        fun display(token: String): String =
            if (token.length == TOKEN_LENGTH) {
                "${token.substring(0, TOKEN_LENGTH / 2)}-${token.substring(TOKEN_LENGTH / 2)}"
            } else {
                token
            }

        fun validToken(token: String): Boolean = normalize(token) != null

        fun load(prefs: SharedPreferences): Pairing? {
            val host = prefs.getString(KEY_HOST, null)?.trim().orEmpty()
            val port = prefs.getInt(KEY_PORT, 0)
            val token = prefs.getString(KEY_TOKEN, null)?.let(::normalize)
            if (host.isEmpty() || host.any { it.isWhitespace() }) return null
            if (port !in 1..65535) return null
            if (token == null) return null
            return Pairing(host, port, token)
        }
    }
}

/** The QR code the till draws in Settings → Scanner. */
object PairingCode {
    fun parse(raw: String): Pairing? {
        val uri = Uri.parse(raw)
        if (uri.scheme != "stridepos" || uri.host != "pair") return null

        val host = uri.getQueryParameter("host")?.trim().orEmpty()
        val port = uri.getQueryParameter("port")?.toIntOrNull() ?: return null
        val token = uri.getQueryParameter("token")?.let(Pairing::normalize) ?: return null
        if (host.isEmpty() || host.any { it.isWhitespace() }) return null
        if (port !in 1..65535) return null
        return Pairing(host, port, token)
    }
}
