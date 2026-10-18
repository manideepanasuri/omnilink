package org.omnilink.notifications

import android.app.Notification
import android.app.RemoteInput
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Bundle
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import android.util.Log
import org.omnilink.ServiceLocator
import org.omnilink.protocol.NotificationAction
import org.omnilink.protocol.NotificationPacket

/**
 * Android NotificationListenerService implementation capturing incoming notifications,
 * extracting RemoteInput actions, and forwarding packets to Omnilink daemon over QUIC.
 */
class OmnilinkNotificationService : NotificationListenerService() {

    var blacklistPackages: Set<String> = DEFAULT_BLACKLIST

    override fun onListenerConnected() {
        super.onListenerConnected()
        Log.i(TAG, "OmnilinkNotificationService connected")
        ServiceLocator.notificationService = this
    }

    override fun onListenerDisconnected() {
        super.onListenerDisconnected()
        Log.i(TAG, "OmnilinkNotificationService disconnected")
        if (ServiceLocator.notificationService === this) {
            ServiceLocator.notificationService = null
        }
    }

    override fun onDestroy() {
        super.onDestroy()
        Log.i(TAG, "OmnilinkNotificationService destroyed")
        if (ServiceLocator.notificationService === this) {
            ServiceLocator.notificationService = null
        }
    }

    override fun onNotificationPosted(sbn: StatusBarNotification?) {
        if (sbn == null) return
        val packageName = sbn.packageName ?: return

        if (packageName == this.packageName) return
        if (sbn.isOngoing) return

        if (isPackageBlacklisted(packageName, blacklistPackages)) {
            Log.d(TAG, "Ignoring notification from blacklisted package: $packageName")
            return
        }

        val notification = sbn.notification ?: return
        val extras = notification.extras

        val id = sbn.key ?: return
        val appName = resolveAppName(packageName, packageManager)

        val title = extras?.getCharSequence(Notification.EXTRA_TITLE)?.toString() ?: ""
        val text = extras?.getCharSequence(Notification.EXTRA_TEXT)?.toString()
            ?: extras?.getCharSequence(Notification.EXTRA_BIG_TEXT)?.toString()
            ?: ""

        val (actions, canReply) = extractActions(notification.actions)

        val timestamp = if (sbn.postTime > 0) sbn.postTime else System.currentTimeMillis()

        val packet = NotificationPacket(
            id = id,
            packageName = packageName,
            appName = appName,
            title = title,
            text = text,
            iconPng = null,
            timestamp = timestamp,
            canReply = canReply,
            actions = actions
        )

        try {
            ServiceLocator.connectionManager.sendNotification(packet)
            Log.d(TAG, "Dispatched notification packet: id=$id, package=$packageName, title=$title, canReply=$canReply")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to dispatch notification packet: $id", e)
        }
    }

    override fun onNotificationRemoved(sbn: StatusBarNotification?) {
        if (sbn == null || sbn.isOngoing || sbn.packageName == this.packageName || isPackageBlacklisted(sbn.packageName, blacklistPackages)) return
        val key = sbn.key ?: return
        try {
            ServiceLocator.connectionManager.sendNotificationDismissed(key)
            Log.d(TAG, "Dispatched notification dismissal: $key")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to dispatch notification dismissal: $key", e)
        }
    }

    /**
     * Executes an inline quick reply for a given notification and action key.
     */
    fun executeReply(notificationKey: String, actionKey: String, replyText: String): Boolean {
        if (replyText.length > 4000) return false

        val activeSbns = try {
            activeNotifications
        } catch (e: Exception) {
            Log.e(TAG, "Failed to retrieve active notifications", e)
            null
        } ?: return false

        val sbn = activeSbns.find { it.key == notificationKey }
        if (sbn == null) {
            Log.w(TAG, "executeReply: Notification with key '$notificationKey' not found in active notifications")
            return false
        }

        val actions = sbn.notification?.actions
        if (actions.isNullOrEmpty()) {
            Log.w(TAG, "executeReply: Notification '$notificationKey' has no actions")
            return false
        }

        val action = actions.find { it?.remoteInputs?.any { ri -> ri.resultKey == actionKey } == true }
            ?: (actionKey.toIntOrNull()?.let { actions.getOrNull(it) }?.takeIf { !it.remoteInputs.isNullOrEmpty() })
            ?: actions.find { it?.title?.toString().equals(actionKey, ignoreCase = true) && !it.remoteInputs.isNullOrEmpty() }
            ?: actions.find { it != null && !it.remoteInputs.isNullOrEmpty() }

        if (action == null) {
            Log.w(TAG, "executeReply: No matching action found for actionKey '$actionKey'")
            return false
        }

        val remoteInputs = action.remoteInputs
        if (remoteInputs.isNullOrEmpty()) {
            Log.w(TAG, "executeReply: Action '$actionKey' has no RemoteInputs")
            return false
        }

        val pendingIntent = action.actionIntent ?: return false

        val intent = Intent()
        val bundle = Bundle()
        for (remoteInput in remoteInputs) {
            bundle.putCharSequence(remoteInput.resultKey, replyText)
        }
        RemoteInput.addResultsToIntent(remoteInputs, intent, bundle)

        return try {
            pendingIntent.send(this, 0, intent)
            Log.i(TAG, "Successfully executed RemoteInput reply for notification: $notificationKey")
            true
        } catch (e: Exception) {
            Log.e(TAG, "Failed to send actionIntent reply for notification: $notificationKey", e)
            false
        }
    }

    /**
     * Cancels an active notification by key.
     */
    fun cancelNotificationByKey(key: String) {
        try {
            cancelNotification(key)
            Log.i(TAG, "Cancelled notification with key: $key")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to cancel notification with key: $key", e)
        }
    }

    companion object {
        private const val TAG = "OmnilinkNotifService"

        val DEFAULT_BLACKLIST: Set<String> = setOf(
            "com.google.android.gms",
            "com.android.systemui",
            "android"
        )

        fun isPackageBlacklisted(packageName: String?, blacklist: Set<String> = DEFAULT_BLACKLIST): Boolean {
            if (packageName == null) return false
            return blacklist.contains(packageName)
        }

        fun resolveAppName(packageName: String, packageManager: PackageManager?): String {
            if (packageManager == null) return packageName
            return try {
                val appInfo = packageManager.getApplicationInfo(packageName, 0)
                packageManager.getApplicationLabel(appInfo).toString()
            } catch (_: Exception) {
                packageName
            }.ifBlank { packageName }
        }

        fun extractActions(actions: Array<out Notification.Action?>?): Pair<List<NotificationAction>, Boolean> {
            if (actions.isNullOrEmpty()) {
                return Pair(emptyList(), false)
            }
            var canReply = false
            val actionList = ArrayList<NotificationAction>(actions.size)
            for ((index, action) in actions.withIndex()) {
                if (action == null) continue
                val remoteInputs = action.remoteInputs
                val hasRemoteInput = !remoteInputs.isNullOrEmpty()
                if (hasRemoteInput) {
                    canReply = true
                }
                val key = remoteInputs?.firstOrNull()?.resultKey ?: index.toString()
                val title = action.title?.toString() ?: ""
                actionList.add(
                    NotificationAction(
                        key = key,
                        title = title,
                        isInput = hasRemoteInput
                    )
                )
            }
            return Pair(actionList, canReply)
        }
    }
}
