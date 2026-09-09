package dev.omacam.companion

import android.hardware.camera2.CameraDevice
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CaptureStartupCallbacksTest {
    private data class Fixture(
        val gate: CaptureCallbackGate,
        val generation: Long,
        val events: MutableList<String>,
        val callbacks: CaptureStartupCallbacks,
    )

    private fun fixture(): Fixture {
        val gate = CaptureCallbackGate()
        val generation = gate.start()
        val events = mutableListOf<String>()
        return Fixture(
            gate,
            generation,
            events,
            CaptureStartupCallbacks(
                gate,
                generation,
                publishCamera = { events += "camera-owned" },
                publishSession = { events += "session-owned" },
                fail = { events += "failed:$it" },
            ),
        )
    }

    @Test
    fun `camera busy is terminal and is not retried`() {
        val fixture = fixture()
        fixture.callbacks.cameraError(4) { fixture.events += "camera-closed" }
        fixture.callbacks.cameraError(4) { fixture.events += "late-camera-closed" }

        assertEquals(
            listOf("failed:Camera failed with code 4", "camera-closed", "late-camera-closed"),
            fixture.events,
        )
        assertFalse(fixture.gate.accepts(fixture.generation))
    }

    @Test
    fun `camera in use reports a user-actionable busy error`() {
        val fixture = fixture()

        fixture.callbacks.cameraError(CameraDevice.StateCallback.ERROR_CAMERA_IN_USE) {
            fixture.events += "camera-closed"
        }

        assertEquals(
            listOf("failed:Camera is being used by another app", "camera-closed"),
            fixture.events,
        )
        assertFalse(fixture.gate.accepts(fixture.generation))
    }

    @Test
    fun `disconnect invalidates before closing the camera`() {
        val fixture = fixture()
        fixture.callbacks.cameraDisconnected {
            assertFalse(fixture.gate.accepts(fixture.generation))
            fixture.events += "camera-closed"
        }

        assertEquals(
            listOf("failed:Camera disconnected", "camera-closed"),
            fixture.events,
        )
    }

    @Test
    fun `stop during partial acquisition rejects and closes late session`() {
        val fixture = fixture()
        assertTrue(fixture.callbacks.cameraOpened { fixture.events += "camera-closed" })
        fixture.gate.stop(fixture.generation)

        assertFalse(fixture.callbacks.sessionConfigured { fixture.events += "session-closed" })
        assertEquals(listOf("camera-owned", "session-closed"), fixture.events)
    }

    @Test
    fun `session configuration failure is terminal`() {
        val fixture = fixture()
        fixture.callbacks.cameraOpened { fixture.events += "camera-closed" }
        fixture.callbacks.sessionConfigureFailed { fixture.events += "session-closed" }

        assertEquals(
            listOf(
                "camera-owned",
                "failed:Camera capture session configuration failed",
                "session-closed",
            ),
            fixture.events,
        )
    }

    @Test
    fun `lease expiry cancellation closes every later callback resource`() {
        val fixture = fixture()
        fixture.gate.stop(fixture.generation)

        fixture.callbacks.cameraOpened { fixture.events += "camera-closed" }
        fixture.callbacks.sessionConfigured { fixture.events += "session-closed" }

        assertEquals(listOf("camera-closed", "session-closed"), fixture.events)
    }
}
