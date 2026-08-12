use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize, de::Error as _};
use thiserror::Error;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::ServerTtl;

pub const MAX_MLS_MESSAGE_BYTES: usize = 256 * 1024;
const MAX_MLS_MESSAGE_BASE64_CHARS: usize = MAX_MLS_MESSAGE_BYTES.div_ceil(3) * 4;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct EventId(Uuid);

impl EventId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    #[must_use]
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for EventId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpaqueMlsMessage(String);

impl OpaqueMlsMessage {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, OpaqueMlsMessageError> {
        if bytes.is_empty() {
            return Err(OpaqueMlsMessageError::Empty);
        }
        if bytes.len() > MAX_MLS_MESSAGE_BYTES {
            return Err(OpaqueMlsMessageError::TooLarge);
        }
        Ok(Self(URL_SAFE_NO_PAD.encode(bytes)))
    }

    pub fn decode(&self) -> Result<Vec<u8>, OpaqueMlsMessageError> {
        URL_SAFE_NO_PAD
            .decode(&self.0)
            .map_err(|_| OpaqueMlsMessageError::InvalidEncoding)
    }

    #[must_use]
    pub fn encoded(&self) -> &str {
        &self.0
    }

    fn from_encoded(encoded: String) -> Result<Self, OpaqueMlsMessageError> {
        if encoded.is_empty() {
            return Err(OpaqueMlsMessageError::Empty);
        }
        if encoded.len() > MAX_MLS_MESSAGE_BASE64_CHARS {
            return Err(OpaqueMlsMessageError::TooLarge);
        }
        let decoded = URL_SAFE_NO_PAD
            .decode(&encoded)
            .map_err(|_| OpaqueMlsMessageError::InvalidEncoding)?;
        if decoded.is_empty() || decoded.len() > MAX_MLS_MESSAGE_BYTES {
            return Err(OpaqueMlsMessageError::TooLarge);
        }
        if URL_SAFE_NO_PAD.encode(decoded) != encoded {
            return Err(OpaqueMlsMessageError::InvalidEncoding);
        }
        Ok(Self(encoded))
    }
}

impl Serialize for OpaqueMlsMessage {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for OpaqueMlsMessage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        Self::from_encoded(encoded).map_err(D::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum OpaqueMlsMessageError {
    #[error("MLS message must not be empty")]
    Empty,
    #[error("MLS message exceeds the transport limit")]
    TooLarge,
    #[error("MLS message is not canonical base64url")]
    InvalidEncoding,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SubmitEvent {
    pub event_id: EventId,
    pub community_id: Uuid,
    pub channel_id: Uuid,
    pub sender_device_id: Uuid,
    pub sender_counter: u64,
    pub mls_message: OpaqueMlsMessage,
    pub ttl_seconds: ServerTtl,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedEvent {
    pub event_id: EventId,
    pub channel_id: Uuid,
    pub sequence: u64,
    #[serde(with = "time::serde::rfc3339")]
    pub accepted_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveredEvent {
    pub receipt: AcceptedEvent,
    pub community_id: Uuid,
    pub sender_device_id: Uuid,
    pub sender_counter: u64,
    pub mls_message: OpaqueMlsMessage,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HealthResponse {
    pub status: String,
    pub service: String,
    pub storage_mode: String,
    pub max_server_ttl_seconds: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_message_round_trips_as_canonical_base64url() {
        let message = OpaqueMlsMessage::from_bytes(b"binary MLS\0message").unwrap();
        let json = serde_json::to_string(&message).unwrap();

        assert_eq!(message.decode().unwrap(), b"binary MLS\0message");
        assert_eq!(
            serde_json::from_str::<OpaqueMlsMessage>(&json).unwrap(),
            message
        );
        assert!(!json.contains("binary MLS"));
    }

    #[test]
    fn malformed_empty_and_oversized_messages_are_rejected() {
        assert_eq!(
            OpaqueMlsMessage::from_bytes(&[]),
            Err(OpaqueMlsMessageError::Empty)
        );
        assert_eq!(
            OpaqueMlsMessage::from_bytes(&vec![0; MAX_MLS_MESSAGE_BYTES + 1]),
            Err(OpaqueMlsMessageError::TooLarge)
        );
        assert!(serde_json::from_str::<OpaqueMlsMessage>("\"not+base64\"").is_err());
        assert!(serde_json::from_str::<OpaqueMlsMessage>("\"\"").is_err());
    }

    #[test]
    fn submit_event_rejects_legacy_or_unknown_fields() {
        let value = serde_json::json!({
            "event_id": Uuid::new_v4(),
            "community_id": Uuid::new_v4(),
            "channel_id": Uuid::new_v4(),
            "sender_device_id": Uuid::new_v4(),
            "sender_counter": 1,
            "previous_event_hash": null,
            "mls_message": OpaqueMlsMessage::from_bytes(b"opaque").unwrap(),
            "ttl_seconds": 300
        });

        assert!(serde_json::from_value::<SubmitEvent>(value).is_err());
    }
}
