//! Mutual TLS 1.3 cryptography, self-signed Ed25519 certificates, and certificate verifiers.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use ed25519_dalek::pkcs8::EncodePrivateKey;
use ed25519_dalek::{SigningKey, VerifyingKey};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{DigitallySignedStruct, SignatureScheme};

use crate::quic::QuicError;

/// ALPN protocol identifiers for Omnilink QUIC transport.
pub const ALPN_OMNILINK: &[u8] = b"omnilink";


/// Installs the default process-wide `ring` crypto provider if not already installed.
pub fn init_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Generates a self-signed Ed25519 TLS 1.3 certificate for the daemon from its identity keypair.
pub fn generate_daemon_certificate(
    signing_key: &SigningKey,
) -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>), QuicError> {
    init_crypto_provider();

    let secret_doc = signing_key
        .to_pkcs8_der()
        .map_err(|e| QuicError::CertificateGeneration(e.to_string()))?;
    let pkcs8_der = PrivatePkcs8KeyDer::from(secret_doc.as_bytes().to_vec());
    let key_pair = rcgen::KeyPair::from_pkcs8_der_and_sign_algo(&pkcs8_der, &rcgen::PKCS_ED25519)
        .map_err(|e| QuicError::CertificateGeneration(e.to_string()))?;

    let mut params = rcgen::CertificateParams::default();
    params.subject_alt_names = vec![
        rcgen::SanType::DnsName(
            rcgen::Ia5String::try_from("localhost")
                .map_err(|e| QuicError::CertificateGeneration(e.to_string()))?,
        ),
        rcgen::SanType::IpAddress(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)),
        rcgen::SanType::IpAddress(std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)),
    ];
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "omnilink-daemon");

    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| QuicError::CertificateGeneration(e.to_string()))?;

    let cert_der = cert.der().clone();
    let priv_key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_pair.serialize_der()));

    Ok((cert_der, priv_key))
}

/// Generates a self-signed Ed25519 TLS 1.3 certificate for an Android companion client.
pub fn generate_client_certificate(
    signing_key: &SigningKey,
) -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>), QuicError> {
    init_crypto_provider();

    let secret_doc = signing_key
        .to_pkcs8_der()
        .map_err(|e| QuicError::CertificateGeneration(e.to_string()))?;
    let pkcs8_der = PrivatePkcs8KeyDer::from(secret_doc.as_bytes().to_vec());
    let key_pair = rcgen::KeyPair::from_pkcs8_der_and_sign_algo(&pkcs8_der, &rcgen::PKCS_ED25519)
        .map_err(|e| QuicError::CertificateGeneration(e.to_string()))?;

    let mut params = rcgen::CertificateParams::default();
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "omnilink-companion");

    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| QuicError::CertificateGeneration(e.to_string()))?;

    let cert_der = cert.der().clone();
    let priv_key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_pair.serialize_der()));

    Ok((cert_der, priv_key))
}

/// Helper function to read a single ASN.1 DER Tag-Length-Value from a byte slice.
fn read_der_tlv<'a>(input: &mut &'a [u8]) -> Result<(u8, &'a [u8]), QuicError> {
    if input.is_empty() {
        return Err(QuicError::InvalidCertificate(
            "Unexpected end of DER input".to_string(),
        ));
    }
    let tag = input[0];
    *input = &input[1..];
    if input.is_empty() {
        return Err(QuicError::InvalidCertificate(
            "Truncated DER length".to_string(),
        ));
    }
    let len_byte = input[0];
    *input = &input[1..];
    let len = if len_byte < 0x80 {
        len_byte as usize
    } else if len_byte == 0x81 {
        if input.is_empty() {
            return Err(QuicError::InvalidCertificate(
                "Truncated DER length".to_string(),
            ));
        }
        let l = input[0] as usize;
        *input = &input[1..];
        l
    } else if len_byte == 0x82 {
        if input.len() < 2 {
            return Err(QuicError::InvalidCertificate(
                "Truncated DER length".to_string(),
            ));
        }
        let l = u16::from_be_bytes([input[0], input[1]]) as usize;
        *input = &input[2..];
        l
    } else {
        return Err(QuicError::InvalidCertificate(format!(
            "Unsupported DER length format: 0x{:02x}",
            len_byte
        )));
    };

    if input.len() < len {
        return Err(QuicError::InvalidCertificate(
            "DER value shorter than specified length".to_string(),
        ));
    }
    let value = &input[..len];
    *input = &input[len..];
    Ok((tag, value))
}

/// Extracts the Ed25519 `VerifyingKey` from an X.509 DER-encoded certificate.
pub fn extract_ed25519_verifying_key(cert_der: &[u8]) -> Result<VerifyingKey, QuicError> {
    let mut input = cert_der;
    let (cert_tag, mut cert_val) = read_der_tlv(&mut input)?;
    if cert_tag != 0x30 {
        return Err(QuicError::InvalidCertificate(
            "Outer certificate is not a SEQUENCE".to_string(),
        ));
    }

    let (tbs_tag, mut tbs_val) = read_der_tlv(&mut cert_val)?;
    if tbs_tag != 0x30 {
        return Err(QuicError::InvalidCertificate(
            "TBSCertificate is not a SEQUENCE".to_string(),
        ));
    }

    // Inside TBSCertificate:
    // Optional Version [0]
    if !tbs_val.is_empty() && tbs_val[0] == 0xa0 {
        let _ = read_der_tlv(&mut tbs_val)?;
    }

    // Serial number (0x02)
    let _ = read_der_tlv(&mut tbs_val)?;
    // Signature Algorithm (0x30)
    let _ = read_der_tlv(&mut tbs_val)?;
    // Issuer (0x30)
    let _ = read_der_tlv(&mut tbs_val)?;
    // Validity (0x30)
    let _ = read_der_tlv(&mut tbs_val)?;
    // Subject (0x30)
    let _ = read_der_tlv(&mut tbs_val)?;

    // SubjectPublicKeyInfo (0x30)
    let (spki_tag, mut spki_val) = read_der_tlv(&mut tbs_val)?;
    if spki_tag != 0x30 {
        return Err(QuicError::InvalidCertificate(
            "SubjectPublicKeyInfo is not a SEQUENCE".to_string(),
        ));
    }

    // Inside SubjectPublicKeyInfo:
    // AlgorithmIdentifier (0x30)
    let (alg_tag, alg_val) = read_der_tlv(&mut spki_val)?;
    if alg_tag != 0x30 {
        return Err(QuicError::InvalidCertificate(
            "SPKI AlgorithmIdentifier is not a SEQUENCE".to_string(),
        ));
    }

    // Ed25519 OID is 1.3.101.112 -> encoded as 0x06 0x03 0x2b 0x65 0x70
    const ED25519_OID_DER: [u8; 5] = [0x06, 0x03, 0x2b, 0x65, 0x70];
    if alg_val != ED25519_OID_DER {
        return Err(QuicError::InvalidCertificate(
            "Certificate public key algorithm is not Ed25519".to_string(),
        ));
    }

    // SubjectPublicKey BIT STRING (0x03)
    let (bit_tag, bit_val) = read_der_tlv(&mut spki_val)?;
    if bit_tag != 0x03 {
        return Err(QuicError::InvalidCertificate(
            "SPKI subjectPublicKey is not a BIT STRING".to_string(),
        ));
    }

    // First byte of BIT STRING is unused bits (must be 0), followed by 32 key bytes
    if bit_val.len() != 33 || bit_val[0] != 0 {
        return Err(QuicError::InvalidCertificate(
            "Invalid Ed25519 public key BIT STRING length or unused bits".to_string(),
        ));
    }

    let key_bytes: [u8; 32] = bit_val[1..33]
        .try_into()
        .map_err(|_| QuicError::InvalidCertificate("Invalid key slice".to_string()))?;

    VerifyingKey::from_bytes(&key_bytes)
        .map_err(|e| QuicError::InvalidCertificate(e.to_string()))
}

/// Client certificate verifier that enforces mutual TLS 1.3 authentication.
/// Validates that the client presents a valid Ed25519 certificate matching
/// the known paired public key.
#[derive(Debug)]
pub struct Ed25519ClientCertVerifier {
    paired_key: Arc<RwLock<Option<VerifyingKey>>>,
}

impl Ed25519ClientCertVerifier {
    /// Creates a verifier with a static expected paired public key.
    pub fn new(expected_public_key: VerifyingKey) -> Self {
        Self {
            paired_key: Arc::new(RwLock::new(Some(expected_public_key))),
        }
    }

    /// Creates a verifier with a dynamically updatable paired public key container.
    pub fn new_dynamic(paired_key: Arc<RwLock<Option<VerifyingKey>>>) -> Self {
        Self { paired_key }
    }

    /// Creates an empty verifier that rejects all incoming connections until a device is paired.
    pub fn empty() -> Self {
        Self {
            paired_key: Arc::new(RwLock::new(None)),
        }
    }

    /// Updates the expected paired key.
    pub fn set_paired_key(&self, key: Option<VerifyingKey>) {
        if let Ok(mut lock) = self.paired_key.write() {
            *lock = key;
        }
    }
}

impl ClientCertVerifier for Ed25519ClientCertVerifier {
    fn root_hint_subjects(&self) -> &[rustls::DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        let extracted_key = extract_ed25519_verifying_key(end_entity.as_ref()).map_err(|e| {
            tracing::warn!(error = %e, "Client certificate rejected: failed to extract Ed25519 key");
            rustls::Error::InvalidCertificate(rustls::CertificateError::BadEncoding)
        })?;

        let expected_key = match self.paired_key.read() {
            Ok(lock) => *lock,
            Err(_) => {
                return Err(rustls::Error::InvalidCertificate(
                    rustls::CertificateError::UnknownIssuer,
                ))
            }
        };

        match expected_key {
            Some(expected) if expected == extracted_key => {
                tracing::debug!("Client certificate validated successfully against paired Ed25519 key");
                Ok(ClientCertVerified::assertion())
            }
            Some(_) => {
                tracing::warn!("Client certificate rejected: untrusted / unknown Ed25519 public key");
                Err(rustls::Error::InvalidCertificate(
                    rustls::CertificateError::UnknownIssuer,
                ))
            }
            None => {
                tracing::warn!("Client certificate rejected: no paired device configured");
                Err(rustls::Error::InvalidCertificate(
                    rustls::CertificateError::UnknownIssuer,
                ))
            }
        }
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::PeerIncompatible(
            rustls::PeerIncompatible::Tls12NotOffered,
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }

    fn offer_client_auth(&self) -> bool {
        true
    }

    fn client_auth_mandatory(&self) -> bool {
        true
    }
}

/// Client-side server certificate verifier that validates the daemon's certificate
/// against the known daemon Ed25519 public key from pairing.
#[derive(Debug)]
pub struct Ed25519ServerCertVerifier {
    expected_daemon_key: VerifyingKey,
}

impl Ed25519ServerCertVerifier {
    /// Creates a new server certificate verifier with the expected daemon public key.
    pub fn new(expected_daemon_key: VerifyingKey) -> Self {
        Self {
            expected_daemon_key,
        }
    }
}

impl ServerCertVerifier for Ed25519ServerCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let extracted_key = extract_ed25519_verifying_key(end_entity.as_ref()).map_err(|e| {
            tracing::warn!(error = %e, "Server certificate rejected: failed to extract Ed25519 key");
            rustls::Error::InvalidCertificate(rustls::CertificateError::BadEncoding)
        })?;

        if extracted_key == self.expected_daemon_key {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::UnknownIssuer,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::PeerIncompatible(
            rustls::PeerIncompatible::Tls12NotOffered,
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }
}

/// Builds a `quinn::ServerConfig` enforcing mutual TLS 1.3 with Ed25519 certificates.
pub fn make_server_config(
    signing_key: &SigningKey,
    client_cert_verifier: Arc<dyn ClientCertVerifier>,
    max_idle_timeout: Option<Duration>,
) -> Result<quinn::ServerConfig, QuicError> {
    init_crypto_provider();

    let (cert_der, priv_key) = generate_daemon_certificate(signing_key)?;

    let mut rustls_config = rustls::ServerConfig::builder()
        .with_client_cert_verifier(client_cert_verifier)
        .with_single_cert(vec![cert_der], priv_key)
        .map_err(QuicError::Rustls)?;

    rustls_config.alpn_protocols = vec![ALPN_OMNILINK.to_vec()];

    let quic_server_config = quinn::crypto::rustls::QuicServerConfig::try_from(rustls_config)
        .map_err(|e| QuicError::CertificateGeneration(e.to_string()))?;

    let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(quic_server_config));

    let mut transport_config = quinn::TransportConfig::default();
    if let Some(timeout) = max_idle_timeout {
        if let Ok(idle_timeout) = quinn::IdleTimeout::try_from(timeout) {
            transport_config.max_idle_timeout(Some(idle_timeout));
        }
    }
    server_config.transport_config(Arc::new(transport_config));

    Ok(server_config)
}

/// Builds a `quinn::ClientConfig` configured for mutual TLS 1.3 with the daemon.
pub fn make_client_config(
    client_signing_key: &SigningKey,
    expected_daemon_public_key: &VerifyingKey,
    max_idle_timeout: Option<Duration>,
) -> Result<quinn::ClientConfig, QuicError> {
    init_crypto_provider();

    let (client_cert, client_priv_key) = generate_client_certificate(client_signing_key)?;

    let server_cert_verifier = Arc::new(Ed25519ServerCertVerifier::new(*expected_daemon_public_key));

    let mut rustls_config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(server_cert_verifier)
        .with_client_auth_cert(vec![client_cert], client_priv_key)
        .map_err(QuicError::Rustls)?;

    rustls_config.alpn_protocols = vec![ALPN_OMNILINK.to_vec()];

    let quic_client_config = quinn::crypto::rustls::QuicClientConfig::try_from(rustls_config)
        .map_err(|e| QuicError::CertificateGeneration(e.to_string()))?;

    let mut client_config = quinn::ClientConfig::new(Arc::new(quic_client_config));

    let mut transport_config = quinn::TransportConfig::default();
    if let Some(timeout) = max_idle_timeout {
        if let Ok(idle_timeout) = quinn::IdleTimeout::try_from(timeout) {
            transport_config.max_idle_timeout(Some(idle_timeout));
        }
    }
    client_config.transport_config(Arc::new(transport_config));

    Ok(client_config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quic_crypto_daemon_cert_generation_and_key_extraction() {
        let mut csprng = rand::rngs::OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        let verifying_key = signing_key.verifying_key();

        let (cert_der, _priv_key) =
            generate_daemon_certificate(&signing_key).expect("Failed to generate daemon cert");

        let extracted_key = extract_ed25519_verifying_key(cert_der.as_ref())
            .expect("Failed to extract verifying key");

        assert_eq!(verifying_key, extracted_key);
    }

    #[test]
    fn test_quic_crypto_client_cert_generation_and_key_extraction() {
        let mut csprng = rand::rngs::OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        let verifying_key = signing_key.verifying_key();

        let (cert_der, _priv_key) =
            generate_client_certificate(&signing_key).expect("Failed to generate client cert");

        let extracted_key = extract_ed25519_verifying_key(cert_der.as_ref())
            .expect("Failed to extract verifying key");

        assert_eq!(verifying_key, extracted_key);
    }

    #[test]
    fn test_quic_crypto_tampered_cert_rejected() {
        // Completely invalid input
        let garbage = vec![0x30, 0x05, 0x01, 0x02, 0x03];
        assert!(extract_ed25519_verifying_key(&garbage).is_err());

        // Truncated input
        let truncated = vec![0x30, 0x82];
        assert!(extract_ed25519_verifying_key(&truncated).is_err());

        // Empty input
        assert!(extract_ed25519_verifying_key(&[]).is_err());

        // Embedded unanchored prefix inside arbitrary DER payload must be rejected
        let mut fake_cert = vec![0x30, 0x50]; // outer sequence header
        fake_cert.extend_from_slice(&[0x04, 0x20]); // OCTET STRING
        fake_cert.extend_from_slice(&[0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00]); // SPKI prefix
        fake_cert.extend_from_slice(&[0xaa; 32]); // Fake key bytes
        assert!(extract_ed25519_verifying_key(&fake_cert).is_err());
    }

    #[test]
    fn test_quic_crypto_client_verifier_rejects_empty_and_unknown() {
        init_crypto_provider();

        let mut csprng = rand::rngs::OsRng;
        let paired_signing_key = SigningKey::generate(&mut csprng);
        let unknown_signing_key = SigningKey::generate(&mut csprng);

        let (paired_cert, _) = generate_client_certificate(&paired_signing_key).unwrap();
        let (unknown_cert, _) = generate_client_certificate(&unknown_signing_key).unwrap();

        let verifier = Ed25519ClientCertVerifier::new(paired_signing_key.verifying_key());

        // Paired cert should verify
        assert!(verifier
            .verify_client_cert(&paired_cert, &[], UnixTime::since_unix_epoch(Duration::ZERO))
            .is_ok());

        // Unknown cert should be rejected
        assert!(verifier
            .verify_client_cert(&unknown_cert, &[], UnixTime::since_unix_epoch(Duration::ZERO))
            .is_err());

        // Empty verifier should reject everything
        let empty_verifier = Ed25519ClientCertVerifier::empty();
        assert!(empty_verifier
            .verify_client_cert(&paired_cert, &[], UnixTime::since_unix_epoch(Duration::ZERO))
            .is_err());
    }
}
