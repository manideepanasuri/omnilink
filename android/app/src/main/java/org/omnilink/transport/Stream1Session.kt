package org.omnilink.transport

import android.util.Log
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.omnilink.clipboard.ClipboardSyncEvent
import org.omnilink.clipboard.OmnilinkClipboardManager
import org.omnilink.protocol.ClipboardPacket
import org.omnilink.protocol.StreamFraming
import java.io.InputStream
import java.io.OutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.atomic.AtomicLong

/**
 * Handles bidirectional Stream 1 for Clipboard Synchronization over InputStream and OutputStream.
 */
class Stream1Session(
    private val inputStream: InputStream,
    private val outputStream: OutputStream,
    private val clipboardManager: OmnilinkClipboardManager,
    private val scope: CoroutineScope,
    private val onPacketReceived: (ClipboardPacket) -> Unit = {},
    private val onPacketSent: (ClipboardPacket) -> Unit = {},
    private val onDisconnect: (Throwable?) -> Unit = {}
) {
    private val writeMutex = Mutex()
    private val sequenceIdGenerator = AtomicLong(0)

    private var inboundJob: Job? = null
    private var outboundJob: Job? = null

    private val _isClosed = java.util.concurrent.atomic.AtomicBoolean(false)
    val isClosed: Boolean
        get() = _isClosed.get()

    fun nextSequenceId(): Long = sequenceIdGenerator.incrementAndGet()

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
                        Log.i(TAG, "Stream 1 reached EOF (peer closed)")
                        break
                    }
                    val length = ByteBuffer.wrap(lenBytes).order(ByteOrder.BIG_ENDIAN).int
                    if (length < 0 || length > MAX_CLIPBOARD_FRAME_SIZE) {
                        throw IllegalArgumentException("Invalid clipboard frame length: $length (max: $MAX_CLIPBOARD_FRAME_SIZE)")
                    }

                    val payload = StreamFraming.readExact(inputStream, length, allowEof = false)
                        ?: throw IllegalStateException("Unexpected EOF reading clipboard frame payload")

                    val packet = ClipboardPacket.decodeBinary(payload)
                    onPacketReceived(packet)

                    clipboardManager.injectClip(packet.mimeType, packet.payload, packet.isSensitive)
                }
            } catch (e: Exception) {
                if (e !is CancellationException && !isClosed) {
                    Log.w(TAG, "Stream 1 inbound pump error: ${e.message}")
                    close(e)
                }
            }
        }
    }

    private fun startOutboundPump() {
        outboundJob = scope.launch {
            try {
                clipboardManager.clipboardEvents.collect { event ->
                    if (isClosed || !isActive) return@collect
                    val packet = ClipboardPacket(
                        sequenceId = sequenceIdGenerator.incrementAndGet(),
                        mimeType = event.mimeType,
                        isSensitive = event.isSensitive,
                        payload = event.payload
                    )
                    sendPacket(packet)
                }
            } catch (e: Exception) {
                if (e !is CancellationException && !isClosed) {
                    Log.w(TAG, "Stream 1 outbound pump error: ${e.message}")
                    close(e)
                }
            }
        }
    }

    suspend fun sendPacket(packet: ClipboardPacket) {
        if (isClosed) return
        val encoded = packet.encodeBinary()
        val frame = ByteBuffer.allocate(4 + encoded.size)
            .order(ByteOrder.BIG_ENDIAN)
            .putInt(encoded.size)
            .put(encoded)
            .array()

        writeMutex.withLock {
            try {
                outputStream.write(frame)
                outputStream.flush()
                onPacketSent(packet)
            } catch (e: Exception) {
                if (!isClosed) {
                    Log.w(TAG, "Stream 1 write error: ${e.message}")
                    close(e)
                }
                throw e
            }
        }
    }

    suspend fun sendClipboardSyncEvent(event: ClipboardSyncEvent) {
        val packet = ClipboardPacket(
            sequenceId = sequenceIdGenerator.incrementAndGet(),
            mimeType = event.mimeType,
            isSensitive = event.isSensitive,
            payload = event.payload
        )
        sendPacket(packet)
    }

    fun close(cause: Throwable? = null) {
        if (!_isClosed.compareAndSet(false, true)) return

        inboundJob?.cancel()
        outboundJob?.cancel()

        try {
            inputStream.close()
        } catch (_: Exception) {}

        try {
            outputStream.close()
        } catch (_: Exception) {}

        onDisconnect(cause)
    }

    companion object {
        private const val TAG = "Stream1Session"
        const val MAX_CLIPBOARD_FRAME_SIZE = 20 * 1024 * 1024 // 20 MiB
    }
}
