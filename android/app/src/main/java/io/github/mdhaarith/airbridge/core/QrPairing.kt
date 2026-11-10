package io.github.mdhaarith.airbridge.core

import com.google.zxing.BinaryBitmap
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.ReaderException
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeReader
import java.net.InetAddress
import java.net.URLDecoder
import java.net.URLEncoder

/**
 * What a pairing QR carries: the certificate to expect, where to reach the
 * device, a one-time key, and the Wi-Fi Direct network when it hosts one.
 */
data class PairCode(
    val fingerprint: String,
    val ip: String,
    val port: Int,
    val token: String,
    val ssid: String? = null,
    val password: String? = null,
    val addresses: List<String> = emptyList(),
) {
    fun uri(): String {
        fun enc(s: String) = URLEncoder.encode(s, "UTF-8")
        val network = if (ssid != null && password != null) "&s=${enc(ssid)}&w=${enc(password)}" else ""
        val extra = if (addresses.isEmpty()) "" else "&a=${addresses.joinToString(",")}"
        return "${PREFIX}f=$fingerprint&i=$ip&p=$port&t=$token$extra$network"
    }

    private fun isValid(): Boolean {
        val networkOk = if (ssid == null || password == null) ssid == password
        else ssid.startsWith("DIRECT-") && ssid.length <= 32 && password.length in 8..63
        return networkOk && FINGERPRINT.matches(fingerprint) && TOKEN.matches(token) && port in 1..65535 && isReachableIp(ip) &&
            addresses.size <= 4 && addresses.all(::isPrivateIpv4)
    }

    companion object {
        private const val PREFIX = "airbridge://pair?"
        const val PATH = "/api/airbridge/v1/pair"
        private val FINGERPRINT = Regex("[0-9A-Fa-f]{64}")
        private val TOKEN = Regex("[0-9A-Za-z]{16,128}")
        private val IPV4 = Regex("(\\d{1,3})\\.(\\d{1,3})\\.(\\d{1,3})\\.(\\d{1,3})")

        // A literal address only: a name would send the lookup, and this phone's identity, elsewhere.
        private fun isReachableIp(ip: String): Boolean {
            val parts = IPV4.matchEntire(ip)?.groupValues?.drop(1) ?: return false
            if (parts.any { it.toInt() > 255 }) return false
            val address = InetAddress.getByAddress(parts.map { it.toInt().toByte() }.toByteArray())
            return !address.isLoopbackAddress && !address.isAnyLocalAddress && !address.isMulticastAddress
        }

        /** Null for anything that is not a well-formed AirBridge code: QR codes come from anywhere. */
        fun parse(text: String): PairCode? = runCatching {
            require(text.startsWith(PREFIX))
            val parameters = text.removePrefix(PREFIX).split('&').map {
                val (key, value) = it.split('=', limit = 2)
                key to URLDecoder.decode(value, "UTF-8")
            }
            require(parameters.map { it.first }.distinct().size == parameters.size)
            val query = parameters.toMap()
            PairCode(query.getValue("f"), query.getValue("i"), query.getValue("p").toInt(), query.getValue("t"), query["s"], query["w"],
                query["a"]?.split(',') ?: emptyList())
                .takeIf { it.isValid() }
        }.getOrNull()
    }
}

@kotlinx.serialization.Serializable
data class PairRequest(val alias: String, val port: Int, val addresses: List<String>) {
    companion object {
        /** An absent body is an older scanner; malformed bodies must not consume the key. */
        fun parse(text: String): PairRequest? {
            if (text.isBlank()) return null
            val request = Protocol.json.decodeFromString(serializer(), text)
            require(request.port in 1024..65535 && request.addresses.size <= 8 && request.addresses.all(::isPrivateIpv4))
            val alias = Protocol.stripInvisible(request.alias.trim()).take(128)
            require(alias.isNotEmpty())
            return request.copy(alias = alias, addresses = request.addresses.distinct())
        }
    }
}

/** Reads a QR code from a camera frame's luminance plane; null when there is none. */
fun decodeQr(luminance: ByteArray, rowStride: Int, width: Int, height: Int): String? = try {
    val source = PlanarYUVLuminanceSource(luminance, rowStride, height, 0, 0, width, height, false)
    QRCodeReader().decode(BinaryBitmap(HybridBinarizer(source))).text
} catch (_: ReaderException) {
    null
}
