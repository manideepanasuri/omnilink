package org.omnilink.protocol

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json

/**
 * Action item associated with an incoming notification.
 * Matches `omnilink_protocol::packet::NotificationAction`.
 */
@Serializable
data class NotificationAction(
    @SerialName("key") val key: String,
    @SerialName("title") val title: String,
    @SerialName("is_input") val isInput: Boolean
)

/**
 * Incoming notification packet received or dispatched over Stream 2.
 * Matches `omnilink_protocol::packet::NotificationPacket`.
 */
@Serializable
data class NotificationPacket(
    @SerialName("id") val id: String,
    @SerialName("package_name") val packageName: String,
    @SerialName("app_name") val appName: String,
    @SerialName("title") val title: String,
    @SerialName("text") val text: String,
    @SerialName("icon_png") val iconPng: List<Int>? = null,
    @SerialName("timestamp") val timestamp: Long,
    @SerialName("can_reply") val canReply: Boolean,
    @SerialName("actions") val actions: List<NotificationAction> = emptyList()
) {
    fun toJson(): String = Companion.toJson(this)

    companion object {
        val json = Json {
            ignoreUnknownKeys = true
            isLenient = true
            encodeDefaults = true
            explicitNulls = true
        }

        fun fromJson(jsonStr: String): NotificationPacket {
            return json.decodeFromString(serializer(), jsonStr)
        }

        fun toJson(packet: NotificationPacket): String {
            return json.encodeToString(serializer(), packet)
        }
    }
}

/**
 * Outgoing reply packet for an actionable notification transmitted over Stream 2.
 * Matches `omnilink_protocol::packet::NotificationReplyPacket`.
 */
@Serializable
data class NotificationReplyPacket(
    @SerialName("notification_id") val notificationId: String,
    @SerialName("action_key") val actionKey: String,
    @SerialName("reply_text") val replyText: String
) {
    fun toJson(): String = Companion.toJson(this)

    companion object {
        val json = Json {
            ignoreUnknownKeys = true
            isLenient = true
            encodeDefaults = true
            explicitNulls = true
        }

        fun fromJson(jsonStr: String): NotificationReplyPacket {
            return json.decodeFromString(serializer(), jsonStr)
        }

        fun toJson(packet: NotificationReplyPacket): String {
            return json.encodeToString(serializer(), packet)
        }
    }
}

/**
 * Bidirectional dismissal synchronization packet for Stream 2.
 * Matches `omnilink_protocol::packet::NotificationDismissPacket`.
 */
@Serializable
data class NotificationDismissPacket(
    @SerialName("id") val id: String
) {
    fun toJson(): String = Companion.toJson(this)

    companion object {
        val json = Json {
            ignoreUnknownKeys = true
            isLenient = true
            encodeDefaults = true
            explicitNulls = true
        }

        fun fromJson(jsonStr: String): NotificationDismissPacket {
            return json.decodeFromString(serializer(), jsonStr)
        }

        fun toJson(packet: NotificationDismissPacket): String {
            return json.encodeToString(serializer(), packet)
        }
    }
}

/**
 * Multiplexed packet types transmitted over QUIC Stream 2.
 * Binary format: `[type_id: 1 byte][payload: N bytes UTF-8 JSON]`
 */
sealed class NotificationStreamPacket {
    data class Posted(val packet: NotificationPacket) : NotificationStreamPacket()
    data class Reply(val packet: NotificationReplyPacket) : NotificationStreamPacket()
    data class Dismiss(val packet: NotificationDismissPacket) : NotificationStreamPacket()

    fun encodeBinary(): ByteArray {
        val (typeId, jsonStr) = when (this) {
            is Posted -> TYPE_POSTED to packet.toJson()
            is Reply -> TYPE_REPLY to packet.toJson()
            is Dismiss -> TYPE_DISMISS to packet.toJson()
        }
        val jsonBytes = jsonStr.toByteArray(Charsets.UTF_8)
        val buf = java.nio.ByteBuffer.allocate(1 + jsonBytes.size)
        buf.put(typeId)
        buf.put(jsonBytes)
        return buf.array()
    }

    companion object {
        const val TYPE_POSTED: Byte = 1
        const val TYPE_REPLY: Byte = 2
        const val TYPE_DISMISS: Byte = 3

        fun decodeBinary(bytes: ByteArray): NotificationStreamPacket {
            require(bytes.isNotEmpty()) { "Empty payload for NotificationStreamPacket" }
            val typeId = bytes[0]
            val jsonStr = String(bytes, 1, bytes.size - 1, Charsets.UTF_8)
            return when (typeId) {
                TYPE_POSTED -> Posted(NotificationPacket.fromJson(jsonStr))
                TYPE_REPLY -> Reply(NotificationReplyPacket.fromJson(jsonStr))
                TYPE_DISMISS -> Dismiss(NotificationDismissPacket.fromJson(jsonStr))
                else -> throw IllegalArgumentException("Unknown Stream 2 packet type ID: $typeId")
            }
        }
    }
}

