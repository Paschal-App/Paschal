//! S3-backed BlobStore for Fargate / production.
//!
//! Fargate task local disk is ephemeral and not shared between blue/green
//! task sets, so sealed-attachment ciphertext must live in S3. Objects are
//! written with server-side encryption (`aws:kms`) under the environment's
//! customer-managed key, giving a second envelope around the already
//! application-sealed bytes.
//!
//! Keys are opaque UUIDs (matching `LocalFilesystemStore`) stored under an
//! optional prefix. The DB only ever sees the returned key string.
//!
//! ## Single store vs. router
//!
//! A lone `S3Store` is single-region: it pins one bucket in one region and
//! ignores the `region` argument to `put` (returning a bare `"<uuid>"` key).
//! `RegionRouter` wraps one `S3Store` per region; its `put` routes to the
//! requested region's store and returns a composite `"<region>:<uuid>"` key so
//! later `get`/`delete` self-route. A bare key (sealed before multi-region, or
//! by a single store) routes to the router's default region.

use std::collections::HashMap;

use async_trait::async_trait;
use aws_sdk_s3::{config::Region, primitives::ByteStream, types::ServerSideEncryption, Client};
use uuid::Uuid;

use crate::{parse_key, BlobError, BlobStore};

impl From<aws_sdk_s3::error::SdkError<aws_sdk_s3::operation::get_object::GetObjectError>>
    for BlobError
{
    fn from(
        e: aws_sdk_s3::error::SdkError<aws_sdk_s3::operation::get_object::GetObjectError>,
    ) -> Self {
        // A missing object surfaces as a NoSuchKey service error.
        if let aws_sdk_s3::error::SdkError::ServiceError(se) = &e {
            if se.err().is_no_such_key() {
                return BlobError::NotFound(e.to_string());
            }
        }
        BlobError::Backend(e.to_string())
    }
}

pub struct S3Store {
    client: Client,
    bucket: String,
    prefix: String,
    /// CMK ARN/ID for SSE-KMS. When `None`, the bucket's default encryption
    /// applies (still encrypted at rest, but not pinned to our CMK).
    kms_key_id: Option<String>,
}

impl S3Store {
    /// Build a store pinned to `region`. Credentials come from the ambient AWS
    /// environment (IRSA / task role). `bucket` is required; `prefix` is
    /// prepended to every key; `kms_key_id` pins SSE-KMS to a specific CMK.
    ///
    /// The region is pinned explicitly (not read from `AWS_REGION`) so a
    /// `RegionRouter` can stand up one store per region in a single process.
    pub async fn from_env(
        region: impl Into<String>,
        bucket: impl Into<String>,
        prefix: impl Into<String>,
        kms_key_id: Option<String>,
    ) -> Self {
        let cfg = aws_config::from_env()
            .region(Region::new(region.into()))
            .load()
            .await;
        let client = Client::new(&cfg);
        Self {
            client,
            bucket: bucket.into(),
            prefix: prefix.into(),
            kms_key_id,
        }
    }

    fn object_key(&self, key: &str) -> String {
        if self.prefix.is_empty() {
            key.to_string()
        } else {
            format!("{}/{}", self.prefix.trim_end_matches('/'), key)
        }
    }

    /// Store `bytes` under a fresh `id`, returning that bare id. The region is
    /// already fixed by this store, so callers (`RegionRouter`) own composing
    /// the `"<region>:<id>"` key.
    async fn put_id(&self, bytes: &[u8]) -> Result<String, BlobError> {
        let id = Uuid::new_v4().to_string();
        let mut req = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(self.object_key(&id))
            .body(ByteStream::from(bytes.to_vec()))
            .server_side_encryption(ServerSideEncryption::AwsKms);
        if let Some(kid) = &self.kms_key_id {
            req = req.ssekms_key_id(kid);
        }
        req.send()
            .await
            .map_err(|e| BlobError::Backend(e.to_string()))?;
        Ok(id)
    }

    async fn get_id(&self, id: &str) -> Result<Vec<u8>, BlobError> {
        let out = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(self.object_key(id))
            .send()
            .await?;
        let data = out
            .body
            .collect()
            .await
            .map_err(|e| BlobError::Backend(e.to_string()))?;
        Ok(data.into_bytes().to_vec())
    }

    async fn delete_id(&self, id: &str) -> Result<(), BlobError> {
        // S3 DeleteObject is idempotent — deleting a missing key is a success.
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(self.object_key(id))
            .send()
            .await
            .map_err(|e| BlobError::Backend(e.to_string()))?;
        Ok(())
    }
}

#[async_trait]
impl BlobStore for S3Store {
    async fn put(&self, _region: &str, bytes: &[u8]) -> Result<String, BlobError> {
        // Single-store backend: the region is fixed by this store, so the
        // argument is ignored and the returned key is a bare uuid.
        self.put_id(bytes).await
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, BlobError> {
        // Tolerate a region-prefixed key from a multi-region deployment.
        let (_region, id) = parse_key(key);
        self.get_id(id).await
    }

    async fn delete(&self, key: &str) -> Result<(), BlobError> {
        let (_region, id) = parse_key(key);
        self.delete_id(id).await
    }
}

// ---------------------------------------------------------------------------
// Multi-region router
// ---------------------------------------------------------------------------

/// Fans `put`/`get`/`delete` out to one `S3Store` per region. The region a
/// blob lives in is encoded in its key (`"<region>:<id>"`), so reads and
/// deletes self-route with no DB column. A bare key (no region) routes to the
/// default region — these are blobs sealed before multi-region shipped, or by
/// a single-store deployment.
pub struct RegionRouter {
    stores: HashMap<String, S3Store>,
    default_region: String,
}

impl RegionRouter {
    /// Build a router from `(region, store)` pairs. `default_region` must be
    /// one of the provided regions; it is the home for bare (region-less) keys
    /// and the fallback when a requested region was not configured.
    pub fn new(
        stores: impl IntoIterator<Item = (String, S3Store)>,
        default_region: impl Into<String>,
    ) -> Result<Self, BlobError> {
        let stores: HashMap<String, S3Store> = stores.into_iter().collect();
        let default_region = default_region.into();
        if !stores.contains_key(&default_region) {
            return Err(BlobError::Backend(format!(
                "default region {default_region:?} has no configured store"
            )));
        }
        Ok(Self {
            stores,
            default_region,
        })
    }

    fn store_for(&self, region: &str) -> Result<&S3Store, BlobError> {
        self.stores
            .get(region)
            .ok_or_else(|| BlobError::Backend(format!("no store configured for region {region:?}")))
    }
}

#[async_trait]
impl BlobStore for RegionRouter {
    async fn put(&self, region: &str, bytes: &[u8]) -> Result<String, BlobError> {
        let store = self.store_for(region)?;
        let id = store.put_id(bytes).await?;
        Ok(format!("{region}:{id}"))
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, BlobError> {
        let (region, id) = parse_key(key);
        let region = region.unwrap_or(&self.default_region);
        self.store_for(region)?.get_id(id).await
    }

    async fn delete(&self, key: &str) -> Result<(), BlobError> {
        let (region, id) = parse_key(key);
        let region = region.unwrap_or(&self.default_region);
        self.store_for(region)?.delete_id(id).await
    }
}
