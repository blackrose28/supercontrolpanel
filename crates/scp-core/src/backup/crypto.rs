//! v1: magic + bounded JSON header length + exact header bytes (AAD) + GCM ciphertext.
use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Nonce,
};
use argon2::{Algorithm, Argon2, Params, Version};
use rand::{rngs::OsRng, RngCore};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use zeroize::Zeroizing;

const MAGIC: &[u8; 8] = b"SCPBAK01";
pub const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_HEADER: usize = 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    version: u32,
    kdf: String,
    cipher: String,
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
    salt: [u8; 16],
    nonce: [u8; 12],
}

pub fn validate_cost(memory: u32, iterations: u32, parallelism: u32) -> Result<(), String> {
    if !(8192..=65536).contains(&memory) || !(1..=3).contains(&iterations) || parallelism != 1 {
        return Err("Unsupported or excessive backup KDF cost".into());
    }
    Ok(())
}

fn derive(header: &Header, passphrase: &str) -> Result<Zeroizing<[u8; 32]>, String> {
    validate_cost(header.memory_kib, header.iterations, header.parallelism)?;
    if passphrase.is_empty() || passphrase.len() > 4096 {
        return Err("Backup passphrase must contain 1–4096 bytes".into());
    }
    let params = Params::new(
        header.memory_kib,
        header.iterations,
        header.parallelism,
        Some(32),
    )
    .map_err(|_| "Invalid backup KDF parameters")?;
    let mut key = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(passphrase.as_bytes(), &header.salt, key.as_mut())
        .map_err(|_| "Backup key derivation failed")?;
    Ok(key)
}

pub fn encrypt<T: Serialize>(payload: &T, passphrase: &str) -> Result<Vec<u8>, String> {
    let mut header = Header {
        version: 1,
        kdf: "argon2id-19".into(),
        cipher: "aes-256-gcm".into(),
        memory_kib: 65536,
        iterations: 3,
        parallelism: 1,
        salt: [0; 16],
        nonce: [0; 12],
    };
    OsRng.fill_bytes(&mut header.salt);
    OsRng.fill_bytes(&mut header.nonce);
    let key = derive(&header, passphrase)?;
    let raw = serde_json::to_vec(&header).map_err(|_| "Could not encode backup header")?;
    let plaintext =
        Zeroizing::new(serde_json::to_vec(payload).map_err(|_| "Could not encode backup")?);
    if plaintext.len() > MAX_BYTES - MAX_HEADER - 32 {
        return Err("Backup is too large".into());
    }
    let mut output = MAGIC.to_vec();
    output.extend_from_slice(&(raw.len() as u32).to_le_bytes());
    output.extend_from_slice(&raw);
    let encrypted = seal(&key, &header.nonce, &plaintext, &output)?;
    output.extend(encrypted);
    Ok(output)
}

pub fn decrypt<T: DeserializeOwned>(bytes: &[u8], passphrase: &str) -> Result<T, String> {
    if bytes.len() < 28 || bytes.len() > MAX_BYTES || &bytes[..8] != MAGIC {
        return Err("Invalid or unsupported settings backup".into());
    }
    let length = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    if length > MAX_HEADER || length + 28 > bytes.len() {
        return Err("Invalid backup header length".into());
    }
    let end = 12 + length;
    let header: Header =
        serde_json::from_slice(&bytes[12..end]).map_err(|_| "Invalid backup header")?;
    if header.version != 1 || header.kdf != "argon2id-19" || header.cipher != "aes-256-gcm" {
        return Err("Unsupported backup version or algorithm".into());
    }
    let key = derive(&header, passphrase)?;
    let plain = open(&key, &header.nonce, &bytes[end..], &bytes[..end])?;
    serde_json::from_slice(&plain).map_err(|_| "Invalid or unsupported backup payload".into())
}

pub fn seal(key: &[u8; 32], nonce: &[u8; 12], bytes: &[u8], aad: &[u8]) -> Result<Vec<u8>, String> {
    Aes256Gcm::new_from_slice(key)
        .map_err(|_| "Invalid encryption key")?
        .encrypt(Nonce::from_slice(nonce), Payload { msg: bytes, aad })
        .map_err(|_| "Backup encryption failed".into())
}

pub fn open(
    key: &[u8; 32],
    nonce: &[u8; 12],
    bytes: &[u8],
    aad: &[u8],
) -> Result<Zeroizing<Vec<u8>>, String> {
    Aes256Gcm::new_from_slice(key)
        .map_err(|_| "Invalid encryption key")?
        .decrypt(Nonce::from_slice(nonce), Payload { msg: bytes, aad })
        .map(Zeroizing::new)
        .map_err(|_| "Wrong backup passphrase, unavailable recovery key, or damaged backup".into())
}
