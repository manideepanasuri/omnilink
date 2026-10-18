package org.omnilink

import android.content.ClipboardManager
import android.content.ContextWrapper
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.omnilink.clipboard.ClipboardDriver
import org.omnilink.clipboard.OmnilinkClipboardManager
import org.omnilink.clipboard.PrimaryClipData
import org.omnilink.protocol.ClipboardPacket
import org.omnilink.transport.Stream1Session
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder

@OptIn(ExperimentalCoroutinesApi::class)
class Stream1SessionTest {

    private class StubClipboardDriver : ClipboardDriver {
        var currentClip: PrimaryClipData? = null
        var lastInjectedMimeType: String? = null
        var lastInjectedPayload: ByteArray? = null
        var lastInjectedIsSensitive: Boolean? = null
        var registeredListener: ClipboardManager.OnPrimaryClipChangedListener? = null

        override fun registerClipChangedListener(listener: ClipboardManager.OnPrimaryClipChangedListener) {
            registeredListener = listener
        }

        override fun unregisterClipChangedListener(listener: ClipboardManager.OnPrimaryClipChangedListener) {
            if (registeredListener === listener) {
                registeredListener = null
            }
        }

        override fun getPrimaryClip(): PrimaryClipData? = currentClip

        override fun setPrimaryClip(mimeType: String, payload: ByteArray, isSensitive: Boolean) {
            lastInjectedMimeType = mimeType
            lastInjectedPayload = payload
            lastInjectedIsSensitive = isSensitive
            currentClip = PrimaryClipData(
                mimeType = mimeType,
                payload = payload,
                isSensitive = isSensitive
            )
            // Real Android triggers onPrimaryClipChanged on clip injection
            registeredListener?.onPrimaryClipChanged()
        }

        fun simulateLocalCopy(mimeType: String, payload: ByteArray, isSensitive: Boolean = false) {
            currentClip = PrimaryClipData(
                mimeType = mimeType,
                payload = payload,
                isSensitive = isSensitive
            )
            registeredListener?.onPrimaryClipChanged()
        }
    }

    private lateinit var stubDriver: StubClipboardDriver
    private lateinit var clipboardManager: OmnilinkClipboardManager

    @Before
    fun setUp() {
        stubDriver = StubClipboardDriver()
        clipboardManager = OmnilinkClipboardManager(
            context = ContextWrapper(null),
            driver = stubDriver
        )
    }

    private fun createLengthPrefixedFrame(packet: ClipboardPacket): ByteArray {
        val encoded = packet.encodeBinary()
        val buffer = ByteBuffer.allocate(4 + encoded.size).order(ByteOrder.BIG_ENDIAN)
        buffer.putInt(encoded.size)
        buffer.put(encoded)
        return buffer.array()
    }

    @Test
    fun testInboundPumpReceivesAndInjectsClip() = runTest {
        clipboardManager.startListening()

        val textPayload = "Hello from Linux host via QUIC Stream 1".toByteArray(Charsets.UTF_8)
        val incomingPacket = ClipboardPacket(
            sequenceId = 100L,
            mimeType = "text/plain",
            isSensitive = false,
            payload = textPayload
        )
        val frameBytes = createLengthPrefixedFrame(incomingPacket)

        val clientInput = ByteArrayInputStream(frameBytes)
        val clientOutput = ByteArrayOutputStream()

        var receivedPacket: ClipboardPacket? = null
        val session = Stream1Session(
            inputStream = clientInput,
            outputStream = clientOutput,
            clipboardManager = clipboardManager,
            scope = this,
            onPacketReceived = { receivedPacket = it }
        )
        session.start()

        testScheduler.advanceUntilIdle()

        assertNotNull(receivedPacket)
        assertEquals(100L, receivedPacket!!.sequenceId)
        assertEquals("text/plain", receivedPacket!!.mimeType)
        assertArrayEquals(textPayload, receivedPacket!!.payload)

        // Verify driver had clip injected
        assertEquals("text/plain", stubDriver.lastInjectedMimeType)
        assertArrayEquals(textPayload, stubDriver.lastInjectedPayload)
        assertEquals(false, stubDriver.lastInjectedIsSensitive)

        session.close()
    }

    @Test
    fun testLoopPreventionInjectedClipDoesNotEmitOutboundFrame() = runTest {
        clipboardManager.startListening()

        val textPayload = "Echo prevention test payload".toByteArray(Charsets.UTF_8)
        val incomingPacket = ClipboardPacket(
            sequenceId = 101L,
            mimeType = "text/plain",
            isSensitive = false,
            payload = textPayload
        )
        val frameBytes = createLengthPrefixedFrame(incomingPacket)

        val clientInput = ByteArrayInputStream(frameBytes)
        val clientOutput = ByteArrayOutputStream()

        val session = Stream1Session(
            inputStream = clientInput,
            outputStream = clientOutput,
            clipboardManager = clipboardManager,
            scope = this
        )
        session.start()

        testScheduler.advanceUntilIdle()

        // Clip was injected into local clipboard
        assertArrayEquals(textPayload, stubDriver.lastInjectedPayload)

        // Loop prevention: Injected clip must NOT cause an outbound packet to be emitted!
        assertEquals(0, clientOutput.size())

        session.close()
    }

    @Test
    fun testOutboundPumpSendsLocalClipboardCopy() = runTest {
        clipboardManager.startListening()

        val clientInput = ByteArrayInputStream(ByteArray(0))
        val clientOutput = ByteArrayOutputStream()

        var sentPacket: ClipboardPacket? = null
        val session = Stream1Session(
            inputStream = clientInput,
            outputStream = clientOutput,
            clipboardManager = clipboardManager,
            scope = this,
            onPacketSent = { sentPacket = it }
        )
        session.start()
        testScheduler.runCurrent()

        val localCopyText = "Locally copied on Android phone"
        val localBytes = localCopyText.toByteArray(Charsets.UTF_8)
        stubDriver.simulateLocalCopy(
            mimeType = "text/plain",
            payload = localBytes,
            isSensitive = false
        )

        testScheduler.advanceUntilIdle()

        assertNotNull(sentPacket)
        assertEquals(1L, sentPacket!!.sequenceId)
        assertEquals("text/plain", sentPacket!!.mimeType)
        assertFalse(sentPacket!!.isSensitive)
        assertArrayEquals(localBytes, sentPacket!!.payload)

        val writtenBytes = clientOutput.toByteArray()
        assertTrue(writtenBytes.size > 4)

        // Read 4-byte BE length prefix
        val length = ByteBuffer.wrap(writtenBytes, 0, 4).order(ByteOrder.BIG_ENDIAN).int
        assertEquals(writtenBytes.size - 4, length)

        // Decode packet from frame payload
        val payloadBytes = writtenBytes.copyOfRange(4, writtenBytes.size)
        val decoded = ClipboardPacket.decodeBinary(payloadBytes)
        assertEquals(sentPacket, decoded)

        session.close()
    }

    @Test
    fun testImagePayloadInboundAndOutbound() = runTest {
        clipboardManager.startListening()

        val pngBytes = byteArrayOf(
            0x89.toByte(), 0x50.toByte(), 0x4E.toByte(), 0x47.toByte(),
            0x0D.toByte(), 0x0A.toByte(), 0x1A.toByte(), 0x0A.toByte(),
            0x01, 0x02, 0x03, 0x04
        )

        // 1. Inbound image packet
        val imagePacket = ClipboardPacket(
            sequenceId = 200L,
            mimeType = "image/png",
            isSensitive = false,
            payload = pngBytes
        )
        val frameBytes = createLengthPrefixedFrame(imagePacket)

        val clientInput = ByteArrayInputStream(frameBytes)
        val clientOutput = ByteArrayOutputStream()

        var receivedPacket: ClipboardPacket? = null
        val session = Stream1Session(
            inputStream = clientInput,
            outputStream = clientOutput,
            clipboardManager = clipboardManager,
            scope = this,
            onPacketReceived = { receivedPacket = it }
        )
        session.start()

        testScheduler.advanceUntilIdle()

        assertNotNull(receivedPacket)
        assertEquals("image/png", receivedPacket!!.mimeType)
        assertArrayEquals(pngBytes, receivedPacket!!.payload)
        assertEquals("image/png", stubDriver.lastInjectedMimeType)
        assertArrayEquals(pngBytes, stubDriver.lastInjectedPayload)

        // 2. Outbound manual send
        val outboundImage = ClipboardPacket(
            sequenceId = session.nextSequenceId(),
            mimeType = "image/png",
            isSensitive = false,
            payload = pngBytes
        )
        session.sendPacket(outboundImage)
        testScheduler.advanceUntilIdle()

        val written = clientOutput.toByteArray()
        val len = ByteBuffer.wrap(written, 0, 4).order(ByteOrder.BIG_ENDIAN).int
        val decodedOut = ClipboardPacket.decodeBinary(written.copyOfRange(4, 4 + len))
        assertEquals(outboundImage, decodedOut)

        session.close()
    }

    @Test
    fun testMultipleSequentialFramesInbound() = runTest {
        clipboardManager.startListening()

        val p1 = ClipboardPacket(1L, "text/plain", false, "Packet 1".toByteArray(Charsets.UTF_8))
        val p2 = ClipboardPacket(2L, "text/plain", false, "Packet 2".toByteArray(Charsets.UTF_8))
        val p3 = ClipboardPacket(3L, "text/plain", false, "Packet 3".toByteArray(Charsets.UTF_8))

        val combinedFrames = createLengthPrefixedFrame(p1) +
                createLengthPrefixedFrame(p2) +
                createLengthPrefixedFrame(p3)

        val clientInput = ByteArrayInputStream(combinedFrames)
        val clientOutput = ByteArrayOutputStream()

        val receivedList = mutableListOf<ClipboardPacket>()
        val session = Stream1Session(
            inputStream = clientInput,
            outputStream = clientOutput,
            clipboardManager = clipboardManager,
            scope = this,
            onPacketReceived = { receivedList.add(it) }
        )
        session.start()

        testScheduler.advanceUntilIdle()

        assertEquals(3, receivedList.size)
        assertEquals(p1, receivedList[0])
        assertEquals(p2, receivedList[1])
        assertEquals(p3, receivedList[2])

        session.close()
    }

    @Test
    fun testLifecycleCloseClosesSessionAndStreams() = runTest {
        var disconnectedCause: Throwable? = null
        var disconnectCalled = false

        val clientInput = ByteArrayInputStream(ByteArray(0))
        val clientOutput = ByteArrayOutputStream()

        val session = Stream1Session(
            inputStream = clientInput,
            outputStream = clientOutput,
            clipboardManager = clipboardManager,
            scope = this,
            onDisconnect = { cause ->
                disconnectCalled = true
                disconnectedCause = cause
            }
        )
        assertFalse(session.isClosed)

        session.start()
        testScheduler.advanceUntilIdle()

        session.close()
        assertTrue(disconnectCalled)
        assertTrue(session.isClosed)

        // Further writes are no-ops
        session.sendPacket(ClipboardPacket(1L, "text/plain", false, "test".toByteArray(Charsets.UTF_8)))
        assertEquals(0, clientOutput.size())
    }
}
