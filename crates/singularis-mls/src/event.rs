use serde::{Deserialize, Serialize};
use singularis_protocol::EventId;
use uuid::Uuid;

use crate::{MlsError, Result};

pub const EVENT_SCHEMA_VERSION: u16 = 1;
const MAX_EVENT_PAYLOAD_BYTES: usize = 96 * 1024;
const MAX_TEXT_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlaintextEvent {
    pub schema_version: u16,
    pub event_id: EventId,
    pub community_id: Uuid,
    pub channel_id: Uuid,
    pub sender_device_id: Uuid,
    pub sender_counter: u64,
    pub created_at_ms: i64,
    pub previous_event_hash: Option<[u8; 32]>,
    pub references: EventReferences,
    pub content: EventContent,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EventReferences {
    pub reply_to: Option<EventId>,
    pub replaces: Option<EventId>,
    pub deletes: Option<EventId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum EventContent {
    Text(TextContent),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TextContent {
    pub body: String,
}

impl PlaintextEvent {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn text(
        event_id: EventId,
        community_id: Uuid,
        channel_id: Uuid,
        sender_device_id: Uuid,
        sender_counter: u64,
        created_at_ms: i64,
        previous_event_hash: Option<[u8; 32]>,
        references: EventReferences,
        body: String,
    ) -> Self {
        Self {
            schema_version: EVENT_SCHEMA_VERSION,
            event_id,
            community_id,
            channel_id,
            sender_device_id,
            sender_counter,
            created_at_ms,
            previous_event_hash,
            references,
            content: EventContent::Text(TextContent { body }),
        }
    }

    pub fn encode_provisional_json(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let encoded = serde_json::to_vec(self)?;
        if encoded.len() > MAX_EVENT_PAYLOAD_BYTES {
            return Err(MlsError::InvalidPayload("event payload is too large"));
        }
        Ok(encoded)
    }

    pub fn decode_provisional_json(encoded: &[u8]) -> Result<Self> {
        if encoded.is_empty() || encoded.len() > MAX_EVENT_PAYLOAD_BYTES {
            return Err(MlsError::InvalidPayload("event payload size is invalid"));
        }
        let event: Self = serde_json::from_slice(encoded)?;
        event.validate()?;
        Ok(event)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != EVENT_SCHEMA_VERSION {
            return Err(MlsError::InvalidPayload("unsupported event schema version"));
        }
        if self.sender_counter == 0 {
            return Err(MlsError::InvalidPayload("sender counter must be positive"));
        }
        if self.created_at_ms < 0 {
            return Err(MlsError::InvalidPayload(
                "client timestamp must not be negative",
            ));
        }
        if [
            self.references.reply_to,
            self.references.replaces,
            self.references.deletes,
        ]
        .into_iter()
        .flatten()
        .any(|referenced| referenced == self.event_id)
        {
            return Err(MlsError::InvalidPayload("event must not reference itself"));
        }

        match &self.content {
            EventContent::Text(text) => {
                if text.body.trim().is_empty() || text.body.len() > MAX_TEXT_BYTES {
                    return Err(MlsError::InvalidPayload("text body size is invalid"));
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_event() -> PlaintextEvent {
        PlaintextEvent::text(
            EventId::new(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            1,
            1_765_000_000_000,
            Some([0x42; 32]),
            EventReferences::default(),
            "Canary stays inside MLS".to_owned(),
        )
    }

    #[test]
    fn provisional_payload_round_trips() {
        let event = sample_event();

        let encoded = event.encode_provisional_json().unwrap();
        let decoded = PlaintextEvent::decode_provisional_json(&encoded).unwrap();

        assert_eq!(decoded, event);
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let mut value = serde_json::to_value(sample_event()).unwrap();
        value["server_visible"] = serde_json::Value::Bool(true);

        let result = PlaintextEvent::decode_provisional_json(&serde_json::to_vec(&value).unwrap());

        assert!(matches!(result, Err(MlsError::PayloadEncoding(_))));
    }

    #[test]
    fn invalid_semantics_are_rejected() {
        let mut event = sample_event();
        event.sender_counter = 0;
        assert!(matches!(
            event.encode_provisional_json(),
            Err(MlsError::InvalidPayload(_))
        ));

        let mut event = sample_event();
        event.references.reply_to = Some(event.event_id);
        assert!(matches!(
            event.encode_provisional_json(),
            Err(MlsError::InvalidPayload(_))
        ));

        let mut event = sample_event();
        event.content = EventContent::Text(TextContent {
            body: " ".to_owned(),
        });
        assert!(matches!(
            event.encode_provisional_json(),
            Err(MlsError::InvalidPayload(_))
        ));
    }

    #[test]
    fn oversized_payload_is_rejected() {
        let mut event = sample_event();
        event.content = EventContent::Text(TextContent {
            body: "x".repeat(MAX_TEXT_BYTES + 1),
        });

        assert!(matches!(
            event.encode_provisional_json(),
            Err(MlsError::InvalidPayload(_))
        ));
    }
}
