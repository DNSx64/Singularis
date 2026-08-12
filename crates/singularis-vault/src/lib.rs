#![forbid(unsafe_code)]

mod error;
mod key_header;
mod vault;

pub use error::{Result, VaultError};
pub use vault::{NewVaultMessage, Vault, VaultMessage, VaultOutboxItem, VaultState, VaultStatus};
