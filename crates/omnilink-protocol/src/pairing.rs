//! Pairing payload wire schema, serialization, and validation.

use base64::Engine;
use serde::{Deserialize, Serialize};

/// Pairing payload exchanged between desktop and Android companion via QR code.
///
/// Encoded as compact JSON conforming to Omnilink Network Protocol Specification Section 2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingPayload {
    /// Protocol schema version (must be 1).
    #[serde(rename = "v")]
    pub version: u32,

    /// Friendly host name (e.g. hostname).
    pub name: String,

    /// Base64 encoded raw 32-byte Ed25519 public key.
    #[serde(rename = "pk")]
    pub public_key_base64: String,

    /// Reachable endpoints formatted as "ip:port" or "[ipv6]:port".
    #[serde(rename = "addrs")]
    pub addresses: Vec<String>,

    /// One-time random pairing secret token.
    pub token: String,
}

impl PairingPayload {
    /// Creates a new pairing payload with default version 1.
    pub fn new(
        name: impl Into<String>,
        public_key_base64: impl Into<String>,
        addresses: Vec<String>,
        token: impl Into<String>,
    ) -> Self {
        Self {
            version: 1,
            name: name.into(),
            public_key_base64: public_key_base64.into(),
            addresses,
            token: token.into(),
        }
    }

    /// Serializes this payload to a compact JSON string.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Serializes this payload to a formatted JSON string.
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserializes a pairing payload from a JSON string.
    pub fn from_json(json_str: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json_str)
    }

    /// Validates payload fields according to protocol specification:
    /// - version == 1
    /// - name is not blank
    /// - public_key_base64 is not blank and is valid non-empty Base64
    /// - addresses contains at least one non-blank address
    /// - token is not blank
    pub fn is_valid(&self) -> bool {
        if self.version != 1 {
            return false;
        }
        if self.name.trim().is_empty() {
            return false;
        }
        if self.public_key_base64.trim().is_empty() || !is_valid_base64(&self.public_key_base64) {
            return false;
        }
        if self.addresses.is_empty() || !self.addresses.iter().any(|a| !a.trim().is_empty()) {
            return false;
        }
        if self.token.trim().is_empty() {
            return false;
        }
        true
    }

    /// Extracts the primary non-blank host address and port from `addresses`.
    /// Defaults port to 42424 if unspecified or invalid.
    pub fn primary_address_and_port(&self) -> Option<(String, u16)> {
        let addr = self.addresses.iter().find(|a| !a.trim().is_empty())?;
        Some(parse_address(addr))
    }
}

/// Validates whether a given string is valid non-empty Base64 (standard or URL-safe, padded or unpadded).
pub fn is_valid_base64(str: &str) -> bool {
    let trimmed = str.trim();
    if trimmed.is_empty() {
        return false;
    }
    use base64::engine::{general_purpose, DecodePaddingMode, GeneralPurpose};
    use base64::alphabet;

    const STANDARD_LENIENT: GeneralPurpose = GeneralPurpose::new(
        &alphabet::STANDARD,
        general_purpose::GeneralPurposeConfig::new()
            .with_decode_allow_trailing_bits(true)
            .with_decode_padding_mode(DecodePaddingMode::Indifferent),
    );

    const URL_SAFE_LENIENT: GeneralPurpose = GeneralPurpose::new(
        &alphabet::URL_SAFE,
        general_purpose::GeneralPurposeConfig::new()
            .with_decode_allow_trailing_bits(true)
            .with_decode_padding_mode(DecodePaddingMode::Indifferent),
    );

    if let Ok(decoded) = STANDARD_LENIENT.decode(trimmed) {
        return !decoded.is_empty();
    }
    if let Ok(decoded) = URL_SAFE_LENIENT.decode(trimmed) {
        return !decoded.is_empty();
    }
    false
}

/// Parses an address string into host and port.
/// Supports standard IPv4 (`"192.168.1.1:42424"`), bracketed IPv6 (`"[fe80::1]:42424"`),
/// hostnames, and fallback to port 42424 when missing or out of valid bounds.
pub fn parse_address(address_str: &str) -> (String, u16) {
    let trimmed = address_str.trim();
    if trimmed.starts_with('[') {
        if let Some(closing_bracket) = trimmed.find(']') {
            let host = &trimmed[1..closing_bracket];
            let rest = &trimmed[closing_bracket + 1..];
            let port_str = rest.strip_prefix(':').unwrap_or(rest);
            let port = port_str
                .parse::<u16>()
                .ok()
                .filter(|&p| p > 0)
                .unwrap_or(42424);
            return (host.to_string(), port);
        }
    }
    if let Some(colon_idx) = trimmed.rfind(':') {
        if colon_idx > 0 {
            let host = &trimmed[..colon_idx];
            let port_str = &trimmed[colon_idx + 1..];
            let port = port_str
                .parse::<u16>()
                .ok()
                .filter(|&p| p > 0)
                .unwrap_or(42424);
            return (host.to_string(), port);
        }
    }
    (trimmed.to_string(), 42424)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_JSON: &str = r#"{
      "v": 1,
      "name": "omarchy-workstation",
      "pk": "dGVzdC1wdWJsaWMta2V5LTMyei1ieXRlcw==",
      "addrs": ["192.168.1.100:42424", "[fe80::1]:42424"],
      "token": "one_time_pairing_secret_123"
    }"#;

    #[test]
    fn test_valid_json_deserialization_and_validation() {
        let payload = PairingPayload::from_json(VALID_JSON).expect("Failed to parse valid JSON");
        assert!(payload.is_valid());
        assert_eq!(payload.version, 1);
        assert_eq!(payload.name, "omarchy-workstation");
        assert_eq!(
            payload.public_key_base64,
            "dGVzdC1wdWJsaWMta2V5LTMyei1ieXRlcw=="
        );
        assert_eq!(
            payload.addresses,
            vec!["192.168.1.100:42424", "[fe80::1]:42424"]
        );
        assert_eq!(payload.token, "one_time_pairing_secret_123");

        let (host, port) = payload
            .primary_address_and_port()
            .expect("Expected primary address");
        assert_eq!(host, "192.168.1.100");
        assert_eq!(port, 42424);
    }

    #[test]
    fn test_url_safe_base64_public_key() {
        let payload = PairingPayload::new(
            "workstation",
            "dGVzdC11cmwtc2FmZS0_-_==".replace('=', ""),
            vec!["10.0.0.1:42424".to_string()],
            "token-xyz",
        );
        assert!(payload.is_valid());
    }

    #[test]
    fn test_malformed_json_handling() {
        let truncated = r#"{"v": 1, "name": "workstation", "pk":"#;
        assert!(PairingPayload::from_json(truncated).is_err());

        let plain_text = "https://omnilink.org/download";
        assert!(PairingPayload::from_json(plain_text).is_err());

        assert!(PairingPayload::from_json("").is_err());
    }

    #[test]
    fn test_missing_required_fields() {
        let missing_version =
            r#"{"name":"pc","pk":"dGVzdA==","addrs":["1.1.1.1:42424"],"token":"t"}"#;
        assert!(PairingPayload::from_json(missing_version).is_err());

        let missing_name = r#"{"v":1,"pk":"dGVzdA==","addrs":["1.1.1.1:42424"],"token":"t"}"#;
        assert!(PairingPayload::from_json(missing_name).is_err());

        let missing_pk = r#"{"v":1,"name":"pc","addrs":["1.1.1.1:42424"],"token":"t"}"#;
        assert!(PairingPayload::from_json(missing_pk).is_err());

        let missing_addrs = r#"{"v":1,"name":"pc","pk":"dGVzdA==","token":"t"}"#;
        assert!(PairingPayload::from_json(missing_addrs).is_err());

        let missing_token =
            r#"{"v":1,"name":"pc","pk":"dGVzdA==","addrs":["1.1.1.1:42424"]}"#;
        assert!(PairingPayload::from_json(missing_token).is_err());
    }

    #[test]
    fn test_invalid_version() {
        let mut payload = PairingPayload::new(
            "pc",
            "dGVzdA==",
            vec!["1.1.1.1:42424".to_string()],
            "token",
        );
        payload.version = 2;
        assert!(!payload.is_valid());

        payload.version = 0;
        assert!(!payload.is_valid());
    }

    #[test]
    fn test_blank_or_empty_name() {
        let mut payload = PairingPayload::new(
            "",
            "dGVzdA==",
            vec!["1.1.1.1:42424".to_string()],
            "token",
        );
        assert!(!payload.is_valid());

        payload.name = "   ".to_string();
        assert!(!payload.is_valid());
    }

    #[test]
    fn test_invalid_base64_public_key() {
        let mut payload = PairingPayload::new(
            "pc",
            "   ",
            vec!["1.1.1.1:42424".to_string()],
            "token",
        );
        assert!(!payload.is_valid());

        payload.public_key_base64 = "!!!not-valid-base64???".to_string();
        assert!(!payload.is_valid());
    }

    #[test]
    fn test_empty_or_blank_addresses() {
        let mut payload = PairingPayload::new(
            "pc",
            "dGVzdA==",
            vec![],
            "token",
        );
        assert!(!payload.is_valid());

        payload.addresses = vec!["".to_string(), "   ".to_string()];
        assert!(!payload.is_valid());
        assert!(payload.primary_address_and_port().is_none());
    }

    #[test]
    fn test_blank_or_empty_token() {
        let mut payload = PairingPayload::new(
            "pc",
            "dGVzdA==",
            vec!["1.1.1.1:42424".to_string()],
            "",
        );
        assert!(!payload.is_valid());

        payload.token = "   ".to_string();
        assert!(!payload.is_valid());
    }

    #[test]
    fn test_address_parsing_variations() {
        let (h1, p1) = parse_address("192.168.1.100:42424");
        assert_eq!(h1, "192.168.1.100");
        assert_eq!(p1, 42424);

        let (h2, p2) = parse_address("[fe80::1]:42424");
        assert_eq!(h2, "fe80::1");
        assert_eq!(p2, 42424);

        let (h3, p3) = parse_address("my-desktop.local:8080");
        assert_eq!(h3, "my-desktop.local");
        assert_eq!(p3, 8080);

        let (h4, p4) = parse_address("192.168.1.50");
        assert_eq!(h4, "192.168.1.50");
        assert_eq!(p4, 42424);

        let (h5, p5) = parse_address("192.168.1.50:99999");
        assert_eq!(h5, "192.168.1.50");
        assert_eq!(p5, 42424);
    }

    #[test]
    fn test_serialization_round_trip() {
        let original = PairingPayload::new(
            "roundtrip-pc",
            "dGVzdC1rZXk=",
            vec!["10.0.0.5:42424".to_string()],
            "secret123",
        );

        let json_string = original.to_json().expect("Serialization failed");
        let parsed = PairingPayload::from_json(&json_string).expect("Deserialization failed");
        assert_eq!(original, parsed);
        assert!(parsed.is_valid());
    }
}
