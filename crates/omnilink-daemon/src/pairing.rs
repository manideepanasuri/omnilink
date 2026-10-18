//! Terminal QR pairing generator and device onboarding credentials.

use base64::Engine;
use ed25519_dalek::SigningKey;
pub use omnilink_protocol::pairing::PairingPayload;
use std::fmt::Write as _;
use thiserror::Error;

pub use omnilink_protocol::pairing::{is_valid_base64, parse_address};

/// Default port for the Omnilink QUIC endpoint.
pub const DEFAULT_PORT: u16 = 42424;

/// Errors that can occur during pairing payload generation or QR rendering.
#[derive(Error, Debug)]
pub enum PairingError {
    /// Identity keystore error.
    #[error("Keystore error: {0}")]
    Identity(#[from] crate::identity::IdentityError),

    /// JSON serialization error.
    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    /// QR code generation error.
    #[error("QR generation error: {0}")]
    Qr(#[from] qrcode::types::QrError),

    /// Pairing payload failed specification validation.
    #[error("Generated pairing payload failed validation: {0}")]
    InvalidPayload(String),
}

/// Retrieves the local hostname for the pairing identity.
pub fn get_hostname() -> String {
    let mut buf = [0u8; 256];
    #[cfg(unix)]
    unsafe {
        let res = libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len());
        if res == 0 {
            if let Ok(s) = std::ffi::CStr::from_bytes_until_nul(&buf) {
                if let Ok(str_slice) = s.to_str() {
                    let trimmed = str_slice.trim();
                    if !trimmed.is_empty() {
                        return trimmed.to_string();
                    }
                }
            }
        }
    }

    if let Ok(h) = std::fs::read_to_string("/etc/hostname") {
        let trimmed = h.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    if let Ok(h) = std::env::var("HOSTNAME") {
        let trimmed = h.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    "omnilink-host".to_string()
}

/// Discovers non-loopback IPv4 and IPv6 network interface addresses.
///
/// Formats IPv4 as `<ip>:<port>` and IPv6 as `[<ip>]:<port>`.
/// If no active non-loopback interfaces are detected, returns `["127.0.0.1:<port>"]`.
pub fn discover_local_addresses(port: u16) -> Vec<String> {
    let mut addrs = Vec::new();

    #[cfg(unix)]
    unsafe {
        let mut ifaddr: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut ifaddr) == 0 && !ifaddr.is_null() {
            let mut curr = ifaddr;
            while !curr.is_null() {
                let ifa = *curr;
                if !ifa.ifa_addr.is_null() {
                    let flags = ifa.ifa_flags as i32;
                    let is_up = (flags & libc::IFF_UP) != 0;
                    let is_loopback = (flags & libc::IFF_LOOPBACK) != 0;

                    if is_up && !is_loopback {
                        let family = (*ifa.ifa_addr).sa_family as i32;
                        if family == libc::AF_INET {
                            let sa = *(ifa.ifa_addr as *const libc::sockaddr_in);
                            let ip = std::net::Ipv4Addr::from(sa.sin_addr.s_addr.to_ne_bytes());
                            if !ip.is_loopback() && !ip.is_unspecified() && !ip.is_broadcast() {
                                let formatted = format!("{}:{}", ip, port);
                                if !addrs.contains(&formatted) {
                                    addrs.push(formatted);
                                }
                            }
                        } else if family == libc::AF_INET6 {
                            let sa = *(ifa.ifa_addr as *const libc::sockaddr_in6);
                            let ip = std::net::Ipv6Addr::from(sa.sin6_addr.s6_addr);
                            if !ip.is_loopback() && !ip.is_unspecified() && !ip.is_multicast() {
                                let formatted = format!("[{}]:{}", ip, port);
                                if !addrs.contains(&formatted) {
                                    addrs.push(formatted);
                                }
                            }
                        }
                    }
                }
                curr = ifa.ifa_next;
            }
            libc::freeifaddrs(ifaddr);
        }
    }

    if addrs.is_empty() {
        addrs.push(format!("127.0.0.1:{}", port));
    }

    addrs
}

/// Generates a cryptographically secure 16-byte random token as a 32-character hex string.
pub fn generate_pairing_token() -> String {
    let mut bytes = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut bytes);
    let mut token = String::with_capacity(32);
    for b in bytes {
        let _ = write!(&mut token, "{:02x}", b);
    }
    token
}

/// Generates a valid pairing payload matching `docs/PROTOCOL.md` Section 2.
pub fn generate_pairing_payload(
    signing_key: &SigningKey,
    port: u16,
) -> Result<PairingPayload, PairingError> {
    let name = get_hostname();
    let verifying_key = signing_key.verifying_key();
    let pk_bytes = verifying_key.to_bytes();
    let pk_base64 = base64::engine::general_purpose::STANDARD.encode(pk_bytes);
    let addresses = discover_local_addresses(port);
    let token = generate_pairing_token();

    let payload = PairingPayload::new(name, pk_base64, addresses, token);
    if !payload.is_valid() {
        return Err(PairingError::InvalidPayload(
            "Payload failed structural validation checks".to_string(),
        ));
    }

    Ok(payload)
}

/// Renders a compact terminal QR code using `qrcode::render::unicode::Dense1x2`.
pub fn render_qr_code(payload_json: &str) -> Result<String, PairingError> {
    let code = qrcode::QrCode::new(payload_json.as_bytes())?;
    let rendered = code.render::<qrcode::render::unicode::Dense1x2>().build();
    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_hostname() {
        let hostname = get_hostname();
        assert!(!hostname.is_empty());
        assert!(!hostname.trim().is_empty());
    }

    #[test]
    fn test_discover_local_addresses() {
        let port = 42424;
        let addrs = discover_local_addresses(port);
        assert!(!addrs.is_empty());
        for addr in &addrs {
            assert!(
                addr.ends_with(":42424"),
                "Address {} does not end with :42424",
                addr
            );
            assert!(
                !addr.starts_with("127.0.0.1") || addrs.len() == 1,
                "Loopback address should only be present as sole fallback"
            );
        }
    }

    #[test]
    fn test_generate_pairing_token() {
        let t1 = generate_pairing_token();
        let t2 = generate_pairing_token();
        assert_eq!(t1.len(), 32);
        assert_eq!(t2.len(), 32);
        assert_ne!(t1, t2);
        assert!(t1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_generate_pairing_payload() {
        let mut csprng = rand::rngs::OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        let payload = generate_pairing_payload(&signing_key, 42424)
            .expect("Failed to generate pairing payload");

        assert!(payload.is_valid());
        assert_eq!(payload.version, 1);
        assert!(!payload.name.is_empty());
        assert_eq!(payload.token.len(), 32);

        let expected_pk =
            base64::engine::general_purpose::STANDARD.encode(signing_key.verifying_key().to_bytes());
        assert_eq!(payload.public_key_base64, expected_pk);
        assert!(!payload.addresses.is_empty());
    }

    #[test]
    fn test_render_qr_code() {
        let mut csprng = rand::rngs::OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        let payload = generate_pairing_payload(&signing_key, 42424)
            .expect("Failed to generate pairing payload");
        let json_str = payload.to_json().expect("Failed to serialize to JSON");

        let rendered = render_qr_code(&json_str).expect("Failed to render QR code");
        assert!(!rendered.is_empty());
        assert!(rendered.contains('▀') || rendered.contains('█') || rendered.contains('▄'));
        assert!(rendered.lines().count() > 5);
    }
}
