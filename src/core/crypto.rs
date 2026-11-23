use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{anyhow, Context};
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::core::model::{FerroError, Result};

type HmacSha256 = Hmac<Sha256>;

/// Cryptographic utilities for secret envelope encryption, token hashing, and Blob HMAC presigning.
#[derive(Clone)]
pub struct Crypto {
    key_bytes: [u8; 32],
}

impl Crypto {
    /// Derive a 32-byte key from `master_key` using SHA-256.
    pub fn from_master_key(master_key: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"ferropod:master-key:v1:");
        hasher.update(master_key.as_bytes());
        let digest = hasher.finalize();
        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(&digest);
        Self { key_bytes }
    }

    /// Encrypt a secret value using AES-256-GCM with a random 96-bit nonce.
    pub fn encrypt_secret(&self, plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
        let cipher = Aes256Gcm::new_from_slice(&self.key_bytes)
            .map_err(|e| FerroError::Internal(anyhow!("aes key init failed: {e}")))?;
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| FerroError::Internal(anyhow!("aes-gcm encrypt failed: {e}")))?;
        Ok((nonce_bytes.to_vec(), ciphertext))
    }

    /// Decrypt an AES-256-GCM secret value.
    pub fn decrypt_secret(&self, nonce_bytes: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>> {
        if nonce_bytes.len() != 12 {
            return Err(FerroError::Internal(anyhow!(
                "invalid aes-gcm nonce length"
            )));
        }
        let cipher = Aes256Gcm::new_from_slice(&self.key_bytes)
            .map_err(|e| FerroError::Internal(anyhow!("aes key init failed: {e}")))?;
        let nonce = Nonce::from_slice(nonce_bytes);
        cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| FerroError::Internal(anyhow!("aes-gcm decrypt failed: {e}")))
    }

    /// Hash a bearer token with SHA-256 hex digest for storage lookup.
    pub fn hash_token(raw_token: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(raw_token.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Generate a random bearer token (`fp_<hex>`).
    pub fn generate_token() -> String {
        let mut raw = [0u8; 24];
        rand::thread_rng().fill_bytes(&mut raw);
        format!("fp_{}", hex::encode(raw))
    }

    /// Sign a presigned Blob URL operation (`get` or `put`).
    pub fn sign_blob_op(&self, op: &str, bucket: &str, key: &str, expires_at: i64) -> String {
        let mut mac =
            <HmacSha256 as Mac>::new_from_slice(&self.key_bytes).expect("HMAC accepts 32-byte key");
        mac.update(format!("{op}:{bucket}:{key}:{expires_at}").as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    /// Compute lowercase hex-encoded HMAC-SHA256 over `payload` with `secret`.
    pub fn hmac_sha256_hex(secret: &[u8], payload: &[u8]) -> String {
        let mut mac =
            <HmacSha256 as Mac>::new_from_slice(secret).expect("HMAC accepts arbitrary key size");
        mac.update(payload);
        hex::encode(mac.finalize().into_bytes())
    }

    /// Verify a presigned Blob URL signature in constant time and check expiry.
    pub fn verify_blob_op(
        &self,
        op: &str,
        bucket: &str,
        key: &str,
        expires_at: i64,
        sig_hex: &str,
        now_secs: i64,
    ) -> Result<()> {
        if now_secs > expires_at {
            return Err(FerroError::Unauthorized("presigned URL expired".into()));
        }
        let sig_bytes = hex::decode(sig_hex)
            .context("invalid presign signature encoding")
            .map_err(|e| FerroError::Unauthorized(e.to_string()))?;
        let mut mac =
            <HmacSha256 as Mac>::new_from_slice(&self.key_bytes).expect("HMAC accepts 32-byte key");
        mac.update(format!("{op}:{bucket}:{key}:{expires_at}").as_bytes());
        mac.verify_slice(&sig_bytes)
            .map_err(|_| FerroError::Unauthorized("invalid presign signature".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_aes_gcm_roundtrip() {
        let crypto = Crypto::from_master_key("super-secret-master-key");
        let (nonce, ct) = crypto.encrypt_secret(b"db_pass_123").unwrap();
        assert_ne!(ct, b"db_pass_123");
        let pt = crypto.decrypt_secret(&nonce, &ct).unwrap();
        assert_eq!(pt, b"db_pass_123");

        let other = Crypto::from_master_key("wrong-key");
        assert!(other.decrypt_secret(&nonce, &ct).is_err());
    }

    #[test]
    fn blob_hmac_presign_and_expiry() {
        let crypto = Crypto::from_master_key("presign-master");
        let sig = crypto.sign_blob_op("put", "assets", "logo.png", 1_700_000_100);
        assert!(crypto
            .verify_blob_op(
                "put",
                "assets",
                "logo.png",
                1_700_000_100,
                &sig,
                1_700_000_000
            )
            .is_ok());
        assert!(crypto
            .verify_blob_op(
                "get",
                "assets",
                "logo.png",
                1_700_000_100,
                &sig,
                1_700_000_000
            )
            .is_err());
        assert!(crypto
            .verify_blob_op(
                "put",
                "assets",
                "logo.png",
                1_700_000_100,
                &sig,
                1_700_000_101
            )
            .is_err());
    }
}
