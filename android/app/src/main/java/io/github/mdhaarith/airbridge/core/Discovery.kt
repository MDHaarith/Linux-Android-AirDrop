package io.github.mdhaarith.airbridge.core

import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.withContext
import java.net.DatagramPacket
import java.net.Inet4Address
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.MulticastSocket
import java.net.NetworkInterface

/**
 * LocalSend discovery: multicast announcements answered by HTTP register
 * requests, plus an HTTP subnet scan for networks that block multicast.
 */
class Discovery(private val platform: Platform, private val engine: Engine) {
    private var socket: MulticastSocket? = null
    private var releaseLock: (() -> Unit)? = null
    private val jobs = mutableListOf<Job>()
    private val pending = java.util.Collections.synchronizedSet(mutableSetOf<String>())
    private val group = InetAddress.getByName(Protocol.MULTICAST_GROUP)
    @Volatile var multicastOk = false
        private set

    data class LocalAddress(val name: String, val address: Inet4Address, val prefix: Int)

    fun start(scope: CoroutineScope) {
        releaseLock = platform.acquireMulticastLock()
        try {
            val s = MulticastSocket(null)
            s.reuseAddress = true
            s.bind(InetSocketAddress(Protocol.DEFAULT_PORT))
            s.timeToLive = 1
            var joined = 0
            for (iface in multicastInterfaces()) {
                runCatching { s.joinGroup(InetSocketAddress(group, Protocol.DEFAULT_PORT), iface); joined++ }
            }
            if (joined == 0) s.joinGroup(InetSocketAddress(group, Protocol.DEFAULT_PORT), null)
            socket = s
            multicastOk = true
            jobs += scope.launch(Dispatchers.IO) { receiveLoop(s, scope) }
        } catch (e: Exception) {
            Log.w(TAG, "multicast unavailable", e)
        }
        jobs += scope.launch {
            while (isActive) {
                announceBurst()
                delay(45_000)
            }
        }
        jobs += scope.launch { livenessLoop() }
    }

    fun stop() {
        jobs.forEach { it.cancel() }
        jobs.clear()
        runCatching { socket?.close() }
        socket = null
        runCatching { releaseLock?.invoke() }
    }

    private fun multicastInterfaces(): List<NetworkInterface> =
        runCatching { NetworkInterface.getNetworkInterfaces().toList() }.getOrDefault(emptyList())
            .filter { runCatching { it.isUp && !it.isLoopback && it.supportsMulticast() }.getOrDefault(false) }
            .filter { iface -> iface.inetAddresses.toList().any { it is Inet4Address } }
            .filterNot { it.name.startsWith("dummy") || it.name.startsWith("rmnet") || it.name.startsWith("ccmni") }

    private fun send(bytes: ByteArray) {
        val s = socket ?: return
        synchronized(this) {
            val interfaces = multicastInterfaces()
            if (interfaces.isEmpty()) {
                runCatching { s.send(DatagramPacket(bytes, bytes.size, group, Protocol.DEFAULT_PORT)) }
                return
            }
            for (iface in interfaces) {
                runCatching {
                    s.networkInterface = iface
                    s.send(DatagramPacket(bytes, bytes.size, group, Protocol.DEFAULT_PORT))
                }.onFailure { Log.d(TAG, "announce on ${iface.name} failed: ${it.message}") }
            }
        }
    }

    suspend fun announceBurst() = withContext(Dispatchers.IO) {
        val payload = Protocol.json.encodeToString(MulticastDto.serializer(), MulticastDto.of(engine.me(), true)).toByteArray()
        for (wait in longArrayOf(100, 500, 2000)) {
            delay(wait)
            send(payload)
        }
    }

    private fun receiveLoop(s: MulticastSocket, scope: CoroutineScope) {
        val buffer = ByteArray(64 * 1024)
        while (!s.isClosed) {
            val packet = DatagramPacket(buffer, buffer.size)
            try {
                s.receive(packet)
            } catch (e: Exception) {
                if (s.isClosed) return
                continue
            }
            val text = String(packet.data, 0, packet.length)
            val dto = runCatching { Protocol.json.decodeFromString(MulticastDto.serializer(), text) }.getOrNull() ?: continue
            val info = dto.info().sanitized() ?: continue
            if (info.fingerprint.isEmpty() || info.fingerprint.equals(engine.identity.fingerprint, true)) continue
            val ip = packet.address.hostAddress ?: continue
            if (dto.isAnnouncement) {
                if (!pending.add(info.fingerprint)) continue
                scope.launch {
                    try {
                        val ok = registerWith(ip, info.port, info.isHttps, info.fingerprint, 4_000)
                        if (ok == null) {
                            // Not reachable over TCP: answer over UDP so they can find us.
                            send(Protocol.json.encodeToString(MulticastDto.serializer(), MulticastDto.of(engine.me(), false)).toByteArray())
                        }
                    } finally {
                        pending.remove(info.fingerprint)
                    }
                }
            } else {
                engine.upsertPeer(info, ip)
            }
        }
    }

    /** POST /register; on success the device is added to the peer list. */
    suspend fun registerWith(ip: String, port: Int, https: Boolean, fingerprint: String?, timeoutMs: Long): DeviceInfo? {
        val me = engine.me()
        val result = runCatching {
            engine.client.post(
                ip, port, https, fingerprint, "${Protocol.API}/register",
                body = PeerClient.json(Protocol.json.encodeToString(DeviceInfo.serializer(), me)),
                readTimeoutMs = timeoutMs, connectTimeoutMs = timeoutMs,
            )
        }.getOrNull() ?: return null
        if (result.status != 200) return null
        val info = runCatching { Protocol.json.decodeFromString(InfoDto.serializer(), result.body) }.getOrNull() ?: return null
        val device = DeviceInfo(
            alias = info.alias,
            version = info.version,
            deviceModel = info.deviceModel,
            deviceType = info.deviceType,
            // Over HTTPS the certificate is the identity, not what the body claims.
            fingerprint = result.peerFingerprint ?: info.fingerprint,
            port = port,
            protocol = if (https) "https" else "http",
            download = info.download,
        ).sanitized() ?: return null
        if (device.fingerprint.isEmpty() || device.fingerprint.equals(engine.identity.fingerprint, true)) return null
        engine.upsertPeer(device, ip)
        return device
    }

    private suspend fun livenessLoop() {
        while (true) {
            delay(12_000)
            val stale = engine.peers.value.filter { System.currentTimeMillis() - it.lastSeen > 25_000 }
            coroutineScopeAll(stale.map { peer ->
                suspend {
                    val ok = registerWith(peer.ip, peer.port, peer.https, peer.id, 4_000)
                    if (ok == null) engine.peerFailed(peer.id)
                }
            })
        }
    }

    private suspend fun coroutineScopeAll(tasks: List<suspend () -> Unit>) = kotlinx.coroutines.coroutineScope {
        tasks.map { async { it() } }.awaitAll()
    }

    /** Probes every host of the local /24 (or smaller) networks. */
    suspend fun scan(): Int = withContext(Dispatchers.IO) {
        val targets = localAddresses().flatMap { local ->
            val prefix = if (local.prefix < 22) 24 else local.prefix
            val mask = if (prefix == 0) 0 else -1 shl (32 - prefix)
            val base = local.address.address.fold(0) { acc, b -> (acc shl 8) or (b.toInt() and 0xff) } and mask
            val broadcast = base or mask.inv()
            ((base + 1) until broadcast).map { ip ->
                "${ip ushr 24 and 0xff}.${ip ushr 16 and 0xff}.${ip ushr 8 and 0xff}.${ip and 0xff}"
            }.filter { it != local.address.hostAddress }
        }.distinct()
        val semaphore = Semaphore(48)
        kotlinx.coroutines.coroutineScope {
            targets.map { ip ->
                async {
                    semaphore.withPermit {
                        registerWith(ip, Protocol.DEFAULT_PORT, true, null, 1_500)
                            ?: registerWith(ip, Protocol.DEFAULT_PORT, false, null, 1_500)
                    }
                }
            }.awaitAll().count { it != null }
        }
    }

    companion object {
        private const val TAG = "Discovery"

        fun localAddresses(): List<LocalAddress> =
            runCatching { NetworkInterface.getNetworkInterfaces().toList() }.getOrDefault(emptyList())
                .filter { runCatching { it.isUp && !it.isLoopback }.getOrDefault(false) }
                .filterNot { it.name.startsWith("rmnet") || it.name.startsWith("ccmni") || it.name.startsWith("dummy") }
                .flatMap { iface ->
                    iface.interfaceAddresses.mapNotNull { a ->
                        (a.address as? Inet4Address)?.let { LocalAddress(iface.name, it, a.networkPrefixLength.toInt()) }
                    }
                }
                .sortedBy { if (it.name.startsWith("wlan")) 0 else 1 }
    }
}
