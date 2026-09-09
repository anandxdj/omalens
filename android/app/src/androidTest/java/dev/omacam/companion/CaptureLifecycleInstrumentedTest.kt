package dev.omacam.companion

import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/** Device-runnable checks for the authorization/generation boundary used by Camera2 callbacks. */
@RunWith(AndroidJUnit4::class)
class CaptureLifecycleInstrumentedTest {
    private val binding = CaptureBinding(
        peer = "11".repeat(32),
        connection = "22".repeat(16),
        session = "33".repeat(16),
        generation = 7,
    )

    @Test
    fun stopInvalidatesAuthorizationBeforeCleanupAndRejectsLateGrant() {
        val policy = CaptureSessionPolicy()
        policy.offer("request")
        policy.approve("request")
        var cleanupSawIdle = false

        assertEquals(CaptureSessionPolicy.Phase.AwaitingGrant("request"), policy.stop())
        CaptureResourceCleanup.release({ cleanupSawIdle = policy.phase() == CaptureSessionPolicy.Phase.Idle })

        assertTrue(cleanupSawIdle)
        assertThrows(IllegalStateException::class.java) { policy.grant("request", binding) }
    }

    @Test
    fun stoppedGenerationCannotPublishCameraOrSessionIntoRestart() {
        val gate = CaptureCallbackGate()
        val stopped = gate.start()
        gate.stop(stopped)
        val current = gate.start()
        val published = mutableListOf<String>()
        val released = mutableListOf<String>()
        val callbacks = CaptureStartupCallbacks(
            gate,
            stopped,
            publishCamera = { published += "camera" },
            publishSession = { published += "session" },
            fail = { published += "failure" },
        )

        assertFalse(callbacks.cameraOpened { released += "camera" })
        assertFalse(callbacks.sessionConfigured { released += "session" })

        assertEquals(emptyList<String>(), published)
        assertEquals(listOf("camera", "session"), released)
        assertTrue(gate.accepts(current))
    }

    @Test
    fun everyThrowingCleanupStillAttemptsEveryOwnedRelease() {
        val releases = mutableListOf<Int>()
        val failures = CaptureResourceCleanup.release(
            *Array(10) { index ->
                {
                    releases += index
                    throw IllegalStateException("failure-$index")
                }
            },
        )

        assertEquals((0 until 10).toList(), releases)
        assertEquals(10, failures.size)
    }

    @Test
    fun terminalCameraFailureInvalidatesOnceAndNeverRetries() {
        val gate = CaptureCallbackGate()
        val generation = gate.start()
        var failures = 0
        var closes = 0
        val callbacks = CaptureStartupCallbacks(
            gate,
            generation,
            publishCamera = {},
            publishSession = {},
            fail = { failures += 1 },
        )

        callbacks.cameraError(4) { closes += 1 }
        callbacks.cameraError(4) { closes += 1 }

        assertEquals(1, failures)
        assertEquals(2, closes)
        assertFalse(gate.accepts(generation))
    }
}
