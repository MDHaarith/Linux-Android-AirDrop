package io.github.mdhaarith.airbridge.share

import io.github.mdhaarith.airbridge.core.Peer

/**
 * Decides which devices appear in the Android share sheet. Devices you trust
 * come first, then the most recently seen; a device that just dropped off
 * the network stays for a while (phones and laptops come and go).
 */
class ShortcutPlanner(private val max: Int, private val keepMillis: Long = 15 * 60_000L) {
    data class Plan(val publish: List<Peer>, val remove: List<String>)

    private class Seen(val peer: Peer, val at: Long)

    private val seen = LinkedHashMap<String, Seen>()
    private var published: List<Pair<String, String>> = emptyList()

    /** Returns what to change, or null when the share sheet is already right. */
    fun update(peers: List<Peer>, trusted: Set<String>, now: Long): Plan? {
        for (peer in peers) seen[peer.id] = Seen(peer, now)
        seen.entries.removeAll { now - it.value.at > keepMillis }
        val ranked = seen.values
            .sortedWith(compareByDescending<Seen> { it.peer.id in trusted }.thenByDescending { it.at }.thenBy { it.peer.alias.lowercase() })
            .take(max)
            .map { it.peer }
        val next = ranked.map { it.id to it.alias }
        if (next == published) return null
        val keep = next.map { it.first }.toSet()
        val remove = published.map { it.first }.filter { it !in keep }
        published = next
        return Plan(ranked, remove)
    }
}
