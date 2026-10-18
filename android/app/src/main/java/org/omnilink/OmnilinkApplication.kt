package org.omnilink

import android.app.Application
import android.util.Log

class OmnilinkApplication : Application() {

    override fun onCreate() {
        super.onCreate()
        ServiceLocator.init(this)
        Log.i(TAG, "OmnilinkApplication initialized. Target SDK: 35, Min SDK: 31.")
    }

    companion object {
        private const val TAG = "OmnilinkApp"
    }
}
