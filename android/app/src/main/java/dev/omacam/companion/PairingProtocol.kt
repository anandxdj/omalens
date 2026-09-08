package dev.omacam.companion

import android.annotation.SuppressLint
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import org.json.JSONObject
import java.io.ByteArrayOutputStream
import java.net.InetAddress
import java.nio.ByteBuffer
import java.nio.charset.StandardCharsets
import java.security.KeyPair
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.MessageDigest
import java.security.SecureRandom
import java.security.Signature
import java.security.cert.X509Certificate
import javax.net.ssl.X509TrustManager

internal const val MAX_QR_BYTES = 4_096
internal const val MAX_CONTROL_BYTES = 65_536
internal const val INVITATION_LIFETIME_SECONDS = 120L
internal const val MAX_CLOCK_SKEW_SECONDS = 30L
internal const val IDENTITY_ALIAS = "omacam-phone-identity-v1"

internal data class Endpoint(val host: String, val port: Int)

internal data class Invitation(
    val version: Int,
    val sessionId: String,
    val secret: String,
    val challenge: String,
    val certificateSha256: String,
    val endpointText: String,
    val endpoint: Endpoint,
    val desktopName: String,
    val expiresAtUnixSeconds: Long,
)

internal data class PreparedClaim(
    val invitation: Invitation,
    val phoneName: String,
    val publicKeyDer: ByteArray,
    val phoneNonce: ByteArray,
    val signatureDer: ByteArray,
    val sas: String,
) {
    fun toJson(): String = JSONObject()
        .put("v", invitation.version)
        .put("sid", invitation.sessionId)
        .put("token", invitation.secret)
        .put("name", phoneName)
        .put("key", publicKeyDer.base64Url())
        .put("nonce", phoneNonce.base64Url())
        .put("sig", signatureDer.base64Url())
        .toString()
}

internal object PairingProtocol {
    private val exactInvitationKeys = setOf(
        "v", "sid", "token", "challenge", "cert", "endpoint", "name", "expires",
    )

    fun parseInvitation(raw: String, nowUnixSeconds: Long): Invitation {
        require(raw.toByteArray(StandardCharsets.UTF_8).size <= MAX_QR_BYTES) {
            "QR payload is too large"
        }
        val json = JSONObject(raw)
        require(json.keys().asSequence().toSet() == exactInvitationKeys) {
            "QR fields are not recognized"
        }
        val version = json.getInt("v")
        require(version == 1) { "This QR uses an unsupported OmaCam version" }
        val sessionId = json.getString("sid").also { it.decodeExact(16) }
        val secret = json.getString("token").also { it.decodeExact(32) }
        val challenge = json.getString("challenge").also { it.decodeExact(32) }
        val certificate = json.getString("cert").also { it.decodeExact(32) }
        val endpointText = json.getString("endpoint")
        val endpoint = parseEndpoint(endpointText)
        val desktopName = json.getString("name")
        require(desktopName.isNotEmpty() && desktopName.toByteArray().size <= 64) {
            "Desktop name is invalid"
        }
        require(desktopName.all(::isSafeDisplayNameCharacter)) { "Desktop name is invalid" }
        val expires = json.getLong("expires")
        require(
            expires + MAX_CLOCK_SKEW_SECONDS > nowUnixSeconds &&
                expires <= nowUnixSeconds + INVITATION_LIFETIME_SECONDS + MAX_CLOCK_SKEW_SECONDS,
        ) {
            "This pairing QR expired"
        }
        return Invitation(
            version,
            sessionId,
            secret,
            challenge,
            certificate,
            endpointText,
            endpoint,
            desktopName,
            expires,
        )
    }

    fun prepareClaim(invitation: Invitation): PreparedClaim {
        val rawPhoneName = listOf(Build.MANUFACTURER, Build.MODEL)
            .filter(String::isNotBlank)
            .joinToString(" ")
            .filter(::isSafeDisplayNameCharacter)
        val phoneName = truncateUtf8(rawPhoneName.ifEmpty { "Android phone" }, 64)
        val keyPair = loadOrCreateIdentity()
        val nonce = ByteArray(32).also(SecureRandom()::nextBytes)
        val transcript = claimTranscript(invitation, phoneName, keyPair.public.encoded, nonce)
        val signer = Signature.getInstance("SHA256withECDSA")
        signer.initSign(keyPair.private)
        signer.update(transcript)
        val signature = signer.sign()
        val digest = MessageDigest.getInstance("SHA-256").digest(transcript)
        val sasNumber = (ByteBuffer.wrap(digest, 0, 4).int.toLong() and 0xffff_ffffL) % 1_000_000L
        return PreparedClaim(
            invitation,
            phoneName,
            keyPair.public.encoded,
            nonce,
            signature,
            "%06d".format(sasNumber),
        )
    }

    // The self-signed leaf is an ephemeral channel certificate authenticated by
    // its exact SHA-256 digest in the physically scanned invitation. Normal CA
    // validation cannot establish that local, non-DNS identity.
    @SuppressLint("CustomX509TrustManager")
    fun pinningTrustManager(expectedDigest: ByteArray): X509TrustManager =
        object : X509TrustManager {
            override fun getAcceptedIssuers(): Array<X509Certificate> = emptyArray()

            override fun checkClientTrusted(chain: Array<X509Certificate>?, authType: String?) {
                throw java.security.cert.CertificateException("client certificates are not accepted")
            }

            override fun checkServerTrusted(chain: Array<X509Certificate>?, authType: String?) {
                if (chain.isNullOrEmpty()) {
                    throw java.security.cert.CertificateException("server sent no certificate")
                }
                chain[0].checkValidity()
                val actual = MessageDigest.getInstance("SHA-256").digest(chain[0].encoded)
                if (!MessageDigest.isEqual(actual, expectedDigest)) {
                    throw java.security.cert.CertificateException("desktop identity does not match QR")
                }
            }
        }

    private fun loadOrCreateIdentity(): KeyPair {
        val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        val existing = keyStore.getEntry(IDENTITY_ALIAS, null) as? KeyStore.PrivateKeyEntry
        if (existing != null) {
            return KeyPair(existing.certificate.publicKey, existing.privateKey)
        }
        val generator = KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC, "AndroidKeyStore")
        val parameters = KeyGenParameterSpec.Builder(
            IDENTITY_ALIAS,
            KeyProperties.PURPOSE_SIGN or KeyProperties.PURPOSE_VERIFY,
        )
            .setAlgorithmParameterSpec(java.security.spec.ECGenParameterSpec("secp256r1"))
            .setDigests(KeyProperties.DIGEST_SHA256)
            .setUserAuthenticationRequired(false)
            .build()
        generator.initialize(parameters)
        return generator.generateKeyPair()
    }

    private fun claimTranscript(
        invitation: Invitation,
        phoneName: String,
        publicKeyDer: ByteArray,
        phoneNonce: ByteArray,
    ): ByteArray {
        val output = ByteArrayOutputStream()
        output.write("OMACAM-PAIR-CLAIM-V1\u0000".toByteArray(StandardCharsets.UTF_8))
        appendField(output, invitation.sessionId.toByteArray(StandardCharsets.UTF_8))
        appendField(output, invitation.challenge.toByteArray(StandardCharsets.UTF_8))
        appendField(output, invitation.certificateSha256.toByteArray(StandardCharsets.UTF_8))
        appendField(output, invitation.endpointText.toByteArray(StandardCharsets.UTF_8))
        appendField(output, invitation.desktopName.toByteArray(StandardCharsets.UTF_8))
        appendField(output, phoneName.toByteArray(StandardCharsets.UTF_8))
        appendField(output, publicKeyDer)
        appendField(output, phoneNonce)
        return output.toByteArray()
    }

    private fun appendField(output: ByteArrayOutputStream, field: ByteArray) {
        output.write(ByteBuffer.allocate(4).putInt(field.size).array())
        output.write(field)
    }

    private fun parseEndpoint(value: String): Endpoint {
        val host: String
        val portText: String
        if (value.startsWith("[")) {
            val closing = value.indexOf(']')
            require(closing > 1 && value.getOrNull(closing + 1) == ':') { "Endpoint is invalid" }
            host = value.substring(1, closing)
            portText = value.substring(closing + 2)
        } else {
            val separator = value.lastIndexOf(':')
            require(separator > 0 && value.indexOf(':') == separator) { "Endpoint is invalid" }
            host = value.substring(0, separator)
            portText = value.substring(separator + 1)
        }
        require(host.all { it.isDigit() || it == '.' || it == ':' || it in 'a'..'f' || it in 'A'..'F' }) {
            "Endpoint must contain a numeric IP address"
        }
        val port = portText.toIntOrNull()
        require(port != null && port in 1..65535) { "Endpoint port is invalid" }
        val address = InetAddress.getByName(host)
        val bytes = address.address
        val isIpv6UniqueLocal = bytes.size == 16 && (bytes[0].toInt() and 0xfe) == 0xfc
        require(address.isSiteLocalAddress || address.isLinkLocalAddress || isIpv6UniqueLocal) {
            "Endpoint must be a private or link-local address"
        }
        return Endpoint(host, port)
    }
}

private fun isSafeDisplayNameCharacter(character: Char): Boolean =
    character.isLetterOrDigit() || character in " -_.'()+"

private fun truncateUtf8(value: String, maximumBytes: Int): String {
    val result = StringBuilder()
    for (character in value) {
        val candidate = result.toString() + character
        if (candidate.toByteArray(StandardCharsets.UTF_8).size > maximumBytes) break
        result.append(character)
    }
    return result.toString()
}

internal fun String.decodeBase64Url(): ByteArray =
    Base64.decode(this, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING)

private fun String.decodeExact(size: Int): ByteArray = decodeBase64Url().also {
    require(it.size == size) { "QR field has an invalid length" }
}

private fun ByteArray.base64Url(): String =
    Base64.encodeToString(this, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING)
