package dev.omacam.companion

import android.os.Bundle
import android.view.Gravity
import android.view.View
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.edit
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.codescanner.GmsBarcodeScannerOptions
import com.google.mlkit.vision.codescanner.GmsBarcodeScanning
import org.json.JSONObject
import java.io.BufferedInputStream
import java.io.BufferedOutputStream
import java.net.InetSocketAddress
import java.net.Socket
import java.nio.charset.StandardCharsets
import java.time.Instant
import java.util.concurrent.Executors
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLSocket

class MainActivity : AppCompatActivity() {
    private val worker = Executors.newSingleThreadExecutor()
    private lateinit var title: TextView
    private lateinit var details: TextView
    private lateinit var primary: Button
    private lateinit var secondary: Button
    @Volatile private var activeSocket: Socket? = null
    private var pendingClaim: PreparedClaim? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        buildUi()
        showReady()
    }

    override fun onDestroy() {
        activeSocket?.close()
        worker.shutdownNow()
        super.onDestroy()
    }

    private fun buildUi() {
        val density = resources.displayMetrics.density
        val padding = (24 * density).toInt()
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(padding, padding, padding, padding)
            setBackgroundColor(getColor(R.color.omacam_surface))
        }
        title = TextView(this).apply {
            setTextColor(getColor(R.color.omacam_text))
            textSize = 28f
        }
        details = TextView(this).apply {
            setTextColor(getColor(R.color.omacam_muted))
            textSize = 17f
            setPadding(0, (16 * density).toInt(), 0, (28 * density).toInt())
        }
        primary = Button(this)
        secondary = Button(this).apply { visibility = View.GONE }
        root.addView(title)
        root.addView(details)
        root.addView(primary)
        root.addView(secondary)
        setContentView(root)
    }

    private fun showReady(message: String? = null) {
        pendingClaim = null
        title.setText(R.string.pair_title)
        details.text = message ?: getString(R.string.pair_intro)
        primary.setText(R.string.scan_qr)
        primary.isEnabled = true
        primary.setOnClickListener { scanQr() }
        secondary.visibility = View.GONE
    }

    private fun scanQr() {
        val options = GmsBarcodeScannerOptions.Builder()
            .setBarcodeFormats(Barcode.FORMAT_QR_CODE)
            .enableAutoZoom()
            .build()
        primary.isEnabled = false
        GmsBarcodeScanning.getClient(this, options).startScan()
            .addOnSuccessListener { barcode ->
                val raw = barcode.rawValue ?: return@addOnSuccessListener showReady("QR has no text payload.")
                try {
                    val invitation = PairingProtocol.parseInvitation(raw, Instant.now().epochSecond)
                    showConfirmation(PairingProtocol.prepareClaim(invitation))
                } catch (error: Exception) {
                    showReady(error.message ?: "Invalid OmaCam QR.")
                }
            }
            .addOnCanceledListener { showReady(getString(R.string.scan_cancelled)) }
            .addOnFailureListener { error -> showReady("Scanner failed: ${error.message}") }
    }

    private fun showConfirmation(claim: PreparedClaim) {
        pendingClaim = claim
        title.text = claim.invitation.desktopName
        details.text = getString(R.string.confirm_code, claim.sas)
        primary.setText(R.string.approve_pairing)
        primary.isEnabled = true
        primary.setOnClickListener { connectAndPair(claim) }
        secondary.visibility = View.VISIBLE
        secondary.setText(R.string.cancel)
        secondary.setOnClickListener {
            activeSocket?.close()
            showReady()
        }
    }

    private fun connectAndPair(claim: PreparedClaim) {
        primary.isEnabled = false
        secondary.isEnabled = false
        details.setText(R.string.waiting_desktop)
        worker.execute {
            try {
                pairOverTls(claim)
                getSharedPreferences("trusted_desktop", MODE_PRIVATE).edit {
                    putString("name", claim.invitation.desktopName)
                    putString("certificate_sha256", claim.invitation.certificateSha256)
                    putLong("paired_at", Instant.now().epochSecond)
                }
                runOnUiThread {
                    title.text = getString(R.string.paired_with, claim.invitation.desktopName)
                    details.setText(R.string.paired_success)
                    primary.visibility = View.GONE
                    secondary.visibility = View.GONE
                }
            } catch (error: Exception) {
                runOnUiThread {
                    secondary.isEnabled = true
                    showReady("Pairing failed: ${error.message}")
                }
            } finally {
                activeSocket = null
            }
        }
    }

    private fun pairOverTls(claim: PreparedClaim) {
        val trustManager = PairingProtocol.pinningTrustManager(
            claim.invitation.certificateSha256.decodeBase64Url(),
        )
        val context = SSLContext.getInstance("TLSv1.3").apply {
            init(null, arrayOf(trustManager), null)
        }
        val endpoint = claim.invitation.endpoint
        val plainSocket = Socket()
        activeSocket = plainSocket
        plainSocket.connect(InetSocketAddress(endpoint.host, endpoint.port), 5_000)
        plainSocket.soTimeout = 130_000
        val socket = context.socketFactory.createSocket(
            plainSocket,
            endpoint.host,
            endpoint.port,
            true,
        ) as SSLSocket
        activeSocket = socket
        socket.enabledProtocols = arrayOf("TLSv1.3")
        socket.startHandshake()
        val output = BufferedOutputStream(socket.outputStream)
        output.write(claim.toJson().toByteArray(StandardCharsets.UTF_8))
        output.write('\n'.code)
        output.flush()
        val input = BufferedInputStream(socket.inputStream)
        val confirmation = JSONObject(readBoundedLine(input))
        require(confirmation.getString("type") == "confirm") {
            confirmation.optString("reason", "desktop rejected claim")
        }
        require(confirmation.getString("sas") == claim.sas) { "confirmation code mismatch" }
        require(confirmation.getString("desktop_name") == claim.invitation.desktopName) {
            "desktop name mismatch"
        }
        require(confirmation.getString("phone_name") == claim.phoneName) { "phone name mismatch" }
        runOnUiThread { details.setText(R.string.waiting_desktop) }
        val result = JSONObject(readBoundedLine(input))
        require(result.getString("type") == "paired") {
            result.optString("reason", "desktop rejected pairing")
        }
        socket.close()
    }

    private fun readBoundedLine(input: BufferedInputStream): String {
        val bytes = java.io.ByteArrayOutputStream()
        while (bytes.size() <= MAX_CONTROL_BYTES) {
            val next = input.read()
            require(next >= 0) { "desktop closed the connection" }
            if (next == '\n'.code) {
                return bytes.toString(StandardCharsets.UTF_8.name())
            }
            bytes.write(next)
        }
        throw IllegalArgumentException("desktop response is too large")
    }
}
