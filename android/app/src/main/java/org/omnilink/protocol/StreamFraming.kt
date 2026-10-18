package org.omnilink.protocol

import java.io.InputStream
import java.io.OutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder

object StreamFraming {
    const val MAX_FRAME_SIZE = 65536 // 64 KB, matches MAX_CONTROL_PACKET_LEN in daemon

    /**
     * Encodes a payload byte array into a 4-byte big-endian length-prefixed frame.
     */
    fun encodeFrame(payload: ByteArray): ByteArray {
        require(payload.size <= MAX_FRAME_SIZE) {
            "Payload size (${payload.size}) exceeds MAX_FRAME_SIZE ($MAX_FRAME_SIZE)"
        }
        val buffer = ByteBuffer.allocate(4 + payload.size)
        buffer.order(ByteOrder.BIG_ENDIAN)
        buffer.putInt(payload.size)
        buffer.put(payload)
        return buffer.array()
    }

    /**
     * Decodes a single frame from raw bytes if a full frame is available.
     * Returns Pair(payload, remainingBytes) or null if data is incomplete.
     */
    fun decodeFrame(data: ByteArray): Pair<ByteArray, ByteArray>? {
        if (data.size < 4) return null
        val length = ByteBuffer.wrap(data, 0, 4).order(ByteOrder.BIG_ENDIAN).int
        require(length in 0..MAX_FRAME_SIZE) {
            "Invalid frame length: $length (max: $MAX_FRAME_SIZE)"
        }
        val totalSize = 4 + length
        if (data.size < totalSize) return null

        val payload = data.copyOfRange(4, totalSize)
        val remaining = data.copyOfRange(totalSize, data.size)
        return Pair(payload, remaining)
    }

    /**
     * Writes a 4-byte BE length-prefixed frame to an OutputStream.
     */
    fun writeFrame(outputStream: OutputStream, payload: ByteArray) {
        val frame = encodeFrame(payload)
        outputStream.write(frame)
        outputStream.flush()
    }

    /**
     * Reads a single 4-byte BE length-prefixed frame from an InputStream.
     * Returns null on clean EOF when no bytes have been read.
     */
    fun readFrame(inputStream: InputStream): ByteArray? {
        val lenBuf = ByteArray(4)
        var bytesRead = 0
        while (bytesRead < 4) {
            val count = inputStream.read(lenBuf, bytesRead, 4 - bytesRead)
            if (count == -1) {
                if (bytesRead == 0) return null
                throw IllegalStateException("Unexpected EOF while reading frame length prefix")
            }
            bytesRead += count
        }

        val length = ByteBuffer.wrap(lenBuf).order(ByteOrder.BIG_ENDIAN).int
        if (length < 0 || length > MAX_FRAME_SIZE) {
            throw IllegalArgumentException("Invalid frame length: $length (max: $MAX_FRAME_SIZE)")
        }

        val payload = ByteArray(length)
        var payloadRead = 0
        while (payloadRead < length) {
            val count = inputStream.read(payload, payloadRead, length - payloadRead)
            if (count == -1) {
                throw IllegalStateException("Unexpected EOF while reading frame payload of size $length")
            }
            payloadRead += count
        }
        return payload
    }
}
