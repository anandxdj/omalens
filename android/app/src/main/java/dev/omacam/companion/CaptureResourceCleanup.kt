package dev.omacam.companion

/** Runs every owned-resource cleanup step even when an earlier platform call fails. */
internal object CaptureResourceCleanup {
    fun release(vararg steps: () -> Unit): List<Exception> = buildList {
        steps.forEach { step ->
            try {
                step()
            } catch (failure: Exception) {
                if (failure is InterruptedException) Thread.currentThread().interrupt()
                add(failure)
            }
        }
    }
}
