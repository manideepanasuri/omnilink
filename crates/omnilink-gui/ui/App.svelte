<script>
  import { onMount, onDestroy } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';

  let activeTab = $state('overview');
  let isDaemonOnline = $state(false);
  let statusLoading = $state(true);
  let actionMessage = $state('');
  let messageTimeout = null;

  // Reactive device state
  let device = $state({
    connected: false,
    device_name: null,
    battery_pct: null,
    is_charging: false,
    wifi_ssid: null,
    cellular_signal: null,
    hotspot_active: false,
  });

  let dndActive = $state(false);
  let isRinging = $state(false);
  let isTogglingHotspot = $state(false);

  let unlistenStatus = null;
  let unlistenClipboard = null;

  function showMessage(msg) {
    actionMessage = msg;
    if (messageTimeout) clearTimeout(messageTimeout);
    messageTimeout = setTimeout(() => {
      actionMessage = '';
    }, 3500);
  }

  function updateDeviceState(data) {
    if (!data) return;
    if (data.connected !== undefined) device.connected = Boolean(data.connected);
    if (data.device_name !== undefined) device.device_name = data.device_name;
    if (data.battery_pct !== undefined) device.battery_pct = data.battery_pct;
    if (data.is_charging !== undefined) device.is_charging = Boolean(data.is_charging);
    if (data.wifi_ssid !== undefined) device.wifi_ssid = data.wifi_ssid;
    if (data.cellular_signal !== undefined) device.cellular_signal = data.cellular_signal;
    if (data.hotspot_active !== undefined) device.hotspot_active = Boolean(data.hotspot_active);
  }

  async function fetchStatus() {
    statusLoading = true;
    try {
      const res = await invoke('get_status');
      isDaemonOnline = true;
      updateDeviceState(res);
    } catch (err) {
      console.warn('Failed to fetch status:', err);
      // Daemon may not be running yet
      device.connected = false;
    } finally {
      statusLoading = false;
    }
  }

  async function handleToggleHotspot() {
    if (isTogglingHotspot) return;
    isTogglingHotspot = true;
    const targetState = !device.hotspot_active;
    try {
      await invoke('toggle_hotspot', { enable: targetState });
      device.hotspot_active = targetState;
      showMessage(targetState ? 'Mobile Hotspot turned ON' : 'Mobile Hotspot turned OFF');
    } catch (err) {
      console.error('Failed to toggle hotspot:', err);
      showMessage(`Hotspot toggle failed: ${err}`);
    } finally {
      isTogglingHotspot = false;
    }
  }

  async function handleRingPhone() {
    if (isRinging) return;
    isRinging = true;
    showMessage('Ringing connected phone at max volume...');
    try {
      await invoke('ring_phone');
    } catch (err) {
      console.error('Failed to ring phone:', err);
      showMessage(`Ring failed: ${err}`);
    } finally {
      setTimeout(() => {
        isRinging = false;
      }, 4000);
    }
  }

  function handleToggleDnd() {
    dndActive = !dndActive;
    showMessage(dndActive ? 'Do Not Disturb enabled' : 'Do Not Disturb disabled');
  }

  onMount(async () => {
    await fetchStatus();

    try {
      unlistenStatus = await listen('device.status_changed', (event) => {
        isDaemonOnline = true;
        updateDeviceState(event.payload);
      });
    } catch (err) {
      console.warn('Event listen status_changed error:', err);
    }

    try {
      unlistenClipboard = await listen('clipboard.updated', (event) => {
        showMessage('Clipboard synced from companion');
      });
    } catch (err) {
      console.warn('Event listen clipboard.updated error:', err);
    }
  });

  onDestroy(() => {
    if (unlistenStatus) unlistenStatus();
    if (unlistenClipboard) unlistenClipboard();
    if (messageTimeout) clearTimeout(messageTimeout);
  });
</script>

<div class="flex h-screen w-screen bg-[#0d1117] text-[#e6edf3] font-sans overflow-hidden">
  <!-- SIDEBAR NAVIGATION -->
  <aside class="w-64 flex-shrink-0 bg-[#0f141c]/95 border-r border-[#30363d]/60 flex flex-col justify-between select-none">
    <div class="p-6">
      <!-- Omnilink Logo & Brand -->
      <div class="flex items-center space-x-3 mb-8">
        <div class="w-10 h-10 rounded-xl bg-gradient-to-tr from-cyan-500 to-emerald-400 p-[1.5px] shadow-lg shadow-cyan-500/20">
          <div class="w-full h-full bg-[#0d1117] rounded-[10px] flex items-center justify-center">
            <svg class="w-5 h-5 text-cyan-400" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M5 12.55a11 11 0 0 1 14.08 0"></path>
              <path d="M1.42 9a16 16 0 0 1 21.16 0"></path>
              <path d="M8.53 16.11a6 6 0 0 1 6.95 0"></path>
              <line x1="12" y1="20" x2="12.01" y2="20"></line>
            </svg>
          </div>
        </div>
        <div>
          <h1 class="text-base font-bold tracking-tight text-white flex items-center gap-1.5">
            Omnilink
            <span class="text-[10px] font-semibold uppercase px-1.5 py-0.5 rounded bg-cyan-950/70 border border-cyan-800/40 text-cyan-400">Wayland</span>
          </h1>
          <p class="text-xs text-[#8b949e]">Linux-Android Bridge</p>
        </div>
      </div>

      <!-- Navigation Links -->
      <nav class="space-y-1.5">
        <!-- Overview Tab -->
        <button
          onclick={() => (activeTab = 'overview')}
          class="w-full flex items-center gap-3.5 px-3.5 py-2.5 rounded-xl text-sm font-medium transition-all duration-150 {activeTab === 'overview'
            ? 'bg-[#161b22] text-white border border-[#30363d] shadow-sm shadow-cyan-500/5'
            : 'text-[#8b949e] hover:text-[#e6edf3] hover:bg-[#161b22]/50'}"
        >
          <div class="relative">
            <svg class="w-4 h-4 {activeTab === 'overview' ? 'text-cyan-400' : 'text-[#8b949e]'}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <rect width="7" height="9" x="3" y="3" rx="1"></rect>
              <rect width="7" height="5" x="14" y="3" rx="1"></rect>
              <rect width="7" height="9" x="14" y="12" rx="1"></rect>
              <rect width="7" height="5" x="3" y="16" rx="1"></rect>
            </svg>
            {#if activeTab === 'overview'}
              <span class="absolute -left-3.5 top-1/2 -translate-y-1/2 w-1 h-4 bg-cyan-400 rounded-r-full shadow-sm shadow-cyan-400"></span>
            {/if}
          </div>
          <span>Overview</span>
        </button>

        <!-- Photos Tab -->
        <button
          onclick={() => (activeTab = 'photos')}
          class="w-full flex items-center gap-3.5 px-3.5 py-2.5 rounded-xl text-sm font-medium transition-all duration-150 {activeTab === 'photos'
            ? 'bg-[#161b22] text-white border border-[#30363d]'
            : 'text-[#8b949e] hover:text-[#e6edf3] hover:bg-[#161b22]/50'}"
        >
          <svg class="w-4 h-4 {activeTab === 'photos' ? 'text-cyan-400' : 'text-[#8b949e]'}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect width="18" height="18" x="3" y="3" rx="2" ry="2"></rect>
            <circle cx="9" cy="9" r="2"></circle>
            <path d="m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21"></path>
          </svg>
          <span>Photos</span>
        </button>

        <!-- Messages Tab -->
        <button
          onclick={() => (activeTab = 'messages')}
          class="w-full flex items-center gap-3.5 px-3.5 py-2.5 rounded-xl text-sm font-medium transition-all duration-150 {activeTab === 'messages'
            ? 'bg-[#161b22] text-white border border-[#30363d]'
            : 'text-[#8b949e] hover:text-[#e6edf3] hover:bg-[#161b22]/50'}"
        >
          <svg class="w-4 h-4 {activeTab === 'messages' ? 'text-cyan-400' : 'text-[#8b949e]'}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z"></path>
          </svg>
          <span>Messages</span>
        </button>

        <!-- App Drawer Tab -->
        <button
          onclick={() => (activeTab = 'apps')}
          class="w-full flex items-center gap-3.5 px-3.5 py-2.5 rounded-xl text-sm font-medium transition-all duration-150 {activeTab === 'apps'
            ? 'bg-[#161b22] text-white border border-[#30363d]'
            : 'text-[#8b949e] hover:text-[#e6edf3] hover:bg-[#161b22]/50'}"
        >
          <svg class="w-4 h-4 {activeTab === 'apps' ? 'text-cyan-400' : 'text-[#8b949e]'}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect width="7" height="7" x="3" y="3" rx="1"></rect>
            <rect width="7" height="7" x="14" y="3" rx="1"></rect>
            <rect width="7" height="7" x="14" y="14" rx="1"></rect>
            <rect width="7" height="7" x="3" y="14" rx="1"></rect>
          </svg>
          <span>App Drawer</span>
        </button>
      </nav>
    </div>

    <!-- Daemon status footer pill -->
    <div class="p-4 border-t border-[#30363d]/40">
      <div class="flex items-center justify-between text-xs px-3 py-2 rounded-lg bg-[#161b22]/60 border border-[#30363d]/40">
        <div class="flex items-center gap-2">
          <span class="relative flex h-2 w-2">
            {#if device.connected}
              <span class="animate-ping absolute inline-flex h-full w-full rounded-full bg-emerald-400 opacity-75"></span>
              <span class="relative inline-flex rounded-full h-2 w-2 bg-emerald-500"></span>
            {:else}
              <span class="relative inline-flex rounded-full h-2 w-2 bg-zinc-600"></span>
            {/if}
          </span>
          <span class="text-[#8b949e]">Daemon IPC</span>
        </div>
        <span class="font-mono text-[11px] {device.connected ? 'text-emerald-400' : 'text-zinc-500'}">
          {device.connected ? 'ACTIVE' : 'IDLE'}
        </span>
      </div>
    </div>
  </aside>

  <!-- MAIN CONTENT AREA -->
  <main class="flex-1 flex flex-col overflow-y-auto bg-[#0d1117] p-8">
    <!-- Action Toast notification -->
    {#if actionMessage}
      <div class="fixed top-5 right-5 z-50 flex items-center gap-3 px-4 py-2.5 rounded-xl bg-[#161b22] border border-cyan-500/40 text-cyan-200 text-xs shadow-xl shadow-cyan-950/40 animate-fade-in">
        <svg class="w-4 h-4 text-cyan-400 animate-spin" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <circle cx="12" cy="12" r="10" stroke-opacity="0.25"></circle>
          <path d="M12 2a10 10 0 0 1 10 10" stroke-linecap="round"></path>
        </svg>
        <span>{actionMessage}</span>
      </div>
    {/if}

    {#if activeTab === 'overview'}
      <!-- TAB: OVERVIEW DASHBOARD -->
      <div class="max-w-4xl w-full mx-auto space-y-6">
        <!-- DEVICE HEADER -->
        <div class="relative overflow-hidden rounded-2xl bg-gradient-to-br from-[#161b22] to-[#0f141c] border border-[#30363d]/80 p-7 shadow-lg">
          <div class="absolute -right-10 -bottom-10 w-60 h-60 bg-cyan-500/5 rounded-full blur-3xl pointer-events-none"></div>

          <div class="flex items-center justify-between">
            <div class="flex items-center gap-5">
              <!-- Smartphone Device Icon Badge -->
              <div class="w-16 h-16 rounded-2xl bg-[#0d1117] border border-[#30363d] flex items-center justify-center shadow-inner">
                <svg class="w-8 h-8 {device.connected ? 'text-cyan-400' : 'text-zinc-600'}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                  <rect width="14" height="20" x="5" y="2" rx="2" ry="2"></rect>
                  <path d="M12 18h.01"></path>
                </svg>
              </div>

              <div>
                <div class="flex items-center gap-3">
                  <h2 class="text-2xl font-bold tracking-tight text-white">
                    {device.device_name || (device.connected ? 'Galaxy S24 Ultra' : 'No Device Paired')}
                  </h2>
                  <!-- Status Badge -->
                  {#if device.connected}
                    <span class="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium bg-emerald-950/60 border border-emerald-500/40 text-emerald-400 shadow-sm shadow-emerald-500/10">
                      <span class="h-1.5 w-1.5 rounded-full bg-emerald-400 animate-pulse"></span>
                      Connected
                    </span>
                  {:else}
                    <span class="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium bg-zinc-800/80 border border-zinc-700/60 text-zinc-400">
                      <span class="h-1.5 w-1.5 rounded-full bg-zinc-500"></span>
                      Disconnected
                    </span>
                  {/if}
                </div>
                <p class="text-sm text-[#8b949e] mt-1">
                  {#if device.connected}
                    Synchronized via secure QUIC Stream multiplexing
                  {:else}
                    Companion offline. Ensure phone is on same LAN or nearby.
                  {/if}
                </p>
              </div>
            </div>

            <!-- Refresh button -->
            <button
              onclick={fetchStatus}
              title="Refresh status"
              class="p-2.5 rounded-xl bg-[#0d1117] border border-[#30363d] text-[#8b949e] hover:text-white hover:border-[#484f58] transition-all"
            >
              <svg class="w-4 h-4 {statusLoading ? 'animate-spin text-cyan-400' : ''}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8"></path>
                <path d="M21 3v5h-5"></path>
              </svg>
            </button>
          </div>
        </div>

        <!-- TELEMETRY CARDS (3-column grid) -->
        <div class="grid grid-cols-1 md:grid-cols-3 gap-5">
          <!-- CARD 1: BATTERY -->
          <div class="rounded-xl bg-[#161b22] border border-[#30363d]/70 p-5 flex flex-col justify-between hover:border-[#484f58] transition-colors">
            <div class="flex items-center justify-between mb-4">
              <span class="text-xs font-semibold uppercase tracking-wider text-[#8b949e]">Battery</span>
              <div class="p-2 rounded-lg bg-[#0d1117] border border-[#30363d]/60 text-cyan-400">
                {#if device.is_charging}
                  <!-- Charging Bolt Icon -->
                  <svg class="w-4 h-4 text-emerald-400 animate-pulse" viewBox="0 0 24 24" fill="currentColor">
                    <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"></polygon>
                  </svg>
                {:else}
                  <!-- Battery Icon -->
                  <svg class="w-4 h-4 text-cyan-400" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <rect width="16" height="10" x="2" y="7" rx="2" ry="2"></rect>
                    <line x1="22" y1="11" x2="22" y2="13"></line>
                  </svg>
                {/if}
              </div>
            </div>

            <div>
              <div class="flex items-baseline gap-2">
                <span class="text-3xl font-extrabold text-white">
                  {device.battery_pct !== null && device.connected ? `${device.battery_pct}%` : '--%'}
                </span>
                {#if device.is_charging && device.connected}
                  <span class="text-xs font-medium text-emerald-400 flex items-center gap-1">
                    <span class="w-1.5 h-1.5 rounded-full bg-emerald-400"></span>
                    Charging
                  </span>
                {/if}
              </div>

              <!-- Battery Level Bar -->
              <div class="w-full bg-[#0d1117] rounded-full h-2 mt-3 overflow-hidden border border-[#30363d]/40">
                <div
                  class="h-full rounded-full transition-all duration-500 {device.is_charging
                    ? 'bg-gradient-to-r from-emerald-500 to-teal-400'
                    : (device.battery_pct || 0) < 20
                    ? 'bg-red-500'
                    : 'bg-gradient-to-r from-cyan-500 to-emerald-400'}"
                  style="width: {device.connected && device.battery_pct !== null ? Math.min(100, Math.max(0, device.battery_pct)) : 0}%"
                ></div>
              </div>
            </div>

            <p class="text-[11px] text-[#8b949e] mt-3">
              {#if device.connected}
                {device.is_charging ? 'Fast charging connected' : 'Discharging normally'}
              {:else}
                Telemetry offline
              {/if}
            </p>
          </div>

          <!-- CARD 2: WI-FI -->
          <div class="rounded-xl bg-[#161b22] border border-[#30363d]/70 p-5 flex flex-col justify-between hover:border-[#484f58] transition-colors">
            <div class="flex items-center justify-between mb-4">
              <span class="text-xs font-semibold uppercase tracking-wider text-[#8b949e]">Wi-Fi Network</span>
              <div class="p-2 rounded-lg bg-[#0d1117] border border-[#30363d]/60 text-cyan-400">
                <svg class="w-4 h-4 text-cyan-400" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M5 12.55a11 11 0 0 1 14.08 0"></path>
                  <path d="M1.42 9a16 16 0 0 1 21.16 0"></path>
                  <path d="M8.53 16.11a6 6 0 0 1 6.95 0"></path>
                  <line x1="12" y1="20" x2="12.01" y2="20"></line>
                </svg>
              </div>
            </div>

            <div>
              <span class="text-xl font-bold text-white block truncate" title={device.wifi_ssid || 'Disconnected'}>
                {device.connected && device.wifi_ssid ? device.wifi_ssid : (device.connected ? 'No Wi-Fi' : '--')}
              </span>
              <p class="text-xs text-emerald-400 mt-1 flex items-center gap-1.5">
                {#if device.connected && device.wifi_ssid}
                  <span class="w-1.5 h-1.5 rounded-full bg-emerald-400"></span>
                  Active WLAN link
                {:else}
                  <span class="text-[#8b949e]">No wireless connection</span>
                {/if}
              </p>
            </div>

            <p class="text-[11px] text-[#8b949e] mt-3">Local subnet transport</p>
          </div>

          <!-- CARD 3: CELLULAR SIGNAL -->
          <div class="rounded-xl bg-[#161b22] border border-[#30363d]/70 p-5 flex flex-col justify-between hover:border-[#484f58] transition-colors">
            <div class="flex items-center justify-between mb-4">
              <span class="text-xs font-semibold uppercase tracking-wider text-[#8b949e]">Cellular Signal</span>
              <div class="p-2 rounded-lg bg-[#0d1117] border border-[#30363d]/60 text-cyan-400">
                <svg class="w-4 h-4 text-cyan-400" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M2 20h.01"></path>
                  <path d="M7 20v-4"></path>
                  <path d="M12 20v-8"></path>
                  <path d="M17 20V4"></path>
                </svg>
              </div>
            </div>

            <div>
              <div class="flex items-center justify-between">
                <span class="text-2xl font-extrabold text-white">
                  {device.connected && device.cellular_signal !== null ? `${device.cellular_signal} / 4` : '--'}
                </span>
                <!-- 4 Visual Bars -->
                <div class="flex items-end gap-1.5 h-7">
                  <!-- Bar 1 (25% height) -->
                  <div
                    class="w-2 rounded-sm transition-all duration-300 {device.connected && (device.cellular_signal || 0) >= 1
                      ? 'bg-cyan-400 shadow-sm shadow-cyan-400/50'
                      : 'bg-zinc-800'}"
                    style="height: 25%;"
                  ></div>
                  <!-- Bar 2 (50% height) -->
                  <div
                    class="w-2 rounded-sm transition-all duration-300 {device.connected && (device.cellular_signal || 0) >= 2
                      ? 'bg-cyan-400 shadow-sm shadow-cyan-400/50'
                      : 'bg-zinc-800'}"
                    style="height: 50%;"
                  ></div>
                  <!-- Bar 3 (75% height) -->
                  <div
                    class="w-2 rounded-sm transition-all duration-300 {device.connected && (device.cellular_signal || 0) >= 3
                      ? 'bg-cyan-400 shadow-sm shadow-cyan-400/50'
                      : 'bg-zinc-800'}"
                    style="height: 75%;"
                  ></div>
                  <!-- Bar 4 (100% height) -->
                  <div
                    class="w-2 rounded-sm transition-all duration-300 {device.connected && (device.cellular_signal || 0) >= 4
                      ? 'bg-cyan-400 shadow-sm shadow-cyan-400/50'
                      : 'bg-zinc-800'}"
                    style="height: 100%;"
                  ></div>
                </div>
              </div>
            </div>

            <p class="text-[11px] text-[#8b949e] mt-3">
              {device.connected && device.cellular_signal !== null ? '5G Carrier Standby' : 'SIM disconnected'}
            </p>
          </div>
        </div>

        <!-- QUICK ACTION TOGGLES -->
        <div class="rounded-2xl bg-[#161b22] border border-[#30363d]/80 p-6 shadow-sm">
          <h3 class="text-sm font-semibold uppercase tracking-wider text-[#8b949e] mb-4">Quick Actions</h3>

          <div class="grid grid-cols-1 md:grid-cols-3 gap-4">
            <!-- TOGGLE 1: MOBILE HOTSPOT -->
            <button
              onclick={handleToggleHotspot}
              disabled={isTogglingHotspot || !device.connected}
              class="flex items-center justify-between p-4 rounded-xl border transition-all duration-200 text-left {device.hotspot_active
                ? 'bg-cyan-950/40 border-cyan-500/60 shadow-md shadow-cyan-500/10'
                : 'bg-[#0d1117] border-[#30363d] hover:border-[#484f58]'} {(!device.connected || isTogglingHotspot) ? 'opacity-60 cursor-not-allowed' : 'cursor-pointer'}"
            >
              <div class="flex items-center gap-3.5">
                <div class="p-2.5 rounded-lg {device.hotspot_active ? 'bg-cyan-500/20 text-cyan-300' : 'bg-[#161b22] text-[#8b949e]'}">
                  <svg class="w-5 h-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M4 11a9 9 0 0 1 9 9"></path>
                    <path d="M4 4a16 16 0 0 1 16 16"></path>
                    <circle cx="5" cy="19" r="1"></circle>
                  </svg>
                </div>
                <div>
                  <span class="text-sm font-semibold text-white block">Mobile Hotspot</span>
                  <span class="text-xs text-[#8b949e]">
                    {device.hotspot_active ? 'Enabled' : 'Disabled'}
                  </span>
                </div>
              </div>

              <!-- Switch Pill -->
              <div class="w-11 h-6 rounded-full transition-colors p-1 flex items-center {device.hotspot_active ? 'bg-cyan-500' : 'bg-zinc-800'}">
                <div class="w-4 h-4 rounded-full bg-white transition-transform duration-200 shadow-md {device.hotspot_active ? 'translate-x-5' : 'translate-x-0'}"></div>
              </div>
            </button>

            <!-- TOGGLE 2: FIND MY PHONE (RING) -->
            <button
              onclick={handleRingPhone}
              disabled={isRinging || !device.connected}
              class="flex items-center justify-between p-4 rounded-xl border transition-all duration-200 text-left {isRinging
                ? 'bg-emerald-950/40 border-emerald-500/60 shadow-md shadow-emerald-500/10 animate-pulse'
                : 'bg-[#0d1117] border-[#30363d] hover:border-[#484f58]'} {(!device.connected || isRinging) ? 'opacity-60 cursor-not-allowed' : 'cursor-pointer'}"
            >
              <div class="flex items-center gap-3.5">
                <div class="p-2.5 rounded-lg {isRinging ? 'bg-emerald-500/20 text-emerald-300' : 'bg-[#161b22] text-[#8b949e]'}">
                  <svg class="w-5 h-5 {isRinging ? 'animate-bounce' : ''}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M22 16.92v3a2 2 0 0 1-2.18 2 19.79 19.79 0 0 1-8.63-3.07 19.5 19.5 0 0 1-6-6 19.79 19.79 0 0 1-3.07-8.67A2 2 0 0 1 4.11 2h3a2 2 0 0 1 2 1.72 12.84 12.84 0 0 0 .7 2.81 2 2 0 0 1-.45 2.11L8.09 9.91a16 16 0 0 0 6 6l1.27-1.27a2 2 0 0 1 2.11-.45 12.84 12.84 0 0 0 2.81.7A2 2 0 0 1 22 16.92z"></path>
                  </svg>
                </div>
                <div>
                  <span class="text-sm font-semibold text-white block">Find My Phone</span>
                  <span class="text-xs text-[#8b949e]">
                    {isRinging ? 'Ringing device...' : 'Play loud alert'}
                  </span>
                </div>
              </div>

              <div class="px-2.5 py-1 rounded-lg text-xs font-semibold bg-[#161b22] text-cyan-400 border border-[#30363d]">
                Ring
              </div>
            </button>

            <!-- TOGGLE 3: DO NOT DISTURB -->
            <button
              onclick={handleToggleDnd}
              disabled={!device.connected}
              class="flex items-center justify-between p-4 rounded-xl border transition-all duration-200 text-left {dndActive
                ? 'bg-purple-950/40 border-purple-500/60 shadow-md shadow-purple-500/10'
                : 'bg-[#0d1117] border-[#30363d] hover:border-[#484f58]'} {!device.connected ? 'opacity-60 cursor-not-allowed' : 'cursor-pointer'}"
            >
              <div class="flex items-center gap-3.5">
                <div class="p-2.5 rounded-lg {dndActive ? 'bg-purple-500/20 text-purple-300' : 'bg-[#161b22] text-[#8b949e]'}">
                  <svg class="w-5 h-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9"></path>
                    <path d="M13.73 21a2 2 0 0 1-3.46 0"></path>
                    {#if dndActive}
                      <line x1="2" y1="2" x2="22" y2="22" stroke-width="2"></line>
                    {/if}
                  </svg>
                </div>
                <div>
                  <span class="text-sm font-semibold text-white block">Do Not Disturb</span>
                  <span class="text-xs text-[#8b949e]">
                    {dndActive ? 'Muted' : 'Normal alerts'}
                  </span>
                </div>
              </div>

              <!-- Switch Pill -->
              <div class="w-11 h-6 rounded-full transition-colors p-1 flex items-center {dndActive ? 'bg-purple-500' : 'bg-zinc-800'}">
                <div class="w-4 h-4 rounded-full bg-white transition-transform duration-200 shadow-md {dndActive ? 'translate-x-5' : 'translate-x-0'}"></div>
              </div>
            </button>
          </div>
        </div>
      </div>
    {:else if activeTab === 'photos'}
      <!-- TAB: PHOTOS PLACEHOLDER -->
      <div class="max-w-4xl w-full mx-auto space-y-6">
        <div class="p-12 text-center rounded-2xl bg-[#161b22] border border-[#30363d]/80">
          <div class="w-16 h-16 mx-auto mb-4 rounded-2xl bg-[#0d1117] border border-[#30363d] flex items-center justify-center text-cyan-400">
            <svg class="w-8 h-8" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
              <rect width="18" height="18" x="3" y="3" rx="2" ry="2"></rect>
              <circle cx="9" cy="9" r="2"></circle>
              <path d="m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21"></path>
            </svg>
          </div>
          <h2 class="text-xl font-bold text-white mb-2">Photos Gallery</h2>
          <p class="text-sm text-[#8b949e] max-w-md mx-auto">
            High-speed MediaStore photo synchronization and thumbnail indexing panel (Issue #25).
          </p>
        </div>
      </div>
    {:else if activeTab === 'messages'}
      <!-- TAB: MESSAGES PLACEHOLDER -->
      <div class="max-w-4xl w-full mx-auto space-y-6">
        <div class="p-12 text-center rounded-2xl bg-[#161b22] border border-[#30363d]/80">
          <div class="w-16 h-16 mx-auto mb-4 rounded-2xl bg-[#0d1117] border border-[#30363d] flex items-center justify-center text-cyan-400">
            <svg class="w-8 h-8" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
              <path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z"></path>
            </svg>
          </div>
          <h2 class="text-xl font-bold text-white mb-2">SMS & RCS Messages</h2>
          <p class="text-sm text-[#8b949e] max-w-md mx-auto">
            Threaded conversation view with direct desktop replying (Issue #26).
          </p>
        </div>
      </div>
    {:else if activeTab === 'apps'}
      <!-- TAB: APP DRAWER PLACEHOLDER -->
      <div class="max-w-4xl w-full mx-auto space-y-6">
        <div class="p-12 text-center rounded-2xl bg-[#161b22] border border-[#30363d]/80">
          <div class="w-16 h-16 mx-auto mb-4 rounded-2xl bg-[#0d1117] border border-[#30363d] flex items-center justify-center text-cyan-400">
            <svg class="w-8 h-8" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
              <rect width="7" height="7" x="3" y="3" rx="1"></rect>
              <rect width="7" height="7" x="14" y="3" rx="1"></rect>
              <rect width="7" height="7" x="14" y="14" rx="1"></rect>
              <rect width="7" height="7" x="3" y="14" rx="1"></rect>
            </svg>
          </div>
          <h2 class="text-xl font-bold text-white mb-2">App Drawer & Mirroring</h2>
          <p class="text-sm text-[#8b949e] max-w-md mx-auto">
            Direct single-app streaming into floating Hyprland windows via virtual display (Issue #27).
          </p>
        </div>
      </div>
    {/if}
  </main>
</div>
