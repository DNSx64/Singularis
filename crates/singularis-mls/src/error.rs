use thiserror::Error;

pub type Result<T> = std::result::Result<T, MlsError>;

#[derive(Debug, Error)]
pub enum MlsError {
    #[error("invalid encrypted event payload: {0}")]
    InvalidPayload(&'static str),
    #[error("client payload is not valid provisional JSON")]
    PayloadEncoding(#[from] serde_json::Error),
    #[error("MLS identity generation failed")]
    IdentityGeneration,
    #[error("MLS state storage failed")]
    StateStorage,
    #[error("MLS state snapshot is invalid")]
    InvalidStateSnapshot,
    #[error("MLS state already exists for this device")]
    StateAlreadyExists,
    #[error("encrypted MLS outbox entry is invalid")]
    InvalidOutboxEntry,
    #[error("encrypted MLS state could not be accessed")]
    Vault(#[from] singularis_vault::VaultError),
    #[error("MLS channel is already joined")]
    ChannelAlreadyJoined,
    #[error("MLS channel is not joined")]
    UnknownChannel,
    #[error("MLS key package is invalid")]
    InvalidKeyPackage,
    #[error("MLS welcome is invalid")]
    InvalidWelcome,
    #[error("MLS message encoding failed")]
    MessageEncoding,
    #[error("MLS message was rejected")]
    MessageRejected,
    #[error("unexpected MLS message type")]
    UnexpectedMessageType,
    #[error("MLS message context does not match its encrypted event")]
    ContextMismatch,
    #[error("MLS sender credential is invalid")]
    InvalidSenderCredential,
    #[error("event was already processed")]
    ReplayDetected,
    #[error("event sender chain is invalid")]
    InvalidSenderChain,
}
