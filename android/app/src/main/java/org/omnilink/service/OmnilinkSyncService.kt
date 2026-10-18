package org.omnilink.service

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import android.util.Log
import androidx.core.app.NotificationCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import org.omnilink.MainActivity
import org.omnilink.R
import org.omnilink.ServiceLocator
import org.omnilink.model.ConnectionState
import org.omnilink.telemetry.DeviceTelemetryCollector

class OmnilinkSyncService : Service() {

    private val serviceScope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private var wakeLock: PowerManager.WakeLock? = null
    private val wakeLockLock = Any()

    private var connectivityManager: ConnectivityManager? = null
    private var networkCallback: ConnectivityManager.NetworkCallback? = null
    private var telemetryCollector: DeviceTelemetryCollector? = null

    override fun onCreate() {
        super.onCreate()
        ServiceLocator.init(applicationContext)
        ServiceLocator.syncService = this

        createNotificationChannel()
        initWakeLock()
        initNetworkCallback()
        initTelemetry()
        observeConnectionState()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val action = intent?.action ?: ACTION_START

        when (action) {
            ACTION_START -> {
                val notification = buildNotification(
                    title = "Omnilink Connecting...",
                    text = "Initiating background connection to desktop"
                )
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                    startForeground(
                        NOTIFICATION_ID,
                        notification,
                        ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE
                    )
                } else {
                    startForeground(NOTIFICATION_ID, notification)
                }
                ServiceLocator.connectionManager.connect()
            }
            ACTION_STOP -> {
                ServiceLocator.connectionManager.disconnect()
                releaseWakeLock()
                stopForeground(STOP_FOREGROUND_REMOVE)
                stopSelf()
            }
        }

        return START_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        super.onDestroy()
        releaseWakeLock()
        unregisterNetworkCallback()
        telemetryCollector?.stop()
        serviceScope.cancel()
        ServiceLocator.syncService = null
        Log.i(TAG, "OmnilinkSyncService destroyed")
    }

    private fun initWakeLock() {
        val powerManager = getSystemService(Context.POWER_SERVICE) as? PowerManager
        wakeLock = powerManager?.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, WAKELOCK_TAG)?.apply {
            setReferenceCounted(false)
        }
    }

    private fun acquireWakeLock() {
        synchronized(wakeLockLock) {
            wakeLock?.let {
                if (!it.isHeld) {
                    it.acquire()
                    Log.d(TAG, "Acquired partial WakeLock for connected session")
                }
            }
        }
    }

    private fun releaseWakeLock() {
        synchronized(wakeLockLock) {
            wakeLock?.let {
                if (it.isHeld) {
                    it.release()
                    Log.d(TAG, "Released partial WakeLock")
                }
            }
        }
    }

    private fun initNetworkCallback() {
        connectivityManager = getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
        val callback = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                Log.i(TAG, "Network available, evaluating auto-reconnect")
                ServiceLocator.connectionManager.triggerReconnect()
            }

            override fun onLost(network: Network) {
                Log.i(TAG, "Network lost, evaluating reconnect")
                ServiceLocator.connectionManager.triggerReconnect()
            }

            override fun onCapabilitiesChanged(network: Network, networkCapabilities: NetworkCapabilities) {
                val isWifi = networkCapabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)
                val isCellular = networkCapabilities.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR)
                Log.d(TAG, "Network transport changed: Wi-Fi=$isWifi, Cellular=$isCellular")
                telemetryCollector?.refreshAndNotify { status ->
                    ServiceLocator.connectionManager.sendDeviceStatus(status)
                }
            }
        }
        networkCallback = callback

        val request = NetworkRequest.Builder()
            .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .build()
        connectivityManager?.registerNetworkCallback(request, callback)
    }

    private fun unregisterNetworkCallback() {
        networkCallback?.let {
            try {
                connectivityManager?.unregisterNetworkCallback(it)
            } catch (e: Exception) {
                Log.w(TAG, "Failed to unregister network callback: ${e.message}")
            }
            networkCallback = null
        }
    }

    private fun initTelemetry() {
        val collector = DeviceTelemetryCollector(applicationContext)
        telemetryCollector = collector
    }

    private fun observeConnectionState() {
        serviceScope.launch {
            ServiceLocator.connectionManager.connectionState.collect { state ->
                val (host, port) = ServiceLocator.connectionManager.getTarget()
                when (state) {
                    ConnectionState.CONNECTED -> {
                        acquireWakeLock()
                        telemetryCollector?.start { status ->
                            ServiceLocator.connectionManager.sendDeviceStatus(status)
                        }
                        updateNotification(
                            title = "Omnilink Connected",
                            text = "Linked with $host:$port"
                        )
                    }
                    ConnectionState.CONNECTING -> {
                        releaseWakeLock()
                        telemetryCollector?.stop()
                        updateNotification(
                            title = "Omnilink Connecting...",
                            text = "Attempting connection to $host:$port"
                        )
                    }
                    ConnectionState.DISCONNECTED -> {
                        releaseWakeLock()
                        telemetryCollector?.stop()
                        updateNotification(
                            title = "Omnilink Offline",
                            text = "Disconnected from $host:$port"
                        )
                    }
                }
            }
        }

        serviceScope.launch {
            ServiceLocator.connectionManager.latencyMs.collect { latency ->
                if (ServiceLocator.connectionManager.connectionState.value == ConnectionState.CONNECTED) {
                    val (host, port) = ServiceLocator.connectionManager.getTarget()
                    val latencyStr = latency?.let { "${it}ms" } ?: "--"
                    updateNotification(
                        title = "Omnilink Connected",
                        text = "Linked with $host:$port ($latencyStr)"
                    )
                }
            }
        }
    }

    private fun createNotificationChannel() {
        val notificationManager = getSystemService(Context.NOTIFICATION_SERVICE) as? NotificationManager ?: return
        val channel = NotificationChannel(
            CHANNEL_ID,
            "Omnilink Sync Channel",
            NotificationManager.IMPORTANCE_LOW
        ).apply {
            description = "Persistent background connection and telemetry synchronization"
            setShowBadge(false)
        }
        notificationManager.createNotificationChannel(channel)
    }

    private fun buildNotification(title: String, text: String): Notification {
        val pendingIntent = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java).apply {
                flags = Intent.FLAG_ACTIVITY_SINGLE_TOP
            },
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )

        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle(title)
            .setContentText(text)
            .setSmallIcon(R.mipmap.ic_launcher)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setContentIntent(pendingIntent)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .build()
    }

    private fun updateNotification(title: String, text: String) {
        val notificationManager = getSystemService(Context.NOTIFICATION_SERVICE) as? NotificationManager
        notificationManager?.notify(NOTIFICATION_ID, buildNotification(title, text))
    }

    companion object {
        private const val TAG = "OmnilinkSyncService"
        const val CHANNEL_ID = "omnilink_sync_channel"
        const val NOTIFICATION_ID = 1001
        private const val WAKELOCK_TAG = "omnilink:sync_wakelock"

        const val ACTION_START = "org.omnilink.service.action.START_SYNC"
        const val ACTION_STOP = "org.omnilink.service.action.STOP_SYNC"

        fun start(context: Context) {
            val intent = Intent(context, OmnilinkSyncService::class.java).apply {
                action = ACTION_START
            }
            context.startForegroundService(intent)
        }

        fun stop(context: Context) {
            val intent = Intent(context, OmnilinkSyncService::class.java).apply {
                action = ACTION_STOP
            }
            context.startService(intent)
        }
    }
}
