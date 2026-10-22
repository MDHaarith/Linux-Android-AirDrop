package io.github.mdhaarith.airbridge.overlay

import io.github.mdhaarith.airbridge.core.Peer

/** Resolve a drop without silently replacing an offline default or broadcasting. */
object DropTargets {
    const val ALL = "@all"

    fun resolve(peers: List<Peer>, target: String?): List<Peer> = when (target) {
        ALL -> peers.distinctBy { it.id }
        null -> peers.singleOrNull()?.let(::listOf) ?: emptyList()
        else -> peers.filter { it.id == target }.take(1)
    }
}
