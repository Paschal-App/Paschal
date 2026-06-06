//! AWS-backed KMS for production.
//!
//! CRITICAL INVARIANT: there is exactly **one persistent DEK per environment**.
//! It is generated once at bootstrap, wrapped (encrypted) under the
//! environment's customer-managed CMK, and the resulting ciphertext is stored
//! in Secrets Manager. At boot the service fetches that ciphertext, calls
//! `kms:Decrypt` **once**, and caches the 32-byte plaintext for the process
//! lifetime.
//!
//! We do NOT call `GenerateDataKey` per seal. Doing so would mint a new key
//! every time and make all previously-sealed ciphertext undecryptable — the
//! exact corruption class that has already bitten this project once in dev.
//! The plaintext DEK is never written to disk in AWS.

use aws_sdk_kms::primitives::Blob;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use tokio::sync::OnceCell;

use crate::{CryptoError, Kms};

/// Production KMS: a single wrapped DEK, decrypted once and cached.
pub struct AwsKms {
    kms: aws_sdk_kms::Client,
    secrets: aws_sdk_secretsmanager::Client,
    /// Secrets Manager secret id/ARN holding the base64 CMK-wrapped DEK.
    wrapped_dek_secret_id: String,
    /// Cached plaintext DEK — populated on first `data_key()` call.
    cached: OnceCell<[u8; 32]>,
}

impl AwsKms {
    pub async fn from_env(wrapped_dek_secret_id: impl Into<String>) -> Self {
        let cfg = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        Self {
            kms: aws_sdk_kms::Client::new(&cfg),
            secrets: aws_sdk_secretsmanager::Client::new(&cfg),
            wrapped_dek_secret_id: wrapped_dek_secret_id.into(),
            cached: OnceCell::new(),
        }
    }

    async fn load_dek(&self) -> Result<[u8; 32], CryptoError> {
        // 1. Fetch the base64 CMK-wrapped DEK ciphertext from Secrets Manager.
        let secret = self
            .secrets
            .get_secret_value()
            .secret_id(&self.wrapped_dek_secret_id)
            .send()
            .await
            .map_err(|e| CryptoError::Aead(format!("secretsmanager get: {e}")))?;
        let b64 = secret
            .secret_string()
            .ok_or_else(|| CryptoError::Aead("wrapped DEK secret has no string value".into()))?;
        let wrapped = STANDARD
            .decode(b64.trim())
            .map_err(|e| CryptoError::Aead(format!("wrapped DEK base64: {e}")))?;

        // 2. Decrypt (unwrap) under the CMK — exactly once per process.
        let out = self
            .kms
            .decrypt()
            .ciphertext_blob(Blob::new(wrapped))
            .send()
            .await
            .map_err(|e| CryptoError::Aead(format!("kms decrypt: {e}")))?;
        let plaintext = out
            .plaintext()
            .ok_or_else(|| CryptoError::Aead("kms decrypt returned no plaintext".into()))?;
        let bytes = plaintext.as_ref();
        if bytes.len() != 32 {
            return Err(CryptoError::KmsKeyLength(bytes.len()));
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(bytes);
        Ok(key)
    }
}

#[async_trait::async_trait]
impl Kms for AwsKms {
    async fn data_key(&self) -> Result<[u8; 32], CryptoError> {
        let key = self
            .cached
            .get_or_try_init(|| async { self.load_dek().await })
            .await?;
        Ok(*key)
    }
}
