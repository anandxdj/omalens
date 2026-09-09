package dev.omacam.companion

import org.junit.Assert.assertEquals
import org.junit.Assert.assertSame
import org.junit.Test

class CaptureResourceCleanupTest {
    @Test
    fun `cleanup continues in order after platform failures`() {
        val calls = mutableListOf<String>()
        val first = IllegalStateException("session stop failed")
        val second = SecurityException("camera close failed")

        val failures = CaptureResourceCleanup.release(
            { calls += "invalidate" },
            { calls += "session-stop"; throw first },
            { calls += "session-close" },
            { calls += "camera-close"; throw second },
            { calls += "encoder-release" },
            { calls += "surface-release" },
            { calls += "socket-close" },
            { calls += "thread-stop" },
        )

        assertEquals(
            listOf(
                "invalidate",
                "session-stop",
                "session-close",
                "camera-close",
                "encoder-release",
                "surface-release",
                "socket-close",
                "thread-stop",
            ),
            calls,
        )
        assertEquals(2, failures.size)
        assertSame(first, failures[0])
        assertSame(second, failures[1])
    }

    @Test
    fun `empty cleanup is safe`() {
        assertEquals(emptyList<Exception>(), CaptureResourceCleanup.release())
    }
}
