package org.omnilink.ui

import android.app.Application
import android.content.Context
import android.os.PowerManager
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import org.omnilink.ServiceLocator
import org.omnilink.model.ConnectionState
import org.omnilink.model.DeviceLinkInfo
import org.omnilink.model.TransportType
import org.omnilink.pairing.IdentityKeyStore
import org.omnilink.pairing.PairingPayload
import org.omnilink.service.OmnilinkSyncService
import org.omnilink.util.BatteryOptimizationHelper
import rikka.shizuku.Shizuku

class MainViewModel(application: Application) : AndroidViewModel(application) {

    private val identityKeyStore = IdentityKeyStore.getInstance(application)

    private val _linkInfo = MutableStateFlow(
        initialLinkInfo()
    )
    val linkInfo: StateFlow<DeviceLinkInfo> = _linkInfo.asStateFlow()

    private fun initialLinkInfo(): DeviceLinkInfo {
        val paired = identityKeyStore.getPairedDevice()
        return if (paired != null) {
            val (host, port) = paired.primaryAddressAndPort() ?: ("192.168.1.100" to 42424)
            DeviceLinkInfo(
                deviceName = paired.name,
                hostAddress = host,
                port = port,
                connectionState = ConnectionState.DISCONNECTED,
                transportType = TransportType.QUIC
            )
        } else {
            DeviceLinkInfo(
                deviceName = "Arch Linux (Hyprland)",
                hostAddress = "192.168.1.150",
                port = 42424,
                connectionState = ConnectionState.DISCONNECTED,
                transportType = TransportType.QUIC
            )
        }
    }

    init {
        refreshSystemStatus()
        observeConnection()
    }

    private fun observeConnection() {
        viewModelScope.launch {
            ServiceLocator.connectionManager.connectionState.collect { state ->
                _linkInfo.update { it.copy(connectionState = state) }
            }
        }

        viewModelScope.launch {
            ServiceLocator.connectionManager.latencyMs.collect { latency ->
                _linkInfo.update { it.copy(latencyMs = latency) }
            }
        }
    }

    fun pairDevice(payload: PairingPayload) {
        identityKeyStore.savePairedDevice(payload)
        val (host, port) = payload.primaryAddressAndPort() ?: ("192.168.1.100" to 42424)
        ServiceLocator.connectionManager.setTarget(host, port)
        _linkInfo.update { current ->
            current.copy(
                deviceName = payload.name,
                hostAddress = host,
                port = port,
                connectionState = ServiceLocator.connectionManager.connectionState.value
            )
        }
    }

    fun unpairDevice() {
        disconnect()
        identityKeyStore.clearPairedDevice()
        _linkInfo.update { current ->
            current.copy(
                deviceName = "No Paired Desktop",
                hostAddress = "--",
                port = 0,
                connectionState = ConnectionState.DISCONNECTED,
                latencyMs = null
            )
        }
    }

    fun refreshSystemStatus() {
        val context = getApplication<Application>()
        val isBatteryExempt = BatteryOptimizationHelper.isIgnoringBatteryOptimizations(context)

        val isShizukuReady = try {
            Shizuku.pingBinder()
        } catch (_: Throwable) {
            false
        }

        _linkInfo.update { current ->
            current.copy(
                batteryOptimizationExempt = isBatteryExempt,
                shizukuAvailable = isShizukuReady
            )
        }
    }

    fun connect() {
        val context = getApplication<Application>()
        OmnilinkSyncService.start(context)
        ServiceLocator.connectionManager.connect()
    }

    fun disconnect() {
        val context = getApplication<Application>()
        ServiceLocator.connectionManager.disconnect()
        OmnilinkSyncService.stop(context)
    }
}
