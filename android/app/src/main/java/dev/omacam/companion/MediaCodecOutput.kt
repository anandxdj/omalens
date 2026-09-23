package dev.omacam.companion

import java.nio.ByteBuffer

/**
 * Copies and combines MediaCodec output only after applying the wire-size bound.
 *
 * BufferInfo is supplied by the Android framework, but its offset and size still
 * must be checked before allocating an array. Keeping this boundary pure makes
 * malformed/fault-injected codec metadata testable without a device codec.
 */
internal object MediaCodecOutput {
    fun copy(buffer: ByteBuffer, offset: Int, size: Int): ByteArray {
        require(offset >= 0 && size >= 0) { "Encoder output bounds are invalid" }
        require(size <= MediaProtocol.MAX_ACCESS_UNIT_BYTES) {
            "Encoded access unit exceeds 1 MiB"
        }
        val end = offset.toLong() + size.toLong()
        require(end <= buffer.capacity().toLong()) { "Encoder output bounds are invalid" }
        val duplicate = buffer.duplicate().apply {
            clear()
            position(offset)
            limit(end.toInt())
        }
        return ByteArray(size).also(duplicate::get)
    }

    fun prependCodecConfig(configuration: ByteArray, accessUnit: ByteArray): ByteArray {
        require(configuration.size <= MediaProtocol.MAX_ACCESS_UNIT_BYTES) {
            "Codec configuration exceeds 1 MiB"
        }
        require(accessUnit.size <= MediaProtocol.MAX_ACCESS_UNIT_BYTES) {
            "Encoded access unit exceeds 1 MiB"
        }
        val total = configuration.size.toLong() + accessUnit.size.toLong()
        require(total <= MediaProtocol.MAX_ACCESS_UNIT_BYTES.toLong()) {
            "Encoded access unit exceeds 1 MiB"
        }
        return ByteArray(total.toInt()).also {
            configuration.copyInto(it)
            accessUnit.copyInto(it, destinationOffset = configuration.size)
        }
    }
}
