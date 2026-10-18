package org.omnilink.protocol

import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * Bidirectional clipboard synchronization packet framed over Stream 1.
 * Matches `omnilink_protocol::packet::ClipboardPacket` byte-for-byte.
 *
 * Wire format:
 * `[sequence_id: 8 bytes BE, is_sensitive: 1 byte, mime_type_len: 2 bytes BE, mime_type: N bytes UTF-8, payload: M bytes]`
 */
data class ClipboardPacket(
    val sequenceId: Long,
    val mimeType: String,
    val isSensitive: Boolean,
    val payload: ByteArray
) {
    /**
     * Encodes the packet into the Stream 1 binary wire format.
     */
    fun encodeBinary(): ByteArray {
        val mimeBytes = mimeType.toByteArray(Charsets.UTF_8)
        require(mimeBytes.size <= 0xFFFF) {
            "MIME type exceeds maximum length of 65535 bytes: ${mimeBytes.size}"
        }
        val buffer = ByteBuffer.allocate(FIXED_HEADER_LEN + mimeBytes.size + payload.size)
        buffer.order(ByteOrder.BIG_ENDIAN)
        buffer.putLong(sequenceId)
        buffer.put(if (isSensitive) 1.toByte() else 0.toByte())
        buffer.putShort(mimeBytes.size.toShort())
        buffer.put(mimeBytes)
        buffer.put(payload)
        return buffer.array()
    }

    override fun equals(other: Any?): Boolean {
        if (this === other) return true
        if (other !is ClipboardPacket) return false
        return sequenceId == other.sequenceId &&
                mimeType == other.mimeType &&
                isSensitive == other.isSensitive &&
                payload.contentEquals(other.payload)
    }

    override fun hashCode(): Int {
        var result = sequenceId.hashCode()
        result = 31 * result + mimeType.hashCode()
        result = 31 * result + isSensitive.hashCode()
        result = 31 * result + payload.contentHashCode()
        return result
    }

    override fun toString(): String {
        return "ClipboardPacket(sequenceId=$sequenceId, mimeType='$mimeType', isSensitive=$isSensitive, payloadBytes=${payload.size})"
    }

    companion object {
        /**
         * Minimum binary header size: 8 bytes (seq) + 1 byte (sensitive) + 2 bytes (mime_len) = 11 bytes.
         */
        const val FIXED_HEADER_LEN: Int = 11

        /**
         * Decodes a packet from the Stream 1 binary wire format.
         */
        fun decodeBinary(bytes: ByteArray): ClipboardPacket {
            require(bytes.size >= FIXED_HEADER_LEN) {
                "Insufficient bytes for clipboard header: expected at least $FIXED_HEADER_LEN, got ${bytes.size}"
            }
            val buffer = ByteBuffer.wrap(bytes).order(ByteOrder.BIG_ENDIAN)
            val sequenceId = buffer.long
            val isSensitive = buffer.get().toInt() != 0
            val mimeTypeLen = buffer.short.toInt() and 0xFFFF

            val mimeEnd = FIXED_HEADER_LEN + mimeTypeLen
            require(bytes.size >= mimeEnd) {
                "Insufficient bytes for MIME type: expected at least $mimeEnd, got ${bytes.size}"
            }
            val mimeBytes = ByteArray(mimeTypeLen)
            buffer.get(mimeBytes)
            val mimeType = String(mimeBytes, Charsets.UTF_8)

            val payload = ByteArray(bytes.size - mimeEnd)
            buffer.get(payload)

            return ClipboardPacket(
                sequenceId = sequenceId,
                mimeType = mimeType,
                isSensitive = isSensitive,
                payload = payload
            )
        }
    }
}
