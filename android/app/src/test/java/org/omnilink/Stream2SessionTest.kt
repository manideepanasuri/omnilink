package org.omnilink

import android.content.Context
import android.content.SharedPreferences
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.mockito.ArgumentMatchers.anyInt
import org.mockito.ArgumentMatchers.anyString
import org.mockito.Mockito.mock
import org.mockito.Mockito.`when`
import org.omnilink.notifications.OmnilinkNotificationService
import org.omnilink.protocol.NotificationAction
import org.omnilink.protocol.NotificationDismissPacket
import org.omnilink.protocol.NotificationPacket
import org.omnilink.protocol.NotificationReplyPacket
import org.omnilink.protocol.NotificationStreamPacket
import org.omnilink.transport.OmnilinkConnectionManager
import org.omnilink.transport.Stream2Session
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder

@OptIn(ExperimentalCoroutinesApi::class)
class Stream2SessionTest {

    private var lastRepliedKey: String? = null
    private var lastRepliedActionKey: String? = null
    private var lastRepliedText: String? = null
    private var lastCanceledKey: String? = null

    private lateinit var connectionManager: OmnilinkConnectionManager

    @Before
    fun setUp() {
        lastRepliedKey = null
        lastRepliedActionKey = null
        lastRepliedText = null
        lastCanceledKey = null

        val mockContext = mock(Context::class.java)
        `when`(mockContext.applicationContext).thenReturn(mockContext)
        val mockPrefs = mock(SharedPreferences::class.java)
        `when`(mockContext.getSharedPreferences(anyString(), anyInt())).thenReturn(mockPrefs)

        connectionManager = OmnilinkConnectionManager(
            context = mockContext
        )
    }

    private fun createLengthPrefixedFrame(packet: NotificationStreamPacket): ByteArray {
        val encoded = packet.encodeBinary()
        val buffer = ByteBuffer.allocate(4 + encoded.size).order(ByteOrder.BIG_ENDIAN)
        buffer.putInt(encoded.size)
        buffer.put(encoded)
        return buffer.array()
    }

    @Test
    fun testNotificationStreamPacketEncodingDecodingRoundtrip() {
        val posted = NotificationStreamPacket.Posted(
            NotificationPacket(
                id = "123",
                packageName = "org.telegram.messenger",
                appName = "Telegram",
                title = "Alice",
                text = "Hello there",
                iconPng = null,
                timestamp = 1729260000000L,
                canReply = true,
                actions = listOf(NotificationAction(key = "inline-reply", title = "Reply", isInput = true))
            )
        )
        val postedBytes = posted.encodeBinary()
        val decodedPosted = NotificationStreamPacket.decodeBinary(postedBytes)
        assertEquals(posted, decodedPosted)

        val reply = NotificationStreamPacket.Reply(
            NotificationReplyPacket(
                notificationId = "123",
                actionKey = "inline-reply",
                replyText = "Hello from Linux desktop!"
            )
        )
        val replyBytes = reply.encodeBinary()
        val decodedReply = NotificationStreamPacket.decodeBinary(replyBytes)
        assertEquals(reply, decodedReply)

        val dismiss = NotificationStreamPacket.Dismiss(
            NotificationDismissPacket(id = "123")
        )
        val dismissBytes = dismiss.encodeBinary()
        val decodedDismiss = NotificationStreamPacket.decodeBinary(dismissBytes)
        assertEquals(dismiss, decodedDismiss)
    }

    @Test
    fun testInboundReplyExecutesRemoteInput() = runTest {
        val replyPacket = NotificationStreamPacket.Reply(
            NotificationReplyPacket(
                notificationId = "0|com.whatsapp|456",
                actionKey = "inline-reply",
                replyText = "Replying to message"
            )
        )
        val frameBytes = createLengthPrefixedFrame(replyPacket)
        val inputStream = ByteArrayInputStream(frameBytes)
        val outputStream = ByteArrayOutputStream()

        var receivedPacket: NotificationStreamPacket? = null
        val session = Stream2Session(
            inputStream = inputStream,
            outputStream = outputStream,
            connectionManager = connectionManager,
            scope = this,
            executeReplyHandler = { key, action, text ->
                lastRepliedKey = key
                lastRepliedActionKey = action
                lastRepliedText = text
                true
            },
            onPacketReceived = { receivedPacket = it }
        )

        session.start()
        testScheduler.advanceUntilIdle()

        assertNotNull(receivedPacket)
        assertEquals(replyPacket, receivedPacket)
        assertEquals("0|com.whatsapp|456", lastRepliedKey)
        assertEquals("inline-reply", lastRepliedActionKey)
        assertEquals("Replying to message", lastRepliedText)

        session.close()
    }

    @Test
    fun testInboundDismissCancelsNotification() = runTest {
        val dismissPacket = NotificationStreamPacket.Dismiss(
            NotificationDismissPacket(id = "0|com.whatsapp|789")
        )
        val frameBytes = createLengthPrefixedFrame(dismissPacket)
        val inputStream = ByteArrayInputStream(frameBytes)
        val outputStream = ByteArrayOutputStream()

        var receivedPacket: NotificationStreamPacket? = null
        val session = Stream2Session(
            inputStream = inputStream,
            outputStream = outputStream,
            connectionManager = connectionManager,
            scope = this,
            cancelNotificationHandler = { key ->
                lastCanceledKey = key
            },
            onPacketReceived = { receivedPacket = it }
        )

        session.start()
        testScheduler.advanceUntilIdle()

        assertNotNull(receivedPacket)
        assertEquals(dismissPacket, receivedPacket)
        assertEquals("0|com.whatsapp|789", lastCanceledKey)

        session.close()
    }

    @Test
    fun testOutboundPostedAndDismissalDispatchedFromConnectionManager() = runTest {
        val inputStream = ByteArrayInputStream(ByteArray(0))
        val outputStream = ByteArrayOutputStream()

        val sentPackets = mutableListOf<NotificationStreamPacket>()
        val session = Stream2Session(
            inputStream = inputStream,
            outputStream = outputStream,
            connectionManager = connectionManager,
            scope = this,
            onPacketSent = { sentPackets.add(it) }
        )

        session.start()
        testScheduler.advanceUntilIdle()

        // 1. Send posted notification
        val posted = NotificationPacket(
            id = "test-notif-1",
            packageName = "org.test",
            appName = "Test App",
            title = "Test Title",
            text = "Test Text",
            iconPng = null,
            timestamp = 1000L,
            canReply = true
        )
        connectionManager.sendNotification(posted)
        testScheduler.advanceUntilIdle()

        assertEquals(1, sentPackets.size)
        assertTrue(sentPackets[0] is NotificationStreamPacket.Posted)
        assertEquals(posted, (sentPackets[0] as NotificationStreamPacket.Posted).packet)

        // 2. Send dismissal
        connectionManager.sendNotificationDismissed("test-notif-1")
        testScheduler.advanceUntilIdle()

        assertEquals(2, sentPackets.size)
        assertTrue(sentPackets[1] is NotificationStreamPacket.Dismiss)
        assertEquals("test-notif-1", (sentPackets[1] as NotificationStreamPacket.Dismiss).packet.id)

        session.close()
    }
}
