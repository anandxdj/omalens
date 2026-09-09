package dev.omacam.companion

import android.Manifest
import android.content.pm.PackageManager
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.os.Bundle
import android.view.Gravity
import android.view.View
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.content.edit
import androidx.core.content.ContextCompat
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
    private val controlSender = Executors.newSingleThreadExecutor()
    private lateinit var title: TextView
    private lateinit var details: TextView
    private lateinit var primary: Button
    private lateinit var secondary: Button
    @Volatile private var activeSocket: Socket? = null
    @Volatile private var activeControlOutput: BufferedOutputStream? = null
    @Volatile private var cameraStreamer: CameraStreamer? = null
    private val capturePolicy = CaptureSessionPolicy()
    @Volatile private var resolvingService = false
    private var pendingClaim: PreparedClaim? = null
    private var pendingStart: ControlResponse.StartRequest? = null
    private var cameraPermissionRequestInFlight = false
    private var activeTrustedDesktop: TrustedDesktop? = null
    private var activeEndpoint: Endpoint? = null
    private var discoveryListener: NsdManager.DiscoveryListener? = null
    private val cameraPermission = registerForActivityResult(
        ActivityResultContracts.RequestPermission(),
    ) { granted ->
        cameraPermissionRequestInFlight = false
        val request = pendingStart ?: return@registerForActivityResult
        if (granted) approveStart(request) else rejectStart(request, getString(R.string.camera_denied))
    }

    private data class TrustedDesktop(val name: String, val certificateSha256: String)
    private data class ControlConnection(
        val socket: SSLSocket,
        val input: BufferedInputStream,
        val output: BufferedOutputStream,
    )

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        buildUi()
        loadTrustedDesktop()?.let(::showTrusted) ?: showReady()
    }

    override fun onDestroy() {
        stopDiscovery()
        stopCapture("App closed")
        sendControlCommand("disconnect")
        closeActiveSocket()
        worker.shutdownNow()
        controlSender.shutdownNow()
        super.onDestroy()
    }

    override fun onPause() {
        if (cameraStreamer != null ||
            (capturePolicy.phase() != CaptureSessionPolicy.Phase.Idle &&
                !cameraPermissionRequestInFlight)
        ) {
            stopCapture("OmaCam left the foreground")
        }
        super.onPause()
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
        primary.visibility = View.VISIBLE
        primary.isEnabled = true
        primary.setOnClickListener { scanQr() }
        secondary.visibility = View.GONE
        secondary.isEnabled = true
    }

    private fun loadTrustedDesktop(): TrustedDesktop? {
        val preferences = getSharedPreferences("trusted_desktop", MODE_PRIVATE)
        val name = preferences.getString("name", null) ?: return null
        val certificate = preferences.getString("certificate_sha256", null) ?: return null
        return try {
            certificate.decodeBase64Url().also { require(it.size == 32) }
            TrustedDesktop(name, certificate)
        } catch (_: Exception) {
            preferences.edit { clear() }
            null
        }
    }

    private fun showTrusted(trusted: TrustedDesktop, message: String? = null) {
        pendingClaim = null
        pendingStart = null
        title.text = getString(R.string.paired_with, trusted.name)
        details.text = message ?: getString(R.string.ready_to_reconnect)
        primary.visibility = View.VISIBLE
        primary.isEnabled = true
        primary.setText(R.string.find_desktop)
        primary.setOnClickListener { discoverTrustedDesktop(trusted) }
        secondary.visibility = View.VISIBLE
        secondary.isEnabled = true
        secondary.setText(R.string.forget_desktop)
        secondary.setOnClickListener { forgetDesktop() }
    }

    private fun forgetDesktop() {
        stopDiscovery()
        stopCapture("Laptop forgotten")
        getSharedPreferences("trusted_desktop", MODE_PRIVATE).edit { clear() }
        primary.isEnabled = false
        secondary.isEnabled = false
        controlSender.execute {
            sendControlCommand("forget_peer")
            closeActiveSocket()
            runOnUiThread {
                if (!isDestroyed) showReady(getString(R.string.desktop_forgotten))
            }
        }
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
            closeActiveSocket()
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
                    showTrusted(
                        TrustedDesktop(
                            claim.invitation.desktopName,
                            claim.invitation.certificateSha256,
                        ),
                        getString(R.string.paired_success),
                    )
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

    private fun discoverTrustedDesktop(trusted: TrustedDesktop) {
        stopDiscovery()
        resolvingService = false
        details.setText(R.string.searching_desktop)
        primary.isEnabled = false
        val manager = getSystemService(NsdManager::class.java)
        val listener = object : NsdManager.DiscoveryListener {
            override fun onDiscoveryStarted(serviceType: String) = Unit

            override fun onServiceFound(serviceInfo: NsdServiceInfo) {
                if (resolvingService) return
                resolvingService = true
                @Suppress("DEPRECATION")
                manager.resolveService(
                    serviceInfo,
                    object : NsdManager.ResolveListener {
                        override fun onResolveFailed(serviceInfo: NsdServiceInfo, errorCode: Int) {
                            resolvingService = false
                        }

                        override fun onServiceResolved(serviceInfo: NsdServiceInfo) {
                            val advertisedCertificate = serviceInfo.attributes["cert"]
                                ?.toString(StandardCharsets.UTF_8)
                            if (advertisedCertificate != trusted.certificateSha256) {
                                resolvingService = false
                                return
                            }
                            val address = serviceInfo.host?.hostAddress
                            if (address == null || serviceInfo.port !in 1..65535) {
                                resolvingService = false
                                return
                            }
                            stopDiscovery()
                            authenticateControl(trusted, Endpoint(address, serviceInfo.port))
                        }
                    },
                )
            }

            override fun onServiceLost(serviceInfo: NsdServiceInfo) = Unit

            override fun onDiscoveryStopped(serviceType: String) = Unit

            override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
                stopDiscovery()
                runOnUiThread {
                    showTrusted(trusted, getString(R.string.discovery_failed, errorCode))
                }
            }

            override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) = Unit
        }
        discoveryListener = listener
        try {
            manager.discoverServices("_omacam._tcp.", NsdManager.PROTOCOL_DNS_SD, listener)
        } catch (error: Exception) {
            discoveryListener = null
            showTrusted(trusted, "Discovery failed: ${error.message}")
        }
    }

    private fun stopDiscovery() {
        val listener = discoveryListener ?: return
        discoveryListener = null
        resolvingService = false
        try {
            getSystemService(NsdManager::class.java).stopServiceDiscovery(listener)
        } catch (_: IllegalArgumentException) {
            // Discovery was already stopped by Android.
        }
    }

    private fun authenticateControl(trusted: TrustedDesktop, endpoint: Endpoint) {
        details.setText(R.string.authenticating_desktop)
        worker.execute {
            try {
                val connection = authenticateControlOverTls(trusted, endpoint)
                activeControlOutput = connection.output
                activeTrustedDesktop = trusted
                activeEndpoint = endpoint
                runOnUiThread {
                    showConnected(trusted, getString(R.string.control_connected))
                }
                maintainControlConnection(connection)
            } catch (error: Exception) {
                runOnUiThread {
                    if (!isDestroyed) {
                        loadTrustedDesktop()?.let {
                            showTrusted(it, "Connection ended: ${error.message}")
                        } ?: showReady(getString(R.string.desktop_forgotten))
                    }
                }
            } finally {
                stopCapture("Authenticated control ended")
                activeControlOutput = null
                activeTrustedDesktop = null
                activeEndpoint = null
                closeActiveSocket()
                activeSocket = null
            }
        }
    }

    private fun authenticateControlOverTls(
        trusted: TrustedDesktop,
        endpoint: Endpoint,
    ): ControlConnection {
        val trustManager = PairingProtocol.pinningTrustManager(
            trusted.certificateSha256.decodeBase64Url(),
        )
        val context = SSLContext.getInstance("TLSv1.3").apply {
            init(null, arrayOf(trustManager), null)
        }
        val plainSocket = Socket()
        activeSocket = plainSocket
        plainSocket.connect(InetSocketAddress(endpoint.host, endpoint.port), 5_000)
        plainSocket.soTimeout = 10_000
        val socket = context.socketFactory.createSocket(
            plainSocket,
            endpoint.host,
            endpoint.port,
            true,
        ) as SSLSocket
        activeSocket = socket
        socket.enabledProtocols = arrayOf("TLSv1.3")
        socket.startHandshake()
        val input = BufferedInputStream(socket.inputStream)
        val hello = PairingProtocol.parseControlHello(
            readBoundedLine(input),
            trusted.certificateSha256,
        )
        val proof = PairingProtocol.prepareControlProof(hello)
        val output = BufferedOutputStream(socket.outputStream)
        output.write(proof.toJson().toByteArray(StandardCharsets.UTF_8))
        output.write('\n'.code)
        output.flush()
        val result = JSONObject(readBoundedLine(input))
        require(result.keys().asSequence().toSet() == setOf("type", "version", "capture_authorized")) {
            "Desktop response fields are not recognized"
        }
        require(result.getString("type") == "authenticated") { "Desktop rejected identity" }
        require(result.getInt("version") == 1) { "Desktop selected an incompatible version" }
        require(!result.getBoolean("capture_authorized")) {
            "Desktop attempted to start capture during reconnect"
        }
        return ControlConnection(socket, input, output)
    }

    private fun maintainControlConnection(connection: ControlConnection) {
        while (!Thread.currentThread().isInterrupted && !connection.socket.isClosed) {
            Thread.sleep(2_000)
            synchronized(connection.output) {
                connection.output.write("{\"type\":\"ping\"}\n".toByteArray(StandardCharsets.UTF_8))
                connection.output.flush()
            }
            handleControlResponse(ControlProtocol.parseResponse(readBoundedLine(connection.input)))
        }
    }

    private fun handleControlResponse(response: ControlResponse) {
        when (response) {
            is ControlResponse.Pong -> {
                val streamer = cameraStreamer
                val streaming = capturePolicy.phase() is CaptureSessionPolicy.Phase.Streaming
                if (response.captureAuthorized && streamer != null && streaming) {
                    streamer.refreshLease()
                } else if (response.captureAuthorized || streamer != null || streaming) {
                    stopCapture("Capture authorization state changed unexpectedly")
                    error("Desktop capture state does not match the phone")
                }
            }
            is ControlResponse.StartRequest -> runOnUiThread { showStartRequest(response) }
            is ControlResponse.CaptureGranted -> startCapture(response.requestId, response.binding)
            is ControlResponse.Stopped -> {
                stopCapture(response.reason)
                activeTrustedDesktop?.let { trusted ->
                    runOnUiThread { showConnected(trusted, getString(R.string.capture_ended, response.reason)) }
                }
            }
            is ControlResponse.StopCapture -> stopCapture(response.reason)
            ControlResponse.Forgotten -> Unit
        }
    }

    private fun showStartRequest(request: ControlResponse.StartRequest) {
        if (cameraStreamer != null || pendingStart != null ||
            capturePolicy.phase() != CaptureSessionPolicy.Phase.Idle
        ) {
            sendControlCommand("start_rejected", request.requestId)
            return
        }
        capturePolicy.offer(request.requestId)
        pendingStart = request
        title.text = getString(R.string.share_camera_title, request.desktopName)
        details.setText(R.string.share_camera_details)
        primary.visibility = View.VISIBLE
        primary.isEnabled = true
        primary.setText(R.string.share_camera)
        primary.setOnClickListener {
            if (ContextCompat.checkSelfPermission(this, Manifest.permission.CAMERA) ==
                PackageManager.PERMISSION_GRANTED
            ) {
                approveStart(request)
            } else {
                cameraPermissionRequestInFlight = true
                cameraPermission.launch(Manifest.permission.CAMERA)
            }
        }
        secondary.visibility = View.VISIBLE
        secondary.isEnabled = true
        secondary.setText(R.string.decline)
        secondary.setOnClickListener { rejectStart(request, getString(R.string.capture_declined)) }
    }

    private fun approveStart(request: ControlResponse.StartRequest) {
        require(pendingStart == request) { "Capture request is no longer pending" }
        capturePolicy.approve(request.requestId)
        pendingStart = null
        primary.isEnabled = false
        secondary.isEnabled = false
        details.setText(R.string.starting_camera)
        sendControlCommand("start_approved", request.requestId)
    }

    private fun rejectStart(request: ControlResponse.StartRequest, message: String) {
        if (pendingStart != request) return
        if (!capturePolicy.reject(request.requestId)) return
        pendingStart = null
        sendControlCommand("start_rejected", request.requestId)
        val trusted = activeTrustedDesktop ?: return
        showTrusted(trusted, message)
    }

    private fun startCapture(requestId: String, binding: CaptureBinding) {
        require(cameraStreamer == null) { "A capture pipeline is already active" }
        capturePolicy.grant(requestId, binding)
        val trusted = activeTrustedDesktop ?: error("Trusted desktop context is unavailable")
        val endpoint = activeEndpoint ?: error("Desktop endpoint is unavailable")
        lateinit var streamer: CameraStreamer
        streamer = CameraStreamer(this, trusted.certificateSha256, endpoint, binding) { reason ->
            if (cameraStreamer === streamer) {
                capturePolicy.stop()
                cameraStreamer = null
                runOnUiThread {
                    if (!isDestroyed && cameraStreamer == null &&
                        capturePolicy.phase() == CaptureSessionPolicy.Phase.Idle
                    ) {
                        showConnected(trusted, getString(R.string.capture_ended, reason))
                    }
                }
            }
        }
        cameraStreamer = streamer
        runOnUiThread {
            title.text = getString(R.string.sharing_with, trusted.name)
            details.setText(R.string.camera_live)
            primary.visibility = View.VISIBLE
            primary.isEnabled = true
            primary.setText(R.string.stop_camera)
            primary.setOnClickListener { stopCapture("Stopped on phone") }
            secondary.visibility = View.GONE
        }
        streamer.start()
    }

    private fun stopCapture(reason: String) {
        val previous = capturePolicy.stop()
        pendingStart = null
        cameraPermissionRequestInFlight = false
        val streamer = cameraStreamer
        cameraStreamer = null
        streamer?.stop(reason)
        if (previous != CaptureSessionPolicy.Phase.Idle) sendControlCommand("stop")
        val trusted = activeTrustedDesktop
        runOnUiThread {
            if (!isDestroyed && trusted != null) showConnected(trusted, getString(R.string.capture_ended, reason))
        }
    }

    private fun showConnected(trusted: TrustedDesktop, message: String) {
        title.text = getString(R.string.connected_to, trusted.name)
        details.text = message
        primary.visibility = View.GONE
        secondary.visibility = View.VISIBLE
        secondary.isEnabled = true
        secondary.setText(R.string.forget_desktop)
        secondary.setOnClickListener { forgetDesktop() }
    }

    private fun sendControlCommand(type: String, requestId: String? = null) {
        val output = activeControlOutput ?: return
        try {
            synchronized(output) {
                output.write(ControlProtocol.command(type, requestId).toByteArray(StandardCharsets.UTF_8))
                output.write('\n'.code)
                output.flush()
            }
        } catch (_: Exception) {
            // Local Stop/Forget still takes effect if the peer is unreachable.
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
        try {
            socket.close()
        } catch (_: Exception) {
            // Pairing has already been committed; a close failure is harmless.
        }
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

    private fun closeActiveSocket() {
        try {
            activeSocket?.close()
        } catch (_: Exception) {
            // Teardown remains best-effort; local trust/capture state is authoritative.
        }
    }
}
