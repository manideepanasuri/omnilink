//! QUIC transport listener, mutual TLS 1.3 security, and stream multiplexer.

pub mod crypto;
pub mod demux;
pub mod server;
pub mod stream0;
pub mod stream1;
pub mod stream2;

pub use crypto::{
    extract_ed25519_verifying_key, generate_client_certificate, generate_daemon_certificate,
    init_crypto_provider, make_client_config, make_server_config, Ed25519ClientCertVerifier,
    Ed25519ServerCertVerifier, ALPN_OMNILINK,
};
pub use demux::handle_connection;
pub use server::{QuicListener, QuicServer, DEFAULT_PORT};
pub use stream0::{read_control_packet, run_stream_0, write_control_packet, HEARTBEAT_TIMEOUT};
pub use stream1::{
    read_clipboard_packet, run_stream_1, write_clipboard_packet, MAX_CLIPBOARD_FRAME_LEN,
};
pub use stream2::{
    read_notification_packet, run_stream_2, write_notification_packet, MAX_NOTIFICATION_FRAME_LEN,
};

use omnilink_protocol::error::ProtocolError;
use thiserror::Error;

/// Errors that can occur during QUIC transport, TLS, or stream multiplexing.
#[derive(Error, Debug)]
pub enum QuicError {
    /// Standard I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// TLS certificate generation or parameter error.
    #[error("Certificate error: {0}")]
    CertificateGeneration(String),

    /// Rustls TLS engine error.
    #[error("Rustls error: {0}")]
    Rustls(#[from] rustls::Error),

    /// Quinn QUIC connection error.
    #[error("Quinn connection error: {0}")]
    Connection(#[from] quinn::ConnectionError),

    /// Quinn client connection initiation error.
    #[error("Quinn connect error: {0}")]
    Connect(#[from] quinn::ConnectError),

    /// Quinn stream write error.
    #[error("Quinn write error: {0}")]
    Write(#[from] quinn::WriteError),

    /// Quinn stream read error.
    #[error("Quinn read error: {0}")]
    Read(#[from] quinn::ReadError),

    /// Quinn stream read exact error.
    #[error("Quinn read exact error: {0}")]
    ReadExact(#[from] quinn::ReadExactError),

    /// Protocol serialization or wire framing error.
    #[error("Protocol error: {0}")]
    Protocol(#[from] ProtocolError),

    /// Peer certificate failed validation.
    #[error("Invalid certificate: {0}")]
    InvalidCertificate(String),

    /// Heartbeat timeout exceeded without receiving pings or telemetry.
    #[error("Heartbeat timeout")]
    HeartbeatTimeout,

    /// Connection or stream closed by peer.
    #[error("Connection closed: {0}")]
    ConnectionClosed(String),

    /// Stream demultiplexer error.
    #[error("Demux error: {0}")]
    Demux(String),
}
