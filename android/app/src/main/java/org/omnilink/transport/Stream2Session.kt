package org.omnilink.transport

import android.util.Log
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.omnilink.notifications.OmnilinkNotificationService
import org.omnilink.protocol.NotificationDismissPacket
import org.omnilink.protocol.NotificationStreamPacket
import org.omnilink.protocol.StreamFraming
import java.io.InputStream
import java.io.OutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Handles bidirectional Stream 2 for Notification & Quick Reply synchronization.
 */
class Stream2Session(
    private val inputStream: InputStream,
    private val outputStream: OutputStream,
    private val connectionManager: OmnilinkConnectionManager,
    private val scope: CoroutineScope,
    private val executeReplyHandler: (notificationKey: String, actionKey: String, replyText: String) -> Boolean = { key, action, text ->
        org.omnilink.ServiceLocator.notificationService?.executeReply(key, action, text) ?: false
    },
    private val cancelNotificationHandler: (notificationKey: String) -> Unit = { key ->
        org.omnilink.ServiceLocator.notificationService?.cancelNotificationByKey(key)
    },
    private val onPacketReceived: (NotificationStreamPacket) -> Unit = {},
    private val onPacketSent: (NotificationStreamPacket) -> Unit = {},
    private val onDisconnect: (Throwable?) -> Unit = {}
) {
    private val writeMutex = Mutex()

    private var inboundJob: Job? = null
    private var outboundPostedJob: Job? = null
    private var outboundDismissJob: Job? = null

    private val _isClosed = AtomicBoolean(false)
    val isClosed: Boolean
        get() = _isClosed.get()

    fun start() {
        startInboundPump()
        startOutboundPump()
    }

    private fun startInboundPump() {
        inboundJob = scope.launch {
            try {
                while (isActive && !isClosed) {
                    val lenBytes = StreamFraming.readExact(inputStream, 4, allowEof = true)
                    if (lenBytes == null) {
                        Log.i(TAG, "Stream 2 reached EOF (peer closed)")
                        break
                    }
                    val length = ByteBuffer.wrap(lenBytes).order(ByteOrder.BIG_ENDIAN).int
                    if (length < 0 || length > MAX_NOTIFICATION_FRAME_SIZE) {
                        throw IllegalArgumentException("Invalid notification frame length: $length (max: $MAX_NOTIFICATION_FRAME_SIZE)")
                    }

                    val payload = StreamFraming.readExact(inputStream, length, allowEof = false)
                        ?: throw IllegalStateException("Unexpected EOF reading notification frame payload")

                    val packet = NotificationStreamPacket.decodeBinary(payload)
                    onPacketReceived(packet)

                    when (packet) {
                        is NotificationStreamPacket.Reply -> {
                            Log.d(TAG, "Stream 2 executing remote inline reply for ${packet.packet.notificationId}")
                            executeReplyHandler(
                                packet.packet.notificationId,
                                packet.packet.actionKey,
                                packet.packet.replyText
                            )
                        }
                        is NotificationStreamPacket.Dismiss -> {
                            Log.d(TAG, "Stream 2 executing remote dismissal for ${packet.packet.id}")
                            cancelNotificationHandler(packet.packet.id)
                        }
                        is NotificationStreamPacket.Posted -> {
                            Log.d(TAG, "Stream 2 received unexpected inbound posted notification: ${packet.packet.id}")
                        }
                    }
                }
            } catch (e: Exception) {
                if (e !is CancellationException && !isClosed) {
                    Log.w(TAG, "Stream 2 inbound pump error: ${e.message}")
                    close(e)
                }
            }
        }
    }

    private fun startOutboundPump() {
        outboundPostedJob = scope.launch {
            try {
                connectionManager.notificationEvents.collect { postedPacket ->
                    if (isClosed || !isActive) return@collect
                    sendPacket(NotificationStreamPacket.Posted(postedPacket))
                }
            } catch (e: Exception) {
                if (e !is CancellationException && !isClosed) {
                    Log.w(TAG, "Stream 2 outbound posted pump error: ${e.message}")
                    close(e)
                }
            }
        }

        outboundDismissJob = scope.launch {
            try {
                connectionManager.dismissalEvents.collect { dismissedKey ->
                    if (isClosed || !isActive) return@collect
                    sendPacket(NotificationStreamPacket.Dismiss(NotificationDismissPacket(dismissedKey)))
                }
            } catch (e: Exception) {
                if (e !is CancellationException && !isClosed) {
                    Log.w(TAG, "Stream 2 outbound dismissal pump error: ${e.message}")
                    close(e)
                }
            }
        }
    }

    suspend fun sendPacket(packet: NotificationStreamPacket) {
        if (isClosed) return
        val raw = packet.encodeBinary()
        if (raw.size > MAX_NOTIFICATION_FRAME_SIZE) {
            throw IllegalArgumentException("Notification packet size ${raw.size} exceeds maximum $MAX_NOTIFICATION_FRAME_SIZE")
        }

        val frame = ByteBuffer.allocate(4 + raw.size)
        frame.order(ByteOrder.BIG_ENDIAN)
        frame.putInt(raw.size)
        frame.put(raw)

        writeMutex.withLock {
            if (isClosed) return
            outputStream.write(frame.array())
            outputStream.flush()
        }
        onPacketSent(packet)
    }

    fun close(cause: Throwable? = null) {
        if (_isClosed.compareAndSet(false, true)) {
            Log.i(TAG, "Closing Stream 2 session (cause: ${cause?.message})")
            inboundJob?.cancel()
            outboundPostedJob?.cancel()
            outboundDismissJob?.cancel()

            try {
                outputStream.close()
            } catch (_: Exception) {}

            try {
                inputStream.close()
            } catch (_: Exception) {}

            onDisconnect(cause)
        }
    }

    companion object {
        private const val TAG = "Stream2Session"
        const val MAX_NOTIFICATION_FRAME_SIZE = 20 * 1024 * 1024 // 20 MiB safety clamp
    }
}
