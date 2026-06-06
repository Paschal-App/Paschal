//! Blob storage abstraction.
//!
//! The Beacon stores sealed attachment ciphertext outside the Postgres
//! database. The MVP ships a local-filesystem-backed implementation; the
//! production seam is an `S3Store` against any S3-compatible service
//! (AWS S3, Cloudflare R2, MinIO).
//!
//! Storage keys are opaque to the DB — the BlobStore is responsible for
//! resolving them. The default `LocalFilesystemStore` uses a sharded
//! two-level prefix to avoid blowing up any one directory.
//!
//! ## Regions
//!
//! `put` takes a region code so a Vault's attachments can be placed in a
//! chosen region (Estate+ / Legacy plans). The returned key encodes the
//! region as a `"<region>:<uuid>"` prefix, so `get`/`delete` self-route with
//! no extra DB column. A bare key (no prefix) is treated as the default
//! region — this keeps attachments sealed before the multi-region feature
//! readable. Single-store backends (`LocalFilesystemStore`, a lone `S3Store`)
//! ignore the region argument; the `RegionRouter` (aws feature) fans out to
//! one `S3Store` per region.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use thiserror::Error;
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
};
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum BlobError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("backend: {0}")]
    Backend(String),
}

#[cfg(feature = "aws")]
mod s3;
#[cfg(feature = "aws")]
pub use s3::{RegionRouter, S3Store};

/// Split a storage key into `(region, id)`. A key may be `"<region>:<uuid>"`
/// (written once the multi-region feature was live) or a bare `"<uuid>"`
/// (sealed before it, or by a single-store backend). A bare key has no region
/// and the caller substitutes its default.
pub fn parse_key(key: &str) -> (Option<&str>, &str) {
    match key.split_once(':') {
        Some((region, id)) => (Some(region), id),
        None => (None, key),
    }
}

#[async_trait]
pub trait BlobStore: Send + Sync {
    /// Persist a blob in the given region and return its opaque key. The key
    /// encodes the region so later `get`/`delete` route to it. Single-region
    /// backends ignore `region`.
    async fn put(&self, region: &str, bytes: &[u8]) -> Result<String, BlobError>;

    /// Fetch a blob by key. The region (if any) is read from the key.
    async fn get(&self, key: &str) -> Result<Vec<u8>, BlobError>;

    /// Delete a blob. Idempotent.
    async fn delete(&self, key: &str) -> Result<(), BlobError>;
}

// ---------------------------------------------------------------------------
// Local filesystem implementation
// ---------------------------------------------------------------------------

pub struct LocalFilesystemStore {
    root: PathBuf,
}

impl LocalFilesystemStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Map an opaque key to an on-disk path with two-level sharding. The
    /// first 2 hex chars of the key become `dir1`, the next 2 become `dir2`.
    fn path_for(&self, key: &str) -> PathBuf {
        let safe = key
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect::<String>();
        let d1 = safe.get(0..2).unwrap_or("xx");
        let d2 = safe.get(2..4).unwrap_or("xx");
        self.root.join(d1).join(d2).join(format!("{safe}.blob"))
    }
}

#[async_trait]
impl BlobStore for LocalFilesystemStore {
    async fn put(&self, _region: &str, bytes: &[u8]) -> Result<String, BlobError> {
        // Single-store backend: region is ignored. Key is a fresh UUIDv4. We
        // use hyphenless hex for the path; the key returned to callers keeps
        // the hyphens for readability.
        let id = Uuid::new_v4();
        let key = id.to_string();
        let path = self.path_for(&key);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let mut tmp = path.clone();
        tmp.set_extension("blob.tmp");
        {
            let mut f = fs::File::create(&tmp).await?;
            f.write_all(bytes).await?;
            f.flush().await?;
        }
        fs::rename(tmp, path).await?;
        Ok(key)
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, BlobError> {
        // Tolerate a region-prefixed key from a prior S3-backed deployment.
        let (_region, id) = crate::parse_key(key);
        let path = self.path_for(id);
        match fs::metadata(&path).await {
            Ok(_) => {
                let mut f = fs::File::open(&path).await?;
                let mut buf = Vec::new();
                f.read_to_end(&mut buf).await?;
                Ok(buf)
            }
            Err(_) => Err(BlobError::NotFound(key.into())),
        }
    }

    async fn delete(&self, key: &str) -> Result<(), BlobError> {
        let (_region, id) = crate::parse_key(key);
        let path = self.path_for(id);
        match fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(BlobError::Io(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn put_get_roundtrip() {
        let dir = TempDir::new().unwrap();
        let store = LocalFilesystemStore::new(dir.path());
        let key = store.put("ap-southeast-2", b"hello world").await.unwrap();
        let back = store.get(&key).await.unwrap();
        assert_eq!(back, b"hello world");
    }

    #[tokio::test]
    async fn delete_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let store = LocalFilesystemStore::new(dir.path());
        let key = store.put("ap-southeast-2", b"x").await.unwrap();
        store.delete(&key).await.unwrap();
        store.delete(&key).await.unwrap(); // second delete OK
        let err = store.get(&key).await.unwrap_err();
        assert!(matches!(err, BlobError::NotFound(_)));
    }

    #[tokio::test]
    async fn keys_are_unique() {
        let dir = TempDir::new().unwrap();
        let store = LocalFilesystemStore::new(dir.path());
        let mut seen = std::collections::HashSet::new();
        for _ in 0..100 {
            let k = store.put("ap-southeast-2", b"same content").await.unwrap();
            assert!(seen.insert(k), "duplicate blob key");
        }
    }

    #[tokio::test]
    async fn missing_key_returns_not_found() {
        let dir = TempDir::new().unwrap();
        let store = LocalFilesystemStore::new(dir.path());
        let err = store
            .get("00000000-0000-0000-0000-000000000000")
            .await
            .unwrap_err();
        assert!(matches!(err, BlobError::NotFound(_)));
    }

    #[test]
    fn parse_key_splits_region_prefix() {
        assert_eq!(
            parse_key("eu-central-1:abc-123"),
            (Some("eu-central-1"), "abc-123")
        );
        assert_eq!(parse_key("abc-123"), (None, "abc-123"));
        // Only the first colon delimits region from id.
        assert_eq!(parse_key("r:a:b"), (Some("r"), "a:b"));
    }

    #[tokio::test]
    async fn get_tolerates_region_prefixed_key() {
        // A LocalFilesystemStore stores by bare uuid, but must still resolve a
        // key that arrived with a region prefix (e.g. from a prior S3 backend).
        let dir = TempDir::new().unwrap();
        let store = LocalFilesystemStore::new(dir.path());
        let key = store.put("ap-southeast-2", b"payload").await.unwrap();
        let prefixed = format!("eu-central-1:{key}");
        let back = store.get(&prefixed).await.unwrap();
        assert_eq!(back, b"payload");
        store.delete(&prefixed).await.unwrap();
        assert!(matches!(
            store.get(&key).await.unwrap_err(),
            BlobError::NotFound(_)
        ));
    }

    #[tokio::test]
    async fn sharding_creates_subdirectories() {
        let dir = TempDir::new().unwrap();
        let store = LocalFilesystemStore::new(dir.path());
        let key = store.put("ap-southeast-2", b"sharded").await.unwrap();
        let path = store.path_for(&key);
        assert!(path.exists());
        // Verify we landed under root/<2-chars>/<2-chars>/.
        let parents: Vec<_> = path.ancestors().collect();
        assert!(parents.len() >= 4);
    }
}
