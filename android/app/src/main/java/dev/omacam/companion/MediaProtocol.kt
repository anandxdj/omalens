package dev.omacam.companion

import java.io.DataOutputStream
import java.nio.charset.StandardCharsets

internal object MediaProtocol {
    const val MAX_ACCESS_UNIT_BYTES = 1_048_576
    const val HEADER_BYTES = 104
    private val magic = "OMACAMM1".toByteArray(StandardCharsets.US_ASCII)

    fun writeAccessUnit(
        output: DataOutputStream,
        binding: CaptureBinding,
        sequence: Long,
        timestampUs: Long,
        keyFrame: Boolean,
        payload: ByteArray,
    ) {
        require(sequence >= 0) { "Media sequence is invalid" }
        require(timestampUs >= 0) { "Media timestamp is invalid" }
        require(payload.isNotEmpty()) { "Encoded access unit is empty" }
        require(payload.size <= MAX_ACCESS_UNIT_BYTES) { "Encoded access unit exceeds 1 MiB" }
        writeHeader(output, binding, 1, if (keyFrame) 1 else 0, sequence, timestampUs, payload.size)
        output.write(payload)
        output.flush()
    }

    fun writeStop(output: DataOutputStream, binding: CaptureBinding, sequence: Long) {
        require(sequence >= 0) { "Media sequence is invalid" }
        writeHeader(output, binding, 2, 0, sequence, 0, 0)
        output.flush()
    }

    private fun writeHeader(
        output: DataOutputStream,
        binding: CaptureBinding,
        kind: Int,
        flags: Int,
        sequence: Long,
        timestampUs: Long,
        payloadSize: Int,
    ) {
        val peer = binding.peer.hexBytes(32)
        val connection = binding.connection.hexBytes(16)
        val session = binding.session.hexBytes(16)
        require(binding.generation > 0) { "Capture generation is invalid" }
        output.write(magic)
        output.writeByte(1)
        output.writeByte(kind)
        output.writeShort(flags)
        output.write(peer)
        output.write(connection)
        output.write(session)
        output.writeLong(binding.generation)
        output.writeLong(sequence)
        output.writeLong(timestampUs)
        output.writeInt(payloadSize)
    }
}

private fun String.hexBytes(expected: Int): ByteArray {
    require(length == expected * 2 && all { it.isDigit() || it.lowercaseChar() in 'a'..'f' }) {
        "Capture binding is malformed"
    }
    return ByteArray(expected) { index ->
        substring(index * 2, index * 2 + 2).toInt(16).toByte()
    }
}
