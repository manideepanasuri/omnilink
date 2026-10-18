package org.omnilink.transport

import android.content.Context
import android.util.Log
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import org.omnilink.model.ConnectionState
import org.omnilink.pairing.IdentityKeyStore
import org.omnilink.protocol.ControlPacket
import java.net.InetSocketAddress
import java.net.Socket

class OmnilinkConnectionManager(
    private val context: Context,
    private val scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
) {
    private val identityKeyStore = IdentityKeyStore.getInstance(context)

    private val _connectionState = MutableStateFlow(ConnectionState.DISCONNECTED)
    val connectionState: StateFlow<ConnectionState> = _connectionState.asStateFlow()

    private val _latencyMs = MutableStateFlow<Long?>(null)
    val latencyMs: StateFlow<Long?> = _latencyMs.asStateFlow()

    private var targetHost: String = "192.168.1.100"
    private var targetPort: Int = 42424

    private var connectionJob: Job? = null
    private var currentSession: Stream0Session? = null
    private var activeSocket: Socket? = null

    @Volatile
    private var shouldBeConnected = false

    @Volatile
    private var backoffMs = INITIAL_BACKOFF_MS

    init {
        loadTargetFromStore()
    }

    private fun loadTargetFromStore() {
        val paired = identityKeyStore.getPairedDevice()
        if (paired != null) {
            val (host, port) = paired.primaryAddressAndPort() ?: ("192.168.1.100" to 42424)
            targetHost = host
            targetPort = port
        }
    }

    fun setTarget(host: String, port: Int) {
        targetHost = host
        targetPort = port
        if (shouldBeConnected) {
            triggerReconnect()
        }
    }

    fun getTarget(): Pair<String, Int> = Pair(targetHost, targetPort)

    @Synchronized
    fun connect() {
        if (shouldBeConnected && _connectionState.value != ConnectionState.DISCONNECTED) {
            return
        }
        shouldBeConnected = true
        backoffMs = INITIAL_BACKOFF_MS
        startConnectionLoop()
    }

    @Synchronized
    fun disconnect() {
        shouldBeConnected = false
        connectionJob?.cancel()
        connectionJob = null
        closeCurrentSession()
        _connectionState.value = ConnectionState.DISCONNECTED
        _latencyMs.value = null
        backoffMs = INITIAL_BACKOFF_MS
    }

    fun triggerReconnect() {
        if (!shouldBeConnected) return
        Log.i(TAG, "Triggering immediate reconnection attempt")
        backoffMs = INITIAL_BACKOFF_MS
        closeCurrentSession()
        // Cancel existing job to break delay loop and restart immediately
        connectionJob?.cancel()
        startConnectionLoop()
    }

    private fun startConnectionLoop() {
        connectionJob?.cancel()
        connectionJob = scope.launch {
            while (isActive && shouldBeConnected) {
                _connectionState.value = ConnectionState.CONNECTING
                _latencyMs.value = null

                loadTargetFromStore()
                val host = targetHost
                val port = targetPort

                var socket: Socket? = null
                try {
                    Log.i(TAG, "Attempting connection to $host:$port...")
                    socket = Socket()
                    socket.tcpNoDelay = true
                    socket.connect(InetSocketAddress(host, port), CONNECT_TIMEOUT_MS)

                    activeSocket = socket
                    _connectionState.value = ConnectionState.CONNECTED
                    backoffMs = INITIAL_BACKOFF_MS
                    Log.i(TAG, "Connected successfully to $host:$port")
                    val sessionEnded = kotlinx.coroutines.CompletableDeferred<Unit>()
                    val session = Stream0Session(
                        inputStream = socket.getInputStream(),
                        outputStream = socket.getOutputStream(),
                        scope = this,
                        onLatencyUpdated = { latency -> _latencyMs.value = latency },
                        onDisconnect = { cause ->
                            Log.i(TAG, "Stream 0 session ended: ${cause?.message}")
                            sessionEnded.complete(Unit)
                        }
                    )
                    currentSession = session
                    session.start()

                    // Suspend non-blockingly until session is terminated or coroutine is cancelled
                    sessionEnded.await()
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    Log.w(TAG, "Connection attempt to $host:$port failed: ${e.message}")
                } finally {
                    closeCurrentSession()
                }

                if (shouldBeConnected && isActive) {
                    _connectionState.value = ConnectionState.CONNECTING
                    Log.i(TAG, "Waiting ${backoffMs}ms before reconnection retry...")
                    delay(backoffMs)
                    backoffMs = (backoffMs * 2).coerceAtMost(MAX_BACKOFF_MS)
                }
            }

            if (!shouldBeConnected) {
                _connectionState.value = ConnectionState.DISCONNECTED
                _latencyMs.value = null
            }
        }
    }

    private fun closeCurrentSession() {
        try {
            currentSession?.close()
        } catch (_: Exception) {}
        currentSession = null

        try {
            activeSocket?.close()
        } catch (_: Exception) {}
        activeSocket = null
    }

    suspend fun sendControlPacket(packet: ControlPacket) {
        val session = currentSession
        if (session != null && _connectionState.value == ConnectionState.CONNECTED) {
            session.sendPacket(packet)
        } else {
            Log.d(TAG, "Cannot send packet, session not connected")
        }
    }

    fun sendDeviceStatus(status: ControlPacket.DeviceStatus) {
        scope.launch {
            try {
                sendControlPacket(status)
            } catch (e: Exception) {
                Log.w(TAG, "Failed to send DeviceStatus telemetry: ${e.message}")
            }
        }
    }

    /**
     * Extension point for Stream 2 notifications per docs/ANDROID_IMPLEMENTATION.md.
     */
    fun sendNotification(packet: Any) {
        Log.d(TAG, "sendNotification queued (Stream 2 extension point): $packet")
    }

    /**
     * Extension point for Stream 1 clipboard synchronization per docs/SUBSYSTEMS.md Section 1.
     */
    fun sendClipboard(event: Any) {
        Log.d(TAG, "sendClipboard queued (Stream 1 extension point): $event")
    }

    companion object {
        private const val TAG = "OmnilinkConnMgr"
        private const val CONNECT_TIMEOUT_MS = 6000
        const val INITIAL_BACKOFF_MS = 1000L
        const val MAX_BACKOFF_MS = 30000L
    }
}
