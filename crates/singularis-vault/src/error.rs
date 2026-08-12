use thiserror::Error;

pub type Result<T> = std::result::Result<T, VaultError>;

#[derive(Debug, Error)]
pub enum VaultError {
    #[error("the vault is already initialized")]
    AlreadyInitialized,
    #[error("the vault is not initialized")]
    NotInitialized,
    #[error("the vault is locked")]
    Locked,
    #[error("the passphrase is incorrect or the key header was modified")]
    AuthenticationFailed,
    #[error("the passphrase does not meet the minimum requirements")]
    WeakPassphrase,
    #[error("invalid vault input: {0}")]
    InvalidInput(&'static str),
    #[error("the vault files are incomplete or damaged")]
    CorruptState,
    #[error("the MLS outbox event ID is already bound to different data")]
    OutboxConflict,
    #[error("unsupported vault key header version {0}")]
    UnsupportedVersion(u32),
    #[error("SQLCipher is unavailable")]
    SqlCipherUnavailable,
    #[error("a cryptographic operation failed")]
    Crypto,
    #[error("a filesystem operation failed")]
    Io(#[from] std::io::Error),
    #[error("a database operation failed")]
    Database(#[from] rusqlite::Error),
    #[error("vault metadata is invalid")]
    Metadata(#[from] serde_json::Error),
}

impl VaultError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::AlreadyInitialized => "already_initialized",
            Self::NotInitialized => "not_initialized",
            Self::Locked => "locked",
            Self::AuthenticationFailed => "authentication_failed",
            Self::WeakPassphrase => "weak_passphrase",
            Self::InvalidInput(_) => "invalid_input",
            Self::CorruptState => "corrupt_state",
            Self::OutboxConflict => "outbox_conflict",
            Self::UnsupportedVersion(_) => "unsupported_version",
            Self::SqlCipherUnavailable => "sqlcipher_unavailable",
            Self::Crypto => "crypto_failed",
            Self::Io(_) => "io_failed",
            Self::Database(_) => "database_failed",
            Self::Metadata(_) => "metadata_invalid",
        }
    }

    #[must_use]
    pub const fn public_message(&self) -> &'static str {
        match self {
            Self::AlreadyInitialized => "Der lokale Vault ist bereits eingerichtet.",
            Self::NotInitialized => "Der lokale Vault ist noch nicht eingerichtet.",
            Self::Locked => "Der lokale Vault ist gesperrt.",
            Self::AuthenticationFailed => "Passphrase falsch oder Schluesseldatei beschaedigt.",
            Self::WeakPassphrase => "Die Passphrase muss mindestens 12 Zeichen lang sein.",
            Self::InvalidInput(message) => message,
            Self::CorruptState => "Die lokalen Vault-Dateien sind unvollstaendig oder beschaedigt.",
            Self::OutboxConflict => {
                "Der lokale Sendeauftrag steht im Konflikt mit vorhandenen Daten."
            }
            Self::UnsupportedVersion(_) => "Diese Vault-Version wird nicht unterstuetzt.",
            Self::SqlCipherUnavailable => "SQLCipher ist in diesem Build nicht verfuegbar.",
            Self::Crypto | Self::Io(_) | Self::Database(_) | Self::Metadata(_) => {
                "Der lokale Vault konnte nicht verarbeitet werden."
            }
        }
    }
}
