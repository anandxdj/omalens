package dev.omacam.companion

import org.junit.Assert.assertFalse
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

class CaptureCallbackGateTest {
    @Test
    fun `stop invalidates a generation before cleanup can run`() {
        val gate = CaptureCallbackGate()
        val generation = gate.start()
        var cleanupObservedInvalidation = false

        assertTrue(gate.stop(generation))
        cleanupObservedInvalidation = !gate.accepts(generation)

        assertTrue(cleanupObservedInvalidation)
        assertFalse(gate.stop(generation))
    }

    @Test
    fun `late callbacks cannot resurrect a stopped generation`() {
        val gate = CaptureCallbackGate()
        val stopped = gate.start()
        var published = false
        gate.stop(stopped)
        val current = gate.start()

        assertFalse(gate.accepts(stopped))
        assertFalse(gate.publish(stopped) { published = true })
        assertFalse(published)
        assertTrue(gate.accepts(current))
    }

    @Test
    fun `only one camera pipeline generation can be active`() {
        val gate = CaptureCallbackGate()
        gate.start()

        assertThrows(IllegalStateException::class.java) { gate.start() }
    }

    @Test
    fun `cancelled startup cannot cross the next acquisition boundary`() {
        val gate = CaptureCallbackGate()
        val generation = gate.start()
        gate.stop(generation)

        assertThrows(IllegalStateException::class.java) { gate.requireCurrent(generation) }
    }

    @Test
    fun `media resource finishing after cancellation is released instead of published`() {
        val gate = CaptureCallbackGate()
        val generation = gate.start()
        var published = false
        var released = false

        assertThrows(IllegalStateException::class.java) {
            gate.acquire(
                generation,
                create = {
                    gate.stop(generation)
                    "encoder"
                },
                publish = { published = true },
                releaseLate = { released = true },
            )
        }

        assertFalse(published)
        assertTrue(released)
    }

    @Test
    fun `media acquisition failure is returned once without retry`() {
        val gate = CaptureCallbackGate()
        val generation = gate.start()
        var attempts = 0

        assertThrows(IllegalStateException::class.java) {
            gate.acquire(
                generation,
                create = {
                    attempts += 1
                    error("codec unavailable")
                },
                publish = {},
                releaseLate = {},
            )
        }

        assertEquals(1, attempts)
    }
}
