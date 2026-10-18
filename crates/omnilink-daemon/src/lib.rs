//! Omnilink Daemon library.
//!
//! Provides identity key management, onboarding pairing generator,
//! mutual TLS 1.3 QUIC endpoint listener, and session orchestration.

pub mod identity;
pub mod pairing;
pub mod quic;
pub mod state;

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
    init_crypto_provider, make_client_config, make_server_config, read_control_packet,
    run_stream_0, write_control_packet, Ed25519ClientCertVerifier, Ed25519ServerCertVerifier,
    QuicError, QuicListener, QuicServer, ALPN_OMNILINK, HEARTBEAT_TIMEOUT,
};
pub use state::{ActiveSession, DaemonState, PairedDevice, SessionState};
