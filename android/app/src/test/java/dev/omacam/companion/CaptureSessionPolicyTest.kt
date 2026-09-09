package dev.omacam.companion

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

class CaptureSessionPolicyTest {
    private val binding = CaptureBinding(
        peer = "11".repeat(32),
        connection = "22".repeat(16),
        session = "33".repeat(16),
        generation = 7,
    )

    @Test
    fun `explicit consent is required before a grant`() {
        val policy = CaptureSessionPolicy()

        assertThrows(IllegalStateException::class.java) { policy.grant("request-a", binding) }
        policy.offer("request-a")
        assertThrows(IllegalStateException::class.java) { policy.grant("request-a", binding) }
        policy.approve("request-a")
        assertThrows(IllegalStateException::class.java) { policy.grant("request-b", binding) }
        policy.grant("request-a", binding)

        assertEquals(CaptureSessionPolicy.Phase.Streaming(binding), policy.phase())
    }

    @Test
    fun `approval is bound to the exact pending request`() {
        val policy = CaptureSessionPolicy()
        policy.offer("request-a")

        assertThrows(IllegalStateException::class.java) { policy.approve("request-b") }
        assertEquals(CaptureSessionPolicy.Phase.AwaitingConsent("request-a"), policy.phase())
    }

    @Test
    fun `second request and duplicate grant cannot replace owner`() {
        val policy = CaptureSessionPolicy()
        policy.offer("request-a")
        assertThrows(IllegalStateException::class.java) { policy.offer("request-b") }
        policy.approve("request-a")
        policy.grant("request-a", binding)

        assertThrows(IllegalStateException::class.java) {
            policy.grant("request-a", binding.copy(generation = 8))
        }
        assertEquals(CaptureSessionPolicy.Phase.Streaming(binding), policy.phase())
    }

    @Test
    fun `reject and stop invalidate before a late grant`() {
        val policy = CaptureSessionPolicy()
        policy.offer("request-a")
        assertFalse(policy.reject("request-b"))
        assertTrue(policy.reject("request-a"))
        assertEquals(CaptureSessionPolicy.Phase.Idle, policy.phase())
        assertThrows(IllegalStateException::class.java) { policy.grant("request-a", binding) }

        policy.offer("request-c")
        policy.approve("request-c")
        assertEquals(CaptureSessionPolicy.Phase.AwaitingGrant("request-c"), policy.stop())
        assertEquals(CaptureSessionPolicy.Phase.Idle, policy.phase())
        assertThrows(IllegalStateException::class.java) { policy.grant("request-c", binding) }
    }

    @Test
    fun `new process policy never persists authorization`() {
        val old = CaptureSessionPolicy()
        old.offer("request-a")
        old.approve("request-a")
        old.grant("request-a", binding)

        assertEquals(CaptureSessionPolicy.Phase.Idle, CaptureSessionPolicy().phase())
    }
}
