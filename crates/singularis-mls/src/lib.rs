#![forbid(unsafe_code)]

mod client;
mod error;
mod event;

use openmls::prelude::Ciphersuite;
use openmls_rust_crypto::OpenMlsRustCrypto;

pub use client::{EncryptedEvent, MemberAddition, MlsClient, VaultMlsClient};
pub use error::{MlsError, Result};
pub use event::{EVENT_SCHEMA_VERSION, EventContent, EventReferences, PlaintextEvent, TextContent};

pub const CIPHERSUITE: Ciphersuite =
    Ciphersuite::MLS_128_DHKEMX25519_CHACHA20POLY1305_SHA256_Ed25519;

#[must_use]
pub fn in_memory_provider() -> OpenMlsRustCrypto {
    OpenMlsRustCrypto::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use openmls::prelude::SignatureScheme;

    #[test]
    fn stable_openmls_stack_is_available() {
        let _provider = in_memory_provider();
        assert_eq!(CIPHERSUITE.signature_algorithm(), SignatureScheme::ED25519);
    }
}
