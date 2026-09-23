package dev.omacam.companion

import android.annotation.SuppressLint
import android.content.Context
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureRequest
import android.hardware.camera2.CaptureResult
import android.hardware.camera2.TotalCaptureResult
import android.graphics.Rect
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
import kotlin.math.roundToInt
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLSocket

/** Owns exactly one Camera2 -> MediaCodec -> pinned TLS media pipeline. */
internal class CameraStreamer(
    private val context: Context,
    private val trustedCertificate: String,
    private val endpoint: Endpoint,
    private val binding: CaptureBinding,
    private val onCameraState: (JSONObject) -> Unit,
    private val onScreenDimChange: (Boolean) -> Boolean,
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
    @Volatile private var cameraHandler: Handler? = null
    private var requestBuilder: CaptureRequest.Builder? = null
    private var selectedId = ""
    private var characteristics: CameraCharacteristics? = null
    private var supportedCameras: List<SupportedCamera> = emptyList()
    private var pendingCommand: String? = null
    private var pendingControls: JSONObject? = null
    private var statePublished = false
    private var screenDimmed = false
    private var appliedState = JSONObject()

    /** Commands are serialized onto Camera2; only capture results acknowledge application. */
    fun applyControls(commandId: String, generation: Long, controls: JSONObject) {
        val handler = cameraHandler ?: return
        handler.post {
            if (!active.get() || generation != binding.generation) return@post
            if (pendingCommand != null) {
                publishState(commandId, "A camera adjustment is still pending")
                return@post
            }
            try {
                val allowed = setOf(
                    "zoom", "exposure", "torch", "cameraId", "width", "height", "fps",
                    "stop", "previewMirrored", "screenDimmed",
                )
                require(controls.keys().asSequence().all { it in allowed }) { "Unknown camera control" }
                require(!controls.hasValue("cameraId") || supportedCameras.any { it.id == controls.getString("cameraId") }) {
                    "Camera does not support the native 720p30 output"
                }
                require(!controls.hasValue("width") || controls.getInt("width") == TARGET_SIZE.width) {
                    "This native output supports 720p30"
                }
                require(!controls.hasValue("height") || controls.getInt("height") == TARGET_SIZE.height) {
                    "This native output supports 720p30"
                }
                require(!controls.hasValue("fps") || controls.getDouble("fps") == TARGET_FPS.toDouble()) {
                    "This native output supports 720p30"
                }
                require(!controls.hasValue("previewMirrored")) { "Native companion has no preview to mirror" }
                require(!controls.optBoolean("stop", false)) { "Stop must use the session Stop action" }
                val selected = supportedCameras.firstOrNull {
                    it.id == controls.optString("cameraId", selectedId)
                } ?: error("Camera is not available")
                if (selected.id != selectedId) {
                    switchCamera(selected, commandId, controls)
                } else {
                    applyCurrentCameraControls(commandId, controls)
                }
            } catch (error: Exception) {
                pendingCommand = null
                pendingControls = null
                publishState(commandId, error.message ?: "Camera adjustment rejected")
            }
        }
    }

    private fun applyCurrentCameraControls(commandId: String, controls: JSONObject) {
        val handler = cameraHandler ?: error("Camera handler is unavailable")
        val chars = characteristics ?: error("Camera capabilities unavailable")
        val builder = requestBuilder ?: error("Camera is not ready")
        val previousState = JSONObject(appliedState.toString())
        val previousDimmed = screenDimmed
        try {
            validateAndApplyCameraSettings(builder, chars, controls)
            if (controls.hasValue("screenDimmed")) {
                val requested = controls.getBoolean("screenDimmed")
                require(onScreenDimChange(requested)) { "Phone could not change screen brightness" }
                screenDimmed = requested
            }
            pendingCommand = commandId
            pendingControls = JSONObject(controls.toString())
            val request = builder.apply { setTag(commandId) }.build()
            session?.setRepeatingRequest(request, captureResults, handler)
                ?: error("Camera session ended")
            scheduleAcknowledgementTimeout(commandId)
        } catch (error: Exception) {
            pendingCommand = null
            if (screenDimmed != previousDimmed) {
                onScreenDimChange(previousDimmed)
                screenDimmed = previousDimmed
            }
            rebuildConfirmedRequest(previousState)
            throw error
        }
    }

    private fun validateAndApplyCameraSettings(
        builder: CaptureRequest.Builder,
        chars: CameraCharacteristics,
        controls: JSONObject,
    ) {
        if (controls.hasValue("zoom")) {
            val value = controls.getDouble("zoom")
            val zoomRatioRange = chars.get(CameraCharacteristics.CONTROL_ZOOM_RATIO_RANGE)
            val digitalZoomMax = chars.get(CameraCharacteristics.SCALER_AVAILABLE_MAX_DIGITAL_ZOOM)
            val minimum = zoomRatioRange?.lower ?: 1f
            val maximum = zoomRatioRange?.upper ?: digitalZoomMax
            require(value.isFinite() && maximum != null && value >= minimum && value <= maximum) {
                "Unsupported zoom"
            }
            if (zoomRatioRange != null) {
                builder.set(CaptureRequest.CONTROL_ZOOM_RATIO, value.toFloat())
            } else {
                val activeArray = chars.get(CameraCharacteristics.SENSOR_INFO_ACTIVE_ARRAY_SIZE)
                    ?: error("Camera crop information is unavailable")
                val cropWidth = (activeArray.width() / value).toInt().coerceAtLeast(1)
                val cropHeight = (activeArray.height() / value).toInt().coerceAtLeast(1)
                val left = activeArray.left + (activeArray.width() - cropWidth) / 2
                val top = activeArray.top + (activeArray.height() - cropHeight) / 2
                builder.set(CaptureRequest.SCALER_CROP_REGION, Rect(left, top, left + cropWidth, top + cropHeight))
            }
        }
        if (controls.hasValue("exposure")) {
            val requestedEv = controls.getDouble("exposure")
            val range = chars.get(CameraCharacteristics.CONTROL_AE_COMPENSATION_RANGE)
            val step = chars.get(CameraCharacteristics.CONTROL_AE_COMPENSATION_STEP)?.toFloat()
            require(requestedEv.isFinite() && range != null && step != null && step > 0f) {
                "Exposure compensation is unavailable"
            }
            val stepEv = step.toDouble()
            val index = (requestedEv / stepEv).roundToInt()
            require(range.contains(index) && kotlin.math.abs(index * stepEv - requestedEv) <= maxOf(0.0001, stepEv * 0.001)) {
                "Unsupported exposure compensation step"
            }
            builder.set(CaptureRequest.CONTROL_AE_EXPOSURE_COMPENSATION, index)
        }
        if (controls.hasValue("torch")) {
            val torch = controls.getBoolean("torch")
            require(chars.get(CameraCharacteristics.FLASH_INFO_AVAILABLE) == true || !torch) {
                "Torch unavailable"
            }
            builder.set(CaptureRequest.FLASH_MODE, if (torch) CaptureRequest.FLASH_MODE_TORCH else CaptureRequest.FLASH_MODE_OFF)
        }
    }

    private fun rebuildConfirmedRequest(previous: JSONObject) {
        val opened = camera ?: return
        val surface = encoderSurface ?: return
        val chars = characteristics ?: return
        try {
            val builder = opened.createCaptureRequest(CameraDevice.TEMPLATE_RECORD).apply {
                addTarget(surface)
                set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, Range(TARGET_FPS, TARGET_FPS))
            }
            if (previous.hasValue("zoom")) {
                validateAndApplyCameraSettings(builder, chars, JSONObject().put("zoom", previous.getDouble("zoom")))
            }
            if (previous.hasValue("exposure")) {
                validateAndApplyCameraSettings(builder, chars, JSONObject().put("exposure", previous.getDouble("exposure")))
            }
            if (previous.optBoolean("torch", false)) {
                validateAndApplyCameraSettings(builder, chars, JSONObject().put("torch", true))
            }
            requestBuilder = builder
            val request = builder.build()
            session?.setRepeatingRequest(request, captureResults, cameraHandler)
        } catch (_: Exception) {
            // A rejected request is reported to the desktop; the current stream stays fail-closed.
        }
    }

    private fun switchCamera(target: SupportedCamera, commandId: String, controls: JSONObject) {
        val handler = cameraHandler ?: error("Camera handler is unavailable")
        val manager = context.getSystemService(CameraManager::class.java)
        val surface = encoderSurface ?: error("Encoder surface is unavailable")
        val generation = callbackGeneration
        val previousDimmed = screenDimmed
        pendingCommand = commandId
        pendingControls = JSONObject(controls.toString())
        if (controls.hasValue("screenDimmed")) {
            val requested = controls.getBoolean("screenDimmed")
            require(onScreenDimChange(requested)) { "Phone could not change screen brightness" }
            screenDimmed = requested
        }
        scheduleAcknowledgementTimeout(commandId)
        session?.let {
            runCatching { it.stopRepeating() }
            runCatching { it.abortCaptures() }
            it.close()
        }
        session = null
        camera?.close()
        camera = null
        requestBuilder = null
        try {
            manager.openCamera(target.id, object : CameraDevice.StateCallback() {
                override fun onOpened(device: CameraDevice) {
                    if (!active.get() || generation != callbackGeneration) {
                        device.close()
                        return
                    }
                    camera = device
                    try {
                        val targetCharacteristics = manager.getCameraCharacteristics(target.id)
                        device.createCaptureSession(listOf(surface), object : CameraCaptureSession.StateCallback() {
                        override fun onConfigured(configured: CameraCaptureSession) {
                            if (!active.get() || generation != callbackGeneration) {
                                configured.close()
                                device.close()
                                return
                            }
                            try {
                                val builder = device.createCaptureRequest(CameraDevice.TEMPLATE_RECORD).apply {
                                    addTarget(surface)
                                    set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, Range(target.fpsRangeLower, target.fpsRangeUpper))
                                }
                                validateAndApplyCameraSettings(builder, targetCharacteristics, controls)
                                selectedId = target.id
                                characteristics = targetCharacteristics
                                session = configured
                                requestBuilder = builder
                                val request = builder.apply { setTag(commandId) }.build()
                                configured.setRepeatingRequest(request, captureResults, handler)
                            } catch (error: Exception) {
                                configured.close()
                                device.close()
                                camera = null
                                session = null
                                restoreScreenDimmed(previousDimmed)
                                failCameraCommand(commandId, error.message ?: "Camera switch failed")
                            }
                        }

                        override fun onConfigureFailed(configured: CameraCaptureSession) {
                            configured.close()
                            device.close()
                            restoreScreenDimmed(previousDimmed)
                            failCameraCommand(commandId, "Camera session could not be configured")
                        }
                        }, handler)
                    } catch (error: Exception) {
                        device.close()
                        camera = null
                        restoreScreenDimmed(previousDimmed)
                        failCameraCommand(commandId, error.message ?: "Camera could not be configured")
                    }
                }

                override fun onDisconnected(device: CameraDevice) {
                    device.close()
                    restoreScreenDimmed(previousDimmed)
                    failCameraCommand(commandId, "Camera disconnected during switch")
                }

                override fun onError(device: CameraDevice, error: Int) {
                    device.close()
                    restoreScreenDimmed(previousDimmed)
                    failCameraCommand(commandId, "Camera switch failed with code $error")
                }
            }, handler)
        } catch (error: Exception) {
            restoreScreenDimmed(previousDimmed)
            failCameraCommand(commandId, error.message ?: "Camera switch failed")
        }
    }

    private fun restoreScreenDimmed(previous: Boolean) {
        if (screenDimmed != previous) {
            onScreenDimChange(previous)
            screenDimmed = previous
        }
    }

    private fun scheduleAcknowledgementTimeout(commandId: String) {
        cameraHandler?.postDelayed({
            if (pendingCommand == commandId && active.get()) {
                pendingCommand = null
                pendingControls = null
                publishState(commandId, "Camera did not acknowledge adjustment before timeout")
                active.set(false)
            }
        }, 3_000)
    }

    private fun failCameraCommand(commandId: String, reason: String) {
        if (pendingCommand == commandId) {
            pendingCommand = null
            pendingControls = null
            publishState(commandId, reason)
        } else if (pendingCommand == null) {
            publishState("", reason)
        }
        active.set(false)
    }

    private fun JSONObject.hasValue(key: String) = has(key) && !isNull(key)

    private val captureResults = object : CameraCaptureSession.CaptureCallback() {
        override fun onCaptureCompleted(session: CameraCaptureSession, request: CaptureRequest, result: TotalCaptureResult) {
            if (!active.get()) return
            val activeArray = characteristics?.get(CameraCharacteristics.SENSOR_INFO_ACTIVE_ARRAY_SIZE)
            val actualZoom = result.get(CaptureResult.CONTROL_ZOOM_RATIO)?.toDouble()
                ?: result.get(CaptureResult.SCALER_CROP_REGION)?.let { crop ->
                    crop.width().takeIf { it > 0 }?.let { width -> activeArray?.width()?.toDouble()?.div(width) }
                }
            appliedState = JSONObject().put("cameraId", selectedId)
                .put("width", TARGET_SIZE.width).put("height", TARGET_SIZE.height)
                .put("fps", result.get(CaptureResult.CONTROL_AE_TARGET_FPS_RANGE)?.upper ?: TARGET_FPS)
                .put("zoom", actualZoom ?: JSONObject.NULL)
                .put("exposure", result.get(CaptureResult.CONTROL_AE_EXPOSURE_COMPENSATION)?.let { index ->
                    characteristics?.get(CameraCharacteristics.CONTROL_AE_COMPENSATION_STEP)?.toFloat()?.let { index * it }
                } ?: JSONObject.NULL)
                .put("torch", result.get(CaptureResult.FLASH_MODE) == CaptureRequest.FLASH_MODE_TORCH)
                .put("previewMirrored", false).put("screenDimmed", screenDimmed)
            val command = if (request.tag == pendingCommand) pendingCommand else null
            if (!statePublished || command != null) {
                statePublished = true
                if (command == null) {
                    publishState("", null)
                } else {
                    val expected = pendingControls
                    val error = if (expected == null) {
                        "Camera control request was not retained"
                    } else {
                        appliedControlError(expected)
                    }
                    pendingCommand = null
                    pendingControls = null
                    publishState(command, error)
                    if (error != null) active.set(false)
                }
            }
        }
    }

    private fun appliedControlError(controls: JSONObject): String? {
        if (controls.hasValue("cameraId") && appliedState.optString("cameraId") != controls.getString("cameraId")) {
            return "Camera did not acknowledge the selected camera"
        }
        if ((controls.hasValue("width") && appliedState.optInt("width") != controls.getInt("width")) ||
            (controls.hasValue("height") && appliedState.optInt("height") != controls.getInt("height"))
        ) {
            return "Camera did not acknowledge the selected size"
        }
        if (controls.hasValue("fps") &&
            kotlin.math.abs(appliedState.optDouble("fps", Double.NaN) - controls.getDouble("fps")) > 0.001
        ) {
            return "Camera did not acknowledge the selected frame rate"
        }
        if (controls.hasValue("zoom")) {
            val requested = controls.getDouble("zoom")
            val applied = appliedState.optDouble("zoom", Double.NaN)
            if (!applied.isFinite() || kotlin.math.abs(applied - requested) > maxOf(0.02, requested * 0.002)) {
                return "Camera did not apply the requested zoom"
            }
        }
        if (controls.hasValue("exposure")) {
            val requested = controls.getDouble("exposure")
            val applied = appliedState.optDouble("exposure", Double.NaN)
            val step = characteristics?.get(CameraCharacteristics.CONTROL_AE_COMPENSATION_STEP)?.toFloat()
                ?.toDouble() ?: 0.0
            if (!applied.isFinite() || kotlin.math.abs(applied - requested) > maxOf(0.0001, step * 0.001)) {
                return "Camera did not apply the requested exposure compensation"
            }
        }
        if (controls.hasValue("torch") && appliedState.optBoolean("torch") != controls.getBoolean("torch")) {
            return "Camera did not apply the requested torch state"
        }
        if (controls.hasValue("screenDimmed") &&
            appliedState.optBoolean("screenDimmed") != controls.getBoolean("screenDimmed")
        ) {
            return "Phone did not apply the requested screen brightness"
        }
        return null
    }

    private fun publishState(commandId: String, error: String?) {
        if (!active.get() || appliedState.length() == 0) return
        runCatching {
            onCameraState(JSONObject().put("type", "camera_state").put("command_id", commandId)
                .put("generation", binding.generation).put("capabilities", capabilities())
                .put("applied", appliedState).put("error", error?.take(256) ?: JSONObject.NULL))
        }
    }

    private fun capabilities(): JSONObject {
        return CameraCapabilities.toJson(supportedCameras, selectedId, screenDimSupported = true)
    }

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
        plain.tcpNoDelay = true
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

    private fun selectCamera(): SupportedCamera {
        val manager = context.getSystemService(CameraManager::class.java)
        val candidates = manager.cameraIdList.mapNotNull { id ->
            runCatching {
                val characteristics = manager.getCameraCharacteristics(id)
                val map = characteristics.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)
                    ?: return@mapNotNull null
                val sizes = map.getOutputSizes(MediaCodec::class.java) ?: return@mapNotNull null
                val ranges = characteristics.get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES)
                    ?: return@mapNotNull null
                if (!CameraCapabilities.supportsNativeTuple(
                        TARGET_SIZE.width,
                        TARGET_SIZE.height,
                        sizes.map { it.width to it.height },
                        ranges.map { it.lower to it.upper },
                    )
                ) return@mapNotNull null
                val fps = ranges.first { it.lower <= TARGET_FPS && TARGET_FPS <= it.upper }
                val facingValue = characteristics.get(CameraCharacteristics.LENS_FACING)
                val facing = when (facingValue) {
                    CameraCharacteristics.LENS_FACING_FRONT -> "user"
                    CameraCharacteristics.LENS_FACING_BACK -> "environment"
                    CameraCharacteristics.LENS_FACING_EXTERNAL -> "external"
                    else -> "unknown"
                }
                val label = when (facing) {
                    "user" -> "Front camera"
                    "external" -> "External camera"
                    "environment" -> "Rear camera"
                    else -> "Camera"
                }
                val ratioRange = characteristics.get(CameraCharacteristics.CONTROL_ZOOM_RATIO_RANGE)
                val digitalZoom = characteristics.get(CameraCharacteristics.SCALER_AVAILABLE_MAX_DIGITAL_ZOOM)
                val zoomMin = ratioRange?.lower ?: 1f.takeIf { digitalZoom != null && digitalZoom > 1f }
                val zoomMax = ratioRange?.upper ?: digitalZoom?.takeIf { it > 1f }
                val exposureRange = characteristics.get(CameraCharacteristics.CONTROL_AE_COMPENSATION_RANGE)
                    ?.takeIf { it.lower != it.upper }
                val exposureStep = characteristics.get(CameraCharacteristics.CONTROL_AE_COMPENSATION_STEP)
                    ?.toFloat()?.takeIf { it.isFinite() && it > 0f }
                SupportedCamera(
                    id = id,
                    label = label,
                    facing = facing,
                    fpsRangeLower = fps.lower,
                    fpsRangeUpper = fps.upper,
                    zoomMin = zoomMin,
                    zoomMax = zoomMax,
                    exposureMin = exposureRange?.lower?.takeIf { exposureStep != null },
                    exposureMax = exposureRange?.upper?.takeIf { exposureStep != null },
                    exposureStepEv = exposureStep,
                    torch = characteristics.get(CameraCharacteristics.FLASH_INFO_AVAILABLE) == true,
                )
            }.getOrNull()
        }
        supportedCameras = candidates
        return CameraCapabilities.defaultCamera(candidates)
            ?: error("No camera exposes the required 1280×720 at fixed 30 fps tuple")
    }

    @SuppressLint("MissingPermission")
    @Suppress("DEPRECATION")
    private fun configureEncoderAndCamera(selected: SupportedCamera) {
        selectedId = selected.id
        characteristics = context.getSystemService(CameraManager::class.java).getCameraCharacteristics(selected.id)
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
            setInteger(MediaFormat.KEY_MAX_B_FRAMES, 0)
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
        cameraHandler = handler
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
        val builder = opened.createCaptureRequest(CameraDevice.TEMPLATE_RECORD).apply {
            addTarget(surface)
            set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, Range(selected.fpsRangeLower, selected.fpsRangeUpper))
        }
        requestBuilder = builder
        session?.setRepeatingRequest(builder.build(), captureResults, handler)
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
                    val encoded = MediaCodecOutput.copy(buffer, info.offset, info.size)
                    val configuration = info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0
                    val keyFrame = info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0
                    if (configuration) {
                        codecConfig = encoded
                    } else if (encoded.isNotEmpty()) {
                        val payload = if (keyFrame && codecConfig.isNotEmpty()) {
                            MediaCodecOutput.prependCodecConfig(codecConfig, encoded)
                        } else {
                            encoded
                        }
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
        private val TARGET_SIZE = Size(CameraCapabilities.NATIVE_WIDTH, CameraCapabilities.NATIVE_HEIGHT)
        private const val TARGET_FPS = CameraCapabilities.NATIVE_FPS
        private const val LEASE_MILLIS = 10_000L
    }
}
