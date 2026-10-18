//! QUIC server endpoint listener and incoming connection acceptor.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use quinn::Endpoint;
use rustls::server::danger::ClientCertVerifier;

use crate::quic::crypto::{make_server_config, Ed25519ClientCertVerifier};
use crate::quic::demux::handle_connection;
use crate::quic::QuicError;
use crate::state::DaemonState;

/// Default port for the Omnilink QUIC endpoint listener.
pub const DEFAULT_PORT: u16 = 42424;

/// Default max idle timeout for QUIC connections (5 seconds).
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(5);

/// QUIC server endpoint managing incoming authenticated connections.
pub struct QuicServer {
    endpoint: Endpoint,
    state: Arc<DaemonState>,
}

/// Type alias for `QuicServer` to support `QuicListener` naming convention.
pub type QuicListener = QuicServer;

impl QuicServer {
    /// Binds a new QUIC listener on the specified `SocketAddr` with mutual TLS 1.3
    /// configured from the daemon's state.
    pub async fn bind(addr: SocketAddr, state: Arc<DaemonState>) -> Result<Self, QuicError> {
        let verifier = Arc::new(Ed25519ClientCertVerifier::new_dynamic(
            state.paired_verifier_key.clone(),
        ));

        Self::bind_with_verifier(addr, state, verifier, Some(DEFAULT_IDLE_TIMEOUT)).await
    }

    /// Binds a new QUIC listener with an explicit client certificate verifier and optional idle timeout.
    pub async fn bind_with_verifier(
        addr: SocketAddr,
        state: Arc<DaemonState>,
        verifier: Arc<dyn ClientCertVerifier>,
        max_idle_timeout: Option<Duration>,
    ) -> Result<Self, QuicError> {
        let server_config = make_server_config(&state.identity, verifier, max_idle_timeout)?;
        let endpoint = Endpoint::server(server_config, addr)?;

        tracing::info!(local_addr = %endpoint.local_addr()?, "QUIC listener bound successfully");

        Ok(Self { endpoint, state })
    }

    /// Returns the local socket address this listener is bound to.
    pub fn local_addr(&self) -> Result<SocketAddr, std::io::Error> {
        self.endpoint.local_addr()
    }

    /// Returns a reference to the underlying Quinn endpoint.
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Runs the accept loop, accepting incoming authenticated connections and dispatching
    /// them to the stream demultiplexer.
    pub async fn run(self) -> Result<(), QuicError> {
        tracing::info!("QUIC server accept loop started");

        while let Some(incoming) = self.endpoint.accept().await {
            let state = self.state.clone();

            tokio::spawn(async move {
                tracing::debug!(remote_addr = %incoming.remote_address(), "Incoming connection handshake started");

                match incoming.await {
                    Ok(connection) => {
                        let paired_name = state
                            .paired_device
                            .read()
                            .await
                            .as_ref()
                            .map(|d| d.name.clone())
                            .unwrap_or_else(|| "paired-companion".to_string());

                        tracing::info!(
                            remote_addr = %connection.remote_address(),
                            device = %paired_name,
                            "Mutual TLS handshake successful"
                        );

                        if let Err(e) = handle_connection(connection, state, paired_name).await {
                            tracing::debug!(error = %e, "Connection handler terminated");
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Mutual TLS handshake rejected unauthenticated or invalid client");
                    }
                }
            });
        }

        tracing::info!("QUIC server accept loop terminated");
        Ok(())
    }

    /// Starts the QUIC server accept loop in a background asynchronous task.
    pub fn start(self) -> tokio::task::JoinHandle<Result<(), QuicError>> {
        tokio::spawn(async move { self.run().await })
    }

    /// Closes the server endpoint, terminating all active connections.
    pub fn close(&self) {
        self.endpoint.close(0u32.into(), b"server shutdown");
    }
}
