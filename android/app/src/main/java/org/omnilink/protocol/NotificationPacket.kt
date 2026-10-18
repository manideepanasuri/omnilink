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
