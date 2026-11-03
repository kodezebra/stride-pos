package com.stride.scan

import android.content.SharedPreferences
import android.net.Uri

/**
 * Where the till is and how to prove the phone belongs to it.
 *
 * The token is not a password the operator types every day: it is copied once,
 * from the till's QR code, and then kept in this app's private preferences.
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
        private val TOKEN = Regex("[0-9a-f]{32}")

        fun validToken(token: String): Boolean = TOKEN.matches(token.lowercase())

        fun load(prefs: SharedPreferences): Pairing? {
            val host = prefs.getString(KEY_HOST, null)?.trim().orEmpty()
            val port = prefs.getInt(KEY_PORT, 0)
            val token = prefs.getString(KEY_TOKEN, null)?.trim()?.lowercase().orEmpty()
            if (host.isEmpty() || host.any { it.isWhitespace() }) return null
            if (port !in 1..65535) return null
            if (!validToken(token)) return null
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
        val token = uri.getQueryParameter("token")?.trim()?.lowercase().orEmpty()
        if (host.isEmpty() || host.any { it.isWhitespace() }) return null
        if (port !in 1..65535) return null
        if (!Pairing.validToken(token)) return null
        return Pairing(host, port, token)
    }
}
