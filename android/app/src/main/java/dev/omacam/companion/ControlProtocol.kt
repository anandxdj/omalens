package dev.omacam.companion

import org.json.JSONObject
import java.nio.charset.StandardCharsets

internal sealed interface ControlResponse {
    data class Pong(val captureAuthorized: Boolean) : ControlResponse
    data class StartRequest(val requestId: String, val desktopName: String) : ControlResponse
    data class CaptureGranted(val requestId: String, val binding: CaptureBinding) : ControlResponse
    data class Stopped(val reason: String) : ControlResponse
    data class StopCapture(val reason: String) : ControlResponse
    data object Forgotten : ControlResponse
}

internal data class CaptureBinding(
    val peer: String,
    val connection: String,
    val session: String,
    val generation: Long,
) {
    fun mediaOpenJson(): String = JSONObject()
        .put("type", "media_open")
        .put("peer", peer)
        .put("connection", connection)
        .put("session", session)
        .put("generation", generation)
        .toString()
}

internal object ControlProtocol {
    fun parseResponse(raw: String): ControlResponse {
        require(raw.toByteArray(StandardCharsets.UTF_8).size <= MAX_CONTROL_BYTES) {
            "Desktop control message is too large"
        }
        val json = JSONObject(raw)
        return when (json.getString("type")) {
            "pong" -> {
                requireKeys(json, "type", "capture_authorized")
                ControlResponse.Pong(json.getBoolean("capture_authorized"))
            }
            "start_request" -> {
                requireKeys(
                    json,
                    "type",
                    "request_id",
                    "desktop_name",
                    "width",
                    "height",
                    "fps",
                    "codec",
                )
                val requestId = json.getString("request_id").also { it.decodeExactControl(16) }
                val name = json.getString("desktop_name")
                require(name.isNotEmpty() && name.toByteArray().size <= 64) {
                    "Desktop name is invalid"
                }
                require(json.getInt("width") == 1280 && json.getInt("height") == 720) {
                    "Desktop requested an unsupported capture size"
                }
                require(json.getInt("fps") == 30) { "Desktop requested an unsupported frame rate" }
                require(json.getString("codec") == "h264-constrained-baseline") {
                    "Desktop requested an unsupported codec"
                }
                ControlResponse.StartRequest(requestId, name)
            }
            "capture_granted" -> {
                requireKeys(
                    json,
                    "type",
                    "request_id",
                    "peer",
                    "connection",
                    "session",
                    "generation",
                )
                val requestId = json.getString("request_id").also { it.decodeExactControl(16) }
                val peer = json.getString("peer").also { requireHex(it, 64) }
                val connection = json.getString("connection").also { requireHex(it, 32) }
                val session = json.getString("session").also { requireHex(it, 32) }
                val generation = json.getLong("generation")
                require(generation > 0) { "Capture generation is invalid" }
                ControlResponse.CaptureGranted(
                    requestId,
                    CaptureBinding(peer, connection, session, generation),
                )
            }
            "stopped" -> {
                requireKeys(json, "type", "reason")
                val reason = json.getString("reason")
                require(reason.toByteArray().size <= 256) { "Stop reason is too large" }
                ControlResponse.Stopped(reason)
            }
            "stop_capture" -> {
                requireKeys(json, "type", "reason")
                val reason = json.getString("reason")
                require(reason.toByteArray().size <= 256) { "Stop reason is too large" }
                ControlResponse.StopCapture(reason)
            }
            "forgotten" -> {
                requireKeys(json, "type")
                ControlResponse.Forgotten
            }
            else -> error("Desktop sent an unexpected control response")
        }
    }

    fun command(type: String, requestId: String? = null): String = JSONObject()
        .put("type", type)
        .apply { if (requestId != null) put("request_id", requestId) }
        .toString()

    private fun requireKeys(json: JSONObject, vararg keys: String) {
        require(json.keys().asSequence().toSet() == keys.toSet()) {
            "Desktop control fields are not recognized"
        }
    }

    private fun requireHex(value: String, length: Int) {
        require(value.length == length && value.all { it.isDigit() || it.lowercaseChar() in 'a'..'f' }) {
            "Capture binding is malformed"
        }
    }
}

private fun String.decodeExactControl(size: Int): ByteArray = decodeBase64Url().also {
    require(it.size == size) { "Control field has an invalid length" }
}
