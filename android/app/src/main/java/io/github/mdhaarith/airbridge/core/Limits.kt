package io.github.mdhaarith.airbridge.core

import java.net.InetAddress

/**
 * At most [max] events per address within [windowMs], and [maxTotal] for
 * everyone together, so rotating addresses does not help (IPv6 addresses
 * count per /64, which one device can fill at will).
 */
class RateLimit(private val max: Int, private val maxTotal: Int, private val windowMs: Long) {
    private val hits = HashMap<String, ArrayDeque<Long>>()
    private val total = ArrayDeque<Long>()

    private fun bucket(ip: String): String = runCatching {
        val bytes = InetAddress.getByName(ip).address
        if (bytes.size == 16) bytes.copyOf(8).joinToString(":") { "%02x".format(it) } else ip
    }.getOrDefault(ip)

    private fun ArrayDeque<Long>.prune(now: Long) {
        while (isNotEmpty() && now - first() >= windowMs) removeFirst()
    }

    /** Whether [ip] is still under the limit (without counting an event). */
    @Synchronized
    fun allows(ip: String): Boolean {
        val now = System.currentTimeMillis()
        total.prune(now)
        if (total.size >= maxTotal) return false
        val list = hits[bucket(ip)] ?: return true
        list.prune(now)
        return list.size < max
    }

    /** Counts an event from [ip]. */
    @Synchronized
    fun record(ip: String) {
        val now = System.currentTimeMillis()
        total.addLast(now)
        if (total.size > maxTotal * 2) total.removeFirst()
        hits.getOrPut(bucket(ip)) { ArrayDeque() }.addLast(now)
        if (hits.size > 4096) {
            hits.values.forEach { it.prune(now) }
            hits.values.removeAll { it.isEmpty() }
        }
    }

    /** Counts an event and says whether it was allowed. */
    @Synchronized
    fun hit(ip: String): Boolean = allows(ip).also { if (it) record(ip) }
}
