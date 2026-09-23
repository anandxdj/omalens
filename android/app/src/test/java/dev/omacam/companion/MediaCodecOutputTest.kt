package dev.omacam.companion

import java.nio.ByteBuffer
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertThrows
import org.junit.Test

class MediaCodecOutputTest {
    @Test
    fun `copy validates framework output bounds before allocation`() {
        val buffer = ByteBuffer.wrap(ByteArray(8) { it.toByte() })

        assertArrayEquals(byteArrayOf(2, 3, 4), MediaCodecOutput.copy(buffer, 2, 3))
        assertThrows(IllegalArgumentException::class.java) {
            MediaCodecOutput.copy(buffer, -1, 1)
        }
        assertThrows(IllegalArgumentException::class.java) {
            MediaCodecOutput.copy(buffer, 7, 2)
        }
        assertThrows(IllegalArgumentException::class.java) {
            MediaCodecOutput.copy(buffer, 0, MediaProtocol.MAX_ACCESS_UNIT_BYTES + 1)
        }
    }

    @Test
    fun `codec configuration is prepended without exceeding wire bound`() {
        assertArrayEquals(
            byteArrayOf(1, 2, 3, 4),
            MediaCodecOutput.prependCodecConfig(byteArrayOf(1, 2), byteArrayOf(3, 4)),
        )
        assertThrows(IllegalArgumentException::class.java) {
            MediaCodecOutput.prependCodecConfig(
                ByteArray(MediaProtocol.MAX_ACCESS_UNIT_BYTES),
                byteArrayOf(1),
            )
        }
    }
}
