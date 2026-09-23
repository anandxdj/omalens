package dev.omacam.companion

import org.json.JSONArray
import org.json.JSONObject

/** Camera2 facts narrowed to the one format the native encoder/output path can carry. */
internal data class SupportedCamera(
    val id: String,
    val label: String,
    val facing: String,
    val fpsRangeLower: Int,
    val fpsRangeUpper: Int,
    val zoomMin: Float?,
    val zoomMax: Float?,
    val exposureMin: Int?,
    val exposureMax: Int?,
    val exposureStepEv: Float?,
    val torch: Boolean,
)

internal object CameraCapabilities {
    fun supportsNativeTuple(
        width: Int,
        height: Int,
        sizes: Collection<Pair<Int, Int>>,
        fpsRanges: Collection<Pair<Int, Int>>,
    ): Boolean = (width to height) in sizes && fpsRanges.any { (lower, upper) ->
        lower <= NATIVE_FPS && NATIVE_FPS <= upper
    }

    fun defaultCamera(cameras: List<SupportedCamera>): SupportedCamera? =
        cameras.firstOrNull { it.facing == "environment" } ?: cameras.firstOrNull()

    fun toJson(
        cameras: List<SupportedCamera>,
        selectedId: String,
        screenDimSupported: Boolean,
    ): JSONObject {
        val selected = cameras.firstOrNull { it.id == selectedId }
            ?: error("Selected camera is not in the supported camera list")
        val advertisedCameras = JSONArray()
        for (camera in cameras) {
            advertisedCameras.put(
                JSONObject()
                    .put("id", camera.id)
                    .put("label", camera.label)
                    .put("facing", camera.facing)
                    .put(
                        "modes",
                        JSONArray().put(
                            JSONObject()
                                .put("width", NATIVE_WIDTH)
                                .put("height", NATIVE_HEIGHT)
                                .put("fps", NATIVE_FPS),
                        ),
                    )
                    .put("frameRates", JSONArray().put(NATIVE_FPS)),
            )
        }

        val capabilities = JSONObject()
            .put("cameras", advertisedCameras)
            .put("torch", selected.torch)
            .put("screenDimSupported", screenDimSupported)
            .put("focusModes", JSONArray())
        if (selected.zoomMin != null && selected.zoomMax != null) {
            capabilities.put(
                "zoom",
                JSONObject()
                    .put("min", selected.zoomMin)
                    .put("max", selected.zoomMax)
                    // Camera2 reports a continuous range, not a discrete step size.
                    .put("step", 0.0),
            )
        }
        if (selected.exposureMin != null && selected.exposureMax != null &&
            selected.exposureStepEv != null && selected.exposureStepEv > 0f
        ) {
            capabilities.put(
                "exposureCompensation",
                JSONObject()
                    .put("min", selected.exposureMin * selected.exposureStepEv)
                    .put("max", selected.exposureMax * selected.exposureStepEv)
                    .put("step", selected.exposureStepEv),
            )
        }
        return capabilities
    }

    const val NATIVE_WIDTH = 1280
    const val NATIVE_HEIGHT = 720
    const val NATIVE_FPS = 30
}
