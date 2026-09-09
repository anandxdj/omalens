package dev.omacam.companion

import android.annotation.SuppressLint
import android.content.Context
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureRequest
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.os.Handler
import android.os.HandlerThread
import android.os.SystemClock
import android.util.Range
import android.util.Size
import android.view.Surface
import org.json.JSONObject
import java.io.BufferedInputStream
import java.io.BufferedOutputStream
import java.io.DataOutputStream
import java.net.InetSocketAddress
import java.net.Socket
import java.nio.charset.StandardCharsets
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.atomic.AtomicReference
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLSocket

/** Owns exactly one Camera2 -> MediaCodec -> pinned TLS media pipeline. */
internal class CameraStreamer(
    private val context: Context,
    private val trustedCertificate: String,
    private val endpoint: Endpoint,
    private val binding: CaptureBinding,
    private val onStopped: (String) -> Unit,
) {
    private val active = AtomicBoolean(false)
    private val leaseDeadline = AtomicLong(0)
    private val terminalSent = AtomicBoolean(false)
    private val nextSequence = AtomicLong(0)
    private val callbackGate = CaptureCallbackGate()
    @Volatile private var callbackGeneration = 0L
    private val thread = Thread(::run, "omacam-camera-stream")
    @Volatile private var socket: Socket? = null
    @Volatile private var output: DataOutputStream? = null
    @Volatile private var camera: CameraDevice? = null
    @Volatile private var session: CameraCaptureSession? = null
    @Volatile private var encoder: MediaCodec? = null
    @Volatile private var encoderSurface: Surface? = null
    @Volatile private var cameraThread: HandlerThread? = null

    fun start() {
        check(active.compareAndSet(false, true)) { "Capture pipeline is already active" }
        callbackGeneration = callbackGate.start()
        refreshLease()
        thread.start()
    }

    fun refreshLease() {
        if (active.get()) leaseDeadline.set(SystemClock.elapsedRealtime() + LEASE_MILLIS)
    }

    /** Invalidate first, then send terminal Stop and release owned resources. */
    fun stop(reason: String) {
        val wasActive = active.getAndSet(false)
        callbackGate.stop(callbackGeneration)
        // Closing the transport first interrupts a blocked media write. Stop is
        // local and must not wait for a stalled network peer before cleanup.
        try {
            socket?.close()
        } catch (_: Exception) {
            // The remaining owned resources still need to be released.
        }
        if (wasActive || output != null) sendTerminalStop()
        if (Thread.currentThread() !== thread) thread.join(1_000)
        releaseResources()
    }

    private fun run() {
        val generation = callbackGeneration
        var reason = "Capture ended"
        try {
            callbackGate.acquire(
                generation,
                create = { openAuthenticatedMedia(generation) },
                publish = { output = DataOutputStream(it) },
                releaseLate = { it.close() },
            )
            callbackGate.requireCurrent(generation)
            val selectedCamera = selectCamera()
            configureEncoderAndCamera(selectedCamera)
            drainEncoder()
            if (SystemClock.elapsedRealtime() >= leaseDeadline.get()) {
                reason = "Capture authorization lease expired"
            }
        } catch (error: Exception) {
            reason = error.message ?: "Camera pipeline failed"
        } finally {
            active.set(false)
            callbackGate.stop(generation)
            sendTerminalStop()
            releaseResources()
            onStopped(reason)
        }
    }

    private fun openAuthenticatedMedia(generation: Long): BufferedOutputStream {
        val trustManager = PairingProtocol.pinningTrustManager(trustedCertificate.decodeBase64Url())
        val tlsContext = SSLContext.getInstance("TLSv1.3").apply {
            init(null, arrayOf(trustManager), null)
        }
        val plain = Socket()
        if (!callbackGate.publish(generation) { socket = plain }) {
            plain.close()
            error("Capture startup was cancelled")
        }
        plain.connect(InetSocketAddress(endpoint.host, endpoint.port), 5_000)
        plain.soTimeout = 10_000
        val tls = tlsContext.socketFactory.createSocket(
            plain,
            endpoint.host,
            endpoint.port,
            true,
        ) as SSLSocket
        if (!callbackGate.publish(generation) { socket = tls }) {
            tls.close()
            error("Capture startup was cancelled")
        }
        tls.enabledProtocols = arrayOf("TLSv1.3")
        tls.startHandshake()
        val input = BufferedInputStream(tls.inputStream)
        val hello = PairingProtocol.parseControlHello(readBoundedLine(input), trustedCertificate)
        val proof = PairingProtocol.prepareControlProof(hello)
        val rawOutput = BufferedOutputStream(tls.outputStream)
        writeLine(rawOutput, proof.toJson())
        val authenticated = JSONObject(readBoundedLine(input))
        require(authenticated.keys().asSequence().toSet() == setOf("type", "version", "capture_authorized")) {
            "Media authentication response is malformed"
        }
        require(authenticated.getString("type") == "authenticated") {
            "Desktop rejected media identity"
        }
        require(authenticated.getInt("version") == 1) { "Desktop selected an incompatible version" }
        require(!authenticated.getBoolean("capture_authorized")) {
            "Media authentication attempted to grant capture"
        }
        writeLine(rawOutput, binding.mediaOpenJson())
        val ready = JSONObject(readBoundedLine(input))
        require(ready.keys().asSequence().toSet() == setOf("type")) {
            "Media-ready response is malformed"
        }
        require(ready.getString("type") == "media_ready") { "Desktop rejected capture binding" }
        return rawOutput
    }

    private data class SelectedCamera(val id: String, val fpsRange: Range<Int>)

    private fun selectCamera(): SelectedCamera {
        val manager = context.getSystemService(CameraManager::class.java)
        val candidates = manager.cameraIdList.mapNotNull { id ->
            val characteristics = manager.getCameraCharacteristics(id)
            val map = characteristics.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)
                ?: return@mapNotNull null
            val sizes = map.getOutputSizes(MediaCodec::class.java) ?: emptyArray()
            if (TARGET_SIZE !in sizes) return@mapNotNull null
            val ranges = characteristics.get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES)
                ?: return@mapNotNull null
            val exact = ranges.firstOrNull { it.lower == TARGET_FPS && it.upper == TARGET_FPS }
                ?: return@mapNotNull null
            Triple(id, characteristics.get(CameraCharacteristics.LENS_FACING), exact)
        }
        val selected = candidates.firstOrNull { it.second == CameraCharacteristics.LENS_FACING_BACK }
            ?: candidates.firstOrNull()
            ?: error("No camera exposes the required 1280×720 at fixed 30 fps tuple")
        return SelectedCamera(selected.first, selected.third)
    }

    @SuppressLint("MissingPermission")
    @Suppress("DEPRECATION")
    private fun configureEncoderAndCamera(selected: SelectedCamera) {
        val generation = callbackGeneration
        val codec = callbackGate.acquire(
            generation,
            create = { MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_VIDEO_AVC) },
            publish = { encoder = it },
            releaseLate = { it.release() },
        )
        val format = MediaFormat.createVideoFormat(
            MediaFormat.MIMETYPE_VIDEO_AVC,
            TARGET_SIZE.width,
            TARGET_SIZE.height,
        ).apply {
            setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
            setInteger(MediaFormat.KEY_BIT_RATE, 4_000_000)
            setInteger(MediaFormat.KEY_FRAME_RATE, TARGET_FPS)
            setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 1)
            setInteger(MediaFormat.KEY_PROFILE, MediaCodecInfo.CodecProfileLevel.AVCProfileBaseline)
            setInteger(MediaFormat.KEY_LEVEL, MediaCodecInfo.CodecProfileLevel.AVCLevel31)
        }
        codec.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        callbackGate.requireCurrent(generation)
        val surface = callbackGate.acquire(
            generation,
            create = codec::createInputSurface,
            publish = { encoderSurface = it },
            releaseLate = { it.release() },
        )
        codec.start()
        callbackGate.requireCurrent(generation)

        val handlerThread = callbackGate.acquire(
            generation,
            create = { HandlerThread("omacam-camera2").also { it.start() } },
            publish = { cameraThread = it },
            releaseLate = {
                it.quitSafely()
                it.join(1_000)
            },
        )
        val handler = Handler(handlerThread.looper)
        val manager = context.getSystemService(CameraManager::class.java)
        val cameraLatch = CountDownLatch(1)
        val failure = AtomicReference<String?>()
        lateinit var openedCamera: CameraDevice
        lateinit var configuredSession: CameraCaptureSession
        val callbacks = CaptureStartupCallbacks(
            callbackGate,
            generation,
            publishCamera = { camera = openedCamera },
            publishSession = { session = configuredSession },
            fail = { reason ->
                failure.compareAndSet(null, reason)
                active.set(false)
            },
        )
        manager.openCamera(
            selected.id,
            object : CameraDevice.StateCallback() {
                override fun onOpened(device: CameraDevice) {
                    openedCamera = device
                    callbacks.cameraOpened(device::close)
                    cameraLatch.countDown()
                }

                override fun onDisconnected(device: CameraDevice) {
                    callbacks.cameraDisconnected(device::close)
                    cameraLatch.countDown()
                }

                override fun onError(device: CameraDevice, error: Int) {
                    callbacks.cameraError(error, device::close)
                    cameraLatch.countDown()
                }
            },
            handler,
        )
        require(cameraLatch.await(5, TimeUnit.SECONDS)) { "Timed out opening camera" }
        failure.get()?.let(::error)
        callbackGate.requireCurrent(generation)
        val opened = camera ?: error("Camera did not open")
        val sessionLatch = CountDownLatch(1)
        opened.createCaptureSession(
            listOf(surface),
            object : CameraCaptureSession.StateCallback() {
                override fun onConfigured(configured: CameraCaptureSession) {
                    configuredSession = configured
                    callbacks.sessionConfigured(configured::close)
                    sessionLatch.countDown()
                }

                override fun onConfigureFailed(configured: CameraCaptureSession) {
                    callbacks.sessionConfigureFailed(configured::close)
                    sessionLatch.countDown()
                }
            },
            handler,
        )
        require(sessionLatch.await(5, TimeUnit.SECONDS)) { "Timed out configuring camera" }
        failure.get()?.let(::error)
        callbackGate.requireCurrent(generation)
        val request = opened.createCaptureRequest(CameraDevice.TEMPLATE_RECORD).apply {
            addTarget(surface)
            set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, selected.fpsRange)
        }.build()
        session?.setRepeatingRequest(request, null, handler)
            ?: error("Camera capture session was not created")
    }

    private fun drainEncoder() {
        val codec = encoder ?: error("Encoder is unavailable")
        val info = MediaCodec.BufferInfo()
        var codecConfig = ByteArray(0)
        while (active.get() && SystemClock.elapsedRealtime() < leaseDeadline.get()) {
            val index = codec.dequeueOutputBuffer(info, 50_000)
            when {
                index == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> {
                    codecConfig = codec.outputFormat.codecConfiguration()
                }
                index >= 0 -> {
                    val buffer = codec.getOutputBuffer(index) ?: error("Encoder output buffer is missing")
                    val encoded = ByteArray(info.size)
                    buffer.position(info.offset)
                    buffer.limit(info.offset + info.size)
                    buffer.get(encoded)
                    val configuration = info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0
                    val keyFrame = info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0
                    if (configuration) {
                        codecConfig = encoded
                    } else if (encoded.isNotEmpty()) {
                        val payload = if (keyFrame && codecConfig.isNotEmpty()) codecConfig + encoded else encoded
                        val sequence = nextSequence.getAndIncrement()
                        writeMediaRecord(sequence, info.presentationTimeUs, keyFrame, payload)
                    }
                    codec.releaseOutputBuffer(index, false)
                }
            }
        }
    }

    private fun MediaFormat.codecConfiguration(): ByteArray {
        val pieces = listOf("csd-0", "csd-1").mapNotNull { key ->
            getByteBuffer(key)?.let { buffer ->
                val duplicate = buffer.duplicate()
                ByteArray(duplicate.remaining()).also(duplicate::get)
            }
        }
        return pieces.fold(ByteArray(0)) { result, bytes -> result + bytes }
    }

    @Synchronized
    private fun writeMediaRecord(sequence: Long, timestampUs: Long, keyFrame: Boolean, payload: ByteArray) {
        if (!active.get()) return
        val target = output ?: error("Authenticated media output is unavailable")
        MediaProtocol.writeAccessUnit(target, binding, sequence, timestampUs, keyFrame, payload)
    }

    @Synchronized
    private fun sendTerminalStop() {
        if (!terminalSent.compareAndSet(false, true)) return
        val target = output ?: return
        try {
            MediaProtocol.writeStop(target, binding, nextSequence.get())
        } catch (_: Exception) {
            // Local invalidation and deterministic release do not depend on reachability.
        }
    }

    private fun releaseResources() {
        val owned = synchronized(this) {
            OwnedResources(
                session = session.also { session = null },
                camera = camera.also { camera = null },
                encoder = encoder.also { encoder = null },
                encoderSurface = encoderSurface.also { encoderSurface = null },
                output = output.also { output = null },
                socket = socket.also { socket = null },
                cameraThread = cameraThread.also { cameraThread = null },
            )
        }
        CaptureResourceCleanup.release(
            { owned.session?.stopRepeating() },
            { owned.session?.abortCaptures() },
            { owned.session?.close() },
            { owned.camera?.close() },
            { owned.encoder?.stop() },
            { owned.encoder?.release() },
            { owned.encoderSurface?.release() },
            { owned.output?.close() },
            { owned.socket?.close() },
            { owned.cameraThread?.quitSafely() },
            { owned.cameraThread?.join(1_000) },
        )
    }

    private data class OwnedResources(
        val session: CameraCaptureSession?,
        val camera: CameraDevice?,
        val encoder: MediaCodec?,
        val encoderSurface: Surface?,
        val output: DataOutputStream?,
        val socket: Socket?,
        val cameraThread: HandlerThread?,
    )

    private fun readBoundedLine(input: BufferedInputStream): String {
        val bytes = java.io.ByteArrayOutputStream()
        while (bytes.size() <= MAX_CONTROL_BYTES) {
            val next = input.read()
            require(next >= 0) { "Desktop closed media authentication" }
            if (next == '\n'.code) return bytes.toString(StandardCharsets.UTF_8.name())
            bytes.write(next)
        }
        error("Desktop media response is too large")
    }

    private fun writeLine(output: BufferedOutputStream, text: String) {
        output.write(text.toByteArray(StandardCharsets.UTF_8))
        output.write('\n'.code)
        output.flush()
    }

    companion object {
        private val TARGET_SIZE = Size(1280, 720)
        private const val TARGET_FPS = 30
        private const val LEASE_MILLIS = 10_000L
    }
}
