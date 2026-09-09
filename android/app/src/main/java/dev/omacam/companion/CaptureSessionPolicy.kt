package dev.omacam.companion

/**
 * Phone-side capture authorization state.
 *
 * Authenticated control is deliberately insufficient to enter [Phase.Streaming]:
 * the exact request must first be offered to the user and explicitly approved.
 * This state is memory-only so an activity/process restart always returns idle.
 */
internal class CaptureSessionPolicy {
    internal sealed interface Phase {
        data object Idle : Phase
        data class AwaitingConsent(val requestId: String) : Phase
        data class AwaitingGrant(val requestId: String) : Phase
        data class Streaming(val binding: CaptureBinding) : Phase
    }

    private var phase: Phase = Phase.Idle

    @Synchronized
    fun phase(): Phase = phase

    @Synchronized
    fun offer(requestId: String) {
        check(phase == Phase.Idle) { "Another capture request is active" }
        phase = Phase.AwaitingConsent(requestId)
    }

    @Synchronized
    fun approve(requestId: String) {
        check(phase == Phase.AwaitingConsent(requestId)) {
            "Capture request is no longer pending"
        }
        phase = Phase.AwaitingGrant(requestId)
    }

    @Synchronized
    fun reject(requestId: String): Boolean {
        if (phase != Phase.AwaitingConsent(requestId)) return false
        phase = Phase.Idle
        return true
    }

    @Synchronized
    fun grant(requestId: String, binding: CaptureBinding) {
        check(phase == Phase.AwaitingGrant(requestId)) {
            "Capture was not explicitly approved on this phone"
        }
        phase = Phase.Streaming(binding)
    }

    /** Invalidates authorization before callers release resources or notify the peer. */
    @Synchronized
    fun stop(): Phase {
        val previous = phase
        phase = Phase.Idle
        return previous
    }
}
