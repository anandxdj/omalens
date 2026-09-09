package dev.omacam.companion

/** Rejects callbacks from a capture generation after it has been invalidated. */
internal class CaptureCallbackGate {
    private var nextGeneration = 1L
    private var activeGeneration: Long? = null

    @Synchronized
    fun start(): Long {
        check(activeGeneration == null) { "A capture generation is already active" }
        return nextGeneration++.also { activeGeneration = it }
    }

    @Synchronized
    fun accepts(generation: Long): Boolean = activeGeneration == generation

    @Synchronized
    fun requireCurrent(generation: Long) {
        check(activeGeneration == generation) { "Capture startup was cancelled" }
    }

    /** Runs callback publication under the same lock used for invalidation. */
    @Synchronized
    fun publish(generation: Long, action: () -> Unit): Boolean {
        if (activeGeneration != generation) return false
        action()
        return true
    }

    /** Acquires outside the lock, then either publishes or immediately releases the result. */
    fun <T> acquire(
        generation: Long,
        create: () -> T,
        publish: (T) -> Unit,
        releaseLate: (T) -> Unit,
    ): T {
        requireCurrent(generation)
        val resource = create()
        if (!publish(generation) { publish(resource) }) releaseLate(resource)
        requireCurrent(generation)
        return resource
    }

    /** Invalidates first; callers can safely perform fallible cleanup afterwards. */
    @Synchronized
    fun stop(generation: Long): Boolean {
        if (activeGeneration != generation) return false
        activeGeneration = null
        return true
    }
}
