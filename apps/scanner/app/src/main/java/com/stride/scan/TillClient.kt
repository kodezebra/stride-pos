package com.stride.scan

import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.Executors
import java.util.concurrent.RejectedExecutionException
import org.json.JSONObject

/**
 * The only network code in the app.
 *
 * Requests are serialised on one worker so a burst of scans reaches the till in
 * the order the operator made them. Each call is short and local: the phone and
 * the till are on the same Wi-Fi, and a timeout means the till is asleep or the
 * scanner is off, not that a cloud is slow.
 */
class TillClient(private val pairing: Pairing) {
    private val worker = Executors.newSingleThreadExecutor { runnable ->
        Thread(runnable, "till-client").apply { isDaemon = true }
    }

    fun send(barcode: String, action: String, done: (Result<Unit>) -> Unit) {
        val body = JSONObject()
            .put("barcode", barcode)
            .put("action", action)
            .toString()
        post("/v1/scan", body, done)
    }

    fun ping(done: (Result<Unit>) -> Unit) {
        post("/v1/ping", "{}", done)
    }

    fun close() {
        worker.shutdownNow()
    }

    private fun post(path: String, body: String, done: (Result<Unit>) -> Unit) {
        try {
            worker.execute {
                done(request(path, body))
            }
        } catch (error: RejectedExecutionException) {
            done(Result.failure(TillException("The scanner is closing.", error)))
        }
    }

    private fun request(path: String, body: String): Result<Unit> {
        val connection = try {
            URL(pairing.endpoint + path).openConnection() as HttpURLConnection
        } catch (error: Exception) {
            return Result.failure(TillException("The till address is not valid.", error))
        }

        return try {
            connection.requestMethod = "POST"
            connection.connectTimeout = CONNECT_TIMEOUT_MS
            connection.readTimeout = READ_TIMEOUT_MS
            connection.doOutput = true
            connection.setRequestProperty("Content-Type", "application/json; charset=utf-8")
            connection.setRequestProperty("Accept", "application/json")
            connection.setRequestProperty("X-Till-Token", pairing.token)

            val payload = body.toByteArray(Charsets.UTF_8)
            connection.setFixedLengthStreamingMode(payload.size)
            connection.outputStream.use { it.write(payload) }

            val code = connection.responseCode
            if (code in 200..299) {
                connection.inputStream?.use { it.readBytes() }
                Result.success(Unit)
            } else {
                val text = connection.errorStream
                    ?.bufferedReader(Charsets.UTF_8)
                    ?.use { it.readText() }
                    .orEmpty()
                Result.failure(TillException(refusal(code, text)))
            }
        } catch (error: IOException) {
            Result.failure(
                TillException(
                    "Could not reach the till. Check both devices are on the same Wi-Fi " +
                        "and the scanner is on in Settings.",
                    error,
                ),
            )
        } finally {
            connection.disconnect()
        }
    }

    private fun refusal(code: Int, body: String): String {
        val detail = runCatching { JSONObject(body).optString("error") }.getOrNull()
        return when {
            !detail.isNullOrBlank() -> "The till refused the scan: $detail"
            code == 401 -> "The pairing token is wrong. Pair the phone again."
            else -> "The till answered with error $code."
        }
    }

    private companion object {
        const val CONNECT_TIMEOUT_MS = 4_000
        const val READ_TIMEOUT_MS = 5_000
    }
}

class TillException(message: String, cause: Throwable? = null) : Exception(message, cause)
