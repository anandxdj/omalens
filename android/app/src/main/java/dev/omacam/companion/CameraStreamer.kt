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
    private val thread = Thread(::run, "omacam-camera-stream")
    @Volatile private var socket: SSLSocket? = null
    @Volatile private var output: DataOutputStream? = null
    @Volatile private var camera: CameraDevice? = null
    @Volatile private var session: CameraCaptureSession? = null
    @Volatile private var encoder: MediaCodec? = null
    @Volatile private var encoderSurface: Surface? = null
    @Volatile private var cameraThread: HandlerThread? = null

    fun start() {
        check(active.compareAndSet(false, true)) { "Capture pipeline is already active" }
        refreshLease()
        thread.start()
    }

    fun refreshLease() {
        if (active.get()) leaseDeadline.set(SystemClock.elapsedRealtime() + LEASE_MILLIS)
    }

    /** Invalidate first, then send terminal Stop and release owned resources. */
    fun stop(reason: String) {
        if (active.getAndSet(false)) {
            sendTerminalStop()
            socket?.close()
        }
        if (Thread.currentThread() !== thread) thread.join(1_000)
        releaseResources()
    }

    private fun run() {
        var reason = "Capture ended"
        try {
            val mediaOutput = openAuthenticatedMedia()
            output = DataOutputStream(mediaOutput)
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
            sendTerminalStop()
            releaseResources()
            onStopped(reason)
        }
    }

    private fun openAuthenticatedMedia(): BufferedOutputStream {
        val trustManager = PairingProtocol.pinningTrustManager(trustedCertificate.decodeBase64Url())
        val tlsContext = SSLContext.getInstance("TLSv1.3").apply {
            init(null, arrayOf(trustManager), null)
        }
        val plain = Socket()
        plain.connect(InetSocketAddress(endpoint.host, endpoint.port), 5_000)
        plain.soTimeout = 10_000
        val tls = tlsContext.socketFactory.createSocket(
            plain,
            endpoint.host,
            endpoint.port,
            true,
        ) as SSLSocket
        socket = tls
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
        val codec = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_VIDEO_AVC)
        encoder = codec
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
        val surface = codec.createInputSurface()
        encoderSurface = surface
        codec.start()

        val handlerThread = HandlerThread("omacam-camera2").also { it.start() }
        cameraThread = handlerThread
        val handler = Handler(handlerThread.looper)
        val manager = context.getSystemService(CameraManager::class.java)
        val cameraLatch = CountDownLatch(1)
        val failure = AtomicReference<String?>()
        manager.openCamera(
            selected.id,
            object : CameraDevice.StateCallback() {
                override fun onOpened(device: CameraDevice) {
                    camera = device
                    cameraLatch.countDown()
                }

                override fun onDisconnected(device: CameraDevice) {
                    failure.set("Camera disconnected")
                    device.close()
                    cameraLatch.countDown()
                }

                override fun onError(device: CameraDevice, error: Int) {
                    failure.set("Camera failed with code $error")
                    device.close()
                    cameraLatch.countDown()
                }
            },
            handler,
        )
        require(cameraLatch.await(5, TimeUnit.SECONDS)) { "Timed out opening camera" }
        failure.get()?.let(::error)
        val opened = camera ?: error("Camera did not open")
        val sessionLatch = CountDownLatch(1)
        opened.createCaptureSession(
            listOf(surface),
            object : CameraCaptureSession.StateCallback() {
                override fun onConfigured(configured: CameraCaptureSession) {
                    session = configured
                    sessionLatch.countDown()
                }

                override fun onConfigureFailed(configured: CameraCaptureSession) {
                    failure.set("Camera capture session configuration failed")
                    configured.close()
                    sessionLatch.countDown()
                }
            },
            handler,
        )
        require(sessionLatch.await(5, TimeUnit.SECONDS)) { "Timed out configuring camera" }
        failure.get()?.let(::error)
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
                        require(payload.size <= MAX_ACCESS_UNIT_BYTES) { "Encoded access unit exceeds 1 MiB" }
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
        target.write(MEDIA_MAGIC)
        target.writeByte(1)
        target.writeByte(1)
        target.writeShort(if (keyFrame) 1 else 0)
        target.write(binding.peer.hexBytes(32))
        target.write(binding.connection.hexBytes(16))
        target.write(binding.session.hexBytes(16))
        target.writeLong(binding.generation)
        target.writeLong(sequence)
        target.writeLong(timestampUs)
        target.writeInt(payload.size)
        target.write(payload)
        target.flush()
    }

    @Synchronized
    private fun sendTerminalStop() {
        if (!terminalSent.compareAndSet(false, true)) return
        val target = output ?: return
        try {
            target.write(MEDIA_MAGIC)
            target.writeByte(1)
            target.writeByte(2)
            target.writeShort(0)
            target.write(binding.peer.hexBytes(32))
            target.write(binding.connection.hexBytes(16))
            target.write(binding.session.hexBytes(16))
            target.writeLong(binding.generation)
            target.writeLong(nextSequence.get())
            target.writeLong(0)
            target.writeInt(0)
            target.flush()
        } catch (_: Exception) {
            // Local invalidation and deterministic release do not depend on reachability.
        }
    }

    @Synchronized
    private fun releaseResources() {
        try { session?.stopRepeating() } catch (_: Exception) { }
        try { session?.abortCaptures() } catch (_: Exception) { }
        session?.close()
        session = null
        camera?.close()
        camera = null
        try { encoder?.stop() } catch (_: Exception) { }
        encoder?.release()
        encoder = null
        encoderSurface?.release()
        encoderSurface = null
        output = null
        try { socket?.close() } catch (_: Exception) { }
        socket = null
        cameraThread?.quitSafely()
        cameraThread = null
    }

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
        private val MEDIA_MAGIC = "OMACAMM1".toByteArray(StandardCharsets.US_ASCII)
        private val TARGET_SIZE = Size(1280, 720)
        private const val TARGET_FPS = 30
        private const val LEASE_MILLIS = 10_000L
        private const val MAX_ACCESS_UNIT_BYTES = 1_048_576
    }
}

private fun String.hexBytes(expected: Int): ByteArray {
    require(length == expected * 2) { "Capture binding length is invalid" }
    return ByteArray(expected) { index ->
        substring(index * 2, index * 2 + 2).toInt(16).toByte()
    }
}
