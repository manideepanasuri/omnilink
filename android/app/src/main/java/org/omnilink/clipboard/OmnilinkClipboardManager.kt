package org.omnilink.clipboard

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.PersistableBundle
import android.util.Log
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import java.io.File
import java.security.MessageDigest

/**
 * Event representing captured clipboard content ready for synchronization.
 */
data class ClipboardSyncEvent(
    val mimeType: String,
    val payload: ByteArray,
    val isSensitive: Boolean
) {
    override fun equals(other: Any?): Boolean {
        if (this === other) return true
        if (other !is ClipboardSyncEvent) return false
        return mimeType == other.mimeType &&
                isSensitive == other.isSensitive &&
                payload.contentEquals(other.payload)
    }

    override fun hashCode(): Int {
        var result = mimeType.hashCode()
        result = 31 * result + payload.contentHashCode()
        result = 31 * result + isSensitive.hashCode()
        return result
    }
}

/**
 * Normalized clip data representation extracted by the driver.
 */
data class PrimaryClipData(
    val mimeType: String,
    val payload: ByteArray,
    val isSensitive: Boolean = false,
    val description: ClipDescription? = null
) {
    override fun equals(other: Any?): Boolean {
        if (this === other) return true
        if (other !is PrimaryClipData) return false
        return mimeType == other.mimeType &&
                isSensitive == other.isSensitive &&
                payload.contentEquals(other.payload)
    }

    override fun hashCode(): Int {
        var result = mimeType.hashCode()
        result = 31 * result + payload.contentHashCode()
        result = 31 * result + isSensitive.hashCode()
        return result
    }
}

/**
 * Driver abstraction for Android system ClipboardManager for testability.
 */
interface ClipboardDriver {
    fun registerClipChangedListener(listener: ClipboardManager.OnPrimaryClipChangedListener)
    fun unregisterClipChangedListener(listener: ClipboardManager.OnPrimaryClipChangedListener)
    fun getPrimaryClip(): PrimaryClipData?
    fun setPrimaryClip(mimeType: String, payload: ByteArray, isSensitive: Boolean)
}

/**
 * Android framework implementation of [ClipboardDriver].
 */
class AndroidClipboardDriver(
    private val context: Context
) : ClipboardDriver {

    private val clipboardManager: ClipboardManager? by lazy {
        context.getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager
    }

    private var registeredListener: ClipboardManager.OnPrimaryClipChangedListener? = null

    override fun registerClipChangedListener(listener: ClipboardManager.OnPrimaryClipChangedListener) {
        unregisterClipChangedListener(listener)
        val cm = clipboardManager ?: return
        registeredListener = listener
        cm.addPrimaryClipChangedListener(listener)
    }

    override fun unregisterClipChangedListener(listener: ClipboardManager.OnPrimaryClipChangedListener) {
        registeredListener?.let {
            clipboardManager?.removePrimaryClipChangedListener(it)
        }
        registeredListener = null
    }

    override fun getPrimaryClip(): PrimaryClipData? {
        val cm = clipboardManager ?: return null
        val clip = cm.primaryClip ?: return null
        if (clip.itemCount == 0) return null
        val desc = clip.description ?: return null

        val isSensitive = OmnilinkClipboardManager.isClipDescriptionSensitive(desc)

        val rawMime = when {
            desc.hasMimeType(ClipDescription.MIMETYPE_TEXT_PLAIN) ||
                    desc.hasMimeType("text/plain") -> OmnilinkClipboardManager.MIME_TEXT_PLAIN
            desc.hasMimeType(OmnilinkClipboardManager.MIME_IMAGE_PNG) -> OmnilinkClipboardManager.MIME_IMAGE_PNG
            desc.hasMimeType(OmnilinkClipboardManager.MIME_IMAGE_JPEG) -> OmnilinkClipboardManager.MIME_IMAGE_JPEG
            desc.mimeTypeCount > 0 -> desc.getMimeType(0)
            else -> return null
        }

        val item = clip.getItemAt(0) ?: return null
        val payload: ByteArray? = when {
            rawMime == OmnilinkClipboardManager.MIME_TEXT_PLAIN || rawMime.startsWith("text/") -> {
                val text = item.text?.toString() ?: item.coerceToText(context)?.toString()
                text?.toByteArray(Charsets.UTF_8)
            }
            rawMime == OmnilinkClipboardManager.MIME_IMAGE_PNG || rawMime == OmnilinkClipboardManager.MIME_IMAGE_JPEG -> {
                val uri = item.uri
                if (uri != null) {
                    try {
                        context.contentResolver.openInputStream(uri)?.use { stream ->
                            val maxBytes = 20 * 1024 * 1024 // 20 MiB safety clamp
                            val output = java.io.ByteArrayOutputStream()
                            val chunk = ByteArray(8192)
                            var totalRead = 0
                            while (true) {
                                val n = stream.read(chunk)
                                if (n == -1) break
                                if (totalRead + n > maxBytes) {
                                    output.write(chunk, 0, maxBytes - totalRead)
                                    break
                                }
                                output.write(chunk, 0, n)
                                totalRead += n
                            }
                            output.toByteArray()
                        }
                    } catch (e: Exception) {
                        Log.w(TAG, "Failed to read image bytes from $uri: ${e.message}")
                        null
                    }
                } else {
                    null
                }
            }
            else -> {
                item.text?.toString()?.toByteArray(Charsets.UTF_8)
            }
        }

        if (payload == null) return null

        return PrimaryClipData(
            mimeType = rawMime,
            payload = payload,
            isSensitive = isSensitive,
            description = desc
        )
    }

    override fun setPrimaryClip(mimeType: String, payload: ByteArray, isSensitive: Boolean) {
        val cm = clipboardManager ?: return
        val clipData = when {
            mimeType == OmnilinkClipboardManager.MIME_TEXT_PLAIN || mimeType.startsWith("text/") -> {
                val text = String(payload, Charsets.UTF_8)
                ClipData.newPlainText("Omnilink Text", text)
            }
            mimeType == OmnilinkClipboardManager.MIME_IMAGE_PNG || mimeType == OmnilinkClipboardManager.MIME_IMAGE_JPEG -> {
                val ext = if (mimeType == OmnilinkClipboardManager.MIME_IMAGE_JPEG) "jpg" else "png"
                val cacheDir = File(context.cacheDir, "clipboard").apply { mkdirs() }
                val uniqueId = System.currentTimeMillis().toString() + "_" + java.util.UUID.randomUUID().toString().take(8)
                val file = File(cacheDir, "omnilink_clip_$uniqueId.$ext")
                file.writeBytes(payload)
                val uri = try {
                    androidx.core.content.FileProvider.getUriForFile(
                        context,
                        "${context.packageName}.fileprovider",
                        file
                    )
                } catch (e: Exception) {
                    Log.w(TAG, "FileProvider getUriForFile failed, falling back to Uri.fromFile: ${e.message}")
                    Uri.fromFile(file)
                }
                ClipData.newUri(context.contentResolver, "Omnilink Image", uri).apply {
                    try {
                        context.grantUriPermission(context.packageName, uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
                    } catch (_: Exception) {}
                }
            }
            else -> {
                ClipData.newPlainText("Omnilink Payload", String(payload, Charsets.UTF_8))
            }
        }

        if (isSensitive) {
            val bundle = PersistableBundle().apply {
                putBoolean(OmnilinkClipboardManager.EXTRA_IS_SENSITIVE, true)
            }
            clipData.description.extras = bundle
        }

        try {
            cm.setPrimaryClip(clipData)
        } catch (e: Exception) {
            Log.e(TAG, "Failed to set primary clip: ${e.message}", e)
        }
    }

    companion object {
        private const val TAG = "AndroidClipboardDriver"
    }
}

/**
 * Manages clipboard monitoring, sensitive content filtering, echo loop suppression,
 * and safe remote clip injection per docs/SUBSYSTEMS.md Section 1.
 */
class OmnilinkClipboardManager(
    private val context: Context,
    private val driver: ClipboardDriver = AndroidClipboardDriver(context)
) : ClipboardManager.OnPrimaryClipChangedListener {

    private val _isListening = MutableStateFlow(false)
    val isListeningFlow: StateFlow<Boolean> = _isListening.asStateFlow()
    val isListening: Boolean
        get() = _isListening.value

    private val _clipboardEvents = MutableSharedFlow<ClipboardSyncEvent>(
        replay = 0,
        extraBufferCapacity = 16,
        onBufferOverflow = BufferOverflow.DROP_OLDEST
    )
    val clipboardEvents: SharedFlow<ClipboardSyncEvent> = _clipboardEvents.asSharedFlow()

    @Volatile
    var onClipboardEvent: ((ClipboardSyncEvent) -> Unit)? = null

    private val hashLock = Any()
    private val recentHashes = ArrayDeque<String>(MAX_RECENT_HASHES)
    private val recentHashSet = HashSet<String>(MAX_RECENT_HASHES)

    @Synchronized
    fun startListening() {
        if (_isListening.value) return
        _isListening.value = true
        driver.registerClipChangedListener(this)
        Log.i(TAG, "Started listening to primary clipboard changes")
    }

    @Synchronized
    fun stopListening() {
        if (!_isListening.value) return
        _isListening.value = false
        driver.unregisterClipChangedListener(this)
        Log.i(TAG, "Stopped listening to primary clipboard changes")
    }

    override fun onPrimaryClipChanged() {
        handlePrimaryClipChanged()
    }

    fun handlePrimaryClipChanged() {
        if (!_isListening.value) return

        val clip = driver.getPrimaryClip() ?: return

        // 1. Sensitive check: honor ClipDescription.EXTRA_IS_SENSITIVE (Android 13+ / API 33+)
        val isSensitive = clip.isSensitive || isClipDescriptionSensitive(clip.description)
        if (isSensitive) {
            Log.d(TAG, "Skipping sensitive primary clip synchronization")
            return
        }

        // 2. MIME filtering: supports text/plain and image/png
        val cleanMime = sanitizeMimeType(clip.mimeType)
        if (!isSupportedMimeType(cleanMime)) {
            Log.d(TAG, "Skipping unsupported clipboard MIME type: ${clip.mimeType}")
            return
        }

        // 3. Echo loop prevention: maintains recent hashes circular buffer
        val hash = computeHash(clip.payload)
        synchronized(hashLock) {
            if (hasRecentHashLocked(hash)) {
                Log.d(TAG, "Echo loop prevention: skipping recently injected/emitted hash $hash")
                return
            }
            recordHashLocked(hash)
        }

        // 4. Emit sync event
        val event = ClipboardSyncEvent(
            mimeType = cleanMime,
            payload = clip.payload,
            isSensitive = false
        )
        _clipboardEvents.tryEmit(event)
        onClipboardEvent?.invoke(event)
    }

    /**
     * Injects clipboard payload received from remote companion into local Android clipboard.
     * Prevents echo loops by recording the payload hash prior to injection.
     */
    fun injectClip(mimeType: String, payload: ByteArray, isSensitive: Boolean = false) {
        val cleanMime = sanitizeMimeType(mimeType)
        val hash = computeHash(payload)

        synchronized(hashLock) {
            recordHashLocked(hash)
        }

        driver.setPrimaryClip(cleanMime, payload, isSensitive)
    }

    fun hasRecentHash(payload: ByteArray): Boolean {
        val hash = computeHash(payload)
        synchronized(hashLock) {
            return hasRecentHashLocked(hash)
        }
    }

    fun clearRecentHashes() {
        synchronized(hashLock) {
            recentHashes.clear()
            recentHashSet.clear()
        }
    }

    private fun recordHashLocked(hash: String) {
        if (recentHashSet.contains(hash)) return
        if (recentHashes.size >= MAX_RECENT_HASHES) {
            val oldest = recentHashes.removeFirst()
            recentHashSet.remove(oldest)
        }
        recentHashes.addLast(hash)
        recentHashSet.add(hash)
    }

    private fun hasRecentHashLocked(hash: String): Boolean {
        return recentHashSet.contains(hash)
    }

    companion object {
        private const val TAG = "OmnilinkClipboardMgr"
        const val EXTRA_IS_SENSITIVE = "android.content.extra.IS_SENSITIVE"
        const val MIME_TEXT_PLAIN = "text/plain"
        const val MIME_IMAGE_PNG = "image/png"
        const val MIME_IMAGE_JPEG = "image/jpeg"
        const val MAX_RECENT_HASHES = 32

        fun sanitizeMimeType(mimeType: String): String {
            val clean = mimeType.substringBefore(";").trim().lowercase()
            return when (clean) {
                "text/plain", "text" -> MIME_TEXT_PLAIN
                "image/png" -> MIME_IMAGE_PNG
                "image/jpeg", "image/jpg" -> MIME_IMAGE_JPEG
                else -> clean
            }
        }

        fun isSupportedMimeType(mimeType: String): Boolean {
            val sanitized = sanitizeMimeType(mimeType)
            return sanitized == MIME_TEXT_PLAIN ||
                    sanitized == MIME_IMAGE_PNG ||
                    sanitized == MIME_IMAGE_JPEG
        }

        fun isClipDescriptionSensitive(desc: ClipDescription?): Boolean {
            if (desc == null) return false
            val extras = desc.extras ?: return false
            return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                extras.getBoolean(ClipDescription.EXTRA_IS_SENSITIVE, false)
            } else {
                extras.getBoolean(EXTRA_IS_SENSITIVE, false)
            }
        }

        fun computeHash(payload: ByteArray): String {
            val md = MessageDigest.getInstance("SHA-256")
            val digest = md.digest(payload)
            val sb = StringBuilder(digest.size * 2)
            for (b in digest) {
                val v = b.toInt() and 0xFF
                if (v < 16) sb.append('0')
                sb.append(v.toString(16))
            }
            return sb.toString()
        }
    }
}
