package org.omnilink.protocol

import kotlinx.serialization.KSerializer
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.descriptors.buildClassSerialDescriptor
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonDecoder
import kotlinx.serialization.json.JsonEncoder
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonPrimitive

@Serializable
internal data class PingPayload(
    @SerialName("timestamp") val timestamp: Long
)

@Serializable
internal data class PongPayload(
    @SerialName("timestamp") val timestamp: Long
)

@Serializable
internal data class DeviceStatusPayload(
    @SerialName("battery_pct") val batteryPct: Int,
    @SerialName("is_charging") val isCharging: Boolean,
    @SerialName("wifi_ssid") val wifiSsid: String? = null,
    @SerialName("cellular_signal") val cellularSignal: Int = 0,
    @SerialName("hotspot_active") val hotspotActive: Boolean = false
)

@Serializable
internal data class CommandPayload(
    @SerialName("action") val action: String
)

@Serializable(with = ControlPacketSerializer::class)
sealed class ControlPacket {

    data class Ping(
        val timestamp: Long
    ) : ControlPacket()

    data class Pong(
        val timestamp: Long
    ) : ControlPacket()

    data class DeviceStatus(
        val batteryPct: Int,
        val isCharging: Boolean,
        val wifiSsid: String? = null,
        val cellularSignal: Int = 0,
        val hotspotActive: Boolean = false
    ) : ControlPacket()

    data class Command(
        val action: String
    ) : ControlPacket()

    companion object {
        val json = Json {
            ignoreUnknownKeys = true
            isLenient = true
            encodeDefaults = true
            explicitNulls = true
        }

        fun fromJson(jsonStr: String): ControlPacket {
            return json.decodeFromString(ControlPacketSerializer, jsonStr)
        }

        fun toJson(packet: ControlPacket): String {
            return json.encodeToString(ControlPacketSerializer, packet)
        }
    }
}

object ControlPacketSerializer : KSerializer<ControlPacket> {
    override val descriptor: SerialDescriptor = buildClassSerialDescriptor("ControlPacket")

    override fun serialize(encoder: Encoder, value: ControlPacket) {
        val jsonEncoder = encoder as? JsonEncoder ?: error("ControlPacket can only be serialized with JSON")
        val json = jsonEncoder.json
        val element = when (value) {
            is ControlPacket.Ping -> buildJsonObject {
                put("Ping", json.encodeToJsonElement(PingPayload.serializer(), PingPayload(value.timestamp)))
            }
            is ControlPacket.Pong -> buildJsonObject {
                put("Pong", json.encodeToJsonElement(PongPayload.serializer(), PongPayload(value.timestamp)))
            }
            is ControlPacket.DeviceStatus -> buildJsonObject {
                put(
                    "DeviceStatus",
                    json.encodeToJsonElement(
                        DeviceStatusPayload.serializer(),
                        DeviceStatusPayload(
                            batteryPct = value.batteryPct,
                            isCharging = value.isCharging,
                            wifiSsid = value.wifiSsid,
                            cellularSignal = value.cellularSignal,
                            hotspotActive = value.hotspotActive
                        )
                    )
                )
            }
            is ControlPacket.Command -> buildJsonObject {
                put(
                    "Command",
                    json.encodeToJsonElement(CommandPayload.serializer(), CommandPayload(value.action))
                )
            }
        }
        jsonEncoder.encodeJsonElement(element)
    }

    override fun deserialize(decoder: Decoder): ControlPacket {
        val jsonDecoder = decoder as? JsonDecoder ?: error("ControlPacket can only be deserialized with JSON")
        val json = jsonDecoder.json
        val root = jsonDecoder.decodeJsonElement() as? JsonObject
            ?: error("Expected JSON object for ControlPacket root")

        return when {
            root.containsKey("Ping") -> {
                val pingObj = root["Ping"] ?: error("Missing payload in Ping")
                val payload = json.decodeFromJsonElement(PingPayload.serializer(), pingObj)
                ControlPacket.Ping(payload.timestamp)
            }
            root.containsKey("Pong") -> {
                val pongObj = root["Pong"] ?: error("Missing payload in Pong")
                val payload = json.decodeFromJsonElement(PongPayload.serializer(), pongObj)
                ControlPacket.Pong(payload.timestamp)
            }
            root.containsKey("DeviceStatus") -> {
                val statusObj = root["DeviceStatus"] ?: error("Missing payload in DeviceStatus")
                val payload = json.decodeFromJsonElement(DeviceStatusPayload.serializer(), statusObj)
                ControlPacket.DeviceStatus(
                    batteryPct = payload.batteryPct,
                    isCharging = payload.isCharging,
                    wifiSsid = payload.wifiSsid,
                    cellularSignal = payload.cellularSignal,
                    hotspotActive = payload.hotspotActive
                )
            }
            root.containsKey("Command") -> {
                val cmdObj = root["Command"] ?: error("Missing payload in Command")
                val actionStr = if (cmdObj is JsonObject) {
                    cmdObj["action"]?.jsonPrimitive?.content ?: cmdObj.toString()
                } else {
                    cmdObj.jsonPrimitive.content
                }
                ControlPacket.Command(actionStr)
            }
            // Fallback for flat internal tagged format: {"type": "Ping", "timestamp": ...}
            root.containsKey("type") -> {
                when (val type = root["type"]?.jsonPrimitive?.content) {
                    "Ping" -> {
                        val payload = json.decodeFromJsonElement(PingPayload.serializer(), root)
                        ControlPacket.Ping(payload.timestamp)
                    }
                    "Pong" -> {
                        val payload = json.decodeFromJsonElement(PongPayload.serializer(), root)
                        ControlPacket.Pong(payload.timestamp)
                    }
                    "DeviceStatus" -> {
                        val payload = json.decodeFromJsonElement(DeviceStatusPayload.serializer(), root)
                        ControlPacket.DeviceStatus(
                            batteryPct = payload.batteryPct,
                            isCharging = payload.isCharging,
                            wifiSsid = payload.wifiSsid,
                            cellularSignal = payload.cellularSignal,
                            hotspotActive = payload.hotspotActive
                        )
                    }
                    else -> error("Unknown ControlPacket type: $type")
                }
            }
            else -> error("Unrecognized ControlPacket format: $root")
        }
    }
}
