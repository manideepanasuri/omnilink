package org.omnilink

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Test
import org.omnilink.protocol.StreamFraming
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream

class StreamFramingTest {

    @Test
    fun testEncodeFrameBigEndian() {
        val payload = "Hello Omnilink".toByteArray(Charsets.UTF_8)
        val frame = StreamFraming.encodeFrame(payload)

        // 4 bytes prefix + payload
        assertEquals(4 + payload.size, frame.size)
        // Length 14 in big-endian bytes: 0, 0, 0, 14
        assertEquals(0.toByte(), frame[0])
        assertEquals(0.toByte(), frame[1])
        assertEquals(0.toByte(), frame[2])
        assertEquals(14.toByte(), frame[3])

        val payloadExtracted = frame.copyOfRange(4, frame.size)
        assertArrayEquals(payload, payloadExtracted)
    }

    @Test
    fun testEncodeZeroLengthPayload() {
        val empty = ByteArray(0)
        val frame = StreamFraming.encodeFrame(empty)
        assertEquals(4, frame.size)
        assertArrayEquals(byteArrayOf(0, 0, 0, 0), frame)
    }

    @Test
    fun testDecodeFullFrame() {
        val payload = "Omnilink Stream 0 Frame".toByteArray(Charsets.UTF_8)
        val frame = StreamFraming.encodeFrame(payload)

        val result = StreamFraming.decodeFrame(frame)
        assertNotNull(result)
        val (decodedPayload, remaining) = result!!
        assertArrayEquals(payload, decodedPayload)
        assertEquals(0, remaining.size)
    }

    @Test
    fun testDecodeIncompleteHeaderReturnsNull() {
        val partialHeader = byteArrayOf(0, 0, 0) // only 3 bytes
        val result = StreamFraming.decodeFrame(partialHeader)
        assertNull(result)
    }

    @Test
    fun testDecodeIncompletePayloadReturnsNull() {
        // Length says 10 bytes, but only 5 are appended
        val incomplete = byteArrayOf(0, 0, 0, 10, 1, 2, 3, 4, 5)
        val result = StreamFraming.decodeFrame(incomplete)
        assertNull(result)
    }

    @Test
    fun testDecodeMultipleConcatenatedFrames() {
        val p1 = "First".toByteArray(Charsets.UTF_8)
        val p2 = "Second".toByteArray(Charsets.UTF_8)

        val f1 = StreamFraming.encodeFrame(p1)
        val f2 = StreamFraming.encodeFrame(p2)
        val concatenated = f1 + f2

        val result1 = StreamFraming.decodeFrame(concatenated)
        assertNotNull(result1)
        val (decodedP1, remaining1) = result1!!
        assertArrayEquals(p1, decodedP1)
        assertEquals(f2.size, remaining1.size)

        val result2 = StreamFraming.decodeFrame(remaining1)
        assertNotNull(result2)
        val (decodedP2, remaining2) = result2!!
        assertArrayEquals(p2, decodedP2)
        assertEquals(0, remaining2.size)
    }

    @Test
    fun testExceedingMaxFrameSizeThrows() {
        val oversize = ByteArray(StreamFraming.MAX_FRAME_SIZE + 1)
        assertThrows(IllegalArgumentException::class.java) {
            StreamFraming.encodeFrame(oversize)
        }
    }

    @Test
    fun testStreamWriteAndReadFrame() {
        val payload = "Test Stream IO Framing".toByteArray(Charsets.UTF_8)
        val outStream = ByteArrayOutputStream()

        StreamFraming.writeFrame(outStream, payload)
        val writtenBytes = outStream.toByteArray()

        val inStream = ByteArrayInputStream(writtenBytes)
        val readPayload = StreamFraming.readFrame(inStream)
        assertNotNull(readPayload)
        assertArrayEquals(payload, readPayload)

        // Reading again on empty stream should return null (clean EOF)
        val eofResult = StreamFraming.readFrame(inStream)
        assertNull(eofResult)
    }
}
