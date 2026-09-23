package dev.omacam.companion

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CameraCapabilitiesTest {
    @Test
    fun `native tuple filter requires the advertised size and a frame rate range containing target`() {
        assertTrue(
            CameraCapabilities.supportsNativeTuple(
                1280,
                720,
                listOf(1280 to 720, 1920 to 1080),
                listOf(15 to 30),
            ),
        )
        assertTrue(
            CameraCapabilities.supportsNativeTuple(
                1280,
                720,
                listOf(1280 to 720),
                listOf(30 to 60),
            ),
        )
        assertFalse(
            CameraCapabilities.supportsNativeTuple(
                1920,
                1080,
                listOf(1280 to 720),
                listOf(15 to 30),
            ),
        )
        assertFalse(
            CameraCapabilities.supportsNativeTuple(
                1280,
                720,
                listOf(1280 to 720),
                listOf(24 to 29),
            ),
        )
        assertFalse(
            CameraCapabilities.supportsNativeTuple(
                1280,
                720,
                listOf(1280 to 720),
                listOf(31 to 60),
            ),
        )
    }

    @Test
    fun `capabilities list only the native mode and selected camera controls`() {
        val cameras = listOf(
            SupportedCamera(
                id = "0",
                label = "Rear camera",
                facing = "environment",
                fpsRangeLower = 30,
                fpsRangeUpper = 30,
                zoomMin = 1f,
                zoomMax = 4f,
                exposureMin = -6,
                exposureMax = 6,
                exposureStepEv = 0.33333334f,
                torch = true,
            ),
            SupportedCamera(
                id = "1",
                label = "Front camera",
                facing = "user",
                fpsRangeLower = 30,
                fpsRangeUpper = 30,
                zoomMin = null,
                zoomMax = null,
                exposureMin = null,
                exposureMax = null,
                exposureStepEv = null,
                torch = false,
            ),
        )

        assertEquals("0", CameraCapabilities.defaultCamera(cameras)?.id)
        val rear = CameraCapabilities.toJson(cameras, "0", screenDimSupported = true)
        assertEquals(2, rear.getJSONArray("cameras").length())
        assertEquals(1280, rear.getJSONArray("cameras").getJSONObject(0)
            .getJSONArray("modes").getJSONObject(0).getInt("width"))
        assertEquals(30, rear.getJSONArray("cameras").getJSONObject(1)
            .getJSONArray("frameRates").getInt(0))
        assertEquals(4.0, rear.getJSONObject("zoom").getDouble("max"), 0.0001)
        assertEquals(0.0, rear.getJSONObject("zoom").getDouble("step"), 0.0001)
        assertEquals(0.33333334, rear.getJSONObject("exposureCompensation").getDouble("step"), 0.0001)
        assertTrue(rear.getBoolean("torch"))
        assertTrue(rear.getBoolean("screenDimSupported"))

        val front = CameraCapabilities.toJson(cameras, "1", screenDimSupported = true)
        assertFalse(front.has("zoom"))
        assertFalse(front.has("exposureCompensation"))
        assertFalse(front.getBoolean("torch"))
    }
}
