package org.omnilink.transport

import android.util.Log
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.omnilink.protocol.ControlPacket
import org.omnilink.protocol.StreamFraming
import java.io.InputStream
import java.io.OutputStream

class Stream0Session(
    private val inputStream: InputStream,
    private val outputStream: OutputStream,
    private val scope: CoroutineScope,
    private val onLatencyUpdated: (Long) -> Unit,
    private val onPacketReceived: (ControlPacket) -> Unit = {},
    private val onDisconnect: (Throwable?) -> Unit = {}
) {
    private val writeMutex = Mutex()
    private var readJob: Job? = null
    private var heartbeatJob: Job? = null

    @Volatile
    private var isClosed = false

    @Volatile
    private var lastPongTimeMs: Long = System.currentTimeMillis()

    @Volatile
    private var lastPingSentTimeMs: Long = 0L

    fun start() {
        startReader()
        startHeartbeat()
    }

    private fun startReader() {
        readJob = scope.launch {
            try {
                while (isActive && !isClosed) {
                    val framePayload = StreamFraming.readFrame(inputStream)
                    if (framePayload == null) {
                        Log.i(TAG, "Stream 0 reached EOF (peer closed)")
                        break
                    }

                    val jsonStr = framePayload.toString(Charsets.UTF_8)
                    val packet = try {
                        ControlPacket.fromJson(jsonStr)
                    } catch (e: Exception) {
                        Log.w(TAG, "Failed to parse ControlPacket: $jsonStr", e)
                        continue
                    }

                    handleIncomingPacket(packet)
                }
                close(null)
            } catch (e: Exception) {
                if (e !is CancellationException && !isClosed) {
                    Log.w(TAG, "Stream 0 read error: ${e.message}")
                    close(e)
                }
            }
        }
    }

    private suspend fun handleIncomingPacket(packet: ControlPacket) {
        when (packet) {
            is ControlPacket.Ping -> {
                // Immediate Pong reply echoing original timestamp
                sendPacket(ControlPacket.Pong(packet.timestamp))
            }
            is ControlPacket.Pong -> {
                val now = System.currentTimeMillis()
                lastPongTimeMs = now
                val latency = (now - packet.timestamp).coerceAtLeast(0L)
                onLatencyUpdated(latency)
            }
            is ControlPacket.DeviceStatus, is ControlPacket.Command -> {
                onPacketReceived(packet)
            }
        }
    }

    private fun startHeartbeat() {
        heartbeatJob = scope.launch {
            try {
                while (isActive && !isClosed) {
                    delay(HEARTBEAT_INTERVAL_MS)

                    // If a ping was previously sent and peer did not reply with Pong within timeout
                    val now = System.currentTimeMillis()
                    if (lastPingSentTimeMs > lastPongTimeMs && (now - lastPingSentTimeMs > HEARTBEAT_TIMEOUT_MS)) {
                        Log.w(TAG, "Stream 0 heartbeat timed out (> ${HEARTBEAT_TIMEOUT_MS}ms without Pong)")
                        close(IllegalStateException("Heartbeat timeout"))
                        break
                    }

                    lastPingSentTimeMs = now
                    sendPacket(ControlPacket.Ping(now))
                }
            } catch (e: Exception) {
                if (e !is CancellationException && !isClosed) {
                    Log.w(TAG, "Stream 0 heartbeat error: ${e.message}")
                    close(e)
                }
            }
        }
    }

    suspend fun sendPacket(packet: ControlPacket) {
        if (isClosed) return
        val jsonStr = ControlPacket.toJson(packet)
        val payload = jsonStr.toByteArray(Charsets.UTF_8)
        writeMutex.withLock {
            try {
                StreamFraming.writeFrame(outputStream, payload)
            } catch (e: Exception) {
                if (!isClosed) {
                    Log.w(TAG, "Stream 0 write error: ${e.message}")
                    close(e)
                }
                throw e
            }
        }
    }

    fun close(cause: Throwable? = null) {
        if (isClosed) return
        isClosed = true

        heartbeatJob?.cancel()
        readJob?.cancel()

        try {
            inputStream.close()
        } catch (_: Exception) {}

        try {
            outputStream.close()
        } catch (_: Exception) {}

        onDisconnect(cause)
    }

    companion object {
        private const val TAG = "Stream0Session"
        const val HEARTBEAT_INTERVAL_MS = 5000L
        const val HEARTBEAT_TIMEOUT_MS = 5000L
    }
}
