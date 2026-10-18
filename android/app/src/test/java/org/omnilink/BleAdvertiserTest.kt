package org.omnilink

import android.bluetooth.le.AdvertiseCallback
import android.bluetooth.le.AdvertiseSettings
import android.content.BroadcastReceiver
import android.content.Context
import android.content.ContextWrapper
import android.content.Intent
import android.content.IntentFilter
import android.net.ConnectivityManager
import android.net.wifi.WifiManager
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import org.omnilink.ble.BleAdvertiseCallback
import org.omnilink.ble.BleAdvertisePayload
import org.omnilink.ble.BleAdvertiserDriver
import org.omnilink.ble.BleAdvertisingState
import org.omnilink.ble.BleProximityAdvertiser
import org.omnilink.ble.BleProximityTriggerReceiver
import org.omnilink.ble.BleTriggerEvent
import java.util.UUID

@OptIn(ExperimentalCoroutinesApi::class)
class BleAdvertiserTest {

    private class StubBleAdvertiserDriver : BleAdvertiserDriver {
        var isAdvertising = false
        var lastPayload: BleAdvertisePayload? = null
        var lastMode: Int? = null
        var simulateErrorCode: Int? = null
        var startCount = 0
        var stopCount = 0

        override fun startAdvertising(
            payload: BleAdvertisePayload,
            advertiseMode: Int,
            callback: BleAdvertiseCallback
        ) {
            startCount++
            lastPayload = payload
            lastMode = advertiseMode
            val err = simulateErrorCode
            if (err != null) {
                isAdvertising = false
                callback.onFailure(err, "Simulation error: $err")
            } else {
                isAdvertising = true
                callback.onSuccess()
            }
        }

        override fun stopAdvertising(callback: BleAdvertiseCallback) {
            stopCount++
            isAdvertising = false
            callback.onStopped()
        }
    }

    private class FakeContext : ContextWrapper(null) {
        var registeredReceiver: BroadcastReceiver? = null
        var unregisteredReceiver: BroadcastReceiver? = null

        override fun registerReceiver(receiver: BroadcastReceiver?, filter: IntentFilter?): Intent? {
            registeredReceiver = receiver
            return null
        }

        override fun registerReceiver(
            receiver: BroadcastReceiver?,
            filter: IntentFilter?,
            flags: Int
        ): Intent? {
            registeredReceiver = receiver
            return null
        }

        override fun registerReceiver(
            receiver: BroadcastReceiver?,
            filter: IntentFilter?,
            broadcastPermission: String?,
            scheduler: android.os.Handler?
        ): Intent? {
            registeredReceiver = receiver
            return null
        }

        override fun registerReceiver(
            receiver: BroadcastReceiver?,
            filter: IntentFilter?,
            broadcastPermission: String?,
            scheduler: android.os.Handler?,
            flags: Int
        ): Intent? {
            registeredReceiver = receiver
            return null
        }

        override fun unregisterReceiver(receiver: BroadcastReceiver?) {
            unregisteredReceiver = receiver
        }
    }

    @Test
    fun testServiceUuidMatchesOmnilinkSpecification() {
        val expectedUuidString = "6f6d6e69-6c69-6e6b-0001-000000000000"
        assertEquals(expectedUuidString, BleProximityAdvertiser.OMNILINK_SERVICE_UUID_STRING)

        val expectedUuid = UUID.fromString(expectedUuidString)
        assertEquals(expectedUuid, BleProximityAdvertiser.SERVICE_UUID)
    }

    @Test
    fun testPayloadCreationValidToken() {
        val token = "ephemeral_secret_token_12345"
        val payload = BleProximityAdvertiser.createPayload(token)

        assertEquals(BleProximityAdvertiser.SERVICE_UUID, payload.serviceUuid)
        assertEquals(token, payload.token)
        assertEquals(8, payload.tokenBytes.size)
        assertArrayEquals(token.toByteArray(Charsets.UTF_8).take(8).toByteArray(), payload.tokenBytes)
    }

    @Test
    fun testPayloadByteSizeWithin31ByteMtu() {
        val token = "0123456789abcdef0123456789abcdef"
        val payload = BleProximityAdvertiser.createPayload(token)
        // Flags: 3 bytes; 128-bit Service Data: 1 len + 1 type + 16 UUID + tokenBytes
        val serviceDataSize = 1 + 1 + 16 + payload.tokenBytes.size
        val flagsSize = 3
        val totalSize = flagsSize + serviceDataSize
        assertTrue("Advertisement packet size ($totalSize) must be <= 31 bytes MTU", totalSize <= 31)
        assertEquals(8, payload.tokenBytes.size)
    }

    @Test
    fun testPayloadCreationBlankTokenFails() {
        try {
            BleProximityAdvertiser.createPayload("")
            fail("Expected IllegalArgumentException for empty token")
        } catch (_: IllegalArgumentException) {}

        try {
            BleProximityAdvertiser.createPayload("   ")
            fail("Expected IllegalArgumentException for whitespace token")
        } catch (_: IllegalArgumentException) {}
    }

    @Test
    fun testPayloadEqualityAndHashCode() {
        val token = "token-abc"
        val payload1 = BleProximityAdvertiser.createPayload(token)
        val payload2 = BleProximityAdvertiser.createPayload(token)
        val payload3 = BleProximityAdvertiser.createPayload("token-other")

        assertEquals(payload1, payload2)
        assertEquals(payload1.hashCode(), payload2.hashCode())
        assertFalse(payload1 == payload3)
    }

    @Test
    fun testStartAdvertisingSuccess() {
        val fakeContext = FakeContext()
        val driver = StubBleAdvertiserDriver()
        var listenerInvoked = false
        var listenerIsAdvertising: Boolean? = null

        val advertiser = BleProximityAdvertiser(
            context = fakeContext,
            driver = driver,
            permissionChecker = { true },
            bluetoothChecker = { true }
        )
        advertiser.onStateChangedListener = { isAdv, _ ->
            listenerInvoked = true
            listenerIsAdvertising = isAdv
        }

        assertFalse(advertiser.isAdvertising())
        assertEquals(BleAdvertisingState.Idle, advertiser.state.value)

        val token = "paired-token-xyz"
        val result = advertiser.startAdvertising(token)

        assertTrue(result)
        assertTrue(advertiser.isAdvertising())
        assertTrue(driver.isAdvertising)
        assertEquals(1, driver.startCount)
        assertEquals(AdvertiseSettings.ADVERTISE_MODE_LOW_POWER, driver.lastMode)
        assertEquals(token, driver.lastPayload?.token)
        assertTrue(advertiser.state.value is BleAdvertisingState.Advertising)
        assertTrue(listenerInvoked)
        assertEquals(true, listenerIsAdvertising)
    }

    @Test
    fun testStopAdvertisingSuccess() {
        val fakeContext = FakeContext()
        val driver = StubBleAdvertiserDriver()
        val advertiser = BleProximityAdvertiser(
            context = fakeContext,
            driver = driver,
            permissionChecker = { true },
            bluetoothChecker = { true }
        )

        advertiser.startAdvertising("test-token")
        assertTrue(advertiser.isAdvertising())

        advertiser.stopAdvertising()
        assertFalse(advertiser.isAdvertising())
        assertFalse(driver.isAdvertising)
        assertEquals(1, driver.stopCount)
        assertEquals(BleAdvertisingState.Idle, advertiser.state.value)

        // Stopping again should be safe and idempotent
        advertiser.stopAdvertising()
        assertEquals(1, driver.stopCount)
    }

    @Test
    fun testStartAdvertisingIdempotentWithSameParameters() {
        val fakeContext = FakeContext()
        val driver = StubBleAdvertiserDriver()
        val advertiser = BleProximityAdvertiser(
            context = fakeContext,
            driver = driver,
            permissionChecker = { true },
            bluetoothChecker = { true }
        )

        val token = "idempotent-token"
        assertTrue(advertiser.startAdvertising(token))
        assertEquals(1, driver.startCount)

        // Second call with same token and mode does not restart
        assertTrue(advertiser.startAdvertising(token))
        assertEquals(1, driver.startCount)
    }

    @Test
    fun testStartAdvertisingFailsWhenBluetoothDisabled() {
        val fakeContext = FakeContext()
        val driver = StubBleAdvertiserDriver()
        val advertiser = BleProximityAdvertiser(
            context = fakeContext,
            driver = driver,
            permissionChecker = { true },
            bluetoothChecker = { false } // Bluetooth disabled
        )

        val result = advertiser.startAdvertising("my-token")
        assertFalse(result)
        assertFalse(advertiser.isAdvertising())
        assertEquals(0, driver.startCount)
        assertTrue(advertiser.state.value is BleAdvertisingState.Error)
    }

    @Test
    fun testStartAdvertisingFailsWhenPermissionMissing() {
        val fakeContext = FakeContext()
        val driver = StubBleAdvertiserDriver()
        val advertiser = BleProximityAdvertiser(
            context = fakeContext,
            driver = driver,
            permissionChecker = { false }, // Missing permission
            bluetoothChecker = { true }
        )

        val result = advertiser.startAdvertising("my-token")
        assertFalse(result)
        assertFalse(advertiser.isAdvertising())
        assertEquals(0, driver.startCount)
        assertTrue(advertiser.state.value is BleAdvertisingState.Error)
    }

    @Test
    fun testStartAdvertisingFailsWhenTokenBlank() {
        val fakeContext = FakeContext()
        val driver = StubBleAdvertiserDriver()
        val advertiser = BleProximityAdvertiser(
            context = fakeContext,
            driver = driver,
            permissionChecker = { true },
            bluetoothChecker = { true }
        )

        val result = advertiser.startAdvertising("")
        assertFalse(result)
        assertFalse(advertiser.isAdvertising())
        assertEquals(0, driver.startCount)
        assertTrue(advertiser.state.value is BleAdvertisingState.Error)
    }

    @Test
    fun testDriverFailureHandledGracefully() {
        val fakeContext = FakeContext()
        val driver = StubBleAdvertiserDriver()
        driver.simulateErrorCode = AdvertiseCallback.ADVERTISE_FAILED_FEATURE_UNSUPPORTED

        val advertiser = BleProximityAdvertiser(
            context = fakeContext,
            driver = driver,
            permissionChecker = { true },
            bluetoothChecker = { true }
        )

        val result = advertiser.startAdvertising("my-token")
        assertTrue(result) // driver invoked
        assertFalse(advertiser.isAdvertising()) // failed on callback
        assertTrue(advertiser.state.value is BleAdvertisingState.Error)
        val errorState = advertiser.state.value as BleAdvertisingState.Error
        assertEquals(AdvertiseCallback.ADVERTISE_FAILED_FEATURE_UNSUPPORTED, errorState.errorCode)
    }

    @Test
    fun testBurstAdvertisingTransitionsToLowPower() = runTest {
        val fakeContext = FakeContext()
        val driver = StubBleAdvertiserDriver()
        val advertiser = BleProximityAdvertiser(
            context = fakeContext,
            driver = driver,
            permissionChecker = { true },
            bluetoothChecker = { true },
            scope = this
        )

        val token = "burst-token"
        val burstDuration = 5_000L

        assertTrue(advertiser.startBurstAdvertising(token, burstDurationMs = burstDuration))
        assertTrue(advertiser.isAdvertising())
        assertEquals(AdvertiseSettings.ADVERTISE_MODE_LOW_LATENCY, driver.lastMode)

        val initialAdv = advertiser.state.value as BleAdvertisingState.Advertising
        assertTrue(initialAdv.isBurst)

        // Advance time to expire burst
        advanceTimeBy(burstDuration + 100L)

        // Now driver should have transitioned to LOW_POWER
        assertEquals(AdvertiseSettings.ADVERTISE_MODE_LOW_POWER, driver.lastMode)
        val transitionedAdv = advertiser.state.value as BleAdvertisingState.Advertising
        assertFalse(transitionedAdv.isBurst)
    }

    @Test
    fun testStopAdvertisingDuringBurstCancelsTransition() = runTest {
        val fakeContext = FakeContext()
        val driver = StubBleAdvertiserDriver()
        val advertiser = BleProximityAdvertiser(
            context = fakeContext,
            driver = driver,
            permissionChecker = { true },
            bluetoothChecker = { true },
            scope = this
        )

        advertiser.startBurstAdvertising("burst-token", burstDurationMs = 5_000L)
        assertTrue(advertiser.isAdvertising())

        advertiser.stopAdvertising()
        assertFalse(advertiser.isAdvertising())
        val stopsBeforeAdvance = driver.stopCount

        advanceTimeBy(10_000L)

        // Should still be stopped and never transitioned
        assertFalse(advertiser.isAdvertising())
        assertEquals(stopsBeforeAdvance, driver.stopCount)
    }

    @Test
    fun testTriggerReceiverActionMatching() {
        assertEquals(
            BleTriggerEvent.SCREEN_ON,
            BleProximityTriggerReceiver.handleAction(Intent.ACTION_SCREEN_ON)
        )
        assertEquals(
            BleTriggerEvent.WIFI_CHANGED,
            BleProximityTriggerReceiver.handleAction(WifiManager.NETWORK_STATE_CHANGED_ACTION)
        )
        @Suppress("DEPRECATION")
        assertEquals(
            BleTriggerEvent.WIFI_CHANGED,
            BleProximityTriggerReceiver.handleAction(ConnectivityManager.CONNECTIVITY_ACTION)
        )
        assertNull(BleProximityTriggerReceiver.handleAction("com.unrelated.action"))
        assertNull(BleProximityTriggerReceiver.handleAction(null))
    }

    @Test
    fun testRegisterAndUnregisterTriggers() {
        val fakeContext = FakeContext()
        val driver = StubBleAdvertiserDriver()
        val advertiser = BleProximityAdvertiser(
            context = fakeContext,
            driver = driver,
            permissionChecker = { true },
            bluetoothChecker = { true }
        )

        advertiser.registerTriggers(fakeContext) { "test-token" }
        assertNotNull(fakeContext.registeredReceiver)

        advertiser.unregisterTriggers(fakeContext)
        assertEquals(fakeContext.registeredReceiver, fakeContext.unregisteredReceiver)
    }

    @Test
    fun testErrorMessageForCodes() {
        assertEquals(
            "Advertising already started",
            BleProximityAdvertiser.errorMessageForCode(AdvertiseCallback.ADVERTISE_FAILED_ALREADY_STARTED)
        )
        assertEquals(
            "Advertise data too large (>31 bytes)",
            BleProximityAdvertiser.errorMessageForCode(AdvertiseCallback.ADVERTISE_FAILED_DATA_TOO_LARGE)
        )
        assertEquals(
            "Internal BLE advertising error",
            BleProximityAdvertiser.errorMessageForCode(AdvertiseCallback.ADVERTISE_FAILED_INTERNAL_ERROR)
        )
        assertEquals(
            "BLE advertising feature unsupported",
            BleProximityAdvertiser.errorMessageForCode(AdvertiseCallback.ADVERTISE_FAILED_FEATURE_UNSUPPORTED)
        )
        assertEquals(
            "Too many advertisers running",
            BleProximityAdvertiser.errorMessageForCode(AdvertiseCallback.ADVERTISE_FAILED_TOO_MANY_ADVERTISERS)
        )
        assertTrue(BleProximityAdvertiser.errorMessageForCode(99).contains("99"))
    }
}
