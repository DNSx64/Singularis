use argon2::{Algorithm, Argon2, Params, Version};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::{Result, VaultError};

const MAGIC: &str = "singularis-vault-key";
const HEADER_VERSION: u32 = 1;
const ARGON2_VERSION: u32 = 0x13;
const DATA_KEY_LEN: usize = 32;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const WRAPPED_KEY_LEN: usize = DATA_KEY_LEN + 16;
const DEFAULT_MEMORY_KIB: u32 = 64 * 1024;
const DEFAULT_ITERATIONS: u32 = 3;
const DEFAULT_LANES: u32 = 1;
const MAX_MEMORY_KIB: u32 = 1024 * 1024;
const MAX_ITERATIONS: u32 = 10;
const MAX_LANES: u32 = 16;
const MAX_PASSPHRASE_BYTES: usize = 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyHeader {
    magic: String,
    version: u32,
    kdf: KdfHeader,
    nonce: String,
    wrapped_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KdfHeader {
    algorithm: String,
    version: u32,
    memory_kib: u32,
    iterations: u32,
    lanes: u32,
    salt: String,
}

pub(crate) fn create(passphrase: &str) -> Result<(Vec<u8>, Zeroizing<[u8; DATA_KEY_LEN]>)> {
    validate_new_passphrase(passphrase)?;

    let mut salt = [0_u8; SALT_LEN];
    let mut nonce = [0_u8; NONCE_LEN];
    let mut data_key = Zeroizing::new([0_u8; DATA_KEY_LEN]);
    getrandom::fill(&mut salt).map_err(|_| VaultError::Crypto)?;
    getrandom::fill(&mut nonce).map_err(|_| VaultError::Crypto)?;
    getrandom::fill(data_key.as_mut()).map_err(|_| VaultError::Crypto)?;

    let kdf = KdfHeader {
        algorithm: "argon2id".to_owned(),
        version: ARGON2_VERSION,
        memory_kib: DEFAULT_MEMORY_KIB,
        iterations: DEFAULT_ITERATIONS,
        lanes: DEFAULT_LANES,
        salt: URL_SAFE_NO_PAD.encode(salt),
    };
    let wrapping_key = derive_wrapping_key(passphrase, &salt, &kdf)?;
    let cipher =
        XChaCha20Poly1305::new_from_slice(wrapping_key.as_ref()).map_err(|_| VaultError::Crypto)?;
    let associated_data = associated_data(HEADER_VERSION, &kdf, &salt);
    let wrapped_key = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: data_key.as_ref(),
                aad: &associated_data,
            },
        )
        .map_err(|_| VaultError::Crypto)?;

    let header = KeyHeader {
        magic: MAGIC.to_owned(),
        version: HEADER_VERSION,
        kdf,
        nonce: URL_SAFE_NO_PAD.encode(nonce),
        wrapped_key: URL_SAFE_NO_PAD.encode(wrapped_key),
    };

    Ok((serde_json::to_vec_pretty(&header)?, data_key))
}

pub(crate) fn unlock(
    header_bytes: &[u8],
    passphrase: &str,
) -> Result<Zeroizing<[u8; DATA_KEY_LEN]>> {
    if passphrase.len() > MAX_PASSPHRASE_BYTES {
        return Err(VaultError::AuthenticationFailed);
    }

    let header: KeyHeader =
        serde_json::from_slice(header_bytes).map_err(|_| VaultError::CorruptState)?;
    validate_header(&header)?;

    let salt = decode_array::<SALT_LEN>(&header.kdf.salt)?;
    let nonce = decode_array::<NONCE_LEN>(&header.nonce)?;
    let wrapped_key = URL_SAFE_NO_PAD
        .decode(&header.wrapped_key)
        .map_err(|_| VaultError::CorruptState)?;
    if wrapped_key.len() != WRAPPED_KEY_LEN {
        return Err(VaultError::CorruptState);
    }

    let wrapping_key = derive_wrapping_key(passphrase, &salt, &header.kdf)?;
    let cipher =
        XChaCha20Poly1305::new_from_slice(wrapping_key.as_ref()).map_err(|_| VaultError::Crypto)?;
    let associated_data = associated_data(header.version, &header.kdf, &salt);
    let plaintext = Zeroizing::new(
        cipher
            .decrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &wrapped_key,
                    aad: &associated_data,
                },
            )
            .map_err(|_| VaultError::AuthenticationFailed)?,
    );
    if plaintext.len() != DATA_KEY_LEN {
        return Err(VaultError::AuthenticationFailed);
    }

    let mut data_key = Zeroizing::new([0_u8; DATA_KEY_LEN]);
    data_key.copy_from_slice(plaintext.as_slice());
    Ok(data_key)
}

fn validate_new_passphrase(passphrase: &str) -> Result<()> {
    if passphrase.chars().count() < 12 || passphrase.len() > MAX_PASSPHRASE_BYTES {
        return Err(VaultError::WeakPassphrase);
    }

    Ok(())
}

fn validate_header(header: &KeyHeader) -> Result<()> {
    if header.magic != MAGIC || header.kdf.algorithm != "argon2id" {
        return Err(VaultError::CorruptState);
    }
    if header.version != HEADER_VERSION {
        return Err(VaultError::UnsupportedVersion(header.version));
    }
    if header.kdf.version != ARGON2_VERSION
        || !(DEFAULT_MEMORY_KIB..=MAX_MEMORY_KIB).contains(&header.kdf.memory_kib)
        || !(DEFAULT_ITERATIONS..=MAX_ITERATIONS).contains(&header.kdf.iterations)
        || !(DEFAULT_LANES..=MAX_LANES).contains(&header.kdf.lanes)
    {
        return Err(VaultError::CorruptState);
    }

    Ok(())
}

fn derive_wrapping_key(
    passphrase: &str,
    salt: &[u8; SALT_LEN],
    kdf: &KdfHeader,
) -> Result<Zeroizing<[u8; DATA_KEY_LEN]>> {
    let params = Params::new(
        kdf.memory_kib,
        kdf.iterations,
        kdf.lanes,
        Some(DATA_KEY_LEN),
    )
    .map_err(|_| VaultError::CorruptState)?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut wrapping_key = Zeroizing::new([0_u8; DATA_KEY_LEN]);
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, wrapping_key.as_mut())
        .map_err(|_| VaultError::Crypto)?;
    Ok(wrapping_key)
}

fn associated_data(version: u32, kdf: &KdfHeader, salt: &[u8; SALT_LEN]) -> Vec<u8> {
    let mut data = Vec::with_capacity(64);
    data.extend_from_slice(MAGIC.as_bytes());
    data.push(0);
    data.extend_from_slice(&version.to_le_bytes());
    data.extend_from_slice(&kdf.version.to_le_bytes());
    data.extend_from_slice(&kdf.memory_kib.to_le_bytes());
    data.extend_from_slice(&kdf.iterations.to_le_bytes());
    data.extend_from_slice(&kdf.lanes.to_le_bytes());
    data.extend_from_slice(salt);
    data
}

fn decode_array<const LENGTH: usize>(encoded: &str) -> Result<[u8; LENGTH]> {
    let decoded = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| VaultError::CorruptState)?;
    decoded.try_into().map_err(|_| VaultError::CorruptState)
}
