use singularis_mls::{EventReferences, MlsError, VaultMlsClient};
use singularis_protocol::ServerTtl;
use singularis_vault::{NewVaultMessage, Vault, VaultError, VaultMessage};
use thiserror::Error;
use uuid::Uuid;

const NEXUS_LAB_ID: Uuid = Uuid::from_u128(0x01989550_e7f8_7000_8000_000000000100);
const BRIEFING_ID: Uuid = Uuid::from_u128(0x01989550_e7f8_7000_8000_000000000101);
const DEVELOPMENT_ID: Uuid = Uuid::from_u128(0x01989550_e7f8_7000_8000_000000000102);
const SECURITY_ID: Uuid = Uuid::from_u128(0x01989550_e7f8_7000_8000_000000000103);
const OFFTOPIC_ID: Uuid = Uuid::from_u128(0x01989550_e7f8_7000_8000_000000000104);

#[derive(Debug, Error)]
pub(crate) enum MessagingError {
    #[error("the local channel is not part of the prototype MLS context")]
    UnknownChannel,
    #[error(transparent)]
    Vault(#[from] VaultError),
    #[error(transparent)]
    Mls(#[from] MlsError),
}

impl MessagingError {
    pub(crate) const fn code(&self) -> &'static str {
        match self {
            Self::UnknownChannel => "unknown_channel",
            Self::Vault(error) | Self::Mls(MlsError::Vault(error)) => error.code(),
            Self::Mls(_) => "mls_failed",
        }
    }

    pub(crate) const fn public_message(&self) -> &'static str {
        match self {
            Self::UnknownChannel => "Dieser Kanal ist noch nicht fuer MLS eingerichtet.",
            Self::Vault(error) | Self::Mls(MlsError::Vault(error)) => error.public_message(),
            Self::Mls(_) => "Die Nachricht konnte nicht verschluesselt werden.",
        }
    }
}

pub(crate) fn queue_message(
    vault: &Vault,
    message: &NewVaultMessage,
) -> Result<VaultMessage, MessagingError> {
    let channel_id = protocol_channel_id(&message.channel_id)?;
    let device_id = vault.local_device_id()?;
    let mut client = match VaultMlsClient::load(vault, device_id)? {
        Some(client) => client,
        None => VaultMlsClient::create(vault, device_id)?,
    };
    if !client.has_channel(channel_id) {
        client.create_channel(NEXUS_LAB_ID, channel_id)?;
    }
    let (_, stored) = client.queue_text_and_store_message(
        channel_id,
        EventReferences::default(),
        message,
        ServerTtl::MAX,
    )?;
    Ok(stored)
}

fn protocol_channel_id(local_channel_id: &str) -> Result<Uuid, MessagingError> {
    match local_channel_id {
        "briefing" => Ok(BRIEFING_ID),
        "entwicklung" => Ok(DEVELOPMENT_ID),
        "security" => Ok(SECURITY_ID),
        "offtopic" => Ok(OFFTOPIC_ID),
        _ => Err(MessagingError::UnknownChannel),
    }
}

#[cfg(test)]
mod tests {
    use singularis_protocol::SubmitEvent;
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn queue_message_bootstraps_mls_and_commits_an_opaque_request() {
        let temporary_directory = tempdir().unwrap();
        let mut vault = Vault::new(temporary_directory.path().join("vault-v1"));
        vault.initialize("correct horse battery staple").unwrap();
        let message = NewVaultMessage {
            id: Uuid::new_v4().to_string(),
            channel_id: "briefing".to_owned(),
            body: "native composer MLS canary".to_owned(),
            created_at_ms: 1_754_700_000_000,
        };

        let stored = queue_message(&vault, &message).unwrap();

        assert_eq!(stored.body, message.body);
        let outbox = vault.list_all_mls_outbox().unwrap();
        assert_eq!(outbox.len(), 1);
        assert!(
            !outbox[0]
                .payload
                .windows(message.body.len())
                .any(|bytes| bytes == message.body.as_bytes())
        );
        let submission: SubmitEvent = serde_json::from_slice(&outbox[0].payload).unwrap();
        assert_eq!(submission.community_id, NEXUS_LAB_ID);
        assert_eq!(submission.channel_id, BRIEFING_ID);
        assert_eq!(
            submission.sender_device_id,
            vault.local_device_id().unwrap()
        );
        assert_eq!(submission.sender_counter, 1);
    }

    #[test]
    fn unknown_local_channel_is_rejected_before_identity_creation() {
        let temporary_directory = tempdir().unwrap();
        let mut vault = Vault::new(temporary_directory.path().join("vault-v1"));
        vault.initialize("correct horse battery staple").unwrap();
        let message = NewVaultMessage {
            id: Uuid::new_v4().to_string(),
            channel_id: "not-a-channel".to_owned(),
            body: "must not be queued".to_owned(),
            created_at_ms: 1_754_700_000_000,
        };

        assert!(matches!(
            queue_message(&vault, &message),
            Err(MessagingError::UnknownChannel)
        ));
        assert!(vault.list_all_mls_outbox().unwrap().is_empty());
    }
}
