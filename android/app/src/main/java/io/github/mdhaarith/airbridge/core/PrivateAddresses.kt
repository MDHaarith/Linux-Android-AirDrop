package io.github.mdhaarith.airbridge.core

import java.net.Inet4Address
import java.net.NetworkInterface

/** Literal IPv4 only: never let an address supplied by another device trigger DNS. */
fun isPrivateIpv4(ip: String): Boolean {
    val parts = ip.split('.')
    if (parts.size != 4 || parts.any { it.isEmpty() || it.length > 3 || it.any { c -> c !in '0'..'9' } || (it.length > 1 && it[0] == '0') }) return false
    val octets = parts.map { it.toInt() }
    if (octets.any { it > 255 }) return false
    return octets[0] == 10 || octets[0] == 172 && octets[1] in 16..31 ||
        octets[0] == 192 && octets[1] == 168 || octets[0] == 100 && octets[1] in 64..127
}

fun ownAddresses(): List<String> =
    runCatching { NetworkInterface.getNetworkInterfaces().toList() }.getOrDefault(emptyList())
        .filter { runCatching { it.isUp && !it.isLoopback }.getOrDefault(false) }
        // Keep the VPN address even on phones with several active physical interfaces.
        .sortedBy { if (it.name.startsWith("tun")) 0 else 1 }
        .flatMap { it.inetAddresses.toList() }
        .filterIsInstance<Inet4Address>()
        .mapNotNull { it.hostAddress }
        .filter(::isPrivateIpv4).distinct().take(4)
