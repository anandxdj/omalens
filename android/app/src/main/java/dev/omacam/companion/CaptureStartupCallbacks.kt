package dev.omacam.companion

import android.hardware.camera2.CameraDevice

/** Testable boundary for asynchronous Camera2 startup callbacks. */
internal class CaptureStartupCallbacks(
    private val gate: CaptureCallbackGate,
    private val generation: Long,
    private val publishCamera: () -> Unit,
    private val publishSession: () -> Unit,
    private val fail: (String) -> Unit,
) {
    fun cameraOpened(close: () -> Unit): Boolean =
        gate.publish(generation, publishCamera).also { accepted ->
            if (!accepted) close()
        }

    fun cameraDisconnected(close: () -> Unit) = terminalFailure("Camera disconnected", close)

    fun cameraError(code: Int, close: () -> Unit) =
        terminalFailure(cameraErrorReason(code), close)

    fun sessionConfigured(close: () -> Unit): Boolean =
        gate.publish(generation, publishSession).also { accepted ->
            if (!accepted) close()
        }

    fun sessionConfigureFailed(close: () -> Unit) =
        terminalFailure("Camera capture session configuration failed", close)

    private fun terminalFailure(reason: String, close: () -> Unit) {
        if (gate.stop(generation)) fail(reason)
        close()
    }

    private fun cameraErrorReason(code: Int): String = when (code) {
        CameraDevice.StateCallback.ERROR_CAMERA_IN_USE,
        CameraDevice.StateCallback.ERROR_MAX_CAMERAS_IN_USE -> "Camera is being used by another app"
        else -> "Camera failed with code $code"
    }
}
