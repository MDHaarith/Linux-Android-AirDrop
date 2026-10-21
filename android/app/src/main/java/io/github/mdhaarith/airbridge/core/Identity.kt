package io.github.mdhaarith.airbridge.core

import org.bouncycastle.asn1.x500.X500Name
import org.bouncycastle.cert.jcajce.JcaX509CertificateConverter
import org.bouncycastle.cert.jcajce.JcaX509v3CertificateBuilder
import org.bouncycastle.operator.jcajce.JcaContentSignerBuilder
import java.io.File
import java.math.BigInteger
import java.net.Socket
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.MessageDigest
import java.security.Principal
import java.security.PrivateKey
import java.security.SecureRandom
import java.security.cert.CertificateException
import java.security.cert.X509Certificate
import java.util.Date
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLEngine
import javax.net.ssl.X509ExtendedKeyManager
import javax.net.ssl.X509TrustManager

/**
 * The device identity: a self-signed RSA certificate. Its SHA-256 (uppercase
 * hex, DER) is the fingerprint other LocalSend devices know us by. The same
 * certificate is our HTTPS server certificate and our TLS client certificate.
 */
class Identity private constructor(
    val certificate: X509Certificate,
    private val privateKey: PrivateKey,
) {
    val fingerprint: String = fingerprintOf(certificate)

    /** Always offers our certificate, whatever CA hints the peer sends. */
    private val keyManager = object : X509ExtendedKeyManager() {
        override fun getClientAliases(keyType: String?, issuers: Array<out Principal>?) = arrayOf(ALIAS)
        override fun chooseClientAlias(keyType: Array<out String>?, issuers: Array<out Principal>?, socket: Socket?) = ALIAS
        override fun getServerAliases(keyType: String?, issuers: Array<out Principal>?) = arrayOf(ALIAS)
        override fun chooseServerAlias(keyType: String?, issuers: Array<out Principal>?, socket: Socket?) = ALIAS
        override fun chooseEngineClientAlias(keyType: Array<out String>?, issuers: Array<out Principal>?, engine: SSLEngine?) = ALIAS
        override fun chooseEngineServerAlias(keyType: String?, issuers: Array<out Principal>?, engine: SSLEngine?) = ALIAS
        override fun getCertificateChain(alias: String?) = arrayOf(certificate)
        override fun getPrivateKey(alias: String?) = privateKey
    }

    /** Server context: requests (but does not require) client certificates. */
    val serverContext: SSLContext by lazy {
        SSLContext.getInstance("TLS").apply { init(arrayOf(keyManager), arrayOf(PinnedTrustManager(null)), SecureRandom()) }
    }

    /** Client context pinned to [expectedFingerprint] (null = accept any valid self-signed certificate). */
    fun clientContext(expectedFingerprint: String?): Pair<SSLContext, X509TrustManager> {
        val trust = PinnedTrustManager(expectedFingerprint)
        val context = SSLContext.getInstance("TLS").apply { init(arrayOf(keyManager), arrayOf(trust), SecureRandom()) }
        return context to trust
    }

    companion object {
        private const val ALIAS = "airbridge"
        private const val PASSWORD = "airbridge"

        fun fingerprintOf(cert: X509Certificate): String =
            MessageDigest.getInstance("SHA-256").digest(cert.encoded).joinToString("") { "%02X".format(it) }

        fun loadOrCreate(dir: File): Identity {
            val file = File(dir, "identity.p12")
            if (file.exists()) {
                runCatching {
                    val store = KeyStore.getInstance("PKCS12")
                    file.inputStream().use { store.load(it, PASSWORD.toCharArray()) }
                    val key = store.getKey(ALIAS, PASSWORD.toCharArray()) as PrivateKey
                    val cert = store.getCertificate(ALIAS) as X509Certificate
                    return Identity(cert, key)
                }
            }
            // RSA-2048, like the official LocalSend apps, for maximum compatibility.
            val pair = KeyPairGenerator.getInstance("RSA").apply { initialize(2048) }.generateKeyPair()
            val name = X500Name("CN=AirBridge User")
            val now = System.currentTimeMillis()
            val builder = JcaX509v3CertificateBuilder(
                name,
                BigInteger(64, SecureRandom()).abs(),
                Date(now - 24L * 3600 * 1000),
                Date(now + 20L * 365 * 24 * 3600 * 1000),
                name,
                pair.public,
            )
            val signer = JcaContentSignerBuilder("SHA256withRSA").build(pair.private)
            val cert = JcaX509CertificateConverter().getCertificate(builder.build(signer))
            val store = KeyStore.getInstance("PKCS12").apply { load(null, null) }
            store.setKeyEntry(ALIAS, pair.private, PASSWORD.toCharArray(), arrayOf(cert))
            dir.mkdirs()
            file.outputStream().use { store.store(it, PASSWORD.toCharArray()) }
            return Identity(cert, pair.private)
        }
    }
}

/**
 * Trust manager for self-signed peers: the certificate must be currently valid
 * and correctly self-signed, and (if a pin is given) have that fingerprint.
 * Throwing here aborts the TLS handshake before any request byte is sent.
 */
class PinnedTrustManager(private val expected: String?) : X509TrustManager {
    @Volatile
    var lastFingerprint: String? = null
        private set

    private fun check(chain: Array<out X509Certificate>?) {
        val cert = chain?.firstOrNull() ?: throw CertificateException("no certificate")
        cert.checkValidity()
        try {
            cert.verify(cert.publicKey)
        } catch (e: Exception) {
            throw CertificateException("certificate is not self-signed correctly", e)
        }
        val actual = Identity.fingerprintOf(cert)
        lastFingerprint = actual
        if (expected != null && !actual.equals(expected, ignoreCase = true)) {
            throw CertificateException("certificate fingerprint mismatch")
        }
    }

    override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?) = check(chain)
    override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?) = check(chain)
    override fun getAcceptedIssuers(): Array<X509Certificate> = emptyArray()
}
