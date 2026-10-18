//! Protocol errors and result types.

use thiserror::Error;

/// Errors that can occur during protocol serialization, deserialization, or validation.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Deserialization error: {0}")]
    Deserialization(String),

    #[error("Invalid packet header: {0}")]
    InvalidHeader(String),

    #[error("Unknown packet type: {0}")]
    UnknownPacketType(u8),
}

/// A specialized `Result` type for protocol operations.
pub type Result<T> = std::result::Result<T, ProtocolError>;

impl From<rmp_serde::encode::Error> for ProtocolError {
    fn from(err: rmp_serde::encode::Error) -> Self {
        ProtocolError::Serialization(err.to_string())
    }
}

impl From<rmp_serde::decode::Error> for ProtocolError {
    fn from(err: rmp_serde::decode::Error) -> Self {
        ProtocolError::Deserialization(err.to_string())
    }
}

impl From<std::string::FromUtf8Error> for ProtocolError {
    fn from(err: std::string::FromUtf8Error) -> Self {
        ProtocolError::Deserialization(format!("Invalid UTF-8: {err}"))
    }
}

impl From<std::str::Utf8Error> for ProtocolError {
    fn from(err: std::str::Utf8Error) -> Self {
        ProtocolError::Deserialization(format!("Invalid UTF-8: {err}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = ProtocolError::Serialization("failed to serialize".to_string());
        assert_eq!(err.to_string(), "Serialization error: failed to serialize");

        let err = ProtocolError::Deserialization("invalid json".to_string());
        assert_eq!(err.to_string(), "Deserialization error: invalid json");

        let err = ProtocolError::InvalidHeader("magic byte mismatch".to_string());
        assert_eq!(
            err.to_string(),
            "Invalid packet header: magic byte mismatch"
        );

        let err = ProtocolError::UnknownPacketType(42);
        assert_eq!(err.to_string(), "Unknown packet type: 42");
    }
}
