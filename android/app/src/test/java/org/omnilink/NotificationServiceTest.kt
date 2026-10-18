package org.omnilink

import android.app.Notification
import android.app.RemoteInput
import android.service.notification.StatusBarNotification
import kotlinx.coroutines.launch
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.mockito.Mockito.doReturn
import org.mockito.Mockito.mock
import org.mockito.Mockito.spy
import org.mockito.Mockito.`when`
import org.omnilink.notifications.OmnilinkNotificationService
import org.omnilink.protocol.NotificationAction
import org.omnilink.protocol.NotificationPacket
import org.omnilink.protocol.NotificationReplyPacket

@OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
class NotificationServiceTest {

    private fun initTestServiceLocator(): org.omnilink.transport.OmnilinkConnectionManager {
        val mockContext = mock(android.content.Context::class.java)
        `when`(mockContext.applicationContext).thenReturn(mockContext)
        val mockPrefs = mock(android.content.SharedPreferences::class.java)
        `when`(mockContext.getSharedPreferences(org.mockito.ArgumentMatchers.anyString(), org.mockito.ArgumentMatchers.anyInt()))
            .thenReturn(mockPrefs)
        ServiceLocator.init(mockContext)
        return ServiceLocator.connectionManager
    }

    private fun createMockService(): OmnilinkNotificationService {
        val service = spy(OmnilinkNotificationService())
        doReturn("org.omnilink").`when`(service).packageName
        val mockPm = mock(android.content.pm.PackageManager::class.java)
        doReturn(mockPm).`when`(service).packageManager
        return service
    }

    @Test
    fun testNotificationPacketJsonRoundtrip() {
        val actions = listOf(
            NotificationAction(
                key = "quick_reply",
                title = "Reply",
                isInput = true
            ),
            NotificationAction(
                key = "mark_read",
                title = "Mark as read",
                isInput = false
            )
        )
        val packet = NotificationPacket(
            id = "0|org.telegram.messenger|1001|null|10123",
            packageName = "org.telegram.messenger",
            appName = "Telegram",
            title = "Alice",
            text = "Hey, are you free for coffee?",
            iconPng = listOf(137, 80, 78, 71, 13, 10, 26, 10),
            timestamp = 1718000000000L,
            canReply = true,
            actions = actions
        )

        val jsonString = packet.toJson()
        assertTrue(jsonString.contains("\"id\":\"0|org.telegram.messenger|1001|null|10123\""))
        assertTrue(jsonString.contains("\"package_name\":\"org.telegram.messenger\""))
        assertTrue(jsonString.contains("\"app_name\":\"Telegram\""))
        assertTrue(jsonString.contains("\"title\":\"Alice\""))
        assertTrue(jsonString.contains("\"text\":\"Hey, are you free for coffee?\""))
        assertTrue(jsonString.contains("\"can_reply\":true"))
        assertTrue(jsonString.contains("\"key\":\"quick_reply\""))
        assertTrue(jsonString.contains("\"is_input\":true"))
        assertTrue(jsonString.contains("\"key\":\"mark_read\""))
        assertTrue(jsonString.contains("\"is_input\":false"))

        val decoded = NotificationPacket.fromJson(jsonString)
        assertEquals(packet, decoded)
    }

    @Test
    fun testNotificationPacketJsonRoundtripWithNullIconAndDefaultActions() {
        val packet = NotificationPacket(
            id = "test-id-123",
            packageName = "com.whatsapp",
            appName = "WhatsApp",
            title = "Bob",
            text = "Meeting at 3pm",
            iconPng = null,
            timestamp = 1719000000000L,
            canReply = false
        )

        val jsonString = packet.toJson()
        val decoded = NotificationPacket.fromJson(jsonString)
        assertEquals(packet, decoded)
        assertNull(decoded.iconPng)
        assertTrue(decoded.actions.isEmpty())
        assertFalse(decoded.canReply)
    }

    @Test
    fun testNotificationPacketFromJsonSchemaCompatibility() {
        val rustJson = """
            {
                "id": "notif-rust-456",
                "package_name": "org.thoughtcrime.securesms",
                "app_name": "Signal",
                "title": "Charlie",
                "text": "Code review ready",
                "icon_png": null,
                "timestamp": 1720000000000,
                "can_reply": true,
                "actions": [
                    {
                        "key": "reply_action",
                        "title": "Reply",
                        "is_input": true
                    }
                ]
            }
        """.trimIndent()

        val packet = NotificationPacket.fromJson(rustJson)
        assertEquals("notif-rust-456", packet.id)
        assertEquals("org.thoughtcrime.securesms", packet.packageName)
        assertEquals("Signal", packet.appName)
        assertEquals("Charlie", packet.title)
        assertEquals("Code review ready", packet.text)
        assertNull(packet.iconPng)
        assertEquals(1720000000000L, packet.timestamp)
        assertTrue(packet.canReply)
        assertEquals(1, packet.actions.size)
        assertEquals("reply_action", packet.actions[0].key)
        assertEquals("Reply", packet.actions[0].title)
        assertTrue(packet.actions[0].isInput)
    }

    @Test
    fun testNotificationReplyPacketJsonRoundtrip() {
        val replyPacket = NotificationReplyPacket(
            notificationId = "notif-rust-456",
            actionKey = "reply_action",
            replyText = "Looks good to me!"
        )

        val jsonString = replyPacket.toJson()
        assertTrue(jsonString.contains("\"notification_id\":\"notif-rust-456\""))
        assertTrue(jsonString.contains("\"action_key\":\"reply_action\""))
        assertTrue(jsonString.contains("\"reply_text\":\"Looks good to me!\""))

        val decoded = NotificationReplyPacket.fromJson(jsonString)
        assertEquals(replyPacket, decoded)
    }

    @Test
    fun testPackageBlacklistFiltering() {
        // Default blacklisted packages
        assertTrue(OmnilinkNotificationService.isPackageBlacklisted("com.google.android.gms"))
        assertTrue(OmnilinkNotificationService.isPackageBlacklisted("com.android.systemui"))
        assertTrue(OmnilinkNotificationService.isPackageBlacklisted("android"))

        // Standard user apps must NOT be blacklisted by default
        assertFalse(OmnilinkNotificationService.isPackageBlacklisted("org.telegram.messenger"))
        assertFalse(OmnilinkNotificationService.isPackageBlacklisted("com.whatsapp"))
        assertFalse(OmnilinkNotificationService.isPackageBlacklisted("org.thoughtcrime.securesms"))
        assertFalse(OmnilinkNotificationService.isPackageBlacklisted("com.slack"))
        assertFalse(OmnilinkNotificationService.isPackageBlacklisted("org.omnilink"))

        // Custom blacklist override
        val customBlacklist = setOf("com.example.spammy", "com.annoying.app")
        assertTrue(OmnilinkNotificationService.isPackageBlacklisted("com.example.spammy", customBlacklist))
        assertTrue(OmnilinkNotificationService.isPackageBlacklisted("com.annoying.app", customBlacklist))
        assertFalse(OmnilinkNotificationService.isPackageBlacklisted("com.google.android.gms", customBlacklist))
    }

    @Test
    fun testRemoteInputExtractionWithSingleRemoteInputAction() {
        val mockRemoteInput = mock(RemoteInput::class.java)
        `when`(mockRemoteInput.resultKey).thenReturn("quick_reply_key")

        val mockAction = mock(Notification.Action::class.java)
        `when`(mockAction.remoteInputs).thenReturn(arrayOf(mockRemoteInput))
        mockAction.title = "Inline Reply"

        val (actions, canReply) = OmnilinkNotificationService.extractActions(arrayOf(mockAction))

        assertTrue(canReply)
        assertEquals(1, actions.size)
        val action = actions[0]
        assertEquals("quick_reply_key", action.key)
        assertEquals("Inline Reply", action.title)
        assertTrue(action.isInput)
    }

    @Test
    fun testRemoteInputExtractionWithoutRemoteInput() {
        val mockAction = mock(Notification.Action::class.java)
        `when`(mockAction.remoteInputs).thenReturn(null)
        mockAction.title = "Mark as read"

        val (actions, canReply) = OmnilinkNotificationService.extractActions(arrayOf(mockAction))

        assertFalse(canReply)
        assertEquals(1, actions.size)
        val action = actions[0]
        assertEquals("0", action.key)
        assertEquals("Mark as read", action.title)
        assertFalse(action.isInput)
    }

    @Test
    fun testRemoteInputExtractionMixedActions() {
        val mockAction1 = mock(Notification.Action::class.java)
        `when`(mockAction1.remoteInputs).thenReturn(null)
        mockAction1.title = "Archive"

        val mockRemoteInput = mock(RemoteInput::class.java)
        `when`(mockRemoteInput.resultKey).thenReturn("key_reply")

        val mockAction2 = mock(Notification.Action::class.java)
        `when`(mockAction2.remoteInputs).thenReturn(arrayOf(mockRemoteInput))
        mockAction2.title = "Reply"

        val (actions, canReply) = OmnilinkNotificationService.extractActions(arrayOf(mockAction1, mockAction2))

        assertTrue(canReply)
        assertEquals(2, actions.size)

        assertEquals("0", actions[0].key)
        assertEquals("Archive", actions[0].title)
        assertFalse(actions[0].isInput)

        assertEquals("key_reply", actions[1].key)
        assertEquals("Reply", actions[1].title)
        assertTrue(actions[1].isInput)
    }

    @Test
    fun testRemoteInputExtractionNullAndEmpty() {
        val (nullActions, nullCanReply) = OmnilinkNotificationService.extractActions(null)
        assertFalse(nullCanReply)
        assertTrue(nullActions.isEmpty())

        val (emptyActions, emptyCanReply) = OmnilinkNotificationService.extractActions(emptyArray())
        assertFalse(emptyCanReply)
        assertTrue(emptyActions.isEmpty())
    }

    @Test
    fun testConnectionManagerNotificationEventsFlow() = kotlinx.coroutines.test.runTest {
        val mockContext = mock(android.content.Context::class.java)
        `when`(mockContext.applicationContext).thenReturn(mockContext)
        val mockPrefs = mock(android.content.SharedPreferences::class.java)
        `when`(mockContext.getSharedPreferences(org.mockito.ArgumentMatchers.anyString(), org.mockito.ArgumentMatchers.anyInt()))
            .thenReturn(mockPrefs)

        val connMgr = org.omnilink.transport.OmnilinkConnectionManager(mockContext)
        val packet = NotificationPacket(
            id = "test-notif-1",
            packageName = "com.test.app",
            appName = "TestApp",
            title = "Test",
            text = "Testing notification flow",
            timestamp = 1000L,
            canReply = true
        )

        val collected = mutableListOf<NotificationPacket>()
        val job = backgroundScope.launch(kotlinx.coroutines.test.UnconfinedTestDispatcher(testScheduler)) {
            connMgr.notificationEvents.collect { collected.add(it) }
        }

        connMgr.sendNotification(packet)
        assertEquals(1, collected.size)
        assertEquals(packet, collected[0])
        job.cancel()
    }

    @Test
    fun testConnectionManagerDismissalEventsFlow() = kotlinx.coroutines.test.runTest {
        val mockContext = mock(android.content.Context::class.java)
        `when`(mockContext.applicationContext).thenReturn(mockContext)
        val mockPrefs = mock(android.content.SharedPreferences::class.java)
        `when`(mockContext.getSharedPreferences(org.mockito.ArgumentMatchers.anyString(), org.mockito.ArgumentMatchers.anyInt()))
            .thenReturn(mockPrefs)

        val connMgr = org.omnilink.transport.OmnilinkConnectionManager(mockContext)
        val collected = mutableListOf<String>()
        val job = backgroundScope.launch(kotlinx.coroutines.test.UnconfinedTestDispatcher(testScheduler)) {
            connMgr.dismissalEvents.collect { collected.add(it) }
        }

        connMgr.sendNotificationDismissed("notif-dismiss-456")
        assertEquals(1, collected.size)
        assertEquals("notif-dismiss-456", collected[0])
        job.cancel()
    }

    @Test
    fun testSelfPackageFiltering() = kotlinx.coroutines.test.runTest {
        val connMgr = initTestServiceLocator()
        val notifEvents = mutableListOf<NotificationPacket>()
        val dismissEvents = mutableListOf<String>()

        val notifJob = backgroundScope.launch(kotlinx.coroutines.test.UnconfinedTestDispatcher(testScheduler)) {
            connMgr.notificationEvents.collect { notifEvents.add(it) }
        }
        val dismissJob = backgroundScope.launch(kotlinx.coroutines.test.UnconfinedTestDispatcher(testScheduler)) {
            connMgr.dismissalEvents.collect { dismissEvents.add(it) }
        }

        val service = createMockService()

        val sbn = mock(StatusBarNotification::class.java)
        `when`(sbn.packageName).thenReturn("org.omnilink")
        `when`(sbn.isOngoing).thenReturn(false)
        `when`(sbn.key).thenReturn("omnilink-self-notif")

        service.onNotificationPosted(sbn)
        service.onNotificationRemoved(sbn)

        assertTrue("Self-package notification must not be posted", notifEvents.isEmpty())
        assertTrue("Self-package notification must not trigger dismissal", dismissEvents.isEmpty())

        notifJob.cancel()
        dismissJob.cancel()
    }

    @Test
    fun testOngoingNotificationFiltering() = kotlinx.coroutines.test.runTest {
        val connMgr = initTestServiceLocator()
        val notifEvents = mutableListOf<NotificationPacket>()
        val dismissEvents = mutableListOf<String>()

        val notifJob = backgroundScope.launch(kotlinx.coroutines.test.UnconfinedTestDispatcher(testScheduler)) {
            connMgr.notificationEvents.collect { notifEvents.add(it) }
        }
        val dismissJob = backgroundScope.launch(kotlinx.coroutines.test.UnconfinedTestDispatcher(testScheduler)) {
            connMgr.dismissalEvents.collect { dismissEvents.add(it) }
        }

        val service = createMockService()

        val sbn = mock(StatusBarNotification::class.java)
        `when`(sbn.packageName).thenReturn("com.spotify.music")
        `when`(sbn.isOngoing).thenReturn(true)
        `when`(sbn.key).thenReturn("spotify-ongoing-notif")

        service.onNotificationPosted(sbn)
        service.onNotificationRemoved(sbn)

        assertTrue("Ongoing notification must not be posted", notifEvents.isEmpty())
        assertTrue("Ongoing notification must not trigger dismissal", dismissEvents.isEmpty())

        notifJob.cancel()
        dismissJob.cancel()
    }

    @Test
    fun testDismissalEventEmission() = kotlinx.coroutines.test.runTest {
        val connMgr = initTestServiceLocator()
        val dismissEvents = mutableListOf<String>()

        val dismissJob = backgroundScope.launch(kotlinx.coroutines.test.UnconfinedTestDispatcher(testScheduler)) {
            connMgr.dismissalEvents.collect { dismissEvents.add(it) }
        }

        val service = createMockService()

        val sbn = mock(StatusBarNotification::class.java)
        `when`(sbn.packageName).thenReturn("com.whatsapp")
        `when`(sbn.isOngoing).thenReturn(false)
        `when`(sbn.key).thenReturn("whatsapp-notif-123")

        service.onNotificationRemoved(sbn)

        assertEquals(1, dismissEvents.size)
        assertEquals("whatsapp-notif-123", dismissEvents[0])

        dismissJob.cancel()
    }

    @Test
    fun testExtractActionsWithNullElements() {
        val mockAction = mock(Notification.Action::class.java)
        `when`(mockAction.remoteInputs).thenReturn(null)
        mockAction.title = "Action 1"

        val actionsArray = arrayOf(null, mockAction, null)
        val (actions, canReply) = OmnilinkNotificationService.extractActions(actionsArray)
        assertFalse(canReply)
        assertEquals(1, actions.size)
        assertEquals("Action 1", actions[0].title)
    }

    @Test
    fun testExecuteReplyLengthLimit() {
        val service = createMockService()
        val longReply = "a".repeat(4001)
        val result = service.executeReply("any-key", "0", longReply)
        assertFalse(result)
    }

    @Test
    fun testExecuteReplySelectsRemoteInputBearingActionOverIndexZero() {
        val service = createMockService()

        val mockAction0 = mock(Notification.Action::class.java)
        `when`(mockAction0.remoteInputs).thenReturn(null)
        mockAction0.title = "Mark Read"

        val mockRemoteInput = mock(RemoteInput::class.java)
        `when`(mockRemoteInput.resultKey).thenReturn("quick_reply_key")

        val mockPendingIntent = mock(android.app.PendingIntent::class.java)
        val mockAction1 = mock(Notification.Action::class.java)
        `when`(mockAction1.remoteInputs).thenReturn(arrayOf(mockRemoteInput))
        mockAction1.title = "Reply"
        mockAction1.actionIntent = mockPendingIntent

        val mockNotification = mock(Notification::class.java)
        mockNotification.actions = arrayOf(mockAction0, mockAction1)

        val mockSbn = mock(StatusBarNotification::class.java)
        `when`(mockSbn.key).thenReturn("notif-key-reply-test")
        `when`(mockSbn.notification).thenReturn(mockNotification)

        doReturn(arrayOf(mockSbn)).`when`(service).activeNotifications

        // actionKey = "0" -> should NOT fail on action 0, should select action 1 that has RemoteInputs
        val result = service.executeReply("notif-key-reply-test", "0", "Quick answer")
        assertTrue("Should succeed by finding RemoteInput-bearing action even if index 0 has none", result)
    }

    @Test
    fun testExecuteReplyNullActionIntentReturnsFalse() {
        val service = createMockService()

        val mockRemoteInput = mock(RemoteInput::class.java)
        `when`(mockRemoteInput.resultKey).thenReturn("quick_reply_key")

        val mockAction = mock(Notification.Action::class.java)
        `when`(mockAction.remoteInputs).thenReturn(arrayOf(mockRemoteInput))
        mockAction.title = "Reply"
        mockAction.actionIntent = null

        val mockNotification = mock(Notification::class.java)
        mockNotification.actions = arrayOf(mockAction)

        val mockSbn = mock(StatusBarNotification::class.java)
        `when`(mockSbn.key).thenReturn("notif-null-intent")
        `when`(mockSbn.notification).thenReturn(mockNotification)

        doReturn(arrayOf(mockSbn)).`when`(service).activeNotifications

        val result = service.executeReply("notif-null-intent", "quick_reply_key", "Quick answer")
        assertFalse("Should return false when actionIntent is null", result)
    }
}
