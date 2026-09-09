package dev.omacam.companion

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test
import java.io.ByteArrayOutputStream
import java.io.DataOutputStream
import java.nio.ByteBuffer
import java.nio.charset.StandardCharsets

class MediaProtocolTest {
    private val binding = CaptureBinding(
        peer = "11".repeat(32),
        connection = "22".repeat(16),
        session = "33".repeat(16),
        generation = 7,
    )

    @Test
    fun `access unit matches the desktop 104 byte framing contract`() {
        val bytes = output { stream ->
            MediaProtocol.writeAccessUnit(stream, binding, 3, 42_000, true, byteArrayOf(1, 2, 3))
        }
        val header = ByteBuffer.wrap(bytes)

        assertEquals(MediaProtocol.HEADER_BYTES + 3, bytes.size)
        assertArrayEquals("OMACAMM1".toByteArray(StandardCharsets.US_ASCII), bytes.copyOfRange(0, 8))
        assertEquals(1, bytes[8].toInt())
        assertEquals(1, bytes[9].toInt())
        assertEquals(1, header.getShort(10).toInt())
        assertArrayEquals(ByteArray(32) { 0x11 }, bytes.copyOfRange(12, 44))
        assertArrayEquals(ByteArray(16) { 0x22 }, bytes.copyOfRange(44, 60))
        assertArrayEquals(ByteArray(16) { 0x33 }, bytes.copyOfRange(60, 76))
        assertEquals(7, header.getLong(76))
        assertEquals(3, header.getLong(84))
        assertEquals(42_000, header.getLong(92))
        assertEquals(3, header.getInt(100))
        assertArrayEquals(byteArrayOf(1, 2, 3), bytes.copyOfRange(104, 107))
    }

    @Test
    fun `terminal stop has no flags timestamp or payload`() {
        val bytes = output { stream -> MediaProtocol.writeStop(stream, binding, 4) }
        val header = ByteBuffer.wrap(bytes)

        assertEquals(MediaProtocol.HEADER_BYTES, bytes.size)
        assertEquals(2, bytes[9].toInt())
        assertEquals(0, header.getShort(10).toInt())
        assertEquals(4, header.getLong(84))
        assertEquals(0, header.getLong(92))
        assertEquals(0, header.getInt(100))
    }

    @Test
    fun `invalid encoder output shapes fail before bytes are written`() {
        for (action in listOf<(DataOutputStream) -> Unit>(
            { MediaProtocol.writeAccessUnit(it, binding, 0, 0, false, ByteArray(0)) },
            {
                MediaProtocol.writeAccessUnit(
                    it,
                    binding,
                    0,
                    0,
                    false,
                    ByteArray(MediaProtocol.MAX_ACCESS_UNIT_BYTES + 1),
                )
            },
            { MediaProtocol.writeAccessUnit(it, binding, -1, 0, false, byteArrayOf(1)) },
            { MediaProtocol.writeAccessUnit(it, binding, 0, -1, false, byteArrayOf(1)) },
            { MediaProtocol.writeStop(it, binding.copy(peer = "zz".repeat(32)), 0) },
            { MediaProtocol.writeStop(it, binding.copy(generation = 0), 0) },
        )) {
            val target = ByteArrayOutputStream()
            assertThrows(IllegalArgumentException::class.java) {
                action(DataOutputStream(target))
            }
            assertEquals(0, target.size())
        }
    }

    private fun output(write: (DataOutputStream) -> Unit): ByteArray =
        ByteArrayOutputStream().also { write(DataOutputStream(it)) }.toByteArray()
}
