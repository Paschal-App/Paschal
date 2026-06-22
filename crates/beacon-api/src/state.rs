use std::sync::Arc;

use beacon_db as db;
use blob_store::{BlobStore, LocalFilesystemStore};
use crypto_stub::{Kms, LocalFileKms};
use paschal_transform::{default_pipeline, Transformer};
use sqlx::PgPool;
use url::Url;
use webauthn_rs::WebauthnBuilder;

use crate::notifications::{NotificationSink, StubNotifications};

/// AppState is the dependency container threaded through every handler.
///
/// It is `Clone` because axum requires the state to be cloneable; the
/// underlying Pg pool and the KMS handle are cheap to clone (Arc-wrapped).
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub kms: Arc<dyn Kms>,
    pub blob_store: Arc<dyn BlobStore>,
    pub transformers: Arc<Vec<Box<dyn Transformer>>>,
    pub notifications: Arc<dyn NotificationSink>,
    pub metrics: Arc<crate::metrics::Metrics>,
    pub config: Arc<Config>,
    pub webauthn: Arc<webauthn_rs::Webauthn>,
}

pub struct Config {
    pub public_base_url: String,
    pub trial_days: i64,
    pub retention_days: i64,
    pub default_cooling_off_seconds: i32,
    /// Maximum size (bytes) of a single uploaded attachment before transformation.
    pub max_upload_bytes: usize,
    /// Maximum total request body size (bytes) for multipart uploads.
    pub max_request_bytes: usize,
    /// Per-IP request budget per minute (global). 0 disables.
    pub rate_limit_per_minute: u32,
    /// Tighter per-IP budget on auth endpoints (signup, signin, magic-link), on
    /// top of the global limit, to slow credential-stuffing. 0 disables.
    pub auth_rate_limit_per_minute: u32,
    /// Signal aggregator tick (seconds). MVP runs every few seconds for demo
    /// responsiveness; production goes minutes/hours.
    pub aggregator_tick_seconds: u64,
    /// Elect a single scheduler leader (via a Postgres advisory lock) so the
    /// background loops run on only one replica. Production default is `true`;
    /// single-process / test deployments set `false`.
    pub scheduler_leader_election: bool,
    /// Hold (seconds) before a Co-Steward's post-mortem recipient-contact
    /// change takes effect. During the hold the change is visible (logged +
    /// the old address notified) and any deputy can cancel it.
    pub recipient_change_hold_seconds: i64,
    /// Trigger threshold for ACTIVE → SUSPICIOUS → ALERT progression.
    pub trigger_threshold: f32,
    pub min_independent_signals: usize,
    /// Minimum dwell time in SUSPICIOUS / ALERT before further escalation.
    /// Wired in at v1 when the aggregator gains a dwell-tracker.
    #[allow(dead_code)]
    pub suspicious_dwell_seconds: i64,
    #[allow(dead_code)]
    pub alert_dwell_seconds: i64,
    /// Heartbeat freshness expectations (seconds). After this, missing
    /// Heartbeat contributes positively to the trigger score.
    pub heartbeat_max_gap_seconds: i64,
}

impl AppState {
    /// Construct an AppState for integration tests. Uses an existing pool
    /// (so the test harness can truncate between cases) and an in-memory
    /// no-op notification sink.
    pub fn for_tests(pool: PgPool, kms: Arc<LocalFileKms>) -> Self {
        let notifications: Arc<dyn NotificationSink> =
            Arc::new(crate::notifications::TestNotifications::default());
        let tmp = std::env::temp_dir().join(format!("paschal-blobs-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).expect("blob dir");
        let blob_store: Arc<dyn BlobStore> = Arc::new(LocalFilesystemStore::new(tmp));
        let transformers: Arc<Vec<Box<dyn Transformer>>> = Arc::new(default_pipeline());
        let webauthn = Arc::new(
            WebauthnBuilder::new("localhost", &Url::parse("http://localhost").unwrap())
                .unwrap()
                .rp_name("Paschal (test)")
                .build()
                .unwrap(),
        );
        Self {
            pool,
            kms,
            blob_store,
            transformers,
            notifications,
            webauthn,
            metrics: Arc::new(crate::metrics::Metrics::new()),
            config: Arc::new(Config {
                public_base_url: "http://test.localhost".into(),
                trial_days: 30,
                retention_days: 1095,
                default_cooling_off_seconds: 2,
                aggregator_tick_seconds: 1,
                scheduler_leader_election: false,
                recipient_change_hold_seconds: 1,
                trigger_threshold: 0.7,
                min_independent_signals: 3,
                suspicious_dwell_seconds: 1,
                alert_dwell_seconds: 1,
                heartbeat_max_gap_seconds: 5,
                max_upload_bytes: 16 * 1024 * 1024,
                max_request_bytes: 32 * 1024 * 1024,
                rate_limit_per_minute: 0,
                auth_rate_limit_per_minute: 0,
            }),
        }
    }

    pub async fn from_env() -> anyhow::Result<Self> {
        let database_url = std::env::var("DATABASE_URL")
            .map_err(|_| anyhow::anyhow!("DATABASE_URL must be set (see .env.example)"))?;
        // Retry connect up to 30s — Postgres may be coming up alongside the
        // Beacon under docker-compose.
        let pool =
            db::connect_with_retry(&database_url, std::time::Duration::from_secs(30)).await?;

        // Migration mode. Under blue/green the pipeline runs migrations as a
        // one-off pre-deploy task and the long-running tasks boot in `verify`
        // mode, so two task sets never race to run DDL during a cutover.
        //   apply  (default) — apply pending migrations in-process (dev/local)
        //   verify           — assert all embedded migrations are applied; fail fast
        //   skip             — do nothing (the pre-deploy task already ran)
        match std::env::var("MIGRATE_ON_BOOT")
            .unwrap_or_else(|_| "apply".into())
            .to_ascii_lowercase()
            .as_str()
        {
            "verify" => {
                db::verify_migrations(&pool).await?;
                tracing::info!("migrations verified — schema is up to date");
            }
            "skip" | "false" | "off" => {
                tracing::info!("MIGRATE_ON_BOOT=skip — not touching schema");
            }
            _ => {
                let n = db::migrate(&pool).await?;
                if n > 0 {
                    tracing::info!(applied = n, "migrations applied");
                }
            }
        }

        // KMS backend: `local` (file-backed, dev) or `aws` (persistent DEK
        // wrapped by a CMK, unwrapped once at boot — see crypto_stub::AwsKms).
        let kms: Arc<dyn Kms> = match std::env::var("KMS_BACKEND")
            .unwrap_or_else(|_| "local".into())
            .to_ascii_lowercase()
            .as_str()
        {
            "aws" => {
                let secret_id = std::env::var("KMS_WRAPPED_DEK_SECRET_ID").map_err(|_| {
                    anyhow::anyhow!("KMS_WRAPPED_DEK_SECRET_ID must be set when KMS_BACKEND=aws")
                })?;
                tracing::info!(%secret_id, "using AWS KMS backend (persistent wrapped DEK)");
                Arc::new(crypto_stub::AwsKms::from_env(secret_id).await)
            }
            _ => {
                let kms_path =
                    std::env::var("BEACON_KMS_KEY_PATH").unwrap_or_else(|_| "./kms.key".into());
                Arc::new(LocalFileKms::new(kms_path))
            }
        };

        let public_base_url = std::env::var("BEACON_PUBLIC_BASE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8080".into());

        let config = Config {
            public_base_url,
            trial_days: env_i64("TRIAL_DAYS", 30),
            retention_days: env_i64("RETENTION_DAYS", 1095),
            default_cooling_off_seconds: env_i32("COOLING_OFF_SECONDS", 15),
            aggregator_tick_seconds: env_u64("AGGREGATOR_TICK_SECONDS", 5),
            scheduler_leader_election: env_bool("BEACON_SCHEDULER_LEADER_ELECTION", true),
            recipient_change_hold_seconds: env_i64("RECIPIENT_CHANGE_HOLD_SECONDS", 86_400),
            trigger_threshold: env_f32("TRIGGER_THRESHOLD", 0.7),
            min_independent_signals: env_usize("MIN_INDEPENDENT_SIGNALS", 3),
            suspicious_dwell_seconds: env_i64("SUSPICIOUS_DWELL_SECONDS", 10),
            alert_dwell_seconds: env_i64("ALERT_DWELL_SECONDS", 10),
            heartbeat_max_gap_seconds: env_i64("HEARTBEAT_MAX_GAP_SECONDS", 30),
            max_upload_bytes: env_usize("MAX_UPLOAD_BYTES", 16 * 1024 * 1024),
            max_request_bytes: env_usize("MAX_REQUEST_BYTES", 32 * 1024 * 1024),
            rate_limit_per_minute: env_u32("RATE_LIMIT_PER_MINUTE", 240),
            auth_rate_limit_per_minute: env_u32("AUTH_RATE_LIMIT_PER_MINUTE", 10),
        };

        // Notification backend: `stub` (default — log to file), `resend` (hosted
        // email API), or `smtp` (any mail server you run). Email is the product's
        // core delivery path, so a misconfigured backend falls back to the stub
        // and logs loudly rather than silently dropping releases and invites.
        let notifications_log = std::env::var("STUB_NOTIFICATIONS_LOG")
            .unwrap_or_else(|_| "./logs/notifications.log".into());
        let email_from =
            std::env::var("EMAIL_FROM").unwrap_or_else(|_| "Paschal <noreply@localhost>".into());
        let notifications: Arc<dyn NotificationSink> = match std::env::var("NOTIFICATIONS_BACKEND")
            .unwrap_or_default()
            .as_str()
        {
            "resend" => {
                // Guard like a feature flag: a non-`re_` value (e.g. an `unset`
                // placeholder) stays in stub mode rather than 401-ing every send.
                let api_key = std::env::var("RESEND_API_KEY").unwrap_or_default();
                if api_key.starts_with("re_") {
                    tracing::info!(from = %email_from, "notifications: Resend");
                    Arc::new(crate::notifications::ResendNotifications::new(
                        api_key,
                        email_from.clone(),
                    ))
                } else {
                    tracing::warn!(
                            "NOTIFICATIONS_BACKEND=resend but RESEND_API_KEY has no re_ prefix; using stub"
                        );
                    Arc::new(StubNotifications::new(notifications_log))
                }
            }
            "smtp" => {
                let host = std::env::var("SMTP_HOST").unwrap_or_default();
                if host.is_empty() {
                    tracing::warn!("NOTIFICATIONS_BACKEND=smtp but SMTP_HOST is empty; using stub");
                    Arc::new(StubNotifications::new(notifications_log))
                } else {
                    let port = std::env::var("SMTP_PORT").ok().and_then(|s| s.parse().ok());
                    let username = std::env::var("SMTP_USERNAME")
                        .ok()
                        .filter(|s| !s.is_empty());
                    let password = std::env::var("SMTP_PASSWORD")
                        .ok()
                        .filter(|s| !s.is_empty());
                    let tls = std::env::var("SMTP_TLS").unwrap_or_else(|_| "starttls".into());
                    match crate::notifications::SmtpNotifications::new(
                        &host,
                        port,
                        username,
                        password,
                        &tls,
                        email_from.clone(),
                    ) {
                        Ok(sink) => {
                            tracing::info!(%host, from = %email_from, "notifications: SMTP");
                            Arc::new(sink)
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "SMTP transport build failed; using stub");
                            Arc::new(StubNotifications::new(notifications_log))
                        }
                    }
                }
            }
            _ => Arc::new(StubNotifications::new(notifications_log)),
        };

        // Blob backend: `local` (filesystem, dev) or `s3` (SSE-KMS, required
        // on Fargate where task disk is ephemeral and not shared blue/green).
        let blob_store: Arc<dyn BlobStore> = match std::env::var("BLOB_BACKEND")
            .unwrap_or_else(|_| "local".into())
            .to_ascii_lowercase()
            .as_str()
        {
            "s3" => build_s3_backend().await?,
            _ => {
                let blob_root =
                    std::env::var("BEACON_BLOB_ROOT").unwrap_or_else(|_| "./blobs".into());
                std::fs::create_dir_all(&blob_root)
                    .map_err(|e| anyhow::anyhow!("create blob root: {e}"))?;
                Arc::new(LocalFilesystemStore::new(blob_root))
            }
        };
        let transformers: Arc<Vec<Box<dyn Transformer>>> = Arc::new(default_pipeline());

        let rp_id = std::env::var("WEBAUTHN_RP_ID").unwrap_or_else(|_| "localhost".into());
        // WEBAUTHN_RP_ORIGIN is comma-separated: the first is primary, the rest are
        // additional allowed origins. This lets one deployment accept, e.g., the
        // Vite dev server on :5173 alongside the API on :8080 — the passkey origin
        // must match the page the ceremony runs from, and in dev they differ.
        let rp_origin =
            std::env::var("WEBAUTHN_RP_ORIGIN").unwrap_or_else(|_| "http://localhost:8080".into());
        let origin_urls: Vec<Url> = rp_origin
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| {
                Url::parse(s).map_err(|e| anyhow::anyhow!("WEBAUTHN_RP_ORIGIN '{s}' invalid: {e}"))
            })
            .collect::<anyhow::Result<_>>()?;
        let (primary, extra) = origin_urls
            .split_first()
            .ok_or_else(|| anyhow::anyhow!("WEBAUTHN_RP_ORIGIN is empty"))?;
        let mut builder = WebauthnBuilder::new(&rp_id, primary)
            .map_err(|e| anyhow::anyhow!("WebauthnBuilder: {e}"))?
            .rp_name("Paschal");
        for url in extra {
            builder = builder.append_allowed_origin(url);
        }
        let webauthn = Arc::new(
            builder
                .build()
                .map_err(|e| anyhow::anyhow!("Webauthn build: {e}"))?,
        );

        Ok(Self {
            pool,
            kms,
            blob_store,
            transformers,
            notifications,
            webauthn,
            metrics: Arc::new(crate::metrics::Metrics::new()),
            config: Arc::new(config),
        })
    }
}

/// Build the S3 blob backend from env. Two shapes:
///
/// - **Single region** (default): `BLOB_S3_BUCKET` (+ optional `BLOB_S3_PREFIX`,
///   `BLOB_S3_KMS_KEY_ID`, `BLOB_S3_REGION` — falling back to `AWS_REGION` then
///   the default region). Returns a lone `S3Store`; the region argument to
///   `put` is ignored and keys are bare uuids.
/// - **Multi region**: `BLOB_S3_REGIONS=ap-southeast-2,eu-central-1,...` builds
///   one `S3Store` per listed region from `BLOB_S3_BUCKET_<REGION>` (and
///   optional `BLOB_S3_KMS_KEY_ID_<REGION>`), wrapped in a `RegionRouter`.
///   `<REGION>` is the AWS code upper-cased with hyphens as underscores, e.g.
///   `BLOB_S3_BUCKET_AP_SOUTHEAST_2`. `BLOB_S3_DEFAULT_REGION` (or the first
///   listed) is the home for bare/legacy keys.
async fn build_s3_backend() -> anyhow::Result<Arc<dyn BlobStore>> {
    let prefix = std::env::var("BLOB_S3_PREFIX").unwrap_or_default();

    if let Ok(regions) = std::env::var("BLOB_S3_REGIONS") {
        let regions: Vec<String> = regions
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if regions.is_empty() {
            anyhow::bail!("BLOB_S3_REGIONS is set but empty");
        }
        let default_region =
            std::env::var("BLOB_S3_DEFAULT_REGION").unwrap_or_else(|_| regions[0].clone());

        let mut stores = Vec::with_capacity(regions.len());
        for region in &regions {
            let suffix = region.to_uppercase().replace('-', "_");
            let bucket = std::env::var(format!("BLOB_S3_BUCKET_{suffix}")).map_err(|_| {
                anyhow::anyhow!(
                    "BLOB_S3_BUCKET_{suffix} must be set (region {region} in BLOB_S3_REGIONS)"
                )
            })?;
            let kms_key_id = std::env::var(format!("BLOB_S3_KMS_KEY_ID_{suffix}")).ok();
            let store =
                blob_store::S3Store::from_env(region.clone(), bucket, prefix.clone(), kms_key_id)
                    .await;
            stores.push((region.clone(), store));
        }
        tracing::info!(?regions, %default_region, "using multi-region S3 blob backend");
        let router = blob_store::RegionRouter::new(stores, default_region)
            .map_err(|e| anyhow::anyhow!("RegionRouter: {e}"))?;
        return Ok(Arc::new(router));
    }

    let bucket = std::env::var("BLOB_S3_BUCKET")
        .map_err(|_| anyhow::anyhow!("BLOB_S3_BUCKET must be set when BLOB_BACKEND=s3"))?;
    let region = std::env::var("BLOB_S3_REGION")
        .or_else(|_| std::env::var("AWS_REGION"))
        .unwrap_or_else(|_| {
            beacon_core::StorageRegion::default()
                .as_aws_str()
                .to_string()
        });
    let kms_key_id = std::env::var("BLOB_S3_KMS_KEY_ID").ok();
    tracing::info!(%bucket, %region, "using single-region S3 blob backend");
    Ok(Arc::new(
        blob_store::S3Store::from_env(region, bucket, prefix, kms_key_id).await,
    ))
}

fn env_i64(name: &str, default: i64) -> i64 {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}
fn env_i32(name: &str, default: i32) -> i32 {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}
fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}
fn env_f32(name: &str, default: f32) -> f32 {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}
fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}
fn env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}
fn env_bool(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .map(|s| {
            matches!(
                s.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}
