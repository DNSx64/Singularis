use std::collections::{HashMap, HashSet};

use openmls::prelude::{
    BasicCredential, CredentialType, CredentialWithKey, GroupId, KeyPackage, MlsGroup,
    MlsGroupCreateConfig, MlsGroupJoinConfig, MlsMessageBodyIn, MlsMessageIn, MlsMessageOut,
    OpenMlsProvider as _, PURE_CIPHERTEXT_WIRE_FORMAT_POLICY, ProcessedMessageContent,
    ProtocolMessage, ProtocolVersion, Sender, SenderRatchetConfiguration, StagedWelcome,
    tls_codec::Deserialize as _,
};
use openmls_basic_credential::SignatureKeyPair;
use openmls_rust_crypto::OpenMlsRustCrypto;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use singularis_protocol::{
    DeliveredEvent, EventId, MAX_MLS_MESSAGE_BYTES, OpaqueMlsMessage, ServerTtl, SubmitEvent,
};
use singularis_vault::{NewVaultMessage, Vault, VaultMessage};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    CIPHERSUITE, EVENT_SCHEMA_VERSION, EventReferences, MlsError, PlaintextEvent, Result,
    in_memory_provider,
};

const TRANSPORT_AAD_VERSION: u16 = 1;
const MAX_KEY_PACKAGE_BYTES: usize = 64 * 1024;
const MAX_WELCOME_BYTES: usize = 384 * 1024;
const MLS_PADDING_BYTES: usize = 256;
const OUT_OF_ORDER_TOLERANCE: u32 = 32;
const MAXIMUM_FORWARD_DISTANCE: u32 = 1000;
const CLIENT_SNAPSHOT_VERSION: u16 = 1;
const MAX_CLIENT_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncryptedEvent {
    pub event_id: EventId,
    pub community_id: Uuid,
    pub channel_id: Uuid,
    pub sender_device_id: Uuid,
    pub sender_counter: u64,
    pub mls_message: Vec<u8>,
}

impl EncryptedEvent {
    pub fn to_submit_event(&self, ttl_seconds: ServerTtl) -> Result<SubmitEvent> {
        Ok(SubmitEvent {
            event_id: self.event_id,
            community_id: self.community_id,
            channel_id: self.channel_id,
            sender_device_id: self.sender_device_id,
            sender_counter: self.sender_counter,
            mls_message: OpaqueMlsMessage::from_bytes(&self.mls_message)
                .map_err(|_| MlsError::MessageEncoding)?,
            ttl_seconds,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemberAddition {
    pub member_device_id: Uuid,
    pub commit: Vec<u8>,
    pub welcome: Vec<u8>,
}

pub struct MlsClient {
    device_id: Uuid,
    provider: OpenMlsRustCrypto,
    signer: SignatureKeyPair,
    credential: CredentialWithKey,
    channels: HashMap<Uuid, ChannelState>,
}

pub struct VaultMlsClient<'vault> {
    vault: &'vault Vault,
    client: MlsClient,
}

struct ChannelState {
    community_id: Uuid,
    group: MlsGroup,
    outgoing_chain: Option<SenderChain>,
    incoming_chains: HashMap<Uuid, SenderChain>,
    seen_event_ids: HashSet<EventId>,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
struct SenderChain {
    counter: u64,
    event_hash: [u8; 32],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MlsClientSnapshot {
    version: u16,
    device_id: Uuid,
    signer_public_key: Vec<u8>,
    provider_values: Vec<ProviderValue>,
    channels: Vec<ChannelSnapshot>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderValue {
    key: Vec<u8>,
    value: Vec<u8>,
}

#[derive(Serialize)]
struct MlsClientSnapshotRef<'a> {
    version: u16,
    device_id: Uuid,
    signer_public_key: &'a [u8],
    provider_values: Vec<ProviderValueRef<'a>>,
    channels: &'a [ChannelSnapshot],
}

#[derive(Serialize)]
struct ProviderValueRef<'a> {
    key: &'a [u8],
    value: &'a [u8],
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ChannelSnapshot {
    community_id: Uuid,
    channel_id: Uuid,
    outgoing_chain: Option<SenderChain>,
    incoming_chains: Vec<IncomingChainSnapshot>,
    seen_event_ids: Vec<EventId>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct IncomingChainSnapshot {
    sender_device_id: Uuid,
    chain: SenderChain,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AuthenticatedTransportMetadata {
    version: u16,
    event_id: EventId,
    community_id: Uuid,
    channel_id: Uuid,
    sender_device_id: Uuid,
    sender_counter: u64,
    event_schema_version: u16,
}

impl MlsClient {
    pub fn new(device_id: Uuid) -> Result<Self> {
        let provider = in_memory_provider();
        let signer = SignatureKeyPair::new(CIPHERSUITE.signature_algorithm())
            .map_err(|_| MlsError::IdentityGeneration)?;
        signer
            .store(provider.storage())
            .map_err(|_| MlsError::StateStorage)?;
        let credential = CredentialWithKey {
            credential: BasicCredential::new(device_id.as_bytes().to_vec()).into(),
            signature_key: signer.public().into(),
        };

        Ok(Self {
            device_id,
            provider,
            signer,
            credential,
            channels: HashMap::new(),
        })
    }

    #[must_use]
    pub const fn device_id(&self) -> Uuid {
        self.device_id
    }

    #[must_use]
    pub fn has_channel(&self, channel_id: Uuid) -> bool {
        self.channels.contains_key(&channel_id)
    }

    fn save_to_vault(&self, vault: &Vault) -> Result<()> {
        let snapshot = self.encode_snapshot()?;
        vault.store_mls_client_snapshot(&self.device_id.to_string(), snapshot.as_slice())?;
        Ok(())
    }

    fn load_from_vault(vault: &Vault, device_id: Uuid) -> Result<Option<Self>> {
        let Some(snapshot) = vault.load_mls_client_snapshot(&device_id.to_string())? else {
            return Ok(None);
        };
        Self::decode_snapshot(snapshot.as_slice(), device_id).map(Some)
    }

    pub fn publish_key_package(&self) -> Result<Vec<u8>> {
        let key_package = KeyPackage::builder()
            .build(
                CIPHERSUITE,
                &self.provider,
                &self.signer,
                self.credential.clone(),
            )
            .map_err(|_| MlsError::InvalidKeyPackage)?;

        MlsMessageOut::from(key_package.key_package().clone())
            .to_bytes()
            .map_err(|_| MlsError::MessageEncoding)
    }

    pub fn create_channel(&mut self, community_id: Uuid, channel_id: Uuid) -> Result<()> {
        if self.channels.contains_key(&channel_id) {
            return Err(MlsError::ChannelAlreadyJoined);
        }

        let group = MlsGroup::new_with_group_id(
            &self.provider,
            &self.signer,
            &group_create_config(),
            GroupId::from_slice(channel_id.as_bytes()),
            self.credential.clone(),
        )
        .map_err(|_| MlsError::StateStorage)?;
        self.channels
            .insert(channel_id, ChannelState::new(community_id, group));
        Ok(())
    }

    pub fn add_member(
        &mut self,
        channel_id: Uuid,
        encoded_key_package: &[u8],
    ) -> Result<MemberAddition> {
        let key_package = decode_key_package(&self.provider, encoded_key_package)?;
        let member_device_id = credential_device_id(key_package.leaf_node().credential())?;
        let state = self
            .channels
            .get_mut(&channel_id)
            .ok_or(MlsError::UnknownChannel)?;
        let (commit, welcome, _) = state
            .group
            .add_members(
                &self.provider,
                &self.signer,
                std::slice::from_ref(&key_package),
            )
            .map_err(|_| MlsError::MessageRejected)?;
        state
            .group
            .merge_pending_commit(&self.provider)
            .map_err(|_| MlsError::StateStorage)?;

        Ok(MemberAddition {
            member_device_id,
            commit: commit.to_bytes().map_err(|_| MlsError::MessageEncoding)?,
            welcome: welcome.to_bytes().map_err(|_| MlsError::MessageEncoding)?,
        })
    }

    pub fn join_channel(
        &mut self,
        community_id: Uuid,
        channel_id: Uuid,
        encoded_welcome: &[u8],
    ) -> Result<()> {
        if self.channels.contains_key(&channel_id) {
            return Err(MlsError::ChannelAlreadyJoined);
        }
        if encoded_welcome.is_empty() || encoded_welcome.len() > MAX_WELCOME_BYTES {
            return Err(MlsError::InvalidWelcome);
        }

        let message = MlsMessageIn::tls_deserialize_exact(encoded_welcome)
            .map_err(|_| MlsError::InvalidWelcome)?;
        let MlsMessageBodyIn::Welcome(welcome) = message.extract() else {
            return Err(MlsError::InvalidWelcome);
        };
        let staged =
            StagedWelcome::new_from_welcome(&self.provider, &group_join_config(), welcome, None)
                .map_err(|_| MlsError::InvalidWelcome)?;
        let group = staged
            .into_group(&self.provider)
            .map_err(|_| MlsError::InvalidWelcome)?;
        if group.group_id().as_slice() != channel_id.as_bytes()
            || group.ciphersuite() != CIPHERSUITE
        {
            return Err(MlsError::ContextMismatch);
        }

        self.channels
            .insert(channel_id, ChannelState::new(community_id, group));
        Ok(())
    }

    pub fn encrypt_text(
        &mut self,
        channel_id: Uuid,
        created_at_ms: i64,
        references: EventReferences,
        body: String,
    ) -> Result<EncryptedEvent> {
        let state = self
            .channels
            .get(&channel_id)
            .ok_or(MlsError::UnknownChannel)?;
        let sender_counter = state
            .outgoing_chain
            .map_or(1, |chain| chain.counter.saturating_add(1));
        let previous_event_hash = state.outgoing_chain.map(|chain| chain.event_hash);
        let event = PlaintextEvent::text(
            EventId::new(),
            state.community_id,
            channel_id,
            self.device_id,
            sender_counter,
            created_at_ms,
            previous_event_hash,
            references,
            body,
        );
        self.encrypt_event(event)
    }

    pub fn encrypt_event(&mut self, event: PlaintextEvent) -> Result<EncryptedEvent> {
        event.validate()?;
        if event.sender_device_id != self.device_id {
            return Err(MlsError::ContextMismatch);
        }
        let state = self
            .channels
            .get_mut(&event.channel_id)
            .ok_or(MlsError::UnknownChannel)?;
        if event.community_id != state.community_id {
            return Err(MlsError::ContextMismatch);
        }

        let (expected_counter, expected_previous_hash) =
            state.outgoing_chain.map_or((1, None), |chain| {
                (chain.counter.saturating_add(1), Some(chain.event_hash))
            });
        if event.sender_counter != expected_counter
            || event.previous_event_hash != expected_previous_hash
            || state.seen_event_ids.contains(&event.event_id)
        {
            return Err(MlsError::InvalidSenderChain);
        }

        let encoded_event = event.encode_provisional_json()?;
        let event_hash = Sha256::digest(&encoded_event).into();
        let metadata = AuthenticatedTransportMetadata::from_event(&event);
        state.group.set_aad(serde_json::to_vec(&metadata)?);
        let mls_message = state
            .group
            .create_message(&self.provider, &self.signer, &encoded_event)
            .map_err(|_| MlsError::MessageRejected)?
            .to_bytes()
            .map_err(|_| MlsError::MessageEncoding)?;
        state.outgoing_chain = Some(SenderChain {
            counter: event.sender_counter,
            event_hash,
        });
        state.seen_event_ids.insert(event.event_id);

        Ok(EncryptedEvent {
            event_id: event.event_id,
            community_id: event.community_id,
            channel_id: event.channel_id,
            sender_device_id: event.sender_device_id,
            sender_counter: event.sender_counter,
            mls_message,
        })
    }

    pub fn decrypt_event(
        &mut self,
        channel_id: Uuid,
        encoded_message: &[u8],
    ) -> Result<PlaintextEvent> {
        self.decrypt_event_with_delivery(channel_id, encoded_message, None)
    }

    pub fn decrypt_delivered_event(
        &mut self,
        delivered: &DeliveredEvent,
    ) -> Result<PlaintextEvent> {
        let encoded_message = delivered
            .mls_message
            .decode()
            .map_err(|_| MlsError::MessageRejected)?;
        self.decrypt_event_with_delivery(
            delivered.receipt.channel_id,
            &encoded_message,
            Some(delivered),
        )
    }

    fn decrypt_event_with_delivery(
        &mut self,
        channel_id: Uuid,
        encoded_message: &[u8],
        delivered: Option<&DeliveredEvent>,
    ) -> Result<PlaintextEvent> {
        if encoded_message.is_empty() || encoded_message.len() > MAX_MLS_MESSAGE_BYTES {
            return Err(MlsError::MessageRejected);
        }
        let message = MlsMessageIn::tls_deserialize_exact(encoded_message)
            .map_err(|_| MlsError::MessageRejected)?;
        let protocol_message = message
            .try_into_protocol_message()
            .map_err(|_| MlsError::UnexpectedMessageType)?;
        let ProtocolMessage::PrivateMessage(private_message) = &protocol_message else {
            return Err(MlsError::UnexpectedMessageType);
        };
        let unverified_metadata: AuthenticatedTransportMetadata =
            serde_json::from_slice(private_message.aad())?;
        if let Some(delivered) = delivered {
            unverified_metadata.validate_delivery(delivered)?;
        }
        let state = self
            .channels
            .get_mut(&channel_id)
            .ok_or(MlsError::UnknownChannel)?;
        if protocol_message.group_id().as_slice() != channel_id.as_bytes() {
            return Err(MlsError::ContextMismatch);
        }

        let group_id = state.group.group_id().clone();
        let processed = match state
            .group
            .process_message(&self.provider, protocol_message)
        {
            Ok(processed) => processed,
            Err(_) => {
                state.group = MlsGroup::load(self.provider.storage(), &group_id)
                    .map_err(|_| MlsError::StateStorage)?
                    .ok_or(MlsError::StateStorage)?;
                return Err(MlsError::MessageRejected);
            }
        };
        if !matches!(processed.sender(), Sender::Member(_)) {
            return Err(MlsError::InvalidSenderCredential);
        }
        let sender_device_id = credential_device_id(processed.credential())?;
        let metadata: AuthenticatedTransportMetadata = serde_json::from_slice(processed.aad())?;
        let ProcessedMessageContent::ApplicationMessage(application_message) =
            processed.into_content()
        else {
            return Err(MlsError::UnexpectedMessageType);
        };
        let event = PlaintextEvent::decode_provisional_json(&application_message.into_bytes())?;
        metadata.validate_event(&event, sender_device_id, state.community_id, channel_id)?;
        if let Some(delivered) = delivered {
            metadata.validate_delivery(delivered)?;
        }
        state.validate_and_record_incoming(&event)?;
        Ok(event)
    }

    fn encode_snapshot(&self) -> Result<Zeroizing<Vec<u8>>> {
        let provider_values = self
            .provider
            .storage()
            .values
            .read()
            .map_err(|_| MlsError::StateStorage)?;
        let mut provider_values = provider_values
            .iter()
            .map(|(key, value)| ProviderValueRef {
                key: key.as_slice(),
                value: value.as_slice(),
            })
            .collect::<Vec<_>>();
        provider_values.sort_unstable_by(|left, right| left.key.cmp(right.key));

        let mut channels = self
            .channels
            .iter()
            .map(|(channel_id, state)| {
                let mut incoming_chains = state
                    .incoming_chains
                    .iter()
                    .map(|(sender_device_id, chain)| IncomingChainSnapshot {
                        sender_device_id: *sender_device_id,
                        chain: *chain,
                    })
                    .collect::<Vec<_>>();
                incoming_chains.sort_unstable_by_key(|entry| entry.sender_device_id);
                let mut seen_event_ids = state.seen_event_ids.iter().copied().collect::<Vec<_>>();
                seen_event_ids.sort_unstable_by(|left, right| {
                    left.as_uuid().as_bytes().cmp(right.as_uuid().as_bytes())
                });
                ChannelSnapshot {
                    community_id: state.community_id,
                    channel_id: *channel_id,
                    outgoing_chain: state.outgoing_chain,
                    incoming_chains,
                    seen_event_ids,
                }
            })
            .collect::<Vec<_>>();
        channels.sort_unstable_by_key(|channel| channel.channel_id);

        let encoded = Zeroizing::new(
            serde_json::to_vec(&MlsClientSnapshotRef {
                version: CLIENT_SNAPSHOT_VERSION,
                device_id: self.device_id,
                signer_public_key: self.signer.public(),
                provider_values,
                channels: &channels,
            })
            .map_err(|_| MlsError::StateStorage)?,
        );
        if encoded.is_empty() || encoded.len() > MAX_CLIENT_SNAPSHOT_BYTES {
            return Err(MlsError::StateStorage);
        }
        Ok(encoded)
    }

    fn decode_snapshot(encoded: &[u8], expected_device_id: Uuid) -> Result<Self> {
        if encoded.is_empty() || encoded.len() > MAX_CLIENT_SNAPSHOT_BYTES {
            return Err(MlsError::InvalidStateSnapshot);
        }
        let snapshot: MlsClientSnapshot =
            serde_json::from_slice(encoded).map_err(|_| MlsError::InvalidStateSnapshot)?;
        if snapshot.version != CLIENT_SNAPSHOT_VERSION
            || snapshot.device_id != expected_device_id
            || snapshot.signer_public_key.is_empty()
        {
            return Err(MlsError::InvalidStateSnapshot);
        }

        let provider = in_memory_provider();
        {
            let mut values = provider
                .storage()
                .values
                .write()
                .map_err(|_| MlsError::StateStorage)?;
            for stored in snapshot.provider_values {
                if stored.key.is_empty()
                    || stored.value.is_empty()
                    || values.insert(stored.key, stored.value).is_some()
                {
                    return Err(MlsError::InvalidStateSnapshot);
                }
            }
        }

        let signer = SignatureKeyPair::read(
            provider.storage(),
            &snapshot.signer_public_key,
            CIPHERSUITE.signature_algorithm(),
        )
        .ok_or(MlsError::InvalidStateSnapshot)?;
        let credential = CredentialWithKey {
            credential: BasicCredential::new(expected_device_id.as_bytes().to_vec()).into(),
            signature_key: signer.public().into(),
        };

        let mut channels = HashMap::with_capacity(snapshot.channels.len());
        for stored in snapshot.channels {
            if stored
                .outgoing_chain
                .is_some_and(|chain| chain.counter == 0)
            {
                return Err(MlsError::InvalidStateSnapshot);
            }
            let mut incoming_chains = HashMap::with_capacity(stored.incoming_chains.len());
            for incoming in stored.incoming_chains {
                if incoming.chain.counter == 0
                    || incoming_chains
                        .insert(incoming.sender_device_id, incoming.chain)
                        .is_some()
                {
                    return Err(MlsError::InvalidStateSnapshot);
                }
            }
            let seen_event_ids = stored
                .seen_event_ids
                .iter()
                .copied()
                .collect::<HashSet<_>>();
            if seen_event_ids.len() != stored.seen_event_ids.len() {
                return Err(MlsError::InvalidStateSnapshot);
            }

            let group_id = GroupId::from_slice(stored.channel_id.as_bytes());
            let group = MlsGroup::load(provider.storage(), &group_id)
                .map_err(|_| MlsError::InvalidStateSnapshot)?
                .ok_or(MlsError::InvalidStateSnapshot)?;
            if group.group_id().as_slice() != stored.channel_id.as_bytes()
                || group.ciphersuite() != CIPHERSUITE
            {
                return Err(MlsError::InvalidStateSnapshot);
            }
            let state = ChannelState {
                community_id: stored.community_id,
                group,
                outgoing_chain: stored.outgoing_chain,
                incoming_chains,
                seen_event_ids,
            };
            if channels.insert(stored.channel_id, state).is_some() {
                return Err(MlsError::InvalidStateSnapshot);
            }
        }

        Ok(Self {
            device_id: expected_device_id,
            provider,
            signer,
            credential,
            channels,
        })
    }
}

impl<'vault> VaultMlsClient<'vault> {
    pub fn create(vault: &'vault Vault, device_id: Uuid) -> Result<Self> {
        if vault
            .load_mls_client_snapshot(&device_id.to_string())?
            .is_some()
        {
            return Err(MlsError::StateAlreadyExists);
        }
        let client = MlsClient::new(device_id)?;
        client.save_to_vault(vault)?;
        Ok(Self { vault, client })
    }

    pub fn load(vault: &'vault Vault, device_id: Uuid) -> Result<Option<Self>> {
        Ok(MlsClient::load_from_vault(vault, device_id)?.map(|client| Self { vault, client }))
    }

    #[must_use]
    pub const fn device_id(&self) -> Uuid {
        self.client.device_id()
    }

    #[must_use]
    pub fn has_channel(&self, channel_id: Uuid) -> bool {
        self.client.has_channel(channel_id)
    }

    pub fn publish_key_package(&mut self) -> Result<Vec<u8>> {
        self.transact(|client| client.publish_key_package())
    }

    pub fn create_channel(&mut self, community_id: Uuid, channel_id: Uuid) -> Result<()> {
        self.transact(|client| client.create_channel(community_id, channel_id))
    }

    pub fn add_member(
        &mut self,
        channel_id: Uuid,
        encoded_key_package: &[u8],
    ) -> Result<MemberAddition> {
        self.transact(|client| client.add_member(channel_id, encoded_key_package))
    }

    pub fn join_channel(
        &mut self,
        community_id: Uuid,
        channel_id: Uuid,
        encoded_welcome: &[u8],
    ) -> Result<()> {
        self.transact(|client| client.join_channel(community_id, channel_id, encoded_welcome))
    }

    pub fn queue_text(
        &mut self,
        channel_id: Uuid,
        created_at_ms: i64,
        references: EventReferences,
        body: String,
        ttl_seconds: ServerTtl,
    ) -> Result<SubmitEvent> {
        self.queue_outgoing(
            ttl_seconds,
            |client| client.encrypt_text(channel_id, created_at_ms, references, body),
            |vault, device_id, snapshot, event_id, payload| {
                vault.store_mls_snapshot_and_outbox(device_id, snapshot, event_id, payload)?;
                Ok(())
            },
        )
        .map(|(submission, ())| submission)
    }

    pub fn queue_text_and_store_message(
        &mut self,
        channel_id: Uuid,
        references: EventReferences,
        message: &NewVaultMessage,
        ttl_seconds: ServerTtl,
    ) -> Result<(SubmitEvent, VaultMessage)> {
        self.queue_outgoing(
            ttl_seconds,
            |client| {
                client.encrypt_text(
                    channel_id,
                    message.created_at_ms,
                    references,
                    message.body.clone(),
                )
            },
            |vault, device_id, snapshot, event_id, payload| {
                Ok(vault.store_message_and_mls_snapshot_and_outbox(
                    message, device_id, snapshot, event_id, payload,
                )?)
            },
        )
    }

    pub fn queue_event(
        &mut self,
        event: PlaintextEvent,
        ttl_seconds: ServerTtl,
    ) -> Result<SubmitEvent> {
        self.queue_outgoing(
            ttl_seconds,
            |client| client.encrypt_event(event),
            |vault, device_id, snapshot, event_id, payload| {
                vault.store_mls_snapshot_and_outbox(device_id, snapshot, event_id, payload)?;
                Ok(())
            },
        )
        .map(|(submission, ())| submission)
    }

    pub fn pending_submissions(&self) -> Result<Vec<SubmitEvent>> {
        self.vault
            .list_mls_outbox(&self.client.device_id.to_string())?
            .into_iter()
            .map(|item| {
                let submission: SubmitEvent = serde_json::from_slice(&item.payload)
                    .map_err(|_| MlsError::InvalidOutboxEntry)?;
                let canonical =
                    serde_json::to_vec(&submission).map_err(|_| MlsError::InvalidOutboxEntry)?;
                if submission.event_id.as_uuid().to_string() != item.event_id
                    || submission.sender_device_id != self.client.device_id
                    || canonical != item.payload
                {
                    return Err(MlsError::InvalidOutboxEntry);
                }
                Ok(submission)
            })
            .collect()
    }

    pub fn acknowledge_submission(&self, event_id: EventId) -> Result<bool> {
        Ok(self.vault.acknowledge_mls_outbox(
            &self.client.device_id.to_string(),
            &event_id.as_uuid().to_string(),
        )?)
    }

    pub fn decrypt_event(
        &mut self,
        channel_id: Uuid,
        encoded_message: &[u8],
    ) -> Result<PlaintextEvent> {
        self.transact(|client| client.decrypt_event(channel_id, encoded_message))
    }

    pub fn decrypt_delivered_event(
        &mut self,
        delivered: &DeliveredEvent,
    ) -> Result<PlaintextEvent> {
        self.transact(|client| client.decrypt_delivered_event(delivered))
    }

    fn transact<T>(&mut self, operation: impl FnOnce(&mut MlsClient) -> Result<T>) -> Result<T> {
        let previous = self.client.encode_snapshot()?;
        match operation(&mut self.client) {
            Ok(value) => match self.client.save_to_vault(self.vault) {
                Ok(()) => Ok(value),
                Err(error) => {
                    self.restore(previous)?;
                    Err(error)
                }
            },
            Err(error) => {
                self.restore(previous)?;
                Err(error)
            }
        }
    }

    fn queue_outgoing<T>(
        &mut self,
        ttl_seconds: ServerTtl,
        operation: impl FnOnce(&mut MlsClient) -> Result<EncryptedEvent>,
        persist: impl FnOnce(&Vault, &str, &[u8], &str, &[u8]) -> Result<T>,
    ) -> Result<(SubmitEvent, T)> {
        let previous = self.client.encode_snapshot()?;
        let result = (|| {
            let encrypted = operation(&mut self.client)?;
            let submission = encrypted.to_submit_event(ttl_seconds)?;
            let payload = serde_json::to_vec(&submission)?;
            let snapshot = self.client.encode_snapshot()?;
            let device_id = self.client.device_id.to_string();
            let event_id = submission.event_id.as_uuid().to_string();
            let persisted = persist(
                self.vault,
                &device_id,
                snapshot.as_slice(),
                &event_id,
                &payload,
            )?;
            Ok((submission, persisted))
        })();
        match result {
            Ok(queued) => Ok(queued),
            Err(error) => {
                self.restore(previous)?;
                Err(error)
            }
        }
    }

    fn restore(&mut self, snapshot: Zeroizing<Vec<u8>>) -> Result<()> {
        self.client = MlsClient::decode_snapshot(snapshot.as_slice(), self.client.device_id)?;
        Ok(())
    }
}

impl ChannelState {
    fn new(community_id: Uuid, group: MlsGroup) -> Self {
        Self {
            community_id,
            group,
            outgoing_chain: None,
            incoming_chains: HashMap::new(),
            seen_event_ids: HashSet::new(),
        }
    }

    fn validate_and_record_incoming(&mut self, event: &PlaintextEvent) -> Result<()> {
        if self.seen_event_ids.contains(&event.event_id) {
            return Err(MlsError::ReplayDetected);
        }
        let (expected_counter, expected_previous_hash) = self
            .incoming_chains
            .get(&event.sender_device_id)
            .map_or((1, None), |chain| {
                (chain.counter.saturating_add(1), Some(chain.event_hash))
            });
        if event.sender_counter != expected_counter
            || event.previous_event_hash != expected_previous_hash
        {
            return Err(MlsError::InvalidSenderChain);
        }

        let encoded = event.encode_provisional_json()?;
        self.incoming_chains.insert(
            event.sender_device_id,
            SenderChain {
                counter: event.sender_counter,
                event_hash: Sha256::digest(encoded).into(),
            },
        );
        self.seen_event_ids.insert(event.event_id);
        Ok(())
    }
}

impl AuthenticatedTransportMetadata {
    fn from_event(event: &PlaintextEvent) -> Self {
        Self {
            version: TRANSPORT_AAD_VERSION,
            event_id: event.event_id,
            community_id: event.community_id,
            channel_id: event.channel_id,
            sender_device_id: event.sender_device_id,
            sender_counter: event.sender_counter,
            event_schema_version: event.schema_version,
        }
    }

    fn validate_event(
        &self,
        event: &PlaintextEvent,
        credential_device_id: Uuid,
        expected_community_id: Uuid,
        expected_channel_id: Uuid,
    ) -> Result<()> {
        if self.version != TRANSPORT_AAD_VERSION
            || self.event_id != event.event_id
            || self.community_id != event.community_id
            || self.channel_id != event.channel_id
            || self.sender_device_id != event.sender_device_id
            || self.sender_counter != event.sender_counter
            || self.event_schema_version != EVENT_SCHEMA_VERSION
            || event.community_id != expected_community_id
            || event.channel_id != expected_channel_id
            || event.sender_device_id != credential_device_id
        {
            return Err(MlsError::ContextMismatch);
        }
        Ok(())
    }

    fn validate_delivery(&self, delivered: &DeliveredEvent) -> Result<()> {
        if self.event_id != delivered.receipt.event_id
            || self.community_id != delivered.community_id
            || self.channel_id != delivered.receipt.channel_id
            || self.sender_device_id != delivered.sender_device_id
            || self.sender_counter != delivered.sender_counter
        {
            return Err(MlsError::ContextMismatch);
        }
        Ok(())
    }
}

fn decode_key_package(
    provider: &OpenMlsRustCrypto,
    encoded_key_package: &[u8],
) -> Result<openmls::prelude::KeyPackage> {
    if encoded_key_package.is_empty() || encoded_key_package.len() > MAX_KEY_PACKAGE_BYTES {
        return Err(MlsError::InvalidKeyPackage);
    }
    let message = MlsMessageIn::tls_deserialize_exact(encoded_key_package)
        .map_err(|_| MlsError::InvalidKeyPackage)?;
    let MlsMessageBodyIn::KeyPackage(key_package): MlsMessageBodyIn = message.extract() else {
        return Err(MlsError::InvalidKeyPackage);
    };
    key_package
        .validate(provider.crypto(), ProtocolVersion::Mls10)
        .map_err(|_| MlsError::InvalidKeyPackage)
}

fn credential_device_id(credential: &openmls::prelude::Credential) -> Result<Uuid> {
    if credential.credential_type() != CredentialType::Basic {
        return Err(MlsError::InvalidSenderCredential);
    }
    Uuid::from_slice(credential.serialized_content()).map_err(|_| MlsError::InvalidSenderCredential)
}

fn group_create_config() -> MlsGroupCreateConfig {
    MlsGroupCreateConfig::builder()
        .wire_format_policy(PURE_CIPHERTEXT_WIRE_FORMAT_POLICY)
        .padding_size(MLS_PADDING_BYTES)
        .sender_ratchet_configuration(SenderRatchetConfiguration::new(
            OUT_OF_ORDER_TOLERANCE,
            MAXIMUM_FORWARD_DISTANCE,
        ))
        .max_past_epochs(0)
        .use_ratchet_tree_extension(true)
        .ciphersuite(CIPHERSUITE)
        .build()
}

fn group_join_config() -> MlsGroupJoinConfig {
    MlsGroupJoinConfig::builder()
        .wire_format_policy(PURE_CIPHERTEXT_WIRE_FORMAT_POLICY)
        .padding_size(MLS_PADDING_BYTES)
        .sender_ratchet_configuration(SenderRatchetConfiguration::new(
            OUT_OF_ORDER_TOLERANCE,
            MAXIMUM_FORWARD_DISTANCE,
        ))
        .max_past_epochs(0)
        .use_ratchet_tree_extension(true)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EventContent;
    use openmls::prelude::LeafNodeParameters;
    use singularis_protocol::AcceptedEvent;
    use singularis_vault::{NewVaultMessage, Vault, VaultError};
    use tempfile::tempdir;
    use time::OffsetDateTime;

    struct Pair {
        community_id: Uuid,
        channel_id: Uuid,
        alice: MlsClient,
        bob: MlsClient,
    }

    fn pair() -> Pair {
        let community_id = Uuid::new_v4();
        let channel_id = Uuid::new_v4();
        let mut alice = MlsClient::new(Uuid::new_v4()).unwrap();
        let mut bob = MlsClient::new(Uuid::new_v4()).unwrap();
        let bob_key_package = bob.publish_key_package().unwrap();
        alice.create_channel(community_id, channel_id).unwrap();
        let addition = alice.add_member(channel_id, &bob_key_package).unwrap();
        assert_eq!(addition.member_device_id, bob.device_id());
        bob.join_channel(community_id, channel_id, &addition.welcome)
            .unwrap();
        Pair {
            community_id,
            channel_id,
            alice,
            bob,
        }
    }

    fn delivered(encrypted: &EncryptedEvent) -> DeliveredEvent {
        let submit = encrypted.to_submit_event(ServerTtl::MIN).unwrap();
        DeliveredEvent {
            receipt: AcceptedEvent {
                event_id: submit.event_id,
                channel_id: submit.channel_id,
                sequence: 1,
                accepted_at: OffsetDateTime::UNIX_EPOCH,
                expires_at: OffsetDateTime::UNIX_EPOCH + time::Duration::minutes(5),
            },
            community_id: submit.community_id,
            sender_device_id: submit.sender_device_id,
            sender_counter: submit.sender_counter,
            mls_message: submit.mls_message,
        }
    }

    #[test]
    fn two_devices_exchange_an_encrypted_event() {
        let mut pair = pair();
        let canary = "MLS-CANARY-ONLY-IN-CLIENTS";

        let encrypted = pair
            .alice
            .encrypt_text(
                pair.channel_id,
                1_765_000_000_000,
                EventReferences::default(),
                canary.to_owned(),
            )
            .unwrap();

        assert_eq!(encrypted.community_id, pair.community_id);
        assert!(
            !encrypted
                .mls_message
                .windows(canary.len())
                .any(|window| window == canary.as_bytes())
        );
        let decrypted = pair
            .bob
            .decrypt_event(pair.channel_id, &encrypted.mls_message)
            .unwrap();
        assert_eq!(decrypted.event_id, encrypted.event_id);
        assert_eq!(decrypted.sender_device_id, pair.alice.device_id());
        assert_eq!(
            decrypted.content,
            EventContent::Text(crate::TextContent {
                body: canary.to_owned()
            })
        );
    }

    #[test]
    fn encrypted_client_state_survives_vault_restart() {
        let temporary_directory = tempdir().unwrap();
        let vault_root = temporary_directory.path().join("vault-v1");
        let mut vault = Vault::new(&vault_root);
        vault.initialize("correct horse battery staple").unwrap();
        let community_id = Uuid::new_v4();
        let channel_id = Uuid::new_v4();
        let alice_device_id = Uuid::new_v4();
        let bob_device_id = Uuid::new_v4();
        let mut alice = VaultMlsClient::create(&vault, alice_device_id).unwrap();
        let mut bob = VaultMlsClient::create(&vault, bob_device_id).unwrap();
        let bob_key_package = bob.publish_key_package().unwrap();
        alice.create_channel(community_id, channel_id).unwrap();
        let addition = alice.add_member(channel_id, &bob_key_package).unwrap();
        bob.join_channel(community_id, channel_id, &addition.welcome)
            .unwrap();

        let first = alice
            .queue_text(
                channel_id,
                1,
                EventReferences::default(),
                "before restart".to_owned(),
                ServerTtl::MIN,
            )
            .unwrap();
        bob.decrypt_event(channel_id, &first.mls_message.decode().unwrap())
            .unwrap();
        drop(alice);
        drop(bob);
        vault.lock().unwrap();
        drop(vault);

        let mut reopened_vault = Vault::new(&vault_root);
        reopened_vault
            .unlock("correct horse battery staple")
            .unwrap();
        let mut alice = VaultMlsClient::load(&reopened_vault, alice_device_id)
            .unwrap()
            .unwrap();
        let mut bob = VaultMlsClient::load(&reopened_vault, bob_device_id)
            .unwrap()
            .unwrap();

        let second = alice
            .queue_text(
                channel_id,
                2,
                EventReferences::default(),
                "after restart".to_owned(),
                ServerTtl::MIN,
            )
            .unwrap();
        assert_eq!(second.sender_counter, 2);
        let decrypted = bob
            .decrypt_event(second.channel_id, &second.mls_message.decode().unwrap())
            .unwrap();
        assert_eq!(decrypted.sender_counter, 2);
        assert!(decrypted.previous_event_hash.is_some());
        assert!(
            bob.decrypt_event(first.channel_id, &first.mls_message.decode().unwrap())
                .is_err()
        );

        let reply = bob
            .queue_text(
                second.channel_id,
                3,
                EventReferences::default(),
                "reply after restart".to_owned(),
                ServerTtl::MIN,
            )
            .unwrap();
        assert!(
            alice
                .decrypt_event(reply.channel_id, &reply.mls_message.decode().unwrap())
                .is_ok()
        );
    }

    #[test]
    fn local_message_and_encrypted_outbox_survive_restart_atomically() {
        let temporary_directory = tempdir().unwrap();
        let vault_root = temporary_directory.path().join("vault-v1");
        let mut vault = Vault::new(&vault_root);
        vault.initialize("correct horse battery staple").unwrap();
        let device_id = vault.local_device_id().unwrap();
        let community_id = Uuid::new_v4();
        let channel_id = Uuid::new_v4();
        let mut client = VaultMlsClient::create(&vault, device_id).unwrap();
        client.create_channel(community_id, channel_id).unwrap();
        let first_message = NewVaultMessage {
            id: Uuid::new_v4().to_string(),
            channel_id: "briefing".to_owned(),
            body: "first encrypted desktop message".to_owned(),
            created_at_ms: 1_754_700_000_000,
        };

        let (first, stored) = client
            .queue_text_and_store_message(
                channel_id,
                EventReferences::default(),
                &first_message,
                ServerTtl::MAX,
            )
            .unwrap();
        assert_eq!(first.sender_counter, 1);
        assert_eq!(stored.body, first_message.body);
        let pending = client.pending_submissions().unwrap();
        assert_eq!(pending, vec![first]);
        let payload = &vault.list_mls_outbox(&device_id.to_string()).unwrap()[0].payload;
        assert!(
            !payload
                .windows(stored.body.len())
                .any(|bytes| bytes == stored.body.as_bytes())
        );
        drop(client);
        vault.lock().unwrap();
        drop(vault);

        let mut reopened = Vault::new(&vault_root);
        reopened.unlock("correct horse battery staple").unwrap();
        assert_eq!(reopened.local_device_id().unwrap(), device_id);
        let mut client = VaultMlsClient::load(&reopened, device_id).unwrap().unwrap();
        assert!(client.has_channel(channel_id));
        let second_message = NewVaultMessage {
            id: Uuid::new_v4().to_string(),
            channel_id: "briefing".to_owned(),
            body: "second encrypted desktop message".to_owned(),
            created_at_ms: 1_754_700_001_000,
        };
        let (second, _) = client
            .queue_text_and_store_message(
                channel_id,
                EventReferences::default(),
                &second_message,
                ServerTtl::MAX,
            )
            .unwrap();

        assert_eq!(second.sender_counter, 2);
        assert_eq!(client.pending_submissions().unwrap().len(), 2);
        assert_eq!(reopened.list_messages("briefing").unwrap().len(), 2);
    }

    #[test]
    fn rejected_persisted_message_preserves_ratchet_across_restart() {
        let temporary_directory = tempdir().unwrap();
        let vault_root = temporary_directory.path().join("vault-v1");
        let mut vault = Vault::new(&vault_root);
        vault.initialize("correct horse battery staple").unwrap();
        let community_id = Uuid::new_v4();
        let channel_id = Uuid::new_v4();
        let alice_device_id = Uuid::new_v4();
        let bob_device_id = Uuid::new_v4();
        let mut alice = VaultMlsClient::create(&vault, alice_device_id).unwrap();
        let mut bob = VaultMlsClient::create(&vault, bob_device_id).unwrap();
        let bob_key_package = bob.publish_key_package().unwrap();
        alice.create_channel(community_id, channel_id).unwrap();
        let addition = alice.add_member(channel_id, &bob_key_package).unwrap();
        bob.join_channel(community_id, channel_id, &addition.welcome)
            .unwrap();
        let encrypted = alice
            .queue_text(
                channel_id,
                1,
                EventReferences::default(),
                "authentic after restart".to_owned(),
                ServerTtl::MIN,
            )
            .unwrap();
        let authentic = encrypted.mls_message.decode().unwrap();
        let mut tampered = authentic.clone();
        *tampered.last_mut().unwrap() ^= 0x01;

        assert!(bob.decrypt_event(channel_id, &tampered).is_err());
        drop(alice);
        drop(bob);
        vault.lock().unwrap();
        drop(vault);

        let mut reopened_vault = Vault::new(&vault_root);
        reopened_vault
            .unlock("correct horse battery staple")
            .unwrap();
        let mut bob = VaultMlsClient::load(&reopened_vault, bob_device_id)
            .unwrap()
            .unwrap();
        assert!(bob.decrypt_event(channel_id, &authentic).is_ok());
    }

    #[test]
    fn failed_checkpoint_restores_memory_and_rejects_invalid_snapshots() {
        let temporary_directory = tempdir().unwrap();
        let mut vault = Vault::new(temporary_directory.path().join("vault-v1"));
        vault.initialize("correct horse battery staple").unwrap();
        let device_id = Uuid::new_v4();
        let mut client = VaultMlsClient::create(&vault, device_id).unwrap();
        let oversized_key = b"oversized-checkpoint".to_vec();

        let result = client.transact(|inner| {
            inner
                .provider
                .storage()
                .values
                .write()
                .unwrap()
                .insert(oversized_key.clone(), vec![0; 9 * 1024 * 1024]);
            Ok(())
        });
        assert!(matches!(result, Err(MlsError::StateStorage)));
        assert!(
            !client
                .client
                .provider
                .storage()
                .values
                .read()
                .unwrap()
                .contains_key(&oversized_key)
        );
        drop(client);
        assert!(VaultMlsClient::load(&vault, device_id).unwrap().is_some());

        let invalid_device_id = Uuid::new_v4();
        vault
            .store_mls_client_snapshot(&invalid_device_id.to_string(), b"{}")
            .unwrap();
        assert!(matches!(
            VaultMlsClient::load(&vault, invalid_device_id),
            Err(MlsError::InvalidStateSnapshot)
        ));
    }

    #[test]
    fn outbox_conflict_restores_the_sender_ratchet() {
        let temporary_directory = tempdir().unwrap();
        let mut vault = Vault::new(temporary_directory.path().join("vault-v1"));
        vault.initialize("correct horse battery staple").unwrap();
        let community_id = Uuid::new_v4();
        let channel_id = Uuid::new_v4();
        let alice_device_id = Uuid::new_v4();
        let mut alice = VaultMlsClient::create(&vault, alice_device_id).unwrap();
        let mut bob = MlsClient::new(Uuid::new_v4()).unwrap();
        let bob_key_package = bob.publish_key_package().unwrap();
        alice.create_channel(community_id, channel_id).unwrap();
        let addition = alice.add_member(channel_id, &bob_key_package).unwrap();
        bob.join_channel(community_id, channel_id, &addition.welcome)
            .unwrap();

        let conflicting_event_id = EventId::new();
        let snapshot = alice.client.encode_snapshot().unwrap();
        vault
            .store_mls_snapshot_and_outbox(
                &alice_device_id.to_string(),
                snapshot.as_slice(),
                &conflicting_event_id.as_uuid().to_string(),
                b"different but bounded outbox payload",
            )
            .unwrap();
        let conflicting_event = PlaintextEvent::text(
            conflicting_event_id,
            community_id,
            channel_id,
            alice_device_id,
            1,
            1,
            None,
            EventReferences::default(),
            "must roll back".to_owned(),
        );

        assert!(matches!(
            alice.queue_event(conflicting_event, ServerTtl::MIN),
            Err(MlsError::Vault(VaultError::OutboxConflict))
        ));
        assert!(alice.acknowledge_submission(conflicting_event_id).unwrap());
        let queued = alice
            .queue_text(
                channel_id,
                2,
                EventReferences::default(),
                "first committed event".to_owned(),
                ServerTtl::MIN,
            )
            .unwrap();
        assert_eq!(queued.sender_counter, 1);
        assert!(
            bob.decrypt_event(channel_id, &queued.mls_message.decode().unwrap())
                .is_ok()
        );
    }

    #[test]
    fn relay_contract_contains_only_opaque_mls_and_routing_metadata() {
        let mut pair = pair();
        let canary = "RELAY-MUST-NOT-SEE-THIS-CANARY";
        let encrypted = pair
            .alice
            .encrypt_text(
                pair.channel_id,
                1,
                EventReferences::default(),
                canary.to_owned(),
            )
            .unwrap();

        let submit = encrypted.to_submit_event(ServerTtl::MIN).unwrap();
        let serialized = serde_json::to_string(&submit).unwrap();

        assert!(!serialized.contains(canary));
        assert!(!serialized.contains("ciphertext"));
        assert!(!serialized.contains("previous_event_hash"));
        assert!(serialized.contains("mls_message"));
        assert_eq!(
            pair.bob
                .decrypt_delivered_event(&delivered(&encrypted))
                .unwrap()
                .content,
            EventContent::Text(crate::TextContent {
                body: canary.to_owned()
            })
        );
    }

    #[test]
    fn altered_relay_metadata_is_rejected_without_consuming_the_message() {
        let mut pair = pair();
        let encrypted = pair
            .alice
            .encrypt_text(
                pair.channel_id,
                1,
                EventReferences::default(),
                "metadata bound".to_owned(),
            )
            .unwrap();
        let correct = delivered(&encrypted);
        let mut altered = correct.clone();
        altered.sender_counter += 1;

        assert!(matches!(
            pair.bob.decrypt_delivered_event(&altered),
            Err(MlsError::ContextMismatch)
        ));
        assert!(pair.bob.decrypt_delivered_event(&correct).is_ok());
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        let mut pair = pair();
        let encrypted = pair
            .alice
            .encrypt_text(
                pair.channel_id,
                1,
                EventReferences::default(),
                "authentic".to_owned(),
            )
            .unwrap();
        let mut tampered = encrypted.mls_message.clone();
        let last = tampered.last_mut().unwrap();
        *last ^= 0x01;

        assert!(pair.bob.decrypt_event(pair.channel_id, &tampered).is_err());
        assert!(
            pair.bob
                .decrypt_event(pair.channel_id, &encrypted.mls_message)
                .is_ok()
        );
    }

    #[test]
    fn replayed_mls_message_is_rejected() {
        let mut pair = pair();
        let encrypted = pair
            .alice
            .encrypt_text(
                pair.channel_id,
                1,
                EventReferences::default(),
                "only once".to_owned(),
            )
            .unwrap();
        pair.bob
            .decrypt_event(pair.channel_id, &encrypted.mls_message)
            .unwrap();

        assert!(
            pair.bob
                .decrypt_event(pair.channel_id, &encrypted.mls_message)
                .is_err()
        );
    }

    #[test]
    fn message_for_another_joined_group_is_rejected() {
        let mut pair = pair();
        let other_channel_id = Uuid::new_v4();
        let second_key_package = pair.bob.publish_key_package().unwrap();
        pair.alice
            .create_channel(pair.community_id, other_channel_id)
            .unwrap();
        let addition = pair
            .alice
            .add_member(other_channel_id, &second_key_package)
            .unwrap();
        pair.bob
            .join_channel(pair.community_id, other_channel_id, &addition.welcome)
            .unwrap();
        let encrypted = pair
            .alice
            .encrypt_text(
                pair.channel_id,
                1,
                EventReferences::default(),
                "channel bound".to_owned(),
            )
            .unwrap();

        assert!(matches!(
            pair.bob
                .decrypt_event(other_channel_id, &encrypted.mls_message),
            Err(MlsError::ContextMismatch)
        ));
    }

    #[test]
    fn message_from_an_expired_epoch_is_rejected() {
        let mut pair = pair();
        let old_message = pair
            .alice
            .encrypt_text(
                pair.channel_id,
                1,
                EventReferences::default(),
                "old epoch".to_owned(),
            )
            .unwrap();

        let commit = {
            let alice = &mut pair.alice;
            let state = alice.channels.get_mut(&pair.channel_id).unwrap();
            let (commit, _, _) = state
                .group
                .self_update(
                    &alice.provider,
                    &alice.signer,
                    LeafNodeParameters::default(),
                )
                .unwrap()
                .into_contents();
            state.group.merge_pending_commit(&alice.provider).unwrap();
            commit.to_bytes().unwrap()
        };

        {
            let bob = &mut pair.bob;
            let state = bob.channels.get_mut(&pair.channel_id).unwrap();
            let message = MlsMessageIn::tls_deserialize_exact(commit).unwrap();
            let processed = state
                .group
                .process_message(&bob.provider, message.try_into_protocol_message().unwrap())
                .unwrap();
            let ProcessedMessageContent::StagedCommitMessage(staged_commit) =
                processed.into_content()
            else {
                panic!("expected staged commit");
            };
            state
                .group
                .merge_staged_commit(&bob.provider, *staged_commit)
                .unwrap();
        }

        assert!(matches!(
            pair.bob
                .decrypt_event(pair.channel_id, &old_message.mls_message),
            Err(MlsError::MessageRejected)
        ));
    }
}
