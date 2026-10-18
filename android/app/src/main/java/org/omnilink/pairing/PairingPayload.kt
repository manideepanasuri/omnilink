package org.omnilink.pairing

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import java.util.Base64

@Serializable
data class PairingPayload(
    @SerialName("v") val version: Int,
    @SerialName("name") val name: String,
    @SerialName("pk") val publicKeyBase64: String,
    @SerialName("addrs") val addresses: List<String>,
    @SerialName("token") val token: String
) {
    /**
     * Validates the payload fields according to the Omnilink specification:
     * - version == 1
     * - name not blank
     * - publicKeyBase64 is non-blank and valid non-empty Base64
     * - addresses not empty (contains at least one non-blank address)
     * - token not blank
     */
    fun isValid(): Boolean {
        if (version != 1) return false
        if (name.isBlank()) return false
        if (publicKeyBase64.isBlank() || !isValidBase64(publicKeyBase64)) return false
        if (addresses.isEmpty() || addresses.all { it.isBlank() }) return false
        if (token.isBlank()) return false
        return true
    }

    /**
     * Extracts host address and port from the primary address entry.
     * Defaults port to 42424 if unspecified.
     */
    fun primaryAddressAndPort(): Pair<String, Int>? {
        val addr = addresses.firstOrNull { it.isNotBlank() } ?: return null
        return parseAddress(addr)
    }

    companion object {
        private val json = Json {
            ignoreUnknownKeys = true
            isLenient = true
        }

        fun fromJson(jsonString: String): PairingPayload {
            val payload = json.decodeFromString<PairingPayload>(jsonString)
            require(payload.isValid()) { "Invalid pairing payload format or fields" }
            return payload
        }

        fun fromJsonOrNull(jsonString: String): PairingPayload? {
            return try {
                val payload = json.decodeFromString<PairingPayload>(jsonString)
                if (payload.isValid()) payload else null
            } catch (_: Throwable) {
                null
            }
        }

        fun isValidBase64(str: String): Boolean {
            val trimmed = str.trim()
            if (trimmed.isEmpty()) return false
            val padded = when (trimmed.length % 4) {
                2 -> "$trimmed=="
                3 -> "$trimmed="
                else -> trimmed
            }
            return try {
                val decoded = Base64.getDecoder().decode(padded)
                decoded.isNotEmpty()
            } catch (_: IllegalArgumentException) {
                try {
                    val decoded = Base64.getUrlDecoder().decode(padded)
                    decoded.isNotEmpty()
                } catch (_: IllegalArgumentException) {
                    false
                }
            }
        }

        fun parseAddress(addressString: String): Pair<String, Int> {
            val trimmed = addressString.trim()
            if (trimmed.startsWith("[")) {
                val closingBracket = trimmed.indexOf("]")
                if (closingBracket != -1) {
                    val host = trimmed.substring(1, closingBracket)
                    val portStr = trimmed.substring(closingBracket + 1).removePrefix(":")
                    val parsedPort = portStr.toIntOrNull()
                    val port = if (parsedPort != null && parsedPort in 1..65535) parsedPort else 42424
                    return Pair(host, port)
                }
            }
            val colonIndex = trimmed.lastIndexOf(":")
            if (colonIndex != -1 && colonIndex > 0) {
                val host = trimmed.substring(0, colonIndex)
                val parsedPort = trimmed.substring(colonIndex + 1).toIntOrNull()
                val port = if (parsedPort != null && parsedPort in 1..65535) parsedPort else 42424
                return Pair(host, port)
            }
            return Pair(trimmed, 42424)
        }
    }
}
