package org.omnilink

import android.content.ClipboardManager
import android.content.ContextWrapper
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.omnilink.clipboard.ClipboardDriver
import org.omnilink.clipboard.ClipboardSyncEvent
import org.omnilink.clipboard.OmnilinkClipboardManager
import org.omnilink.clipboard.PrimaryClipData

@OptIn(ExperimentalCoroutinesApi::class)
class ClipboardManagerTest {

    private class StubClipboardDriver : ClipboardDriver {
        var currentClip: PrimaryClipData? = null
        var lastInjectedMimeType: String? = null
        var lastInjectedPayload: ByteArray? = null
        var lastInjectedIsSensitive: Boolean? = null
        var registeredListener: ClipboardManager.OnPrimaryClipChangedListener? = null
        var registerCount = 0
        var unregisterCount = 0

        override fun registerClipChangedListener(listener: ClipboardManager.OnPrimaryClipChangedListener) {
            registeredListener = listener
            registerCount++
        }

        override fun unregisterClipChangedListener(listener: ClipboardManager.OnPrimaryClipChangedListener) {
            if (registeredListener === listener) {
                registeredListener = null
            }
            unregisterCount++
        }

        override fun getPrimaryClip(): PrimaryClipData? {
            return currentClip
        }

        override fun setPrimaryClip(mimeType: String, payload: ByteArray, isSensitive: Boolean) {
            lastInjectedMimeType = mimeType
            lastInjectedPayload = payload
            lastInjectedIsSensitive = isSensitive
            currentClip = PrimaryClipData(
                mimeType = mimeType,
                payload = payload,
                isSensitive = isSensitive
            )
            // System clipboard change triggers listener in real Android
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

    @Test
    fun testPlainTextCopyingDetectionAndEventPayload() = runTest {
        val receivedEvents = mutableListOf<ClipboardSyncEvent>()
        clipboardManager.onClipboardEvent = { receivedEvents.add(it) }

        val flowEvents = mutableListOf<ClipboardSyncEvent>()
        val job = backgroundScope.launch(UnconfinedTestDispatcher(testScheduler)) {
            clipboardManager.clipboardEvents.collect { flowEvents.add(it) }
        }

        clipboardManager.startListening()
        assertTrue(clipboardManager.isListening)

        val testText = "Hello from Android Companion Clipboard!"
        val testPayload = testText.toByteArray(Charsets.UTF_8)
        stubDriver.simulateLocalCopy(
            mimeType = "text/plain",
            payload = testPayload,
            isSensitive = false
        )

        assertEquals(1, receivedEvents.size)
        assertEquals(1, flowEvents.size)

        val event = receivedEvents[0]
        assertEquals("text/plain", event.mimeType)
        assertArrayEquals(testPayload, event.payload)
        assertEquals(testText, String(event.payload, Charsets.UTF_8))
        assertFalse(event.isSensitive)

        job.cancel()
    }

    @Test
    fun testSensitiveClipsAreIgnored() {
        val receivedEvents = mutableListOf<ClipboardSyncEvent>()
        clipboardManager.onClipboardEvent = { receivedEvents.add(it) }

        clipboardManager.startListening()

        val passwordPayload = "SuperSecretPassword123!".toByteArray(Charsets.UTF_8)
        stubDriver.simulateLocalCopy(
            mimeType = "text/plain",
            payload = passwordPayload,
            isSensitive = true
        )

        assertTrue(
            "Sensitive clip must be ignored and not emitted",
            receivedEvents.isEmpty()
        )
        assertEquals(
            "EXTRA_IS_SENSITIVE constant matches Android 13+ specification",
            "android.content.extra.IS_SENSITIVE",
            OmnilinkClipboardManager.EXTRA_IS_SENSITIVE
        )
    }

    @Test
    fun testLoopPreventionInjectedClipsDoNotEmitOutgoingEvents() {
        val receivedEvents = mutableListOf<ClipboardSyncEvent>()
        clipboardManager.onClipboardEvent = { receivedEvents.add(it) }

        clipboardManager.startListening()

        // 1. Injected clip from desktop should NOT trigger outgoing event
        val injectedPayload = "Injected from Linux desktop via Wayland".toByteArray(Charsets.UTF_8)
        clipboardManager.injectClip(
            mimeType = "text/plain",
            payload = injectedPayload,
            isSensitive = false
        )

        assertEquals("text/plain", stubDriver.lastInjectedMimeType)
        assertArrayEquals(injectedPayload, stubDriver.lastInjectedPayload)
        assertFalse(stubDriver.lastInjectedIsSensitive!!)
        assertTrue(
            "Injected clip must NOT trigger outgoing sync event (loop prevention)",
            receivedEvents.isEmpty()
        )

        // 2. A subsequent distinct local user copy SHOULD trigger an event
        val userPayload = "User copied another text".toByteArray(Charsets.UTF_8)
        stubDriver.simulateLocalCopy(
            mimeType = "text/plain",
            payload = userPayload,
            isSensitive = false
        )

        assertEquals(1, receivedEvents.size)
        assertArrayEquals(userPayload, receivedEvents[0].payload)
    }

    @Test
    fun testMimeTypeFilteringSupportedTypes() {
        // Validation of MIME type filter logic
        assertTrue(OmnilinkClipboardManager.isSupportedMimeType("text/plain"))
        assertTrue(OmnilinkClipboardManager.isSupportedMimeType("text/plain; charset=utf-8"))
        assertTrue(OmnilinkClipboardManager.isSupportedMimeType("image/png"))
        assertTrue(OmnilinkClipboardManager.isSupportedMimeType("image/jpeg"))

        assertFalse(OmnilinkClipboardManager.isSupportedMimeType("application/pdf"))
        assertFalse(OmnilinkClipboardManager.isSupportedMimeType("text/html"))
        assertFalse(OmnilinkClipboardManager.isSupportedMimeType("application/octet-stream"))
        assertFalse(OmnilinkClipboardManager.isSupportedMimeType("audio/mpeg"))
        assertFalse(OmnilinkClipboardManager.isSupportedMimeType(""))

        // Test with simulated driver copies
        val receivedEvents = mutableListOf<ClipboardSyncEvent>()
        clipboardManager.onClipboardEvent = { receivedEvents.add(it) }
        clipboardManager.startListening()

        // Supported image/png
        val pngBytes = byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A)
        stubDriver.simulateLocalCopy(
            mimeType = "image/png",
            payload = pngBytes
        )
        assertEquals(1, receivedEvents.size)
        assertEquals("image/png", receivedEvents[0].mimeType)
        assertArrayEquals(pngBytes, receivedEvents[0].payload)

        // Unsupported types dropped
        val pdfBytes = "%PDF-1.4 header".toByteArray(Charsets.UTF_8)
        stubDriver.simulateLocalCopy(
            mimeType = "application/pdf",
            payload = pdfBytes
        )
        assertEquals("Unsupported MIME type application/pdf must be dropped", 1, receivedEvents.size)

        val htmlBytes = "<p>Html formatted text</p>".toByteArray(Charsets.UTF_8)
        stubDriver.simulateLocalCopy(
            mimeType = "text/html",
            payload = htmlBytes
        )
        assertEquals("Unsupported MIME type text/html must be dropped", 1, receivedEvents.size)
    }

    @Test
    fun testLifecycleStartAndStop() {
        val receivedEvents = mutableListOf<ClipboardSyncEvent>()
        clipboardManager.onClipboardEvent = { receivedEvents.add(it) }

        // Before startListening
        assertFalse(clipboardManager.isListening)
        stubDriver.simulateLocalCopy("text/plain", "Before start".toByteArray())
        assertTrue("No events should be emitted before startListening", receivedEvents.isEmpty())
        assertEquals(0, stubDriver.registerCount)

        // startListening
        clipboardManager.startListening()
        assertTrue(clipboardManager.isListening)
        assertEquals(1, stubDriver.registerCount)

        stubDriver.simulateLocalCopy("text/plain", "During listening".toByteArray())
        assertEquals(1, receivedEvents.size)

        // Calling startListening again is idempotent
        clipboardManager.startListening()
        assertEquals(1, stubDriver.registerCount)

        // stopListening
        clipboardManager.stopListening()
        assertFalse(clipboardManager.isListening)
        assertEquals(1, stubDriver.unregisterCount)

        stubDriver.simulateLocalCopy("text/plain", "After stop".toByteArray())
        assertEquals("No events should be emitted after stopListening", 1, receivedEvents.size)

        // Calling stopListening again is idempotent
        clipboardManager.stopListening()
        assertEquals(1, stubDriver.unregisterCount)
    }

    @Test
    fun testDuplicateCopySuppression() {
        val receivedEvents = mutableListOf<ClipboardSyncEvent>()
        clipboardManager.onClipboardEvent = { receivedEvents.add(it) }
        clipboardManager.startListening()

        val text = "Repeated Copy Payload".toByteArray(Charsets.UTF_8)

        // First copy
        stubDriver.simulateLocalCopy("text/plain", text)
        assertEquals(1, receivedEvents.size)

        // Repeated copy of identical payload without change
        stubDriver.simulateLocalCopy("text/plain", text)
        assertEquals("Duplicate copy should be suppressed by recent hashes check", 1, receivedEvents.size)
    }

    @Test
    fun testInjectSensitiveClip() {
        clipboardManager.startListening()

        val payload = "Sensitive desktop password".toByteArray(Charsets.UTF_8)
        clipboardManager.injectClip(
            mimeType = "text/plain",
            payload = payload,
            isSensitive = true
        )

        assertEquals("text/plain", stubDriver.lastInjectedMimeType)
        assertArrayEquals(payload, stubDriver.lastInjectedPayload)
        assertTrue(stubDriver.lastInjectedIsSensitive == true)
        assertTrue(clipboardManager.hasRecentHash(payload))
    }
}
