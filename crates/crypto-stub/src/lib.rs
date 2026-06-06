//! Walking-skeleton crypto: AES-256-GCM with a local-file KMS.
//!
//! This crate intentionally does not implement Shamir secret sharing, HPKE,
//! or any Tier-2 primitive. It exists so the skeleton compiles and Tier-1
//! sealing works end-to-end. The production replacement is `crates/crypto`
//! per [`style-guide/04-code-style.md`].
//!
//! The `Kms` trait below is the *real* contract — AWS KMS, HashiCorp Vault,
//! or anything else slots in by implementing it.

use std::path::{Path, PathBuf};

use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Key, Nonce,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
};

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("AES-GCM: {0}")]
    Aead(String),

    #[error("KMS key file is the wrong length (expected 32 bytes, got {0})")]
    KmsKeyLength(usize),

    #[error("ciphertext decoded but did not validate")]
    InvalidCiphertext,

    #[error("base64: {0}")]
    Base64(#[from] base64::DecodeError),
}

impl From<aes_gcm::Error> for CryptoError {
    fn from(e: aes_gcm::Error) -> Self {
        Self::Aead(e.to_string())
    }
}

#[cfg(feature = "aws")]
mod aws;
#[cfg(feature = "aws")]
pub use aws::AwsKms;

// ----------------------------------------------------------------------------
// KMS trait + local-file implementation
// ----------------------------------------------------------------------------

/// A key-management contract.
///
/// The walking skeleton implements this with a local file. AWS KMS, Vault, or
/// any other backend slots in by implementing the same trait — see the spec's
/// `KMSService` gRPC contract for the production shape.
#[async_trait::async_trait]
pub trait Kms: Send + Sync {
    /// Return the data-encryption key. In a real KMS this is the result of an
    /// unwrap operation against the master key; in the skeleton it is the
    /// stored master key directly.
    async fn data_key(&self) -> Result<[u8; 32], CryptoError>;
}

/// File-backed KMS for development. Generates the key on first read.
pub struct LocalFileKms {
    path: PathBuf,
}

impl LocalFileKms {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[async_trait::async_trait]
impl Kms for LocalFileKms {
    async fn data_key(&self) -> Result<[u8; 32], CryptoError> {
        match fs::metadata(&self.path).await {
            Ok(_) => {
                let mut file = fs::File::open(&self.path).await?;
                let mut buf = Vec::with_capacity(32);
                file.read_to_end(&mut buf).await?;
                if buf.len() != 32 {
                    return Err(CryptoError::KmsKeyLength(buf.len()));
                }
                let mut key = [0u8; 32];
                key.copy_from_slice(&buf);
                Ok(key)
            }
            Err(_) => {
                tracing::warn!(
                    path = %self.path.display(),
                    "KMS key not present — generating a fresh 32-byte key (walking-skeleton only)"
                );
                let mut key = [0u8; 32];
                OsRng.fill_bytes(&mut key);
                if let Some(parent) = self.path.parent() {
                    if !parent.as_os_str().is_empty() {
                        fs::create_dir_all(parent).await?;
                    }
                }
                let mut f = fs::File::create(&self.path).await?;
                f.write_all(&key).await?;
                f.flush().await?;
                Ok(key)
            }
        }
    }
}

// ----------------------------------------------------------------------------
// Sealing primitives — AES-256-GCM
// ----------------------------------------------------------------------------

/// A sealed payload as returned to the caller (storage layer persists the
/// fields separately).
#[derive(Clone, Debug)]
pub struct Sealed {
    pub nonce: [u8; 12],
    pub ciphertext: Vec<u8>,
}

/// Encrypt `plaintext` under the KMS data key with AES-256-GCM.
pub async fn seal(kms: &dyn Kms, plaintext: &[u8]) -> Result<Sealed, CryptoError> {
    let raw = kms.data_key().await?;
    let key = Key::<Aes256Gcm>::from_slice(&raw);
    let cipher = Aes256Gcm::new(key);
    let nonce_arr = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher.encrypt(&nonce_arr, plaintext)?;
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(nonce_arr.as_slice());
    Ok(Sealed { nonce, ciphertext })
}

/// Decrypt a sealed payload.
pub async fn open(kms: &dyn Kms, sealed: &Sealed) -> Result<Vec<u8>, CryptoError> {
    let raw = kms.data_key().await?;
    let key = Key::<Aes256Gcm>::from_slice(&raw);
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(&sealed.nonce);
    let plaintext = cipher.decrypt(nonce, sealed.ciphertext.as_ref())?;
    Ok(plaintext)
}

/// A passphrase-sealed bundle. Used by Letter exports — the bundle can be
/// stored offline (USB stick, paper QR codes) and unsealed without the
/// operator's KMS being available.
///
/// The current implementation uses SHA-256(salt || passphrase) as the KEK,
/// matching `hash_passphrase`. The production replacement is argon2id with
/// the parameters in specs/14 §2; the format-version byte at the front of
/// the bundle lets us migrate in place.
#[derive(Clone, Debug)]
pub struct PassphraseSealed {
    pub salt: [u8; 16],
    pub nonce: [u8; 12],
    pub ciphertext: Vec<u8>,
}

/// Seal `plaintext` under a passphrase-derived KEK. The salt is random;
/// the caller persists it alongside the ciphertext.
pub fn seal_with_passphrase(
    plaintext: &[u8],
    passphrase: &str,
) -> Result<PassphraseSealed, CryptoError> {
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);
    let mut kek = [0u8; 32];
    derive_kek(passphrase, &salt, &mut kek);
    let key = Key::<Aes256Gcm>::from_slice(&kek);
    let cipher = Aes256Gcm::new(key);
    let nonce_arr = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher.encrypt(&nonce_arr, plaintext)?;
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(nonce_arr.as_slice());
    Ok(PassphraseSealed {
        salt,
        nonce,
        ciphertext,
    })
}

/// Unseal a passphrase-sealed bundle.
pub fn open_with_passphrase(
    sealed: &PassphraseSealed,
    passphrase: &str,
) -> Result<Vec<u8>, CryptoError> {
    let mut kek = [0u8; 32];
    derive_kek(passphrase, &sealed.salt, &mut kek);
    let key = Key::<Aes256Gcm>::from_slice(&kek);
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(&sealed.nonce);
    let plaintext = cipher.decrypt(nonce, sealed.ciphertext.as_ref())?;
    Ok(plaintext)
}

/// Derive a 32-byte KEK from a passphrase. Stub KDF — see specs/14 §2 for
/// the argon2id production replacement.
fn derive_kek(passphrase: &str, salt: &[u8], out: &mut [u8; 32]) {
    let mut h = Sha256::new();
    h.update(b"paschal/passphrase-kek/v1");
    h.update(salt);
    h.update(passphrase.as_bytes());
    out.copy_from_slice(&h.finalize());
}

// ----------------------------------------------------------------------------
// Token + hash helpers
// ----------------------------------------------------------------------------

/// Generate a cryptographically-random 32-byte token, URL-safe base64.
pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Hash a token with SHA-256. The DB stores hashes only.
pub fn hash_token(token: &str) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    let out = h.finalize();
    let mut buf = [0u8; 32];
    buf.copy_from_slice(&out);
    buf
}

/// Hash arbitrary bytes with SHA-256.
pub fn hash_bytes(bytes: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(bytes);
    let out = h.finalize();
    let mut buf = [0u8; 32];
    buf.copy_from_slice(&out);
    buf
}

/// Generate a 16-byte random salt for a Co-Steward passphrase.
pub fn random_salt() -> [u8; 16] {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    bytes
}

/// Derive a passphrase hash from (passphrase, salt). Uses SHA-256 with the
/// salt as a prefix in the walking skeleton; the production replacement is
/// argon2id with the parameters in spec 14 §2.
///
/// The output is a hex string so it round-trips through the DB column
/// safely.
pub fn hash_passphrase(passphrase: &str, salt: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(salt);
    h.update(passphrase.as_bytes());
    hex::encode(h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use tempfile::TempDir;

    #[test]
    fn passphrase_seal_open_roundtrip() {
        let bundle = seal_with_passphrase(b"my wedding letter contents", "this is a long pass").unwrap();
        let back = open_with_passphrase(&bundle, "this is a long pass").unwrap();
        assert_eq!(back, b"my wedding letter contents");
    }

    #[test]
    fn passphrase_open_with_wrong_passphrase_fails() {
        let bundle = seal_with_passphrase(b"contents", "correct horse battery staple").unwrap();
        assert!(open_with_passphrase(&bundle, "wrong passphrase entirely").is_err());
    }

    #[test]
    fn passphrase_seal_uses_fresh_salt_and_nonce() {
        let a = seal_with_passphrase(b"x", "pass").unwrap();
        let b = seal_with_passphrase(b"x", "pass").unwrap();
        assert_ne!(a.salt, b.salt, "salt should be random per seal");
        assert_ne!(a.nonce, b.nonce, "nonce should be random per seal");
        assert_ne!(a.ciphertext, b.ciphertext);
    }

    struct StaticKms([u8; 32]);

    #[async_trait::async_trait]
    impl Kms for StaticKms {
        async fn data_key(&self) -> Result<[u8; 32], CryptoError> {
            Ok(self.0)
        }
    }

    // ---- Basic seal/open ----

    #[tokio::test]
    async fn seal_open_roundtrip_empty() {
        let kms = StaticKms([7u8; 32]);
        let s = seal(&kms, b"").await.unwrap();
        let back = open(&kms, &s).await.unwrap();
        assert_eq!(back, b"");
    }

    #[tokio::test]
    async fn seal_open_roundtrip_short() {
        let kms = StaticKms([7u8; 32]);
        let s = seal(&kms, b"hello world").await.unwrap();
        let back = open(&kms, &s).await.unwrap();
        assert_eq!(back, b"hello world");
    }

    #[tokio::test]
    async fn seal_open_roundtrip_large() {
        let kms = StaticKms([7u8; 32]);
        let plaintext = vec![0xABu8; 1024 * 1024]; // 1 MB
        let s = seal(&kms, &plaintext).await.unwrap();
        let back = open(&kms, &s).await.unwrap();
        assert_eq!(back, plaintext);
    }

    #[tokio::test]
    async fn each_seal_uses_a_fresh_nonce() {
        // Two seals of the same plaintext under the same key must produce
        // different ciphertexts due to fresh nonces — a security invariant.
        let kms = StaticKms([7u8; 32]);
        let a = seal(&kms, b"same input").await.unwrap();
        let b = seal(&kms, b"same input").await.unwrap();
        assert_ne!(a.nonce, b.nonce, "nonces must differ");
        assert_ne!(a.ciphertext, b.ciphertext, "ciphertexts must differ");
    }

    // ---- Tampering detection ----

    #[tokio::test]
    async fn tampering_ciphertext_fails_open() {
        let kms = StaticKms([7u8; 32]);
        let mut s = seal(&kms, b"sensitive material").await.unwrap();
        s.ciphertext[0] ^= 0x01; // flip a bit
        let result = open(&kms, &s).await;
        assert!(result.is_err(), "tampered ciphertext must not open");
    }

    #[tokio::test]
    async fn tampering_nonce_fails_open() {
        let kms = StaticKms([7u8; 32]);
        let mut s = seal(&kms, b"sensitive material").await.unwrap();
        s.nonce[0] ^= 0x01;
        let result = open(&kms, &s).await;
        assert!(result.is_err(), "tampered nonce must not open");
    }

    #[tokio::test]
    async fn truncated_ciphertext_fails_open() {
        let kms = StaticKms([7u8; 32]);
        let mut s = seal(&kms, b"sensitive material").await.unwrap();
        s.ciphertext.truncate(s.ciphertext.len().saturating_sub(1));
        let result = open(&kms, &s).await;
        assert!(result.is_err(), "truncated ciphertext must not open");
    }

    #[tokio::test]
    async fn wrong_key_fails_open() {
        let kms_a = StaticKms([7u8; 32]);
        let kms_b = StaticKms([8u8; 32]);
        let s = seal(&kms_a, b"sensitive material").await.unwrap();
        let result = open(&kms_b, &s).await;
        assert!(result.is_err(), "wrong key must not open");
    }

    // ---- KMS behaviour ----

    #[tokio::test]
    async fn local_file_kms_generates_and_persists() {
        let dir = TempDir::new().unwrap();
        let kms = LocalFileKms::new(dir.path().join("kms.key"));
        let k1 = kms.data_key().await.unwrap();
        let k2 = kms.data_key().await.unwrap();
        assert_eq!(k1, k2, "must persist across reads");
        // And the file should be 32 bytes exactly.
        let meta = std::fs::metadata(dir.path().join("kms.key")).unwrap();
        assert_eq!(meta.len(), 32);
    }

    #[tokio::test]
    async fn local_file_kms_rejects_wrong_length() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("kms.key");
        std::fs::write(&path, b"short").unwrap();
        let kms = LocalFileKms::new(path);
        let err = kms.data_key().await;
        assert!(matches!(err, Err(CryptoError::KmsKeyLength(_))));
    }

    #[tokio::test]
    async fn local_file_kms_creates_parents() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nested").join("dir").join("kms.key");
        let kms = LocalFileKms::new(&path);
        kms.data_key().await.unwrap();
        assert!(path.exists());
    }

    // ---- Token helpers ----

    #[test]
    fn random_token_is_url_safe_and_long_enough() {
        let t = random_token();
        // base64url-no-pad of 32 bytes is 43 chars.
        assert_eq!(t.len(), 43);
        for c in t.chars() {
            assert!(
                c.is_ascii_alphanumeric() || c == '-' || c == '_',
                "non-URL-safe char in token: {c:?}"
            );
        }
    }

    #[test]
    fn random_token_uniqueness() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            assert!(seen.insert(random_token()), "duplicate token in 1000 draws");
        }
    }

    #[test]
    fn hash_token_is_deterministic() {
        let t = "the-quick-brown-fox";
        assert_eq!(hash_token(t), hash_token(t));
    }

    #[test]
    fn hash_token_changes_with_input() {
        assert_ne!(hash_token("a"), hash_token("b"));
    }

    #[test]
    fn hash_bytes_changes_with_input() {
        assert_ne!(hash_bytes(b"a"), hash_bytes(b"b"));
    }

    // ---- Property-based tests ----

    fn rt_helper() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// Any plaintext of any length up to 4 KB must round-trip cleanly.
        #[test]
        fn prop_seal_open_roundtrip(plaintext in proptest::collection::vec(any::<u8>(), 0..4096)) {
            let rt = rt_helper();
            let kms = StaticKms([42u8; 32]);
            rt.block_on(async {
                let s = seal(&kms, &plaintext).await.unwrap();
                let back = open(&kms, &s).await.unwrap();
                prop_assert_eq!(back, plaintext);
                Ok(())
            })?;
        }

        /// Tampering at any single byte position must always fail open.
        #[test]
        fn prop_any_tamper_fails(
            plaintext in proptest::collection::vec(any::<u8>(), 16..256),
            tamper_offset in 0usize..256,
            tamper_xor in 1u8..=255
        ) {
            let rt = rt_helper();
            let kms = StaticKms([42u8; 32]);
            rt.block_on(async {
                let mut s = seal(&kms, &plaintext).await.unwrap();
                if !s.ciphertext.is_empty() {
                    let i = tamper_offset % s.ciphertext.len();
                    s.ciphertext[i] ^= tamper_xor;
                    let r = open(&kms, &s).await;
                    prop_assert!(r.is_err(), "tampered ciphertext must not open");
                }
                Ok(())
            })?;
        }
    }
}

