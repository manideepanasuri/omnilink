//! Omnilink daemon entry point.

use anyhow::Result;
use base64::Engine;
use tracing::info;

/// Returns the package version of the daemon.
pub fn daemon_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    info!(version = daemon_version(), "Starting omnilink-daemon");

    let identity = omnilink_daemon::identity::load_or_generate_identity(None)?;
    let verifying_key = identity.verifying_key();
    let pk_base64 = base64::engine::general_purpose::STANDARD.encode(verifying_key.to_bytes());
    info!(public_key = %pk_base64, "Loaded Ed25519 identity keypair");

    info!("Omnilink daemon initialized successfully");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_daemon_runtime_initialization() {
        assert_eq!(daemon_version(), "0.1.0");
    }
}
