package org.omnilink

import android.annotation.SuppressLint
import android.content.Context
import org.omnilink.pairing.IdentityKeyStore
import org.omnilink.service.OmnilinkSyncService
import org.omnilink.transport.OmnilinkConnectionManager

@SuppressLint("StaticFieldLeak")
object ServiceLocator {

    @Volatile
    private var appContext: Context? = null

    fun init(context: Context) {
        if (appContext == null) {
            synchronized(this) {
                if (appContext == null) {
                    appContext = context.applicationContext
                }
            }
        }
    }

    val context: Context
        get() = appContext ?: error("ServiceLocator must be initialized with Context prior to usage")

    val identityKeyStore: IdentityKeyStore by lazy {
        IdentityKeyStore.getInstance(context)
    }

    val connectionManager: OmnilinkConnectionManager by lazy {
        OmnilinkConnectionManager(context)
    }

    val bleAdvertiser: org.omnilink.ble.BleProximityAdvertiser by lazy {
        org.omnilink.ble.BleProximityAdvertiser(context)
    }

    val clipboardManager: org.omnilink.clipboard.OmnilinkClipboardManager by lazy {
        org.omnilink.clipboard.OmnilinkClipboardManager(context)
    }

    @Volatile
    var syncService: OmnilinkSyncService? = null
}
