package com.stride.scan

data class OutboxResult(
    val accepted: Int = 0,
    val rejected: Int = 0,
    val pending: Int = 0,
    val error: String? = null,
    val busy: Boolean = false,
)

/**
 * The phone's cash outbox, sent one batch at a time.
 *
 * A refusal from the till is not a transport failure: the sale is marked
 * `rejected` and stays visible until a person looks at it. Retrying by itself
 * would loop forever on a validation error. A lost connection, by contrast, is
 * retried with a backoff, because the queue is safe on disk.
 */
class Outbox(
    private val db: TillDb,
    private val client: TillClient,
) {
    private val lock = Any()
    private var busy = false
    private var failures = 0
    private var nextAttemptAt = 0L

    fun due(now: Long = System.currentTimeMillis()): Boolean = synchronized(lock) {
        now >= nextAttemptAt
    }

    fun retrySoon() = synchronized(lock) {
        nextAttemptAt = 0
    }

    fun flush(done: (OutboxResult) -> Unit) {
        synchronized(lock) {
            if (busy) {
                done(OutboxResult(busy = true))
                return
            }
            busy = true
        }

        val batch = db.pendingSales(BATCH)
        if (batch.isEmpty()) {
            finish(OutboxResult(), done)
            return
        }

        client.pushSales(batch.map { it.payload }) { result ->
            result.onSuccess { response ->
                val stored = runCatching {
                    db.markAccepted(response.accepted)
                    for (reject in response.rejected) db.markRejected(reject.uid, reject.reason)
                }
                if (stored.isFailure) {
                    noteFailure()
                    finish(
                        OutboxResult(
                            error = "Could not record the till's answer.",
                            pending = pendingOr(batch.size),
                        ),
                        done,
                    )
                    return@onSuccess
                }

                synchronized(lock) {
                    failures = 0
                    nextAttemptAt = 0
                }
                finish(
                    OutboxResult(
                        accepted = response.accepted.size,
                        rejected = response.rejected.size,
                        pending = pendingOr(0),
                    ),
                    done,
                )
            }.onFailure { error ->
                val message = error.message ?: "Could not reach the till."
                runCatching { db.markAttemptFailed(batch.map { it.uid }, message) }
                noteFailure()
                finish(
                    OutboxResult(error = message, pending = pendingOr(batch.size)),
                    done,
                )
            }
        }
    }

    private fun finish(result: OutboxResult, done: (OutboxResult) -> Unit) {
        synchronized(lock) { busy = false }
        done(result)
    }

    private fun noteFailure() = synchronized(lock) {
        failures = (failures + 1).coerceAtMost(8)
        nextAttemptAt = System.currentTimeMillis() + delayFor(failures)
    }

    /** 30 s, doubling, capped at five minutes. */
    private fun delayFor(failures: Int): Long {
        val wait = 30_000L shl (failures - 1)
        return minOf(wait, 300_000L)
    }

    private fun pendingOr(fallback: Int): Int = runCatching { db.pendingCount() }.getOrDefault(fallback)

    private companion object {
        const val BATCH = 50
    }
}
