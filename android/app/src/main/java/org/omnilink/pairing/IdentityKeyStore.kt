package org.omnilink.pairing

import android.content.Context
import android.content.SharedPreferences
import com.google.crypto.tink.subtle.Ed25519Sign
import kotlinx.serialization.json.Json
import java.util.Base64

class IdentityKeyStore(private val context: Context) {

    private val prefs: SharedPreferences =
        context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    /**
     * Retrieves or generates the local device Ed25519 identity keypair per docs/PROTOCOL.md Section 2.
     * Returns Pair of (Base64 raw 32-byte public key, Base64 raw 32-byte private key).
     */
    @Synchronized
    fun getOrCreateDeviceIdentity(): Pair<String, String> {
        val existingPublic = prefs.getString(KEY_IDENTITY_PUBLIC, null)
        val existingPrivate = prefs.getString(KEY_IDENTITY_PRIVATE, null)
        if (existingPublic != null && existingPrivate != null) {
            return Pair(existingPublic, existingPrivate)
        }

        val keyPair = Ed25519Sign.KeyPair.newKeyPair()
        val pubBase64 = Base64.getEncoder().encodeToString(keyPair.publicKey)
        val privBase64 = Base64.getEncoder().encodeToString(keyPair.privateKey)

        prefs.edit()
            .putString(KEY_IDENTITY_PUBLIC, pubBase64)
            .putString(KEY_IDENTITY_PRIVATE, privBase64)
            .apply()

        return Pair(pubBase64, privBase64)
    }

    /**
     * Persists the paired desktop credentials.
     */
    fun savePairedDevice(payload: PairingPayload) {
        val jsonString = json.encodeToString(PairingPayload.serializer(), payload)
        prefs.edit().putString(KEY_PAIRED_DEVICE, jsonString).apply()
    }

    /**
     * Returns the paired desktop credentials if configured.
     */
    fun getPairedDevice(): PairingPayload? {
        val jsonString = prefs.getString(KEY_PAIRED_DEVICE, null) ?: return null
        return PairingPayload.fromJsonOrNull(jsonString)
    }

    /**
     * Clears any saved paired desktop credentials.
     */
    fun clearPairedDevice() {
        prefs.edit().remove(KEY_PAIRED_DEVICE).apply()
    }

    /**
     * Checks if a paired desktop currently exists.
     */
    fun isPaired(): Boolean = getPairedDevice() != null

    companion object {
        private const val PREFS_NAME = "omnilink_identity_keystore"
        private const val KEY_IDENTITY_PUBLIC = "identity_ed25519_pub"
        private const val KEY_IDENTITY_PRIVATE = "identity_ed25519_priv"
        private const val KEY_PAIRED_DEVICE = "paired_desktop_payload"

        private val json = Json {
            ignoreUnknownKeys = true
            isLenient = true
        }

        @Volatile
        private var instance: IdentityKeyStore? = null

        fun getInstance(context: Context): IdentityKeyStore {
            return instance ?: synchronized(this) {
                instance ?: IdentityKeyStore(context.applicationContext).also { instance = it }
            }
        }
    }
}
