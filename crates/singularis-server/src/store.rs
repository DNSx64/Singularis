use std::collections::HashMap;

use singularis_protocol::{AcceptedEvent, DeliveredEvent, EventId, ServerTtl, SubmitEvent};
use thiserror::Error;
use time::OffsetDateTime;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Default)]
pub struct InMemoryEventStore {
    inner: RwLock<StoreData>,
}

#[derive(Default)]
struct StoreData {
    events_by_channel: HashMap<Uuid, Vec<StoredEvent>>,
    events_by_id: HashMap<EventId, StoredEvent>,
    highest_counter_by_sender: HashMap<(Uuid, Uuid), u64>,
    next_sequence_by_channel: HashMap<Uuid, u64>,
}

#[derive(Clone)]
struct StoredEvent {
    request: SubmitEvent,
    receipt: AcceptedEvent,
}

impl InMemoryEventStore {
    pub async fn accept(
        &self,
        request: SubmitEvent,
        instance_max_ttl: ServerTtl,
        accepted_at: OffsetDateTime,
    ) -> Result<AcceptedEvent, StoreError> {
        let mut data = self.inner.write().await;
        data.prune_expired(accepted_at);

        if let Some(stored) = data.events_by_id.get(&request.event_id) {
            if stored.request == request {
                return Ok(stored.receipt.clone());
            }

            return Err(StoreError::EventIdConflict(request.event_id));
        }

        let sender_key = (request.channel_id, request.sender_device_id);
        if let Some(highest_counter) = data.highest_counter_by_sender.get(&sender_key)
            && request.sender_counter <= *highest_counter
        {
            return Err(StoreError::SenderCounterReplay {
                received: request.sender_counter,
                highest: *highest_counter,
            });
        }

        let effective_ttl = request.ttl_seconds.min(instance_max_ttl);
        let sequence = data
            .next_sequence_by_channel
            .entry(request.channel_id)
            .and_modify(|current| *current += 1)
            .or_insert(1)
            .to_owned();
        let expires_at =
            accepted_at + time::Duration::seconds(i64::from(effective_ttl.as_seconds()));
        let receipt = AcceptedEvent {
            event_id: request.event_id,
            channel_id: request.channel_id,
            sequence,
            accepted_at,
            expires_at,
        };
        let stored = StoredEvent {
            request: request.clone(),
            receipt: receipt.clone(),
        };

        data.events_by_channel
            .entry(request.channel_id)
            .or_default()
            .push(stored.clone());
        data.events_by_id.insert(request.event_id, stored);
        data.highest_counter_by_sender
            .insert(sender_key, request.sender_counter);

        Ok(receipt)
    }

    pub async fn list_active(
        &self,
        channel_id: Uuid,
        after_sequence: u64,
        now: OffsetDateTime,
    ) -> Vec<DeliveredEvent> {
        let mut data = self.inner.write().await;
        data.prune_expired(now);

        data.events_by_channel
            .get(&channel_id)
            .into_iter()
            .flatten()
            .filter(|stored| stored.receipt.sequence > after_sequence)
            .map(StoredEvent::delivered)
            .collect()
    }
}

impl StoreData {
    fn prune_expired(&mut self, now: OffsetDateTime) {
        self.events_by_id
            .retain(|_, stored| stored.receipt.expires_at > now);
        self.events_by_channel.retain(|_, events| {
            events.retain(|stored| stored.receipt.expires_at > now);
            !events.is_empty()
        });
    }
}

impl StoredEvent {
    fn delivered(&self) -> DeliveredEvent {
        DeliveredEvent {
            receipt: self.receipt.clone(),
            community_id: self.request.community_id,
            sender_device_id: self.request.sender_device_id,
            sender_counter: self.request.sender_counter,
            mls_message: self.request.mls_message.clone(),
        }
    }
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("event ID {0:?} was already used for a different payload")]
    EventIdConflict(EventId),
    #[error("sender counter {received} does not advance the stored counter {highest}")]
    SenderCounterReplay { received: u64, highest: u64 },
}

#[cfg(test)]
mod tests {
    use super::*;
    use singularis_protocol::{MAX_SERVER_TTL_SECONDS, OpaqueMlsMessage};

    fn request(event_id: EventId, channel_id: Uuid, ttl_seconds: ServerTtl) -> SubmitEvent {
        SubmitEvent {
            event_id,
            community_id: Uuid::new_v4(),
            channel_id,
            sender_device_id: Uuid::new_v4(),
            sender_counter: 1,
            mls_message: OpaqueMlsMessage::from_bytes(b"test-opaque-mls-message").unwrap(),
            ttl_seconds,
        }
    }

    #[tokio::test]
    async fn repeat_submission_is_idempotent() {
        let store = InMemoryEventStore::default();
        let now = OffsetDateTime::UNIX_EPOCH;
        let request = request(EventId::new(), Uuid::new_v4(), ServerTtl::MAX);

        let first = store
            .accept(request.clone(), ServerTtl::MAX, now)
            .await
            .unwrap();
        let second = store
            .accept(request, ServerTtl::MAX, now + time::Duration::seconds(1))
            .await
            .unwrap();

        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn reused_event_id_with_changed_payload_is_rejected() {
        let store = InMemoryEventStore::default();
        let now = OffsetDateTime::UNIX_EPOCH;
        let mut request = request(EventId::new(), Uuid::new_v4(), ServerTtl::MAX);
        store
            .accept(request.clone(), ServerTtl::MAX, now)
            .await
            .unwrap();
        request.mls_message = OpaqueMlsMessage::from_bytes(b"different-mls-message").unwrap();

        let result = store.accept(request, ServerTtl::MAX, now).await;

        assert!(matches!(result, Err(StoreError::EventIdConflict(_))));
    }

    #[tokio::test]
    async fn instance_policy_clamps_requested_ttl() {
        let store = InMemoryEventStore::default();
        let now = OffsetDateTime::UNIX_EPOCH;
        let instance_ttl = ServerTtl::try_from(60 * 60).unwrap();
        let request = request(EventId::new(), Uuid::new_v4(), ServerTtl::MAX);

        let receipt = store.accept(request, instance_ttl, now).await.unwrap();

        assert_eq!(
            receipt.expires_at - receipt.accepted_at,
            time::Duration::hours(1)
        );
        assert!(
            (receipt.expires_at - receipt.accepted_at).whole_seconds()
                < i64::from(MAX_SERVER_TTL_SECONDS)
        );
    }

    #[tokio::test]
    async fn expired_events_are_not_listed() {
        let store = InMemoryEventStore::default();
        let now = OffsetDateTime::UNIX_EPOCH;
        let channel_id = Uuid::new_v4();
        let request = request(EventId::new(), channel_id, ServerTtl::MIN);
        let receipt = store.accept(request, ServerTtl::MAX, now).await.unwrap();

        assert_eq!(store.list_active(channel_id, 0, now).await.len(), 1);
        assert!(
            store
                .list_active(channel_id, 0, receipt.expires_at)
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn expired_event_cannot_be_replayed_with_a_fresh_ttl() {
        let store = InMemoryEventStore::default();
        let now = OffsetDateTime::UNIX_EPOCH;
        let request = request(EventId::new(), Uuid::new_v4(), ServerTtl::MIN);
        let receipt = store
            .accept(request.clone(), ServerTtl::MAX, now)
            .await
            .unwrap();

        let result = store
            .accept(request, ServerTtl::MAX, receipt.expires_at)
            .await;

        assert!(matches!(
            result,
            Err(StoreError::SenderCounterReplay {
                received: 1,
                highest: 1
            })
        ));
    }

    #[tokio::test]
    async fn next_sender_counter_is_accepted_after_expiry() {
        let store = InMemoryEventStore::default();
        let now = OffsetDateTime::UNIX_EPOCH;
        let first = request(EventId::new(), Uuid::new_v4(), ServerTtl::MIN);
        let receipt = store
            .accept(first.clone(), ServerTtl::MAX, now)
            .await
            .unwrap();
        let mut second = request(EventId::new(), first.channel_id, ServerTtl::MIN);
        second.sender_device_id = first.sender_device_id;
        second.sender_counter = 2;

        let result = store
            .accept(second, ServerTtl::MAX, receipt.expires_at)
            .await;

        assert!(result.is_ok());
    }
}
