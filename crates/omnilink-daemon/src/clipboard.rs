//! Wayland clipboard listener and BLAKE3 loop-prevention filter.
//!
//! Integrates with Wayland compositors via `ext-data-control-v1` (with fallback to
//! `wlr-data-control-unstable-v1`) to monitor desktop clipboard selections without
//! stealing window focus, and provides a BLAKE3 circular buffer filter to avoid
//! clipboard sync echo loops.

use std::collections::VecDeque;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing::{debug, warn};
use wayland_client::backend::WaylandError;
use wayland_client::globals::{registry_queue_init, BindError, GlobalListContents};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1::{self, ExtDataControlDeviceV1},
    ext_data_control_manager_v1::ExtDataControlManagerV1,
    ext_data_control_offer_v1::{self, ExtDataControlOfferV1},
};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1::{self, ZwlrDataControlDeviceV1},
    zwlr_data_control_manager_v1::{self, ZwlrDataControlManagerV1},
    zwlr_data_control_offer_v1::{self, ZwlrDataControlOfferV1},
};

/// Default circular buffer history capacity for clipboard deduplication.
pub const DEFAULT_FILTER_CAPACITY: usize = 16;

/// Maximum payload bytes accepted from clipboard read pipe (20 MiB safety clamp).
pub const MAX_CLIPBOARD_PAYLOAD_SIZE: usize = 20 * 1024 * 1024;

/// Supported clipboard MIME types prioritized for text and images.
pub const SUPPORTED_MIME_TYPES: &[&str] = &[
    "text/plain;charset=utf-8",
    "text/plain",
    "image/png",
];

/// A clipboard event captured from or to be synced across connected devices.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ClipboardEvent {
    /// MIME type of the payload (e.g. `text/plain;charset=utf-8` or `image/png`).
    pub mime_type: String,
    /// Whether the clipboard content originated from a sensitive source (password manager).
    pub is_sensitive: bool,
    /// Raw clipboard data payload.
    pub payload: Vec<u8>,
}

impl ClipboardEvent {
    /// Constructs a new `ClipboardEvent`.
    pub fn new(mime_type: impl Into<String>, is_sensitive: bool, payload: Vec<u8>) -> Self {
        Self {
            mime_type: mime_type.into(),
            is_sensitive,
            payload,
        }
    }
}

/// Normalizes a MIME type string for comparison.
pub fn normalize_mime_type(mime: &str) -> &str {
    let trimmed = mime.trim();
    if trimmed.eq_ignore_ascii_case("text/plain;charset=utf-8")
        || trimmed.eq_ignore_ascii_case("text/plain; charset=utf-8")
    {
        "text/plain;charset=utf-8"
    } else if trimmed.eq_ignore_ascii_case("text/plain") {
        "text/plain"
    } else if trimmed.eq_ignore_ascii_case("image/png") {
        "image/png"
    } else {
        trimmed
    }
}

/// Checks whether a MIME type indicates sensitive contents (password manager hints).
pub fn is_sensitive_mime_type(mime: &str) -> bool {
    let lower = mime.trim().to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "x-kde-passwordmanagerhint"
            | "application/x-kde-passwordmanagerhint"
            | "application/x-secret-service"
            | "application/x-keepassx-password"
            | "application/x-1password"
    )
}

/// Selects the highest priority supported MIME type from an advertised list.
pub fn select_best_mime_type(offered: &[String]) -> Option<&'static str> {
    SUPPORTED_MIME_TYPES
        .iter()
        .copied()
        .find(|&supported| offered.iter().any(|m| normalize_mime_type(m) == supported))
}

/// BLAKE3 circular buffer filter for loop and echo detection.
#[derive(Debug, Clone)]
pub struct ClipboardFilter {
    recent_hashes: VecDeque<blake3::Hash>,
    capacity: usize,
}

impl Default for ClipboardFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl ClipboardFilter {
    /// Creates a new `ClipboardFilter` with default capacity (16 entries).
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_FILTER_CAPACITY)
    }

    /// Creates a new `ClipboardFilter` with the specified ring buffer capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            recent_hashes: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Computes the BLAKE3 hash of `data`.
    ///
    /// Returns `false` if already present in history; otherwise adds to ring buffer
    /// (popping oldest when > capacity) and returns `true`.
    pub fn should_sync(&mut self, data: &[u8]) -> bool {
        let hash = blake3::hash(data);
        if self.recent_hashes.contains(&hash) {
            return false;
        }
        if self.capacity > 0 {
            if self.recent_hashes.len() >= self.capacity {
                self.recent_hashes.pop_front();
            }
            self.recent_hashes.push_back(hash);
        }
        true
    }

    /// Records a hash into the filter (e.g. for injected incoming clips to prevent echo).
    pub fn record(&mut self, data: &[u8]) {
        if self.capacity == 0 {
            return;
        }
        let hash = blake3::hash(data);
        if let Some(pos) = self.recent_hashes.iter().position(|h| h == &hash) {
            self.recent_hashes.remove(pos);
        } else if self.recent_hashes.len() >= self.capacity {
            self.recent_hashes.pop_front();
        }
        self.recent_hashes.push_back(hash);
    }

    /// Returns `true` if the BLAKE3 hash of `data` exists in the recent history buffer.
    pub fn contains(&self, data: &[u8]) -> bool {
        let hash = blake3::hash(data);
        self.recent_hashes.contains(&hash)
    }

    /// Current number of entries stored in the ring buffer.
    pub fn len(&self) -> usize {
        self.recent_hashes.len()
    }

    /// Whether the filter history is currently empty.
    pub fn is_empty(&self) -> bool {
        self.recent_hashes.is_empty()
    }

    /// Maximum capacity of the ring buffer.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Clears all recorded entries.
    pub fn clear(&mut self) {
        self.recent_hashes.clear();
    }
}

/// Errors occurring during Wayland clipboard operations.
#[derive(Debug, thiserror::Error)]
pub enum ClipboardError {
    #[error("WAYLAND_DISPLAY or WAYLAND_SOCKET environment variable not set")]
    NoWaylandDisplay,
    #[error("Failed to connect to Wayland compositor: {0}")]
    ConnectionFailed(#[from] wayland_client::ConnectError),
    #[error("Wayland global error: {0}")]
    Global(#[from] wayland_client::globals::GlobalError),
    #[error("Wayland bind error: {0}")]
    Bind(#[from] BindError),
    #[error("Wayland dispatch error: {0}")]
    Dispatch(#[from] wayland_client::DispatchError),
    #[error("Wayland protocol error: {0}")]
    Wayland(#[from] WaylandError),
    #[error("No data control manager supported by compositor (ext-data-control or wlr-data-control required)")]
    UnsupportedProtocol,
    #[error("No Wayland seat available")]
    NoSeat,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Channel send error: receiver dropped")]
    ChannelClosed,
}

/// Reads all bytes from an OwnedFd pipe until EOF, with timeout and maximum size clamp.
pub fn read_pipe_to_end(read_fd: OwnedFd, timeout_ms: i32) -> std::io::Result<Vec<u8>> {
    use std::io::Read;

    let raw_fd = read_fd.as_raw_fd();
    let mut file = std::fs::File::from(read_fd);
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];

    loop {
        let mut pollfd = libc::pollfd {
            fd: raw_fd,
            events: libc::POLLIN | libc::POLLERR | libc::POLLHUP,
            revents: 0,
        };

        let ret = unsafe { libc::poll(&mut pollfd, 1, timeout_ms) };
        if ret < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        } else if ret == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "Clipboard pipe read timed out",
            ));
        }

        match file.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if buffer.len() + n > MAX_CLIPBOARD_PAYLOAD_SIZE {
                    buffer.extend_from_slice(&chunk[..MAX_CLIPBOARD_PAYLOAD_SIZE - buffer.len()]);
                    break;
                }
                buffer.extend_from_slice(&chunk[..n]);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }

    Ok(buffer)
}

/// Per-offer tracking data for advertised MIME types.
#[derive(Default, Debug)]
pub struct OfferData {
    pub mime_types: Vec<String>,
}

/// Internal Wayland event handling state.
pub struct WaylandClipboardState {
    pub tx: mpsc::Sender<ClipboardEvent>,
    pub filter: Arc<Mutex<ClipboardFilter>>,
    pub current_wlr_offer: Option<ZwlrDataControlOfferV1>,
    pub current_ext_offer: Option<ExtDataControlOfferV1>,
    pub shutdown: Arc<AtomicBool>,
}

impl WaylandClipboardState {
    fn handle_wlr_selection(
        &mut self,
        offer_opt: Option<ZwlrDataControlOfferV1>,
        conn: &Connection,
    ) {
        if let Some(prev) = self.current_wlr_offer.take() {
            prev.destroy();
        }

        if let Some(offer) = offer_opt.as_ref() {
            if let Some(data_arc) = offer.data::<Arc<Mutex<OfferData>>>() {
                let mime_types = {
                    let lock = data_arc.lock().unwrap();
                    lock.mime_types.clone()
                };
                let is_sensitive = mime_types.iter().any(|m| is_sensitive_mime_type(m));
                if is_sensitive {
                    debug!("Ignoring sensitive clipboard selection (password manager marker detected)");
                    self.current_wlr_offer = offer_opt;
                    return;
                }

                if let Some(best_mime) = select_best_mime_type(&mime_types) {
                    let mut fds = [0i32; 2];
                    let ret = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
                    if ret == 0 {
                        let read_fd = unsafe { OwnedFd::from_raw_fd(fds[0]) };
                        let write_fd = unsafe { OwnedFd::from_raw_fd(fds[1]) };

                        offer.receive(best_mime.to_string(), write_fd.as_fd());
                        let _ = conn.flush();
                        drop(write_fd);

                        match read_pipe_to_end(read_fd, 1000) {
                            Ok(payload) => {
                                let should_sync = {
                                    let mut filter_guard = self.filter.lock().unwrap();
                                    filter_guard.should_sync(&payload)
                                };
                                if should_sync {
                                    let event = ClipboardEvent::new(best_mime, false, payload);
                                    match self.tx.try_send(event) {
                                        Ok(()) => {}
                                        Err(mpsc::error::TrySendError::Full(_)) => {
                                            warn!("Clipboard channel full, dropping event to avoid blocking Wayland loop");
                                        }
                                        Err(mpsc::error::TrySendError::Closed(_)) => {
                                            debug!("Clipboard receiver dropped, shutting down listener");
                                            self.shutdown.store(true, Ordering::SeqCst);
                                        }
                                    }
                                }
                            }
                            Err(err) => {
                                warn!("Failed to read clipboard payload: {err}");
                            }
                        }
                    }
                }
            }
        }

        self.current_wlr_offer = offer_opt;
    }

    fn handle_ext_selection(
        &mut self,
        offer_opt: Option<ExtDataControlOfferV1>,
        conn: &Connection,
    ) {
        if let Some(prev) = self.current_ext_offer.take() {
            prev.destroy();
        }

        if let Some(offer) = offer_opt.as_ref() {
            if let Some(data_arc) = offer.data::<Arc<Mutex<OfferData>>>() {
                let mime_types = {
                    let lock = data_arc.lock().unwrap();
                    lock.mime_types.clone()
                };
                let is_sensitive = mime_types.iter().any(|m| is_sensitive_mime_type(m));
                if is_sensitive {
                    debug!("Ignoring sensitive clipboard selection (password manager marker detected)");
                    self.current_ext_offer = offer_opt;
                    return;
                }

                if let Some(best_mime) = select_best_mime_type(&mime_types) {
                    let mut fds = [0i32; 2];
                    let ret = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
                    if ret == 0 {
                        let read_fd = unsafe { OwnedFd::from_raw_fd(fds[0]) };
                        let write_fd = unsafe { OwnedFd::from_raw_fd(fds[1]) };

                        offer.receive(best_mime.to_string(), write_fd.as_fd());
                        let _ = conn.flush();
                        drop(write_fd);

                        match read_pipe_to_end(read_fd, 1000) {
                            Ok(payload) => {
                                let should_sync = {
                                    let mut filter_guard = self.filter.lock().unwrap();
                                    filter_guard.should_sync(&payload)
                                };
                                if should_sync {
                                    let event = ClipboardEvent::new(best_mime, false, payload);
                                    match self.tx.try_send(event) {
                                        Ok(()) => {}
                                        Err(mpsc::error::TrySendError::Full(_)) => {
                                            warn!("Clipboard channel full, dropping event to avoid blocking Wayland loop");
                                        }
                                        Err(mpsc::error::TrySendError::Closed(_)) => {
                                            debug!("Clipboard receiver dropped, shutting down listener");
                                            self.shutdown.store(true, Ordering::SeqCst);
                                        }
                                    }
                                }
                            }
                            Err(err) => {
                                warn!("Failed to read clipboard payload: {err}");
                            }
                        }
                    }
                }
            }
        }

        self.current_ext_offer = offer_opt;
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for WaylandClipboardState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for WaylandClipboardState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_seat::WlSeat,
        _event: wl_seat::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrDataControlManagerV1, ()> for WaylandClipboardState {
    fn event(
        _state: &mut Self,
        _proxy: &ZwlrDataControlManagerV1,
        _event: zwlr_data_control_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrDataControlDeviceV1, ()> for WaylandClipboardState {
    fn event(
        state: &mut Self,
        _proxy: &ZwlrDataControlDeviceV1,
        event: zwlr_data_control_device_v1::Event,
        _data: &(),
        conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_data_control_device_v1::Event::DataOffer { .. } => {}
            zwlr_data_control_device_v1::Event::Selection { id } => {
                state.handle_wlr_selection(id, conn);
            }
            zwlr_data_control_device_v1::Event::PrimarySelection { .. } => {}
            zwlr_data_control_device_v1::Event::Finished => {
                debug!("ZwlrDataControlDeviceV1 finished");
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(
        WaylandClipboardState,
        ZwlrDataControlDeviceV1,
        [
            zwlr_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (
                ZwlrDataControlOfferV1,
                Arc::new(Mutex::new(OfferData::default()))
            )
        ]
    );
}

impl Dispatch<ZwlrDataControlOfferV1, Arc<Mutex<OfferData>>> for WaylandClipboardState {
    fn event(
        _state: &mut Self,
        _proxy: &ZwlrDataControlOfferV1,
        event: zwlr_data_control_offer_v1::Event,
        data: &Arc<Mutex<OfferData>>,
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        if let zwlr_data_control_offer_v1::Event::Offer { mime_type } = event {
            if let Ok(mut lock) = data.lock() {
                lock.mime_types.push(mime_type);
            }
        }
    }
}

impl Dispatch<ExtDataControlManagerV1, ()> for WaylandClipboardState {
    fn event(
        _state: &mut Self,
        _proxy: &ExtDataControlManagerV1,
        _event: wayland_protocols::ext::data_control::v1::client::ext_data_control_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtDataControlDeviceV1, ()> for WaylandClipboardState {
    fn event(
        state: &mut Self,
        _proxy: &ExtDataControlDeviceV1,
        event: ext_data_control_device_v1::Event,
        _data: &(),
        conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        match event {
            ext_data_control_device_v1::Event::DataOffer { .. } => {}
            ext_data_control_device_v1::Event::Selection { id } => {
                state.handle_ext_selection(id, conn);
            }
            ext_data_control_device_v1::Event::PrimarySelection { .. } => {}
            ext_data_control_device_v1::Event::Finished => {
                debug!("ExtDataControlDeviceV1 finished");
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(
        WaylandClipboardState,
        ExtDataControlDeviceV1,
        [
            ext_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (
                ExtDataControlOfferV1,
                Arc::new(Mutex::new(OfferData::default()))
            )
        ]
    );
}

impl Dispatch<ExtDataControlOfferV1, Arc<Mutex<OfferData>>> for WaylandClipboardState {
    fn event(
        _state: &mut Self,
        _proxy: &ExtDataControlOfferV1,
        event: ext_data_control_offer_v1::Event,
        data: &Arc<Mutex<OfferData>>,
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        if let ext_data_control_offer_v1::Event::Offer { mime_type } = event {
            if let Ok(mut lock) = data.lock() {
                lock.mime_types.push(mime_type);
            }
        }
    }
}

/// Handle allowing inspection and shutdown of a running background clipboard listener.
pub struct ClipboardHandle {
    shutdown: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<Result<(), ClipboardError>>>,
}

impl std::fmt::Debug for ClipboardHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipboardHandle")
            .field("is_running", &self.is_running())
            .finish()
    }
}

impl ClipboardHandle {
    /// Signals the background listener thread to stop.
    pub fn stop(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    /// Returns `true` if the listener is running.
    pub fn is_running(&self) -> bool {
        !self.shutdown.load(Ordering::SeqCst)
    }

    /// Stops the listener and waits for the background worker thread to finish.
    pub fn join(mut self) -> Result<(), ClipboardError> {
        self.stop();
        if let Some(handle) = self.handle.take() {
            handle
                .join()
                .unwrap_or_else(|_| Err(ClipboardError::Io(std::io::Error::other("Thread panicked"))))
        } else {
            Ok(())
        }
    }
}

/// Wayland clipboard listener and synchronization manager.
pub struct ClipboardListener {
    tx: mpsc::Sender<ClipboardEvent>,
    filter: Arc<Mutex<ClipboardFilter>>,
}

impl std::fmt::Debug for ClipboardListener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipboardListener")
            .field("filter", &self.filter)
            .finish()
    }
}

impl ClipboardListener {
    /// Constructs a new `ClipboardListener` with a fresh `ClipboardFilter`.
    pub fn new(tx: mpsc::Sender<ClipboardEvent>) -> Self {
        Self::with_filter(tx, Arc::new(Mutex::new(ClipboardFilter::new())))
    }

    /// Constructs a new `ClipboardListener` sharing an existing `ClipboardFilter`.
    pub fn with_filter(
        tx: mpsc::Sender<ClipboardEvent>,
        filter: Arc<Mutex<ClipboardFilter>>,
    ) -> Self {
        Self { tx, filter }
    }

    /// Returns a reference to the shared `ClipboardFilter`.
    pub fn filter(&self) -> &Arc<Mutex<ClipboardFilter>> {
        &self.filter
    }

    /// Checks if a Wayland compositor environment is currently available and accessible.
    pub fn is_wayland_available() -> bool {
        if std::env::var_os("WAYLAND_DISPLAY").is_none()
            && std::env::var_os("WAYLAND_SOCKET").is_none()
        {
            return false;
        }
        Connection::connect_to_env().is_ok()
    }

    /// Spawns the Wayland listener in a background worker thread.
    ///
    /// If `WAYLAND_DISPLAY` is absent or connection fails, returns `Err(ClipboardError)`
    /// without panicking or hanging.
    pub fn spawn(&self) -> Result<ClipboardHandle, ClipboardError> {
        spawn_clipboard_listener(self.tx.clone(), self.filter.clone())
    }

    /// Runs the clipboard event loop synchronously on the current thread until shutdown is signaled.
    pub fn run(&self, shutdown: Arc<AtomicBool>) -> Result<(), ClipboardError> {
        run_listener(self.tx.clone(), self.filter.clone(), shutdown)
    }
}

/// Spawns a background thread listening for Wayland clipboard selection events.
pub fn spawn_clipboard_listener(
    tx: mpsc::Sender<ClipboardEvent>,
    filter: Arc<Mutex<ClipboardFilter>>,
) -> Result<ClipboardHandle, ClipboardError> {
    if std::env::var_os("WAYLAND_DISPLAY").is_none()
        && std::env::var_os("WAYLAND_SOCKET").is_none()
    {
        return Err(ClipboardError::NoWaylandDisplay);
    }

    let conn = Connection::connect_to_env()?;
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_thread = shutdown.clone();

    let handle = std::thread::Builder::new()
        .name("omnilink-wayland-clipboard".into())
        .spawn(move || run_listener_with_conn(conn, tx, filter, shutdown_thread))
        .map_err(ClipboardError::Io)?;

    Ok(ClipboardHandle {
        shutdown,
        handle: Some(handle),
    })
}

/// Runs the Wayland clipboard listener loop synchronously.
pub fn run_listener(
    tx: mpsc::Sender<ClipboardEvent>,
    filter: Arc<Mutex<ClipboardFilter>>,
    shutdown: Arc<AtomicBool>,
) -> Result<(), ClipboardError> {
    if std::env::var_os("WAYLAND_DISPLAY").is_none()
        && std::env::var_os("WAYLAND_SOCKET").is_none()
    {
        return Err(ClipboardError::NoWaylandDisplay);
    }
    let conn = Connection::connect_to_env()?;
    run_listener_with_conn(conn, tx, filter, shutdown)
}

/// Runs the Wayland listener event loop with an already established `Connection`.
pub fn run_listener_with_conn(
    conn: Connection,
    tx: mpsc::Sender<ClipboardEvent>,
    filter: Arc<Mutex<ClipboardFilter>>,
    shutdown: Arc<AtomicBool>,
) -> Result<(), ClipboardError> {
    let (globals, mut event_queue) = registry_queue_init::<WaylandClipboardState>(&conn)?;
    let qh = event_queue.handle();

    let global_list = globals.contents().clone_list();
    let has_ext = global_list
        .iter()
        .any(|g| g.interface == ExtDataControlManagerV1::interface().name);
    let has_wlr = global_list
        .iter()
        .any(|g| g.interface == ZwlrDataControlManagerV1::interface().name);
    let has_seat = global_list
        .iter()
        .any(|g| g.interface == wl_seat::WlSeat::interface().name);

    if !has_seat {
        return Err(ClipboardError::NoSeat);
    }
    if !has_ext && !has_wlr {
        return Err(ClipboardError::UnsupportedProtocol);
    }

    let seat = globals.bind::<wl_seat::WlSeat, WaylandClipboardState, ()>(&qh, 1..=1, ())?;

    let _ext_device = if has_ext {
        let ext_manager = globals
            .bind::<ExtDataControlManagerV1, WaylandClipboardState, ()>(&qh, 1..=1, ())?;
        Some(ext_manager.get_data_device(&seat, &qh, ()))
    } else {
        None
    };

    let _wlr_device = if !has_ext && has_wlr {
        let wlr_manager = globals
            .bind::<ZwlrDataControlManagerV1, WaylandClipboardState, ()>(&qh, 1..=2, ())?;
        Some(wlr_manager.get_data_device(&seat, &qh, ()))
    } else {
        None
    };

    let mut state = WaylandClipboardState {
        tx,
        filter,
        current_wlr_offer: None,
        current_ext_offer: None,
        shutdown: shutdown.clone(),
    };

    // Initial roundtrip to initialize compositor objects and current selection
    event_queue.roundtrip(&mut state)?;

    while !shutdown.load(Ordering::Relaxed) {
        event_queue.dispatch_pending(&mut state)?;
        let _ = event_queue.flush();

        if let Some(guard) = event_queue.prepare_read() {
            let fd = guard.connection_fd();
            let mut pollfd = libc::pollfd {
                fd: fd.as_raw_fd(),
                events: libc::POLLIN | libc::POLLERR | libc::POLLHUP,
                revents: 0,
            };
            let ret = unsafe { libc::poll(&mut pollfd, 1, 200) };
            if ret > 0 {
                let _ = guard.read();
            }
        }

        event_queue.dispatch_pending(&mut state)?;
    }

    if let Some(offer) = state.current_wlr_offer.take() {
        offer.destroy();
    }
    if let Some(offer) = state.current_ext_offer.take() {
        offer.destroy();
    }
    let _ = event_queue.flush();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn test_clipboard_filter_capacity_and_initial_state() {
        let filter = ClipboardFilter::new();
        assert_eq!(filter.capacity(), 16);
        assert_eq!(filter.len(), 0);
        assert!(filter.is_empty());

        let custom = ClipboardFilter::with_capacity(5);
        assert_eq!(custom.capacity(), 5);
        assert_eq!(custom.len(), 0);
        assert!(custom.is_empty());
    }

    #[test]
    fn test_clipboard_filter_blake3_deduplication() {
        let mut filter = ClipboardFilter::new();
        let payload = b"Hello Omnilink Clipboard";

        // First time should sync
        assert!(filter.should_sync(payload));
        assert_eq!(filter.len(), 1);
        assert!(filter.contains(payload));

        // Second time duplicate must be suppressed
        assert!(!filter.should_sync(payload));
        assert_eq!(filter.len(), 1);

        // Different payload should sync
        let payload2 = b"Different clipboard payload";
        assert!(filter.should_sync(payload2));
        assert_eq!(filter.len(), 2);
        assert!(filter.contains(payload2));
    }

    #[test]
    fn test_clipboard_filter_eviction_on_17th_item() {
        let mut filter = ClipboardFilter::new();
        assert_eq!(filter.capacity(), 16);

        // Insert 16 distinct items
        for i in 1..=16 {
            let item = format!("item_{i}");
            assert!(filter.should_sync(item.as_bytes()));
            assert_eq!(filter.len(), i);
            assert!(filter.contains(item.as_bytes()));
        }

        assert_eq!(filter.len(), 16);
        assert!(filter.contains(b"item_1"));
        assert!(filter.contains(b"item_16"));

        // Insert 17th item -> should evict item_1
        assert!(filter.should_sync(b"item_17"));
        assert_eq!(filter.len(), 16);

        // item_1 is now evicted
        assert!(!filter.contains(b"item_1"));
        // item_1 should sync again if re-copied
        assert!(filter.should_sync(b"item_1"));

        // item_17 is still present
        assert!(filter.contains(b"item_17"));

        // items 3..=17 are still present (item_2 was evicted by re-inserting item_1)
        assert!(!filter.contains(b"item_2"));
        assert!(filter.contains(b"item_3"));
    }

    #[test]
    fn test_clipboard_filter_record_incoming_echo_prevention() {
        let mut filter = ClipboardFilter::new();
        let phone_clip = b"Copied on Pixel 8 Pro phone";

        // Record incoming clip from companion device
        filter.record(phone_clip);
        assert_eq!(filter.len(), 1);
        assert!(filter.contains(phone_clip));

        // When Wayland selection fires on Linux desktop, echo sync must be prevented!
        assert!(!filter.should_sync(phone_clip));
        assert_eq!(filter.len(), 1);

        // New local clip should sync
        let local_clip = b"Copied locally on Linux Hyprland";
        assert!(filter.should_sync(local_clip));
        assert_eq!(filter.len(), 2);
    }

    #[test]
    fn test_clipboard_event_creation_and_serde() {
        let payload = b"Sensitive Token 12345".to_vec();
        let event = ClipboardEvent::new("text/plain;charset=utf-8", true, payload.clone());

        assert_eq!(event.mime_type, "text/plain;charset=utf-8");
        assert!(event.is_sensitive);
        assert_eq!(event.payload, payload);

        // Verify Serde JSON round-trip
        let serialized = serde_json::to_string(&event).expect("Serialization failed");
        let deserialized: ClipboardEvent =
            serde_json::from_str(&serialized).expect("Deserialization failed");
        assert_eq!(event, deserialized);
    }

    #[test]
    fn test_mime_type_validation_and_priority() {
        // Normalization
        assert_eq!(
            normalize_mime_type("text/plain; charset=utf-8"),
            "text/plain;charset=utf-8"
        );
        assert_eq!(normalize_mime_type("TEXT/PLAIN"), "text/plain");
        assert_eq!(normalize_mime_type("IMAGE/PNG"), "image/png");

        // Selection priority: text/plain;charset=utf-8 over text/plain
        let offered = vec![
            "text/plain".to_string(),
            "text/plain;charset=utf-8".to_string(),
        ];
        assert_eq!(
            select_best_mime_type(&offered),
            Some("text/plain;charset=utf-8")
        );

        // Selection fallback: only text/plain
        let offered_plain = vec!["text/plain".to_string()];
        assert_eq!(select_best_mime_type(&offered_plain), Some("text/plain"));

        // Selection for image/png
        let offered_image = vec!["image/png".to_string(), "application/octet-stream".to_string()];
        assert_eq!(select_best_mime_type(&offered_image), Some("image/png"));

        // Unsupported MIME types return None
        let offered_unsupported = vec![
            "application/pdf".to_string(),
            "application/zip".to_string(),
        ];
        assert_eq!(select_best_mime_type(&offered_unsupported), None);
    }

    #[test]
    fn test_is_sensitive_mime_type() {
        assert!(is_sensitive_mime_type("x-kde-passwordManagerHint"));
        assert!(is_sensitive_mime_type("application/x-kde-passwordManagerHint"));
        assert!(is_sensitive_mime_type("application/x-secret-service"));
        assert!(is_sensitive_mime_type("application/x-keepassx-password"));
        assert!(is_sensitive_mime_type("application/x-1password"));

        // Non-sensitive
        assert!(!is_sensitive_mime_type("text/plain"));
        assert!(!is_sensitive_mime_type("text/plain;charset=utf-8"));
        assert!(!is_sensitive_mime_type("image/png"));
    }

    #[test]
    fn test_wayland_display_missing_fallback() {
        let _guard = ENV_MUTEX.lock().unwrap();

        // Save original env vars
        let orig_display = std::env::var_os("WAYLAND_DISPLAY");
        let orig_socket = std::env::var_os("WAYLAND_SOCKET");

        // Temporarily remove Wayland env vars
        std::env::remove_var("WAYLAND_DISPLAY");
        std::env::remove_var("WAYLAND_SOCKET");

        assert!(!ClipboardListener::is_wayland_available());

        let (tx, _rx) = mpsc::channel(16);
        let filter = Arc::new(Mutex::new(ClipboardFilter::new()));

        let result = spawn_clipboard_listener(tx.clone(), filter);
        match result {
            Err(ClipboardError::NoWaylandDisplay) => {}
            other => panic!("Expected NoWaylandDisplay, got: {other:?}"),
        }

        let listener = ClipboardListener::new(tx);
        let shutdown = Arc::new(AtomicBool::new(false));
        let run_result = listener.run(shutdown);
        match run_result {
            Err(ClipboardError::NoWaylandDisplay) => {}
            other => panic!("Expected NoWaylandDisplay, got: {other:?}"),
        }

        // Restore original env vars
        if let Some(val) = orig_display {
            std::env::set_var("WAYLAND_DISPLAY", val);
        }
        if let Some(val) = orig_socket {
            std::env::set_var("WAYLAND_SOCKET", val);
        }
    }

    #[test]
    fn test_read_pipe_to_end_success() {
        use std::io::Write;

        let mut fds = [0i32; 2];
        let ret = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
        assert_eq!(ret, 0);

        let read_fd = unsafe { OwnedFd::from_raw_fd(fds[0]) };
        let write_fd = unsafe { OwnedFd::from_raw_fd(fds[1]) };

        let expected_data = b"Testing Wayland clipboard pipe communication payload";

        // Spawn a thread to write data and close write end
        let writer_handle = std::thread::spawn(move || {
            let mut file = std::fs::File::from(write_fd);
            file.write_all(expected_data).unwrap();
            file.flush().unwrap();
            // file is dropped, closing write_fd
        });

        let payload = read_pipe_to_end(read_fd, 2000).expect("read_pipe_to_end failed");
        writer_handle.join().unwrap();

        assert_eq!(payload, expected_data);
    }

    #[tokio::test]
    async fn test_clipboard_event_channel_emission() {
        let (tx, mut rx) = mpsc::channel(16);
        let event = ClipboardEvent::new("image/png", false, vec![0x89, 0x50, 0x4E, 0x47]);

        tx.send(event.clone()).await.expect("Channel send failed");

        let received = rx.recv().await.expect("Channel recv failed");
        assert_eq!(received, event);
    }
}

