package org.omnilink

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.omnilink.protocol.ControlPacket
import org.omnilink.protocol.StreamFraming
import org.omnilink.transport.Stream0Session
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream

@OptIn(ExperimentalCoroutinesApi::class)
class Stream0SessionTest {

    @Test
    fun testPingGeneratesImmediatePongReply() = runTest {
        val pingTimestamp = 1729251234567L
        val pingPacket = ControlPacket.Ping(pingTimestamp)
        val pingBytes = ControlPacket.toJson(pingPacket).toByteArray(Charsets.UTF_8)
        val frameBytes = StreamFraming.encodeFrame(pingBytes)

        val clientInput = ByteArrayInputStream(frameBytes)
        val clientOutput = ByteArrayOutputStream()

        var latencyRecorded: Long? = null
        val session = Stream0Session(
            inputStream = clientInput,
            outputStream = clientOutput,
            scope = this,
            onLatencyUpdated = { latencyRecorded = it }
        )
        session.start()

        testScheduler.advanceUntilIdle()

        val writtenBytes = clientOutput.toByteArray()
        assertTrue(writtenBytes.isNotEmpty())

        val pongFrame = StreamFraming.readFrame(ByteArrayInputStream(writtenBytes))
        assertNotNull(pongFrame)

        val pongJson = String(pongFrame!!, Charsets.UTF_8)
        val replyPacket = ControlPacket.fromJson(pongJson)
        assertTrue(replyPacket is ControlPacket.Pong)
        assertEquals(pingTimestamp, (replyPacket as ControlPacket.Pong).timestamp)

        session.close()
    }

    @Test
    fun testPongUpdatesLatency() = runTest {
        val sentTime = System.currentTimeMillis() - 25L
        val pongPacket = ControlPacket.Pong(sentTime)
        val pongBytes = ControlPacket.toJson(pongPacket).toByteArray(Charsets.UTF_8)
        val frameBytes = StreamFraming.encodeFrame(pongBytes)

        val clientInput = ByteArrayInputStream(frameBytes)
        val clientOutput = ByteArrayOutputStream()

        var latencyRecorded: Long? = null
        val session = Stream0Session(
            inputStream = clientInput,
            outputStream = clientOutput,
            scope = this,
            onLatencyUpdated = { latencyRecorded = it }
        )
        session.start()

        testScheduler.advanceUntilIdle()

        assertNotNull(latencyRecorded)
        assertTrue(latencyRecorded!! >= 20L)

        session.close()
    }

    @Test
    fun testSendPacketFramesProperly() = runTest {
        val inStream = ByteArrayInputStream(ByteArray(0))
        val outStream = ByteArrayOutputStream()

        val session = Stream0Session(
            inputStream = inStream,
            outputStream = outStream,
            scope = this,
            onLatencyUpdated = {}
        )

        val status = ControlPacket.DeviceStatus(
            batteryPct = 90,
            isCharging = true,
            wifiSsid = "TestWifi",
            cellularSignal = 3,
            hotspotActive = false
        )
        session.sendPacket(status)

        val writtenBytes = outStream.toByteArray()
        assertTrue(writtenBytes.size > 4)

        val decodedPayload = StreamFraming.readFrame(ByteArrayInputStream(writtenBytes))
        assertNotNull(decodedPayload)
        val jsonStr = String(decodedPayload!!, Charsets.UTF_8)
        val packet = ControlPacket.fromJson(jsonStr)
        assertTrue(packet is ControlPacket.DeviceStatus)
        assertEquals(90, (packet as ControlPacket.DeviceStatus).batteryPct)

        session.close()
    }

    @Test
    fun testConsecutiveFramesProcessedCleanly() = runTest {
        val ping1 = StreamFraming.encodeFrame(ControlPacket.toJson(ControlPacket.Ping(1001L)).toByteArray(Charsets.UTF_8))
        val ping2 = StreamFraming.encodeFrame(ControlPacket.toJson(ControlPacket.Ping(1002L)).toByteArray(Charsets.UTF_8))
        val ping3 = StreamFraming.encodeFrame(ControlPacket.toJson(ControlPacket.Ping(1003L)).toByteArray(Charsets.UTF_8))
        val combined = ping1 + ping2 + ping3

        val inStream = ByteArrayInputStream(combined)
        val outStream = ByteArrayOutputStream()

        val session = Stream0Session(
            inputStream = inStream,
            outputStream = outStream,
            scope = this,
            onLatencyUpdated = {}
        )
        session.start()

        testScheduler.advanceUntilIdle()

        val responseStream = ByteArrayInputStream(outStream.toByteArray())
        val pong1 = StreamFraming.readFrame(responseStream)
        val pong2 = StreamFraming.readFrame(responseStream)
        val pong3 = StreamFraming.readFrame(responseStream)

        assertNotNull(pong1)
        assertNotNull(pong2)
        assertNotNull(pong3)

        assertEquals(1001L, (ControlPacket.fromJson(String(pong1!!, Charsets.UTF_8)) as ControlPacket.Pong).timestamp)
        assertEquals(1002L, (ControlPacket.fromJson(String(pong2!!, Charsets.UTF_8)) as ControlPacket.Pong).timestamp)
        assertEquals(1003L, (ControlPacket.fromJson(String(pong3!!, Charsets.UTF_8)) as ControlPacket.Pong).timestamp)

        session.close()
    }

    @Test
    fun testMalformedJsonPayloadDoesNotCrashReader() = runTest {
        val malformedBytes = "{ invalid json content".toByteArray(Charsets.UTF_8)
        val frameBytes = StreamFraming.encodeFrame(malformedBytes)

        val inStream = ByteArrayInputStream(frameBytes)
        val outStream = ByteArrayOutputStream()

        val session = Stream0Session(
            inputStream = inStream,
            outputStream = outStream,
            scope = this,
            onLatencyUpdated = {}
        )
        session.start()

        testScheduler.advanceUntilIdle()

        session.close()
    }

    @Test
    fun testHeartbeatTimeoutConditionLogic() {
        val now = 10000L
        val lastPingSent = 9500L
        val lastPongReceived = 9600L // Pong received after ping: healthy!
        val shouldTimeoutHealthy = lastPingSent > lastPongReceived && (now - lastPingSent > Stream0Session.HEARTBEAT_TIMEOUT_MS)
        assertFalse(shouldTimeoutHealthy)

        val unacknowledgedPing = 4000L
        val oldPong = 3000L // Pong older than ping, and > 5000ms elapsed: timeout!
        val shouldTimeoutExpired = unacknowledgedPing > oldPong && (now - unacknowledgedPing > Stream0Session.HEARTBEAT_TIMEOUT_MS)
        assertTrue(shouldTimeoutExpired)
    }
}
