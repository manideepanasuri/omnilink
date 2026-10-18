package org.omnilink

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import org.omnilink.protocol.ClipboardPacket
import java.nio.ByteBuffer
import java.nio.ByteOrder

class ClipboardPacketTest {

    @Test
    fun testTextPayloadBinaryRoundtrip() {
        val original = ClipboardPacket(
            sequenceId = 42L,
            mimeType = "text/plain;charset=utf-8",
            isSensitive = false,
            payload = "Hello, Wayland!".toByteArray(Charsets.UTF_8)
        )

        val encoded = original.encodeBinary()
        val decoded = ClipboardPacket.decodeBinary(encoded)

        assertEquals(original, decoded)
        assertEquals(42L, decoded.sequenceId)
        assertEquals("text/plain;charset=utf-8", decoded.mimeType)
        assertFalse(decoded.isSensitive)
        assertArrayEquals(original.payload, decoded.payload)
    }

    @Test
    fun testSensitivePayloadBinaryRoundtrip() {
        val original = ClipboardPacket(
            sequenceId = 1001L,
            mimeType = "text/plain",
            isSensitive = true,
            payload = "super_secret_master_password".toByteArray(Charsets.UTF_8)
        )

        val encoded = original.encodeBinary()
        val decoded = ClipboardPacket.decodeBinary(encoded)

        assertEquals(original, decoded)
        assertTrue(decoded.isSensitive)
        assertArrayEquals("super_secret_master_password".toByteArray(Charsets.UTF_8), decoded.payload)
    }

    @Test
    fun testEmptyPayloadBinaryRoundtrip() {
        val original = ClipboardPacket(
            sequenceId = 1002L,
            mimeType = "text/plain",
            isSensitive = false,
            payload = ByteArray(0)
        )

        val encoded = original.encodeBinary()
        val decoded = ClipboardPacket.decodeBinary(encoded)

        assertEquals(original, decoded)
        assertEquals(0, decoded.payload.size)
    }

    @Test
    fun testImagePayloadBinaryRoundtrip() {
        val pngBytes = byteArrayOf(
            0x89.toByte(), 0x50.toByte(), 0x4E.toByte(), 0x47.toByte(),
            0x0D.toByte(), 0x0A.toByte(), 0x1A.toByte(), 0x0A.toByte(),
            0x00.toByte(), 0xFF.toByte(), 0xFE.toByte()
        )
        val original = ClipboardPacket(
            sequenceId = 1003L,
            mimeType = "image/png",
            isSensitive = false,
            payload = pngBytes
        )

        val encoded = original.encodeBinary()
        val decoded = ClipboardPacket.decodeBinary(encoded)

        assertEquals(original, decoded)
        assertEquals("image/png", decoded.mimeType)
        assertArrayEquals(pngBytes, decoded.payload)
    }

    @Test
    fun testLargeImagePayloadRoundtrip() {
        val largeBytes = ByteArray(256 * 1024) { (it % 256).toByte() }
        val original = ClipboardPacket(
            sequenceId = 5555L,
            mimeType = "image/png",
            isSensitive = false,
            payload = largeBytes
        )

        val encoded = original.encodeBinary()
        val decoded = ClipboardPacket.decodeBinary(encoded)

        assertEquals(original, decoded)
        assertArrayEquals(largeBytes, decoded.payload)
    }

    @Test
    fun testBinaryWireFormatByteForByte() {
        val seq = 0x0102030405060708L
        val mime = "text/plain"
        val mimeBytes = mime.toByteArray(Charsets.UTF_8)
        val payload = "payload data".toByteArray(Charsets.UTF_8)

        val packet = ClipboardPacket(
            sequenceId = seq,
            mimeType = mime,
            isSensitive = true,
            payload = payload
        )

        val encoded = packet.encodeBinary()
        val expectedSize = ClipboardPacket.FIXED_HEADER_LEN + mimeBytes.size + payload.size
        assertEquals(expectedSize, encoded.size)

        // sequence_id: 8 bytes big-endian
        val seqBuf = ByteBuffer.wrap(encoded, 0, 8).order(ByteOrder.BIG_ENDIAN)
        assertEquals(seq, seqBuf.long)

        // is_sensitive: 1 byte (1)
        assertEquals(1.toByte(), encoded[8])

        // mime_type_len: 2 bytes big-endian
        val mimeLen = ByteBuffer.wrap(encoded, 9, 2).order(ByteOrder.BIG_ENDIAN).short.toInt() and 0xFFFF
        assertEquals(mimeBytes.size, mimeLen)

        // mime_type UTF-8
        val extractedMime = String(encoded, 11, mimeLen, Charsets.UTF_8)
        assertEquals(mime, extractedMime)

        // payload
        val extractedPayload = encoded.copyOfRange(11 + mimeLen, encoded.size)
        assertArrayEquals(payload, extractedPayload)
    }

    @Test
    fun testDecodeTooShortHeaderThrows() {
        val tooShort = ByteArray(10) // less than 11 bytes
        assertThrows(IllegalArgumentException::class.java) {
            ClipboardPacket.decodeBinary(tooShort)
        }
    }

    @Test
    fun testDecodeTruncatedMimeThrows() {
        val buffer = ByteBuffer.allocate(11 + 5).order(ByteOrder.BIG_ENDIAN)
        buffer.putLong(1L)
        buffer.put(0.toByte())
        buffer.putShort(10.toShort()) // Claims MIME length is 10
        buffer.put("abcde".toByteArray(Charsets.UTF_8)) // Only 5 bytes provided
        val truncated = buffer.array()

        assertThrows(IllegalArgumentException::class.java) {
            ClipboardPacket.decodeBinary(truncated)
        }
    }

    @Test
    fun testEqualsAndHashCode() {
        val p1 = ClipboardPacket(10L, "text/plain", false, byteArrayOf(1, 2, 3))
        val p2 = ClipboardPacket(10L, "text/plain", false, byteArrayOf(1, 2, 3))
        val p3 = ClipboardPacket(11L, "text/plain", false, byteArrayOf(1, 2, 3))

        assertEquals(p1, p2)
        assertEquals(p1.hashCode(), p2.hashCode())
        assertNotEquals(p1, p3)
    }
}
