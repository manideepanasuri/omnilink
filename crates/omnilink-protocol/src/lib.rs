//! Omnilink Protocol library.
//!
//! Provides data structures, packet codecs, and wire protocol definitions
//! for communication between Linux desktop components and the Android companion.

pub mod error;
pub mod packet;
pub mod pairing;
pub mod types;

pub use error::{ProtocolError, Result};
pub use packet::{
    decode_clipboard_binary, encode_clipboard_binary, from_json, from_msgpack, to_json, to_msgpack,
    ClipboardPacket, CommandAction, ControlPacket, NotificationAction,
    NotificationDismissPacket, NotificationPacket, NotificationReplyPacket,
    NotificationStreamPacket, Packet,
};
pub use pairing::{is_valid_base64, parse_address, PairingPayload};
pub use types::DeviceStatus;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_protocol_reexports() {
        let err = ProtocolError::UnknownPacketType(1);
        assert_eq!(err.to_string(), "Unknown packet type: 1");

        let ping = ControlPacket::Ping { timestamp: 1000 };
        let msgpack = to_msgpack(&ping).expect("Serialization failed");
        let parsed: ControlPacket = from_msgpack(&msgpack).expect("Deserialization failed");
        assert_eq!(ping, parsed);
    }
}
