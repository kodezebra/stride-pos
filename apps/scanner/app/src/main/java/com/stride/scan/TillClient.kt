package com.stride.scan

import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.Executors
import java.util.concurrent.RejectedExecutionException
import org.json.JSONObject

data class TillInfo(
    val deviceId: String,
    val deviceName: String,
    val serverTime: String?,
    val catalogOwner: Boolean,
)

data class PushReject(val uid: String, val reason: String)

data class PushResult(val accepted: List<String>, val rejected: List<PushReject>)

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
        post("/v1/scan", body) { result -> done(result.map { Unit }) }
    }

    /** Pairing check: any 2xx means the address and token are good. */
    fun ping(done: (Result<Unit>) -> Unit) {
        post("/v1/ping", "{}") { result -> done(result.map { Unit }) }
    }

    /**
     * The till's identity and clock.
     *
     * During a blackout this is how the phone learns the desktop is back and
     * how far its own clock has drifted. A failed call is the normal offline
     * case, not an error the operator needs to see.
     */
    fun info(done: (Result<TillInfo>) -> Unit) {
        post("/v1/ping", "{}") { result ->
            done(result.mapCatching { body ->
                val json = JSONObject(body)
                TillInfo(
                    deviceId = json.optString("device_id", ""),
                    deviceName = json.optString("device_name", ""),
                    serverTime = json.optString("server_time", "").takeIf { it.isNotBlank() },
                    catalogOwner = json.optBoolean("catalog_owner", false),
                )
            })
        }
    }

    fun fetchCatalog(done: (Result<String>) -> Unit) {
        post("/v1/sync/catalog", "{}", done)
    }

    /**
     * Offer queued sales to the till.
     *
     * The payloads are already encoded JSON objects, so they are joined
     * straight into the envelope instead of being parsed and re-serialised.
     * The till answers with what it accepted and what it refused.
     */
    fun pushSales(payloads: List<String>, done: (Result<PushResult>) -> Unit) {
        if (payloads.isEmpty()) {
            done(Result.success(PushResult(emptyList(), emptyList())))
            return
        }
        val body = buildString(1024) {
            append("{\"protocol\":").append(PROTOCOL)
            append(",\"sales\":[")
            payloads.forEachIndexed { index, payload ->
                if (index > 0) append(',')
                append(payload)
            }
            append("]}")
        }
        post("/v1/sync/sales", body) { result ->
            done(result.mapCatching { text -> parsePush(text) })
        }
    }

    fun close() {
        worker.shutdownNow()
    }

    private fun post(path: String, body: String, done: (Result<String>) -> Unit) {
        try {
            worker.execute {
                done(request(path, body))
            }
        } catch (error: RejectedExecutionException) {
            done(Result.failure(TillException("The scanner is closing.", error)))
        }
    }

    private fun request(path: String, body: String): Result<String> {
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
                val text = connection.inputStream
                    ?.bufferedReader(Charsets.UTF_8)
                    ?.use { it.readText() }
                    .orEmpty()
                Result.success(text)
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

    private fun parsePush(body: String): PushResult {
        val json = JSONObject(body)
        val acceptedJson = json.optJSONArray("accepted")
        val accepted = buildList {
            if (acceptedJson != null) {
                for (index in 0 until acceptedJson.length()) {
                    add(acceptedJson.optString(index))
                }
            }
        }
        val rejectedJson = json.optJSONArray("rejected")
        val rejected = buildList {
            if (rejectedJson != null) {
                for (index in 0 until rejectedJson.length()) {
                    val item = rejectedJson.optJSONObject(index) ?: continue
                    add(PushReject(item.optString("uid", ""), item.optString("reason", "")))
                }
            }
        }
        return PushResult(accepted, rejected)
    }

    private fun refusal(code: Int, body: String): String {
        val detail = runCatching { JSONObject(body).optString("error") }.getOrNull()
        return when {
            !detail.isNullOrBlank() -> "The till refused the request: $detail"
            code == 401 -> "The pairing token is wrong. Pair the phone again."
            else -> "The till answered with error $code."
        }
    }

    private companion object {
        const val PROTOCOL = 1
        const val CONNECT_TIMEOUT_MS = 4_000

        // A push can carry a blackout's worth of sales. The phone should give
        // a busy till long enough to write them before calling it offline.
        const val READ_TIMEOUT_MS = 15_000
    }
}

class TillException(message: String, cause: Throwable? = null) : Exception(message, cause)
