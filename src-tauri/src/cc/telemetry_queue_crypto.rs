use aes_gcm::{
    aead::{rand_core::RngCore, Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};

use crate::services::{keyring_get_optional, keyring_set_or_delete};

const KEYRING_CC_TELEMETRY_QUEUE_KEY: &str = "cc-telemetry-queue-key";
const TELEMETRY_QUEUE_KEY_LEN: usize = 32;
const TELEMETRY_QUEUE_NONCE_LEN: usize = 12;

pub(super) struct TelemetryQueueCrypto {
    cipher: Aes256Gcm,
    key_id: String,
}

impl TelemetryQueueCrypto {
    pub(super) fn load_or_create() -> Result<Self, String> {
        let key_bytes = match keyring_get_optional(KEYRING_CC_TELEMETRY_QUEUE_KEY)? {
            Some(encoded) if !encoded.trim().is_empty() => decode_hex(&encoded)?,
            _ => {
                let mut generated = [0u8; TELEMETRY_QUEUE_KEY_LEN];
                OsRng.fill_bytes(&mut generated);
                keyring_set_or_delete(KEYRING_CC_TELEMETRY_QUEUE_KEY, &encode_hex(&generated))?;
                generated.to_vec()
            }
        };

        if key_bytes.len() != TELEMETRY_QUEUE_KEY_LEN {
            return Err("telemetry queue key has invalid length".to_string());
        }

        let key_id = format!("kq-{}", encode_hex(&key_bytes[..8]));
        let cipher = Aes256Gcm::new_from_slice(&key_bytes)
            .map_err(|e| format!("failed to initialize telemetry queue cipher: {e}"))?;
        Ok(Self { cipher, key_id })
    }

    pub(super) fn key_id(&self) -> &str {
        &self.key_id
    }

    pub(super) fn encrypt_record(&self, plaintext: &[u8]) -> Result<EncryptedRecord, String> {
        let mut nonce_bytes = [0u8; TELEMETRY_QUEUE_NONCE_LEN];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext_with_tag = self
            .cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| format!("failed to encrypt telemetry queue record: {e}"))?;

        if ciphertext_with_tag.len() < 16 {
            return Err("encrypted telemetry queue record missing authentication tag".to_string());
        }
        let split_at = ciphertext_with_tag.len() - 16;
        Ok(EncryptedRecord {
            format_version: 1,
            key_id: self.key_id.clone(),
            nonce: encode_hex(&nonce_bytes),
            ciphertext: encode_hex(&ciphertext_with_tag[..split_at]),
            tag: encode_hex(&ciphertext_with_tag[split_at..]),
        })
    }

    pub(super) fn decrypt_record(&self, encrypted: &EncryptedRecord) -> Result<Vec<u8>, String> {
        if encrypted.key_id != self.key_id {
            return Err(format!(
                "telemetry queue key id mismatch: record={}, local={}",
                encrypted.key_id, self.key_id
            ));
        }
        if encrypted.format_version != 1 {
            return Err(format!(
                "unsupported telemetry queue encrypted record version {}",
                encrypted.format_version
            ));
        }

        let nonce_bytes = decode_hex(&encrypted.nonce)?;
        if nonce_bytes.len() != TELEMETRY_QUEUE_NONCE_LEN {
            return Err("telemetry queue record nonce has invalid length".to_string());
        }
        let ciphertext = decode_hex(&encrypted.ciphertext)?;
        let tag = decode_hex(&encrypted.tag)?;

        let mut combined = Vec::with_capacity(ciphertext.len() + tag.len());
        combined.extend_from_slice(&ciphertext);
        combined.extend_from_slice(&tag);

        self.cipher
            .decrypt(Nonce::from_slice(&nonce_bytes), combined.as_ref())
            .map_err(|e| format!("failed to decrypt telemetry queue record: {e}"))
    }

    #[cfg(test)]
    pub(super) fn from_key_bytes_for_test(
        key_bytes: [u8; TELEMETRY_QUEUE_KEY_LEN],
    ) -> Result<Self, String> {
        let key_id = format!("kq-{}", encode_hex(&key_bytes[..8]));
        let cipher = Aes256Gcm::new_from_slice(&key_bytes)
            .map_err(|e| format!("failed to initialize telemetry queue cipher: {e}"))?;
        Ok(Self { cipher, key_id })
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct EncryptedRecord {
    pub(super) format_version: u32,
    pub(super) key_id: String,
    pub(super) nonce: String,
    pub(super) ciphertext: String,
    pub(super) tag: String,
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn decode_hex(input: &str) -> Result<Vec<u8>, String> {
    let trimmed = input.trim();
    if trimmed.len() % 2 != 0 {
        return Err("invalid hex string length".to_string());
    }

    let mut output = Vec::with_capacity(trimmed.len() / 2);
    let bytes = trimmed.as_bytes();
    for chunk in bytes.chunks(2) {
        let high = decode_hex_nibble(chunk[0])?;
        let low = decode_hex_nibble(chunk[1])?;
        output.push((high << 4) | low);
    }
    Ok(output)
}

fn decode_hex_nibble(byte: u8) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err("invalid hex character".to_string()),
    }
}
