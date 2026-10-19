//! Omnilink Daemon library.
//!
//! Provides identity key management, onboarding pairing generator,
//! mutual TLS 1.3 QUIC endpoint listener, and session orchestration.

pub mod ble;
pub mod clipboard;
pub mod identity;
pub mod notifications;
pub mod pairing;
pub mod quic;
pub mod state;

pub use ble::{
    verify_beacon_payload, BleDiscoveryEvent, BleError, BleScanner, OMNILINK_SERVICE_UUID,
};
pub use clipboard::{
    is_sensitive_mime_type, normalize_mime_type, select_best_mime_type, spawn_clipboard_listener,
    ClipboardError, ClipboardEvent, ClipboardFilter, ClipboardHandle, ClipboardListener,
    DEFAULT_FILTER_CAPACITY, MAX_CLIPBOARD_PAYLOAD_SIZE, SUPPORTED_MIME_TYPES,
};
pub use identity::{
    default_key_path, load_identity, load_or_generate_identity, save_identity,
    validate_directory_permissions, validate_file_permissions, IdentityError,
};
pub use pairing::{
    discover_local_addresses, generate_pairing_payload, generate_pairing_token, get_hostname,
    render_qr_code, PairingError, PairingPayload, DEFAULT_PORT,
};
pub use quic::{
    extract_ed25519_verifying_key, generate_client_certificate, generate_daemon_certificate,
    init_crypto_provider, make_client_config, make_server_config, read_clipboard_packet,
    read_control_packet, read_notification_packet, run_stream_0, run_stream_1, run_stream_2,
    write_clipboard_packet, write_control_packet, write_notification_packet,
    Ed25519ClientCertVerifier, Ed25519ServerCertVerifier, QuicError, QuicListener, QuicServer,
    ALPN_OMNILINK, HEARTBEAT_TIMEOUT, MAX_CLIPBOARD_FRAME_LEN, MAX_NOTIFICATION_FRAME_LEN,
};
pub use notifications::{
    format_notification_actions, start_dbus_signal_listener, FreedesktopNotificationsProxy,
    NotificationError, NotificationForwarder, NotificationMap,
};
pub use state::{ActiveSession, DaemonState, PairedDevice, SessionState};

