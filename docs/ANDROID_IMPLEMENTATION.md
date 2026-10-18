# Omnilink Android Companion: Technical Implementation Guide

This document details the Android service architecture, background resilience on Samsung One UI, NotificationListener implementation, Shizuku integration, and thumbnail extraction.

---

## 1. Project Setup & Dependencies

Built with Kotlin targeting Android 12+ (API 31 - 35) with Jetpack Compose.

### `app/build.gradle.kts`
```kotlin
dependencies {
    // AndroidX & Lifecycle
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.8.4")
    implementation("androidx.lifecycle:lifecycle-service:2.8.4")

    // Jetpack Compose
    implementation(platform("androidx.compose:compose-bom:2024.08.00"))
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.material3:material3")

    // Coroutines & Concurrency
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.8.1")

    // Serialization & Cryptography
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.7.1")
    implementation("com.google.crypto.tink:tink-android:1.14.1")

    // Shizuku (Elevated privileges without root)
    implementation("dev.rikka.shizuku:api:13.1.5")
    implementation("dev.rikka.shizuku:provider:13.1.5")

    // QR Code Scanning
    implementation("com.google.mlkit:barcode-scanning:17.3.0")
}
```

---

## 2. Samsung One UI Background Resilience

Samsung devices aggressively kill background services via "Device Care". To ensure 24/7 reliability:

1. **Foreground Service with Sticky Notification**:
   - Run `OmnilinkSyncService` as `FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE`.
2. **Battery Optimization Exemption**:
   ```kotlin
   val powerManager = getSystemService(Context.POWER_SERVICE) as PowerManager
   if (!powerManager.isIgnoringBatteryOptimizations(packageName)) {
       val intent = Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS).apply {
           data = Uri.parse("package:$packageName")
       }
       startActivity(intent)
   }
   ```
3. **Auto-Start & App Sleep Whitelist**:
   Prompt the user to set Omnilink to "Never sleeping apps" in One UI settings.

---

## 3. Notification & RemoteInput Service (`OmnilinkNotificationService.kt`)

Inherits from `NotificationListenerService`.

### Capturing Notifications
```kotlin
override fun onNotificationPosted(sbn: StatusBarNotification) {
    if (shouldIgnore(sbn.packageName)) return

    val notification = sbn.notification
    val extras = notification.extras
    val title = extras.getCharSequence(Notification.EXTRA_TITLE)?.toString() ?: ""
    val text = extras.getCharSequence(Notification.EXTRA_TEXT)?.toString() ?: ""

    // Find first action containing RemoteInput (Inline Reply)
    var replyAction: Notification.Action? = null
    var remoteInput: RemoteInput? = null

    notification.actions?.forEach { action ->
        action.remoteInputs?.firstOrNull()?.let { input ->
            replyAction = action
            remoteInput = input
        }
    }

    val packet = NotificationPostedPacket(
        id = sbn.key,
        packageName = sbn.packageName,
        appName = getAppName(sbn.packageName),
        title = title,
        text = text,
        timestamp = sbn.postTime,
        canReply = (replyAction != null && remoteInput != null)
    )

    // Send packet over QUIC Stream 2
    ServiceLocator.connectionManager.sendNotification(packet)
}
```

### Executing RemoteInput Reply from Linux
When the Linux desktop user replies in their desktop notification popup:
```kotlin
fun executeReply(notificationKey: String, replyText: String) {
    val activeNotifications = activeNotifications ?: return
    val sbn = activeNotifications.find { it.key == notificationKey } ?: return
    val notification = sbn.notification

    for (action in notification.actions ?: emptyArray()) {
        val remoteInput = action.remoteInputs?.firstOrNull() ?: continue

        val intent = Intent()
        val bundle = Bundle()
        bundle.putCharSequence(remoteInput.resultKey, replyText)
        RemoteInput.addResultsToIntent(arrayOf(remoteInput), intent, bundle)

        action.actionIntent.send(this, 0, intent)
        break
    }
}
```

---

## 4. Shizuku & scrcpy Server Integration

When Shizuku is granted, Omnilink executes `scrcpy-server.jar` locally without needing a PC plugged in via USB:

```kotlin
fun launchScrcpyServer(port: Int = 42425) {
    if (Shizuku.checkSelfPermission() != PackageManager.PERMISSION_GRANTED) {
        return
    }

    val serverJar = copyAssetToInternalStorage("scrcpy-server.jar")
    val cmd = arrayOf(
        "app_process",
        "/",
        "com.genymobile.scrcpy.Server",
        "2.4",               // version
        "port=$port",        // listen port
        "audio=false",       // audio handled via BlueZ/PipeWire
        "control=true",      // allow touch & keyboard injection
        "max_size=1920",
        "bit_rate=8000000",
        "max_fps=60"
    )

    val process = Shizuku.newProcess(cmd, null, null)
    // Daemon on Linux connects directly to phone_ip:42425 for video/input
}
```

---

## 5. MediaStore Thumbnail Pipeline

Fast cursor-based query for the photos gallery:

```kotlin
fun queryRecentThumbnails(limit: Int = 100): List<PhotoThumbnail> {
    val photos = mutableListOf<PhotoThumbnail>()
    val projection = arrayOf(
        MediaStore.Images.Media._ID,
        MediaStore.Images.Media.DATE_ADDED,
        MediaStore.Images.Media.MIME_TYPE
    )

    contentResolver.query(
        MediaStore.Images.Media.EXTERNAL_CONTENT_URI,
        projection,
        null,
        null,
        "${MediaStore.Images.Media.DATE_ADDED} DESC"
    )?.use { cursor ->
        val idCol = cursor.getColumnIndexOrThrow(MediaStore.Images.Media._ID)
        var count = 0
        while (cursor.moveToNext() && count < limit) {
            val id = cursor.getLong(idCol)
            val uri = ContentUris.withAppendedId(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, id)
            // Generate 256x256 WebP bitmap in memory
            val thumbnailBitmap = contentResolver.loadThumbnail(uri, Size(256, 256), null)
            val bytes = compressToWebP(thumbnailBitmap)
            photos.add(PhotoThumbnail(id = id, webpBytes = bytes))
            count++
        }
    }
    return photos
}
```
