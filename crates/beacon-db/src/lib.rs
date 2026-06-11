//! Postgres persistence for the Beacon.
//!
//! Uses runtime [`sqlx::query`] (not the macros) so the workspace builds
//! without a live database. Production moves to compile-time-checked
//! `query!` macros per [`style-guide/04-code-style.md`].

use std::time::Duration;

use beacon_core::{
    Attachment, AttachmentId, Buddy, BuddyId, BuddyResponse, DomainError, LetterId, LetterMeta,
    PlanId, Principal, PrincipalId, ReleaseEvent, ReleaseEventId, ReleaseReason, Signal, SignalId,
    SignalSource, SignalSubscription, SignalSubscriptionId, StorageRegion, Subscription,
    SubscriptionId, SubscriptionState, Tier, Vault, VaultId, VaultState,
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use sqlx::{
    postgres::{PgPoolOptions, PgRow},
    PgPool, Row,
};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("sqlx: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("domain: {0}")]
    Domain(#[from] DomainError),
    #[error("not found")]
    NotFound,
    #[error("migration not applied: {0} (run the pre-deploy migration task)")]
    MigrationPending(String),
}

/// Connect with sensible pool defaults.
pub async fn connect(url: &str) -> Result<PgPool, DbError> {
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .acquire_timeout(Duration::from_secs(10))
        .connect(url)
        .await?;
    Ok(pool)
}

/// Connect with exponential backoff. Useful at boot where the database may
/// be coming up alongside the Beacon (e.g. docker-compose).
///
/// Retries every 2 seconds for up to `max_wait`, with explicit logging
/// each attempt. Returns the connected pool or the last error.
pub async fn connect_with_retry(url: &str, max_wait: Duration) -> Result<PgPool, DbError> {
    let start = std::time::Instant::now();
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        match connect(url).await {
            Ok(pool) => {
                tracing::info!(attempt, "DB connected");
                return Ok(pool);
            }
            Err(e) => {
                if start.elapsed() >= max_wait {
                    return Err(e);
                }
                tracing::warn!(?e, attempt, "DB connect failed; retrying in 2s");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Embedded migrations
// ---------------------------------------------------------------------------

/// Migrations embedded into the binary at compile time.
///
/// Each entry is `(name, sql)`. We track the *applied* set in a small
/// `schema_migrations` table; only un-applied migrations are run on each
/// startup. The set is append-only.
pub const EMBEDDED_MIGRATIONS: &[(&str, &str)] = &[
    (
        "0001_init",
        include_str!("../../../migrations/0001_init.sql"),
    ),
    (
        "0002_buddies_signals_drills",
        include_str!("../../../migrations/0002_buddies_signals_drills.sql"),
    ),
    (
        "0003_attachments",
        include_str!("../../../migrations/0003_attachments.sql"),
    ),
    (
        "0004_account_deletion",
        include_str!("../../../migrations/0004_account_deletion.sql"),
    ),
    (
        "0005_pricing_tiers",
        include_str!("../../../migrations/0005_pricing_tiers.sql"),
    ),
    (
        "0006_letter_categories",
        include_str!("../../../migrations/0006_letter_categories.sql"),
    ),
    (
        "0007_co_stewards",
        include_str!("../../../migrations/0007_co_stewards.sql"),
    ),
    (
        "0008_storage_addon.sql",
        include_str!("../../../migrations/0008_storage_addon.sql"),
    ),
    (
        "0009_stripe_billing.sql",
        include_str!("../../../migrations/0009_stripe_billing.sql"),
    ),
    (
        "0010_time_capsule",
        include_str!("../../../migrations/0010_time_capsule.sql"),
    ),
    (
        "0011_durable_release_deadlines",
        include_str!("../../../migrations/0011_durable_release_deadlines.sql"),
    ),
    (
        "0012_co_steward_postmortem",
        include_str!("../../../migrations/0012_co_steward_postmortem.sql"),
    ),
    ("0013_tos", include_str!("../../../migrations/0013_tos.sql")),
    (
        "0014_vault_contacts",
        include_str!("../../../migrations/0014_vault_contacts.sql"),
    ),
    (
        "0015_media_playlist_kind",
        include_str!("../../../migrations/0015_media_playlist_kind.sql"),
    ),
    (
        "0016_bank_dormancy",
        include_str!("../../../migrations/0016_bank_dormancy.sql"),
    ),
    (
        "0017_vault_storage_region",
        include_str!("../../../migrations/0017_vault_storage_region.sql"),
    ),
];

/// Session-level advisory lock key that serialises concurrent boots. Without
/// it, N replicas starting together can each see a migration as un-applied and
/// race to run non-idempotent DDL, crashing all but one. The lock is held on a
/// dedicated connection for the duration of the migration run.
const MIGRATION_ADVISORY_LOCK: i64 = 0x00C5_7117_0011;

/// Apply pending migrations. Idempotent — safe to call on every boot.
///
/// Serialised across processes by a Postgres session-level advisory lock held
/// on a dedicated connection, so concurrent boots (multiple replicas) cannot
/// race to run the same DDL. Other booters block until the holder finishes,
/// then observe the migrations as already applied and skip them.
pub async fn migrate(pool: &PgPool) -> Result<usize, DbError> {
    let mut lock_conn = pool.acquire().await?;
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(MIGRATION_ADVISORY_LOCK)
        .execute(&mut *lock_conn)
        .await?;

    let result = migrate_locked(pool).await;

    // Always release, even on error, before the connection returns to the pool.
    let _ = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(MIGRATION_ADVISORY_LOCK)
        .execute(&mut *lock_conn)
        .await;

    result
}

/// Verify every embedded migration has already been applied, WITHOUT applying
/// anything. Used on the app's boot path under blue/green: the pipeline runs
/// migrations as a one-off pre-deploy task, then the long-running tasks boot in
/// verify mode so two task sets never race to run DDL during a cutover.
///
/// Returns `Err(DbError::MigrationPending)` listing the first missing migration
/// if the database is behind the binary.
pub async fn verify_migrations(pool: &PgPool) -> Result<(), DbError> {
    // If the tracking table doesn't exist yet, nothing has been applied.
    let table_exists: Option<(bool,)> = sqlx::query_as(
        "SELECT EXISTS(
            SELECT 1 FROM information_schema.tables
             WHERE table_schema = current_schema() AND table_name = 'schema_migrations'
         )",
    )
    .fetch_optional(pool)
    .await?;
    if !matches!(table_exists, Some((true,))) {
        return Err(DbError::MigrationPending(
            EMBEDDED_MIGRATIONS[0].0.to_string(),
        ));
    }

    for (name, _) in EMBEDDED_MIGRATIONS {
        let applied: Option<(String,)> =
            sqlx::query_as("SELECT name FROM schema_migrations WHERE name = $1")
                .bind(name)
                .fetch_optional(pool)
                .await?;
        if applied.is_none() {
            return Err(DbError::MigrationPending((*name).to_string()));
        }
    }
    Ok(())
}

/// The migration body, run while holding [`MIGRATION_ADVISORY_LOCK`].
async fn migrate_locked(pool: &PgPool) -> Result<usize, DbError> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            name TEXT PRIMARY KEY,
            applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
         )",
    )
    .execute(pool)
    .await?;

    let mut applied_count = 0;
    for (name, sql) in EMBEDDED_MIGRATIONS {
        let already: Option<(String,)> =
            sqlx::query_as("SELECT name FROM schema_migrations WHERE name = $1")
                .bind(name)
                .fetch_optional(pool)
                .await?;
        if already.is_some() {
            tracing::debug!(migration = name, "already applied");
            continue;
        }

        // Detect a database that was migrated via psql before the embedded
        // runner existed. If the `principal` table exists and this is the
        // 0001 migration, we record it as applied and skip the SQL.
        if *name == "0001_init" {
            let exists: Option<(bool,)> = sqlx::query_as(
                "SELECT EXISTS(
                    SELECT 1 FROM information_schema.tables
                     WHERE table_schema = current_schema() AND table_name = 'principal'
                 )",
            )
            .fetch_optional(pool)
            .await?;
            if matches!(exists, Some((true,))) {
                tracing::info!(migration = name, "found existing schema — marking applied");
                sqlx::query(
                    "INSERT INTO schema_migrations (name) VALUES ($1) ON CONFLICT DO NOTHING",
                )
                .bind(name)
                .execute(pool)
                .await?;
                continue;
            }
        }
        if *name == "0002_buddies_signals_drills" {
            let exists: Option<(bool,)> = sqlx::query_as(
                "SELECT EXISTS(
                    SELECT 1 FROM information_schema.tables
                     WHERE table_schema = current_schema() AND table_name = 'buddy'
                 )",
            )
            .fetch_optional(pool)
            .await?;
            if matches!(exists, Some((true,))) {
                tracing::info!(migration = name, "found existing tables — marking applied");
                sqlx::query(
                    "INSERT INTO schema_migrations (name) VALUES ($1) ON CONFLICT DO NOTHING",
                )
                .bind(name)
                .execute(pool)
                .await?;
                continue;
            }
        }
        if *name == "0003_attachments" {
            let exists: Option<(bool,)> = sqlx::query_as(
                "SELECT EXISTS(
                    SELECT 1 FROM information_schema.tables
                     WHERE table_schema = current_schema() AND table_name = 'attachment'
                 )",
            )
            .fetch_optional(pool)
            .await?;
            if matches!(exists, Some((true,))) {
                tracing::info!(migration = name, "found existing tables — marking applied");
                sqlx::query(
                    "INSERT INTO schema_migrations (name) VALUES ($1) ON CONFLICT DO NOTHING",
                )
                .bind(name)
                .execute(pool)
                .await?;
                continue;
            }
        }

        tracing::info!(migration = name, "applying migration");
        let mut tx = pool.begin().await?;
        sqlx::raw_sql(sql).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO schema_migrations (name) VALUES ($1)")
            .bind(name)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        applied_count += 1;
    }
    Ok(applied_count)
}

// ----------------------------------------------------------------------------
// Principals & magic links
// ----------------------------------------------------------------------------

/// Insert or fetch a Principal by email. Returns true if created.
pub async fn upsert_principal_by_email(
    pool: &PgPool,
    email: &str,
) -> Result<(Principal, bool), DbError> {
    let existing = sqlx::query(
        "SELECT id, display_name, primary_email, created_at FROM principal WHERE primary_email = $1",
    )
    .bind(email)
    .fetch_optional(pool)
    .await?;

    if let Some(row) = existing {
        return Ok((row_to_principal(&row), false));
    }

    let row = sqlx::query(
        "INSERT INTO principal (primary_email) VALUES ($1)
         RETURNING id, display_name, primary_email, created_at",
    )
    .bind(email)
    .fetch_one(pool)
    .await?;
    Ok((row_to_principal(&row), true))
}

fn row_to_principal(row: &PgRow) -> Principal {
    Principal {
        id: PrincipalId(row.get("id")),
        display_name: row.try_get("display_name").ok(),
        primary_email: row.get("primary_email"),
        created_at: row.get("created_at"),
    }
}

pub async fn fetch_principal(pool: &PgPool, id: PrincipalId) -> Result<Principal, DbError> {
    let row = sqlx::query(
        "SELECT id, display_name, primary_email, created_at FROM principal WHERE id = $1",
    )
    .bind(id.as_uuid())
    .fetch_optional(pool)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(row_to_principal(&row))
}

// ----------------------------------------------------------------------------
// Account deletion (specs/10)
// ----------------------------------------------------------------------------

/// Mark a deletion request. The retention scheduler picks this up after the
/// cool-off window elapses and purges all ciphertext.
pub async fn request_account_deletion(
    pool: &PgPool,
    principal_id: PrincipalId,
    cool_off_days: i64,
) -> Result<DateTime<Utc>, DbError> {
    let scheduled = Utc::now() + ChronoDuration::days(cool_off_days);
    let row = sqlx::query(
        "UPDATE principal
            SET deletion_requested_at = COALESCE(deletion_requested_at, now()),
                deletion_scheduled_for = COALESCE(deletion_scheduled_for, $1)
          WHERE id = $2
        RETURNING deletion_scheduled_for",
    )
    .bind(scheduled)
    .bind(principal_id.as_uuid())
    .fetch_optional(pool)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(row.get("deletion_scheduled_for"))
}

pub async fn cancel_account_deletion(
    pool: &PgPool,
    principal_id: PrincipalId,
) -> Result<(), DbError> {
    let affected = sqlx::query(
        "UPDATE principal
            SET deletion_requested_at = NULL,
                deletion_scheduled_for = NULL
          WHERE id = $1
            AND deletion_scheduled_for IS NOT NULL
            AND deletion_scheduled_for > now()",
    )
    .bind(principal_id.as_uuid())
    .execute(pool)
    .await?
    .rows_affected();
    if affected == 0 {
        Err(DbError::NotFound)
    } else {
        Ok(())
    }
}

pub async fn list_due_account_deletions(pool: &PgPool) -> Result<Vec<PrincipalId>, DbError> {
    let rows = sqlx::query(
        "SELECT id FROM principal
          WHERE deletion_scheduled_for IS NOT NULL
            AND deletion_scheduled_for <= now()",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(|r| PrincipalId(r.get("id"))).collect())
}

pub async fn anonymise_principal(pool: &PgPool, principal_id: PrincipalId) -> Result<(), DbError> {
    // Anonymise the principal record. The transparency log retains the
    // hashed entries; the primary_email is replaced by a hex hash to keep
    // the row uniqueness invariant.
    let pid_str = principal_id.as_uuid().to_string();
    let hashed = format!("deleted-{}@paschal.invalid", &pid_str[..8]);
    sqlx::query(
        "UPDATE principal
            SET primary_email = $1,
                display_name = NULL,
                deletion_requested_at = NULL,
                deletion_scheduled_for = NULL
          WHERE id = $2",
    )
    .bind(hashed)
    .bind(principal_id.as_uuid())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn principal_deletion_state(
    pool: &PgPool,
    principal_id: PrincipalId,
) -> Result<Option<(DateTime<Utc>, DateTime<Utc>)>, DbError> {
    let row = sqlx::query(
        "SELECT deletion_requested_at, deletion_scheduled_for
           FROM principal WHERE id = $1",
    )
    .bind(principal_id.as_uuid())
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|r| {
        let req: Option<DateTime<Utc>> = r.try_get("deletion_requested_at").ok();
        let sched: Option<DateTime<Utc>> = r.try_get("deletion_scheduled_for").ok();
        match (req, sched) {
            (Some(a), Some(b)) => Some((a, b)),
            _ => None,
        }
    }))
}

pub async fn create_magic_link(
    pool: &PgPool,
    principal_id: PrincipalId,
    token_hash: &[u8],
    ttl_minutes: i64,
) -> Result<(), DbError> {
    let expires_at = Utc::now() + ChronoDuration::minutes(ttl_minutes);
    sqlx::query(
        "INSERT INTO magic_link (token_hash, principal_id, expires_at)
         VALUES ($1, $2, $3)",
    )
    .bind(token_hash)
    .bind(principal_id.as_uuid())
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Returns the principal_id if the link is valid and unconsumed; marks it
/// consumed atomically.
pub async fn consume_magic_link(
    pool: &PgPool,
    token_hash: &[u8],
) -> Result<Option<PrincipalId>, DbError> {
    let row = sqlx::query(
        "UPDATE magic_link
           SET consumed_at = now()
         WHERE token_hash = $1
           AND consumed_at IS NULL
           AND expires_at > now()
         RETURNING principal_id",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| PrincipalId(r.get("principal_id"))))
}

// ----------------------------------------------------------------------------
// Sessions
// ----------------------------------------------------------------------------

pub async fn create_session(
    pool: &PgPool,
    principal_id: PrincipalId,
    token_hash: &[u8],
    ttl_hours: i64,
) -> Result<(), DbError> {
    let expires_at = Utc::now() + ChronoDuration::hours(ttl_hours);
    sqlx::query(
        "INSERT INTO session (token_hash, principal_id, expires_at)
         VALUES ($1, $2, $3)",
    )
    .bind(token_hash)
    .bind(principal_id.as_uuid())
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn principal_for_session(
    pool: &PgPool,
    token_hash: &[u8],
) -> Result<Option<PrincipalId>, DbError> {
    let row = sqlx::query(
        "SELECT principal_id FROM session
          WHERE token_hash = $1
            AND revoked_at IS NULL
            AND expires_at > now()",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| PrincipalId(r.get("principal_id"))))
}

// ----------------------------------------------------------------------------
// Subscriptions
// ----------------------------------------------------------------------------

pub async fn create_subscription(
    pool: &PgPool,
    principal_id: PrincipalId,
    plan: PlanId,
    trial_days: i64,
) -> Result<Subscription, DbError> {
    let trial_end_at = Utc::now() + ChronoDuration::days(trial_days);
    let row = sqlx::query(
        "INSERT INTO subscription (principal_id, plan_id, state, trial_end_at)
         VALUES ($1, $2, 'TRIALING', $3)
         RETURNING id, principal_id, plan_id, state, started_at, trial_end_at,
                   current_period_end, canceled_at, retention_until, extra_storage_bytes",
    )
    .bind(principal_id.as_uuid())
    .bind(plan.as_db_str())
    .bind(trial_end_at)
    .fetch_one(pool)
    .await?;
    row_to_subscription(&row)
}

pub async fn fetch_subscription(
    pool: &PgPool,
    principal_id: PrincipalId,
) -> Result<Subscription, DbError> {
    let row = sqlx::query(
        "SELECT id, principal_id, plan_id, state, started_at, trial_end_at,
                current_period_end, canceled_at, retention_until, extra_storage_bytes
           FROM subscription WHERE principal_id = $1",
    )
    .bind(principal_id.as_uuid())
    .fetch_optional(pool)
    .await?
    .ok_or(DbError::NotFound)?;
    row_to_subscription(&row)
}

pub async fn cancel_subscription(
    pool: &PgPool,
    principal_id: PrincipalId,
    retention_days: i64,
) -> Result<Subscription, DbError> {
    let now = Utc::now();
    let retention_until = now + ChronoDuration::days(retention_days);
    let row = sqlx::query(
        "UPDATE subscription
            SET state = 'CANCELED',
                canceled_at = $1,
                retention_until = $2
          WHERE principal_id = $3
        RETURNING id, principal_id, plan_id, state, started_at, trial_end_at,
                  current_period_end, canceled_at, retention_until, extra_storage_bytes",
    )
    .bind(now)
    .bind(retention_until)
    .bind(principal_id.as_uuid())
    .fetch_optional(pool)
    .await?
    .ok_or(DbError::NotFound)?;
    row_to_subscription(&row)
}

fn row_to_subscription(row: &PgRow) -> Result<Subscription, DbError> {
    Ok(Subscription {
        id: SubscriptionId(row.get("id")),
        principal_id: PrincipalId(row.get("principal_id")),
        plan_id: PlanId::from_db_str(row.get("plan_id"))?,
        state: SubscriptionState::from_db_str(row.get("state"))?,
        started_at: row.get("started_at"),
        trial_end_at: row.try_get("trial_end_at").ok(),
        current_period_end: row.try_get("current_period_end").ok(),
        canceled_at: row.try_get("canceled_at").ok(),
        retention_until: row.try_get("retention_until").ok(),
        extra_storage_bytes: row.try_get("extra_storage_bytes").unwrap_or(0),
    })
}

// ----------------------------------------------------------------------------
// Vaults
// ----------------------------------------------------------------------------

pub async fn create_vault(
    pool: &PgPool,
    principal_id: PrincipalId,
    name: &str,
    tier: Tier,
    cooling_off_seconds: i32,
    storage_region: StorageRegion,
) -> Result<Vault, DbError> {
    let row = sqlx::query(
        "INSERT INTO vault (principal_id, name, tier, cooling_off_seconds, storage_region)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, principal_id, name, tier, state, cooling_off_seconds, storage_region,
                   last_attestation_at, cooling_off_started_at, released_at, created_at",
    )
    .bind(principal_id.as_uuid())
    .bind(name)
    .bind(tier.as_db_str())
    .bind(cooling_off_seconds)
    .bind(storage_region.as_aws_str())
    .fetch_one(pool)
    .await?;
    row_to_vault(&row)
}

pub async fn fetch_vault(pool: &PgPool, vault_id: VaultId) -> Result<Vault, DbError> {
    let row = sqlx::query(
        "SELECT id, principal_id, name, tier, state, cooling_off_seconds, storage_region,
                last_attestation_at, cooling_off_started_at, released_at, created_at
           FROM vault WHERE id = $1",
    )
    .bind(vault_id.as_uuid())
    .fetch_optional(pool)
    .await?
    .ok_or(DbError::NotFound)?;
    row_to_vault(&row)
}

pub async fn list_vaults(pool: &PgPool, principal_id: PrincipalId) -> Result<Vec<Vault>, DbError> {
    let rows = sqlx::query(
        "SELECT id, principal_id, name, tier, state, cooling_off_seconds, storage_region,
                last_attestation_at, cooling_off_started_at, released_at, created_at
           FROM vault WHERE principal_id = $1 ORDER BY created_at",
    )
    .bind(principal_id.as_uuid())
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_vault).collect()
}

pub async fn set_vault_state(
    pool: &PgPool,
    vault_id: VaultId,
    state: VaultState,
    cooling_off_started_at: Option<DateTime<Utc>>,
    released_at: Option<DateTime<Utc>>,
) -> Result<Vault, DbError> {
    let row = sqlx::query(
        "UPDATE vault
            SET state = $1,
                cooling_off_started_at = COALESCE($2, cooling_off_started_at),
                released_at = COALESCE($3, released_at)
          WHERE id = $4
        RETURNING id, principal_id, name, tier, state, cooling_off_seconds, storage_region,
                  last_attestation_at, cooling_off_started_at, released_at, created_at",
    )
    .bind(state.as_db_str())
    .bind(cooling_off_started_at)
    .bind(released_at)
    .bind(vault_id.as_uuid())
    .fetch_one(pool)
    .await?;
    row_to_vault(&row)
}

/// Repoint a vault at a new storage region. Only the *target* for future blob
/// writes; existing attachment blobs are relocated separately by the move flow
/// (each attachment's storage_key independently records where its bytes live).
pub async fn set_vault_storage_region(
    pool: &PgPool,
    vault_id: VaultId,
    storage_region: StorageRegion,
) -> Result<Vault, DbError> {
    let row = sqlx::query(
        "UPDATE vault SET storage_region = $1, updated_at = now()
          WHERE id = $2
        RETURNING id, principal_id, name, tier, state, cooling_off_seconds, storage_region,
                  last_attestation_at, cooling_off_started_at, released_at, created_at",
    )
    .bind(storage_region.as_aws_str())
    .bind(vault_id.as_uuid())
    .fetch_one(pool)
    .await?;
    row_to_vault(&row)
}

fn row_to_vault(row: &PgRow) -> Result<Vault, DbError> {
    Ok(Vault {
        id: VaultId(row.get("id")),
        principal_id: PrincipalId(row.get("principal_id")),
        name: row.get("name"),
        tier: Tier::from_db_str(row.get("tier"))?,
        state: VaultState::from_db_str(row.get("state"))?,
        cooling_off_seconds: row.get("cooling_off_seconds"),
        storage_region: StorageRegion::from_aws_str(row.get("storage_region"))?,
        last_attestation_at: row.get("last_attestation_at"),
        cooling_off_started_at: row.try_get("cooling_off_started_at").ok(),
        released_at: row.try_get("released_at").ok(),
        created_at: row.get("created_at"),
    })
}

// ----------------------------------------------------------------------------
// Letters
// ----------------------------------------------------------------------------

pub struct LetterSealInput<'a> {
    pub vault_id: VaultId,
    pub title: &'a str,
    pub recipient_email: &'a str,
    pub ciphertext: &'a [u8],
    pub nonce: &'a [u8],
    pub drill_ciphertext: Option<&'a [u8]>,
    pub drill_nonce: Option<&'a [u8]>,
    pub scheduled_release_at: Option<DateTime<Utc>>,
    /// Server-side classification — one of MESSAGE, CREDENTIAL_BUNDLE,
    /// FILE_ARCHIVE, ACTION, WILL_LOCATOR, VIDEO_MESSAGE, AUDIO_MESSAGE.
    /// Defaults to MESSAGE when None.
    pub kind: Option<&'a str>,
    /// Free-form template identifier (e.g. "will_locator",
    /// "letter_to_children"). Informational only; the recipient never
    /// sees it.
    pub category: Option<&'a str>,
    /// One of SIGNAL_OR_SCHEDULED (default), SCHEDULED_ONLY, SIGNAL_ONLY.
    /// Time-capsule support — see specs/12 §5a.
    pub release_mode: Option<&'a str>,
}

pub async fn seal_letter(pool: &PgPool, input: LetterSealInput<'_>) -> Result<LetterMeta, DbError> {
    let row = sqlx::query(
        "INSERT INTO letter
            (vault_id, title, recipient_email, ciphertext, nonce,
             drill_ciphertext, drill_nonce, scheduled_release_at,
             kind, category, release_mode)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8,
                 COALESCE($9, 'MESSAGE'), $10,
                 COALESCE($11, 'SIGNAL_OR_SCHEDULED'))
         RETURNING id, vault_id, title, recipient_email, sealed_at",
    )
    .bind(input.vault_id.as_uuid())
    .bind(input.title)
    .bind(input.recipient_email)
    .bind(input.ciphertext)
    .bind(input.nonce)
    .bind(input.drill_ciphertext)
    .bind(input.drill_nonce)
    .bind(input.scheduled_release_at)
    .bind(input.kind)
    .bind(input.category)
    .bind(input.release_mode)
    .fetch_one(pool)
    .await?;
    Ok(LetterMeta {
        id: LetterId(row.get("id")),
        vault_id: VaultId(row.get("vault_id")),
        title: row.get("title"),
        recipient_email: row.get("recipient_email"),
        sealed_at: row.get("sealed_at"),
    })
}

pub async fn list_letters(pool: &PgPool, vault_id: VaultId) -> Result<Vec<LetterMeta>, DbError> {
    let rows = sqlx::query(
        "SELECT id, vault_id, title, recipient_email, sealed_at
           FROM letter WHERE vault_id = $1 ORDER BY sealed_at",
    )
    .bind(vault_id.as_uuid())
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| LetterMeta {
            id: LetterId(row.get("id")),
            vault_id: VaultId(row.get("vault_id")),
            title: row.get("title"),
            recipient_email: row.get("recipient_email"),
            sealed_at: row.get("sealed_at"),
        })
        .collect())
}

/// Returns (title, recipient_email, ciphertext, nonce) for the letter.
/// If `for_drill` is true and a drill payload exists, that is returned instead.
pub async fn fetch_letter_ciphertext(
    pool: &PgPool,
    letter_id: LetterId,
    for_drill: bool,
) -> Result<(String, String, Vec<u8>, Vec<u8>), DbError> {
    let row = sqlx::query(
        "SELECT title, recipient_email, ciphertext, nonce, drill_ciphertext, drill_nonce
           FROM letter WHERE id = $1",
    )
    .bind(letter_id.as_uuid())
    .fetch_optional(pool)
    .await?
    .ok_or(DbError::NotFound)?;

    if for_drill {
        let dc: Option<Vec<u8>> = row.try_get("drill_ciphertext").ok().flatten();
        let dn: Option<Vec<u8>> = row.try_get("drill_nonce").ok().flatten();
        if let (Some(ct), Some(nc)) = (dc, dn) {
            return Ok((row.get("title"), row.get("recipient_email"), ct, nc));
        }
        // Fall back to the real payload if no drill payload was authored —
        // the orchestrator can still exercise the path, with a clear log.
    }
    Ok((
        row.get("title"),
        row.get("recipient_email"),
        row.get("ciphertext"),
        row.get("nonce"),
    ))
}

/// Full Letter detail for the export endpoint. Returns the metadata the
/// principal needs to identify the Letter offline plus the plaintext-able
/// ciphertext.
pub struct LetterFull {
    pub id: LetterId,
    pub vault_id: VaultId,
    pub title: String,
    pub recipient_email: String,
    pub sealed_at: DateTime<Utc>,
    pub scheduled_release_at: Option<DateTime<Utc>>,
    pub kind: String,
    pub category: Option<String>,
    pub release_mode: String,
    pub ciphertext: Vec<u8>,
    pub nonce: Vec<u8>,
}

pub async fn fetch_letter_full(pool: &PgPool, letter_id: LetterId) -> Result<LetterFull, DbError> {
    let row = sqlx::query(
        "SELECT id, vault_id, title, recipient_email, sealed_at,
                scheduled_release_at, kind, category, release_mode,
                ciphertext, nonce
           FROM letter WHERE id = $1",
    )
    .bind(letter_id.as_uuid())
    .fetch_optional(pool)
    .await?
    .ok_or(DbError::NotFound)?;

    Ok(LetterFull {
        id: LetterId(row.get("id")),
        vault_id: VaultId(row.get("vault_id")),
        title: row.get("title"),
        recipient_email: row.get("recipient_email"),
        sealed_at: row.get("sealed_at"),
        scheduled_release_at: row.try_get("scheduled_release_at").ok(),
        kind: row.get("kind"),
        category: row.try_get("category").ok(),
        release_mode: row.get("release_mode"),
        ciphertext: row.get("ciphertext"),
        nonce: row.get("nonce"),
    })
}

/// Letters with a scheduled release at or before `now`.
///
/// Excludes SIGNAL_ONLY letters (their scheduled_release_at, if set, is
/// metadata only). Includes SCHEDULED_ONLY (time-capsules) and the default
/// SIGNAL_OR_SCHEDULED.
pub async fn list_scheduled_letters_due(
    pool: &PgPool,
    now: DateTime<Utc>,
) -> Result<Vec<(VaultId, LetterId)>, DbError> {
    let rows = sqlx::query(
        "SELECT vault_id, id FROM letter
          WHERE scheduled_release_at IS NOT NULL
            AND scheduled_release_at <= $1
            AND release_mode IN ('SIGNAL_OR_SCHEDULED', 'SCHEDULED_ONLY')",
    )
    .bind(now)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| (VaultId(r.get("vault_id")), LetterId(r.get("id"))))
        .collect())
}

/// Letters in a Vault that should fire on a signal-triggered release.
/// Excludes SCHEDULED_ONLY (time-capsule) letters — those fire only on
/// their own schedule.
pub async fn list_letters_for_signal_release(
    pool: &PgPool,
    vault_id: VaultId,
) -> Result<Vec<LetterMeta>, DbError> {
    let rows = sqlx::query(
        "SELECT id, vault_id, title, recipient_email, sealed_at
           FROM letter
          WHERE vault_id = $1
            AND release_mode IN ('SIGNAL_OR_SCHEDULED', 'SIGNAL_ONLY')
          ORDER BY sealed_at",
    )
    .bind(vault_id.as_uuid())
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| LetterMeta {
            id: LetterId(row.get("id")),
            vault_id: VaultId(row.get("vault_id")),
            title: row.get("title"),
            recipient_email: row.get("recipient_email"),
            sealed_at: row.get("sealed_at"),
        })
        .collect())
}

// ----------------------------------------------------------------------------
// Post-mortem Letter administration (Co-Steward).
//
// These power the surviving deputy's two abilities, both gated in the API
// layer to vaults that have RELEASED: update where a held Letter is delivered,
// and trigger an EVENT_ON_DEMAND ("wedding") Letter when the event happens.
// ----------------------------------------------------------------------------

/// Letter as seen by an administering Co-Steward: enough to identify it, see
/// its current delivery contact, and know whether it is a held event Letter
/// still awaiting a trigger.
pub struct LetterAdmin {
    pub id: LetterId,
    pub vault_id: VaultId,
    pub title: String,
    pub recipient_email: String,
    pub release_mode: String,
    pub event_released_at: Option<DateTime<Utc>>,
    pub scheduled_release_at: Option<DateTime<Utc>>,
}

fn row_to_letter_admin(row: &PgRow) -> LetterAdmin {
    LetterAdmin {
        id: LetterId(row.get("id")),
        vault_id: VaultId(row.get("vault_id")),
        title: row.get("title"),
        recipient_email: row.get("recipient_email"),
        release_mode: row.get("release_mode"),
        event_released_at: row.try_get("event_released_at").ok(),
        scheduled_release_at: row.try_get("scheduled_release_at").ok(),
    }
}

pub async fn list_letters_admin(
    pool: &PgPool,
    vault_id: VaultId,
) -> Result<Vec<LetterAdmin>, DbError> {
    let rows = sqlx::query(
        "SELECT id, vault_id, title, recipient_email, release_mode,
                event_released_at, scheduled_release_at
           FROM letter WHERE vault_id = $1 ORDER BY sealed_at",
    )
    .bind(vault_id.as_uuid())
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_letter_admin).collect())
}

pub async fn fetch_letter_admin(
    pool: &PgPool,
    letter_id: LetterId,
) -> Result<LetterAdmin, DbError> {
    let row = sqlx::query(
        "SELECT id, vault_id, title, recipient_email, release_mode,
                event_released_at, scheduled_release_at
           FROM letter WHERE id = $1",
    )
    .bind(letter_id.as_uuid())
    .fetch_optional(pool)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(row_to_letter_admin(&row))
}

pub async fn fetch_letter_meta(pool: &PgPool, letter_id: LetterId) -> Result<LetterMeta, DbError> {
    let row = sqlx::query(
        "SELECT id, vault_id, title, recipient_email, sealed_at FROM letter WHERE id = $1",
    )
    .bind(letter_id.as_uuid())
    .fetch_optional(pool)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(LetterMeta {
        id: LetterId(row.get("id")),
        vault_id: VaultId(row.get("vault_id")),
        title: row.get("title"),
        recipient_email: row.get("recipient_email"),
        sealed_at: row.get("sealed_at"),
    })
}

/// Atomically mark an EVENT_ON_DEMAND Letter as triggered. Returns `true` iff
/// this caller won — a held event Letter fires exactly once even if two
/// deputies tap "release" at the same moment.
pub async fn mark_letter_event_released(
    pool: &PgPool,
    letter_id: LetterId,
) -> Result<bool, DbError> {
    let res = sqlx::query(
        "UPDATE letter SET event_released_at = now()
          WHERE id = $1 AND release_mode = 'EVENT_ON_DEMAND' AND event_released_at IS NULL",
    )
    .bind(letter_id.as_uuid())
    .execute(pool)
    .await?;
    Ok(res.rows_affected() == 1)
}

pub struct RecipientChange {
    pub id: Uuid,
    pub letter_id: LetterId,
    pub old_email: String,
    pub new_email: String,
    pub requested_at: DateTime<Utc>,
    pub effective_at: DateTime<Utc>,
    pub applied_at: Option<DateTime<Utc>>,
    pub cancelled_at: Option<DateTime<Utc>>,
}

fn row_to_recipient_change(row: &PgRow) -> RecipientChange {
    RecipientChange {
        id: row.get("id"),
        letter_id: LetterId(row.get("letter_id")),
        old_email: row.get("old_email"),
        new_email: row.get("new_email"),
        requested_at: row.get("requested_at"),
        effective_at: row.get("effective_at"),
        applied_at: row.try_get("applied_at").ok(),
        cancelled_at: row.try_get("cancelled_at").ok(),
    }
}

pub async fn request_recipient_change(
    pool: &PgPool,
    letter_id: LetterId,
    co_steward_id: beacon_core::CoStewardId,
    old_email: &str,
    new_email: &str,
    effective_at: DateTime<Utc>,
) -> Result<RecipientChange, DbError> {
    let row = sqlx::query(
        "INSERT INTO letter_recipient_change
            (letter_id, co_steward_id, old_email, new_email, effective_at)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, letter_id, old_email, new_email, requested_at,
                   effective_at, applied_at, cancelled_at",
    )
    .bind(letter_id.as_uuid())
    .bind(co_steward_id.as_uuid())
    .bind(old_email)
    .bind(new_email)
    .bind(effective_at)
    .fetch_one(pool)
    .await?;
    Ok(row_to_recipient_change(&row))
}

pub async fn fetch_recipient_change(
    pool: &PgPool,
    change_id: Uuid,
) -> Result<Option<RecipientChange>, DbError> {
    let row = sqlx::query(
        "SELECT id, letter_id, old_email, new_email, requested_at,
                effective_at, applied_at, cancelled_at
           FROM letter_recipient_change WHERE id = $1",
    )
    .bind(change_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_recipient_change))
}

/// The latest still-pending (un-applied, un-cancelled) change for a Letter.
pub async fn fetch_pending_recipient_change(
    pool: &PgPool,
    letter_id: LetterId,
) -> Result<Option<RecipientChange>, DbError> {
    let row = sqlx::query(
        "SELECT id, letter_id, old_email, new_email, requested_at,
                effective_at, applied_at, cancelled_at
           FROM letter_recipient_change
          WHERE letter_id = $1 AND applied_at IS NULL AND cancelled_at IS NULL
          ORDER BY requested_at DESC LIMIT 1",
    )
    .bind(letter_id.as_uuid())
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_recipient_change))
}

pub async fn cancel_recipient_change(pool: &PgPool, change_id: Uuid) -> Result<bool, DbError> {
    let res = sqlx::query(
        "UPDATE letter_recipient_change SET cancelled_at = now()
          WHERE id = $1 AND applied_at IS NULL AND cancelled_at IS NULL",
    )
    .bind(change_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() == 1)
}

/// Change ids whose hold has elapsed and are still pending.
pub async fn list_due_recipient_changes(
    pool: &PgPool,
    now: DateTime<Utc>,
) -> Result<Vec<Uuid>, DbError> {
    let rows = sqlx::query(
        "SELECT id FROM letter_recipient_change
          WHERE applied_at IS NULL AND cancelled_at IS NULL AND effective_at <= $1",
    )
    .bind(now)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(|r| r.get("id")).collect())
}

/// Apply a due recipient change: set the Letter's delivery address and mark
/// the change applied, atomically. Returns `(letter_id, new_email)` if applied,
/// `None` if it was already applied or cancelled (idempotent).
pub async fn apply_recipient_change(
    pool: &PgPool,
    change_id: Uuid,
) -> Result<Option<(LetterId, String)>, DbError> {
    let row = sqlx::query(
        "WITH chg AS (
            UPDATE letter_recipient_change SET applied_at = now()
             WHERE id = $1 AND applied_at IS NULL AND cancelled_at IS NULL
           RETURNING letter_id, new_email
         )
         UPDATE letter SET recipient_email = chg.new_email
           FROM chg WHERE letter.id = chg.letter_id
         RETURNING letter.id AS letter_id, letter.recipient_email AS new_email",
    )
    .bind(change_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| {
        let id: Uuid = r.get("letter_id");
        (LetterId(id), r.get("new_email"))
    }))
}

/// Crypto-erase letters' ciphertext (for retention expiry). The metadata row
/// stays for audit but the payload is zeroised.
pub async fn purge_letter_ciphertext(pool: &PgPool, letter_id: LetterId) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE letter
            SET ciphertext = E'\\\\x'::bytea,
                drill_ciphertext = E'\\\\x'::bytea,
                nonce = E'\\\\x'::bytea,
                drill_nonce = E'\\\\x'::bytea
          WHERE id = $1",
    )
    .bind(letter_id.as_uuid())
    .execute(pool)
    .await?;
    Ok(())
}

// ----------------------------------------------------------------------------
// Heartbeats
// ----------------------------------------------------------------------------

pub async fn record_heartbeat(
    pool: &PgPool,
    principal_id: PrincipalId,
    via: &str,
) -> Result<DateTime<Utc>, DbError> {
    let row = sqlx::query(
        "INSERT INTO heartbeat (principal_id, via) VALUES ($1, $2) RETURNING received_at",
    )
    .bind(principal_id.as_uuid())
    .bind(via)
    .fetch_one(pool)
    .await?;

    // Bump all the principal's vaults' last_attestation_at.
    sqlx::query("UPDATE vault SET last_attestation_at = now() WHERE principal_id = $1")
        .bind(principal_id.as_uuid())
        .execute(pool)
        .await?;

    Ok(row.get("received_at"))
}

pub async fn last_heartbeat(
    pool: &PgPool,
    principal_id: PrincipalId,
) -> Result<Option<DateTime<Utc>>, DbError> {
    let row = sqlx::query(
        "SELECT received_at FROM heartbeat WHERE principal_id = $1
          ORDER BY received_at DESC LIMIT 1",
    )
    .bind(principal_id.as_uuid())
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.get("received_at")))
}

// ----------------------------------------------------------------------------
// Releases
// ----------------------------------------------------------------------------

pub async fn create_release_event(
    pool: &PgPool,
    vault_id: VaultId,
    reason: ReleaseReason,
    is_drill: bool,
) -> Result<ReleaseEvent, DbError> {
    let row = sqlx::query(
        "INSERT INTO release_event (vault_id, reason, is_drill)
         VALUES ($1, $2, $3)
         RETURNING id, vault_id, reason, triggered_at, released_at, cancelled_at, is_drill",
    )
    .bind(vault_id.as_uuid())
    .bind(reason.as_db_str())
    .bind(is_drill)
    .fetch_one(pool)
    .await?;
    Ok(ReleaseEvent {
        id: ReleaseEventId(row.get("id")),
        vault_id: VaultId(row.get("vault_id")),
        reason,
        triggered_at: row.get("triggered_at"),
        released_at: row.try_get("released_at").ok(),
        cancelled_at: row.try_get("cancelled_at").ok(),
        is_drill: row.get("is_drill"),
    })
}

/// Like [`create_release_event`] but records a durable cooling-off deadline,
/// so the release survives a process restart and can be fired by any replica's
/// poll loop rather than an in-memory timer.
pub async fn create_release_event_with_deadline(
    pool: &PgPool,
    vault_id: VaultId,
    reason: ReleaseReason,
    is_drill: bool,
    cooling_off_ends_at: DateTime<Utc>,
) -> Result<ReleaseEvent, DbError> {
    let row = sqlx::query(
        "INSERT INTO release_event (vault_id, reason, is_drill, cooling_off_ends_at)
         VALUES ($1, $2, $3, $4)
         RETURNING id, vault_id, reason, triggered_at, released_at, cancelled_at, is_drill",
    )
    .bind(vault_id.as_uuid())
    .bind(reason.as_db_str())
    .bind(is_drill)
    .bind(cooling_off_ends_at)
    .fetch_one(pool)
    .await?;
    Ok(ReleaseEvent {
        id: ReleaseEventId(row.get("id")),
        vault_id: VaultId(row.get("vault_id")),
        reason,
        triggered_at: row.get("triggered_at"),
        released_at: row.try_get("released_at").ok(),
        cancelled_at: row.try_get("cancelled_at").ok(),
        is_drill: row.get("is_drill"),
    })
}

/// Atomically transition a Vault `COOLING_OFF → RELEASING`. Returns `true` iff
/// this caller won the transition (exactly one worker can, even across replicas
/// and the per-request timer). The release pipeline runs only for the winner.
pub async fn claim_vault_for_release(pool: &PgPool, vault_id: VaultId) -> Result<bool, DbError> {
    let res =
        sqlx::query("UPDATE vault SET state = 'RELEASING' WHERE id = $1 AND state = 'COOLING_OFF'")
            .bind(vault_id.as_uuid())
            .execute(pool)
            .await?;
    Ok(res.rows_affected() == 1)
}

/// Atomically transition a Vault into `COOLING_OFF` from a known prior state.
/// Returns `true` iff this caller won the transition, so cooling-off side
/// effects (the release event, the notification) fire exactly once.
pub async fn claim_vault_cooling_off(
    pool: &PgPool,
    vault_id: VaultId,
    from: VaultState,
) -> Result<bool, DbError> {
    let res = sqlx::query(
        "UPDATE vault
            SET state = 'COOLING_OFF', cooling_off_started_at = now()
          WHERE id = $1 AND state = $2",
    )
    .bind(vault_id.as_uuid())
    .bind(from.as_db_str())
    .execute(pool)
    .await?;
    Ok(res.rows_affected() == 1)
}

/// Open release events whose durable cooling-off deadline has elapsed and whose
/// Vault is still `COOLING_OFF`. The poll loop claims each via
/// [`claim_vault_for_release`] before performing it. Returns
/// `(vault_id, release_event_id, is_drill)`.
pub async fn list_due_releases(
    pool: &PgPool,
    now: DateTime<Utc>,
) -> Result<Vec<(VaultId, ReleaseEventId, bool)>, DbError> {
    let rows = sqlx::query(
        "SELECT re.id, re.vault_id, re.is_drill
           FROM release_event re
           JOIN vault v ON v.id = re.vault_id
          WHERE re.released_at IS NULL
            AND re.cancelled_at IS NULL
            AND re.cooling_off_ends_at IS NOT NULL
            AND re.cooling_off_ends_at <= $1
            AND v.state = 'COOLING_OFF'",
    )
    .bind(now)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| {
            (
                VaultId(r.get("vault_id")),
                ReleaseEventId(r.get("id")),
                r.get("is_drill"),
            )
        })
        .collect())
}

pub async fn mark_release_released(
    pool: &PgPool,
    release_id: ReleaseEventId,
) -> Result<(), DbError> {
    sqlx::query("UPDATE release_event SET released_at = now() WHERE id = $1")
        .bind(release_id.as_uuid())
        .execute(pool)
        .await?;
    Ok(())
}

/// Close an open release event as cancelled. Used when the principal cancels
/// during cooling-off, so the poll loop will not consider it again.
pub async fn mark_release_cancelled(
    pool: &PgPool,
    release_id: ReleaseEventId,
) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE release_event
            SET cancelled_at = now()
          WHERE id = $1 AND released_at IS NULL AND cancelled_at IS NULL",
    )
    .bind(release_id.as_uuid())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn fetch_open_release(
    pool: &PgPool,
    vault_id: VaultId,
) -> Result<Option<ReleaseEvent>, DbError> {
    let row = sqlx::query(
        "SELECT id, vault_id, reason, triggered_at, released_at, cancelled_at, is_drill
           FROM release_event
          WHERE vault_id = $1 AND released_at IS NULL AND cancelled_at IS NULL
          ORDER BY triggered_at DESC LIMIT 1",
    )
    .bind(vault_id.as_uuid())
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|row| ReleaseEvent {
        id: ReleaseEventId(row.get("id")),
        vault_id: VaultId(row.get("vault_id")),
        reason: ReleaseReason::SignalTrigger, // skeleton: we don't parse it back
        triggered_at: row.get("triggered_at"),
        released_at: row.try_get("released_at").ok(),
        cancelled_at: row.try_get("cancelled_at").ok(),
        is_drill: row.try_get("is_drill").unwrap_or(false),
    }))
}

pub async fn issue_release_claim(
    pool: &PgPool,
    release_event_id: ReleaseEventId,
    letter_id: LetterId,
    recipient_email: &str,
    token_hash: &[u8],
    ttl_days: i64,
) -> Result<(), DbError> {
    let expires_at = Utc::now() + ChronoDuration::days(ttl_days);
    sqlx::query(
        "INSERT INTO release_claim
            (token_hash, release_event_id, letter_id, recipient_email, expires_at)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(token_hash)
    .bind(release_event_id.as_uuid())
    .bind(letter_id.as_uuid())
    .bind(recipient_email)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Returns (letter_id, recipient_email, is_drill) if the claim is valid;
/// marks it claimed atomically.
pub async fn consume_release_claim(
    pool: &PgPool,
    token_hash: &[u8],
) -> Result<Option<(LetterId, String, bool)>, DbError> {
    let row = sqlx::query(
        "WITH consumed AS (
            UPDATE release_claim
               SET claimed_at = now()
             WHERE token_hash = $1
               AND claimed_at IS NULL
               AND expires_at > now()
           RETURNING letter_id, recipient_email, release_event_id
         )
         SELECT c.letter_id, c.recipient_email, re.is_drill
           FROM consumed c
           JOIN release_event re ON re.id = c.release_event_id",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| {
        let id: Uuid = r.get("letter_id");
        (LetterId(id), r.get("recipient_email"), r.get("is_drill"))
    }))
}

// ----------------------------------------------------------------------------
// Transparency log (skeleton: stored, not anchored)
// ----------------------------------------------------------------------------

pub async fn append_transparency_entry(
    pool: &PgPool,
    kind: &str,
    payload_hash: &[u8],
    payload: serde_json::Value,
    principal_id: Option<PrincipalId>,
    vault_id: Option<VaultId>,
) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO transparency_entry
            (kind, payload_hash, payload_json, principal_id, vault_id)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(kind)
    .bind(payload_hash)
    .bind(payload)
    .bind(principal_id.map(|p| p.as_uuid()))
    .bind(vault_id.map(|v| v.as_uuid()))
    .execute(pool)
    .await?;
    Ok(())
}

pub struct TransparencyEntry {
    pub id: Uuid,
    pub kind: String,
    pub ts: DateTime<Utc>,
    pub payload_json: serde_json::Value,
}

pub async fn list_transparency_entries(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<TransparencyEntry>, DbError> {
    let rows = sqlx::query(
        "SELECT id, kind, ts, payload_json FROM transparency_entry ORDER BY ts DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| TransparencyEntry {
            id: r.get("id"),
            kind: r.get("kind"),
            ts: r.get("ts"),
            payload_json: r.get("payload_json"),
        })
        .collect())
}

// ----------------------------------------------------------------------------
// Subscription extras (reactivation, retention sweep)
// ----------------------------------------------------------------------------

pub async fn reactivate_subscription(
    pool: &PgPool,
    principal_id: PrincipalId,
) -> Result<Subscription, DbError> {
    let row = sqlx::query(
        "UPDATE subscription
            SET state = 'ACTIVE',
                canceled_at = NULL,
                retention_until = NULL
          WHERE principal_id = $1
            AND state = 'CANCELED'
        RETURNING id, principal_id, plan_id, state, started_at, trial_end_at,
                  current_period_end, canceled_at, retention_until, extra_storage_bytes",
    )
    .bind(principal_id.as_uuid())
    .fetch_optional(pool)
    .await?
    .ok_or(DbError::NotFound)?;
    row_to_subscription(&row)
}

/// Subscriptions whose retention window has elapsed and are due to enter EXPIRED.
pub async fn list_canceled_past_retention(pool: &PgPool) -> Result<Vec<PrincipalId>, DbError> {
    let rows = sqlx::query(
        "SELECT principal_id FROM subscription
          WHERE state = 'CANCELED'
            AND retention_until IS NOT NULL
            AND retention_until <= now()",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| PrincipalId(r.get("principal_id")))
        .collect())
}

pub async fn expire_subscription(pool: &PgPool, principal_id: PrincipalId) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE subscription SET state = 'EXPIRED' WHERE principal_id = $1 AND state = 'CANCELED'",
    )
    .bind(principal_id.as_uuid())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_subscription(pool: &PgPool, principal_id: PrincipalId) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE subscription SET state = 'DELETED' WHERE principal_id = $1 AND state = 'EXPIRED'",
    )
    .bind(principal_id.as_uuid())
    .execute(pool)
    .await?;
    Ok(())
}

// ----------------------------------------------------------------------------
// Vault iteration (for the signal aggregator + schedulers)
// ----------------------------------------------------------------------------

pub async fn list_vaults_in_states(
    pool: &PgPool,
    states: &[VaultState],
) -> Result<Vec<Vault>, DbError> {
    let state_strs: Vec<&str> = states.iter().map(|s| s.as_db_str()).collect();
    let rows = sqlx::query(
        "SELECT id, principal_id, name, tier, state, cooling_off_seconds,
                last_attestation_at, cooling_off_started_at, released_at, created_at
           FROM vault WHERE state = ANY($1)",
    )
    .bind(&state_strs)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_vault).collect()
}

// ----------------------------------------------------------------------------
// Buddies
// ----------------------------------------------------------------------------

pub struct BuddyInviteInput<'a> {
    pub principal_id: PrincipalId,
    pub display_name: Option<&'a str>,
    pub email: &'a str,
    pub phone: Option<&'a str>,
    pub confirmation_token_hash: &'a [u8],
    pub prompt_cadence_days: i32,
}

pub async fn invite_buddy(pool: &PgPool, input: BuddyInviteInput<'_>) -> Result<Buddy, DbError> {
    let row = sqlx::query(
        "INSERT INTO buddy
            (principal_id, display_name, email, phone, confirmation_token_hash, prompt_cadence_days)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id, principal_id, display_name, email, phone, prompt_cadence_days,
                   last_prompt_at, last_response_at, last_response, confirmed_at, revoked_at",
    )
    .bind(input.principal_id.as_uuid())
    .bind(input.display_name)
    .bind(input.email)
    .bind(input.phone)
    .bind(input.confirmation_token_hash)
    .bind(input.prompt_cadence_days)
    .fetch_one(pool)
    .await?;
    row_to_buddy(&row)
}

pub async fn confirm_buddy(pool: &PgPool, token_hash: &[u8]) -> Result<Option<Buddy>, DbError> {
    let row = sqlx::query(
        "UPDATE buddy
            SET confirmed_at = now(),
                confirmation_token_hash = NULL
          WHERE confirmation_token_hash = $1
            AND confirmed_at IS NULL
            AND revoked_at IS NULL
        RETURNING id, principal_id, display_name, email, phone, prompt_cadence_days,
                  last_prompt_at, last_response_at, last_response, confirmed_at, revoked_at",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;
    match row {
        Some(r) => Ok(Some(row_to_buddy(&r)?)),
        None => Ok(None),
    }
}

pub async fn record_buddy_response(
    pool: &PgPool,
    buddy_id: BuddyId,
    response: BuddyResponse,
) -> Result<Buddy, DbError> {
    let row = sqlx::query(
        "UPDATE buddy
            SET last_response = $1,
                last_response_at = now()
          WHERE id = $2 AND confirmed_at IS NOT NULL AND revoked_at IS NULL
        RETURNING id, principal_id, display_name, email, phone, prompt_cadence_days,
                  last_prompt_at, last_response_at, last_response, confirmed_at, revoked_at",
    )
    .bind(response.as_db_str())
    .bind(buddy_id.as_uuid())
    .fetch_optional(pool)
    .await?
    .ok_or(DbError::NotFound)?;
    row_to_buddy(&row)
}

pub async fn list_buddies(pool: &PgPool, principal_id: PrincipalId) -> Result<Vec<Buddy>, DbError> {
    let rows = sqlx::query(
        "SELECT id, principal_id, display_name, email, phone, prompt_cadence_days,
                last_prompt_at, last_response_at, last_response, confirmed_at, revoked_at
           FROM buddy WHERE principal_id = $1
          ORDER BY created_at",
    )
    .bind(principal_id.as_uuid())
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_buddy).collect()
}

pub async fn fetch_buddy(pool: &PgPool, buddy_id: BuddyId) -> Result<Buddy, DbError> {
    let row = sqlx::query(
        "SELECT id, principal_id, display_name, email, phone, prompt_cadence_days,
                last_prompt_at, last_response_at, last_response, confirmed_at, revoked_at
           FROM buddy WHERE id = $1",
    )
    .bind(buddy_id.as_uuid())
    .fetch_optional(pool)
    .await?
    .ok_or(DbError::NotFound)?;
    row_to_buddy(&row)
}

pub async fn revoke_buddy(pool: &PgPool, buddy_id: BuddyId) -> Result<(), DbError> {
    sqlx::query("UPDATE buddy SET revoked_at = now() WHERE id = $1")
        .bind(buddy_id.as_uuid())
        .execute(pool)
        .await?;
    Ok(())
}

fn row_to_buddy(row: &PgRow) -> Result<Buddy, DbError> {
    Ok(Buddy {
        id: BuddyId(row.get("id")),
        principal_id: PrincipalId(row.get("principal_id")),
        display_name: row.try_get("display_name").ok(),
        email: row.get("email"),
        phone: row.try_get("phone").ok(),
        prompt_cadence_days: row.get("prompt_cadence_days"),
        last_prompt_at: row.try_get("last_prompt_at").ok(),
        last_response_at: row.try_get("last_response_at").ok(),
        last_response: row
            .try_get::<Option<String>, _>("last_response")
            .ok()
            .flatten()
            .map(|s| BuddyResponse::from_db_str(&s))
            .transpose()?,
        confirmed_at: row.try_get("confirmed_at").ok(),
        revoked_at: row.try_get("revoked_at").ok(),
    })
}

// ----------------------------------------------------------------------------
// Co-Stewards (read-only family deputies)
// ----------------------------------------------------------------------------

pub struct CoStewardInviteInput<'a> {
    pub principal_id: PrincipalId,
    pub display_name: Option<&'a str>,
    pub email: &'a str,
    pub confirmation_token_hash: &'a [u8],
}

pub async fn invite_co_steward(
    pool: &PgPool,
    input: CoStewardInviteInput<'_>,
) -> Result<beacon_core::CoSteward, DbError> {
    let row = sqlx::query(
        "INSERT INTO co_steward
            (principal_id, display_name, email, confirmation_token_hash)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (principal_id, email) DO UPDATE
            SET confirmation_token_hash = EXCLUDED.confirmation_token_hash,
                confirmed_at = NULL,
                revoked_at = NULL
         RETURNING id, principal_id, display_name, email, confirmed_at,
                   revoked_at, last_viewed_at, created_at",
    )
    .bind(input.principal_id.as_uuid())
    .bind(input.display_name)
    .bind(input.email)
    .bind(input.confirmation_token_hash)
    .fetch_one(pool)
    .await?;
    Ok(row_to_co_steward(&row))
}

pub async fn confirm_co_steward(
    pool: &PgPool,
    token_hash: &[u8],
    passphrase_hash: &str,
    passphrase_salt: &[u8],
) -> Result<Option<beacon_core::CoSteward>, DbError> {
    let row = sqlx::query(
        "UPDATE co_steward
            SET confirmed_at = COALESCE(confirmed_at, now()),
                passphrase_hash = $2,
                passphrase_salt = $3,
                confirmation_token_hash = NULL
          WHERE confirmation_token_hash = $1
            AND revoked_at IS NULL
         RETURNING id, principal_id, display_name, email, confirmed_at,
                   revoked_at, last_viewed_at, created_at",
    )
    .bind(token_hash)
    .bind(passphrase_hash)
    .bind(passphrase_salt)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| row_to_co_steward(&r)))
}

/// Sign-in: verify the (email, passphrase_hash) pair and return the row if
/// it matches. The caller computes the passphrase hash with the salt
/// returned by `lookup_co_steward_by_email` before calling.
pub async fn sign_in_co_steward(
    pool: &PgPool,
    email: &str,
    passphrase_hash: &str,
) -> Result<Option<beacon_core::CoSteward>, DbError> {
    let row = sqlx::query(
        "SELECT id, principal_id, display_name, email, confirmed_at,
                revoked_at, last_viewed_at, created_at
           FROM co_steward
          WHERE email = $1
            AND confirmed_at IS NOT NULL
            AND revoked_at IS NULL
            AND passphrase_hash = $2",
    )
    .bind(email)
    .bind(passphrase_hash)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| row_to_co_steward(&r)))
}

pub async fn lookup_co_steward_salt(
    pool: &PgPool,
    email: &str,
) -> Result<Option<Vec<u8>>, DbError> {
    let row = sqlx::query(
        "SELECT passphrase_salt FROM co_steward
          WHERE email = $1 AND confirmed_at IS NOT NULL AND revoked_at IS NULL",
    )
    .bind(email)
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|r| r.try_get("passphrase_salt").ok()))
}

pub async fn list_co_stewards(
    pool: &PgPool,
    principal_id: PrincipalId,
) -> Result<Vec<beacon_core::CoSteward>, DbError> {
    let rows = sqlx::query(
        "SELECT id, principal_id, display_name, email, confirmed_at,
                revoked_at, last_viewed_at, created_at
           FROM co_steward
          WHERE principal_id = $1 AND revoked_at IS NULL
          ORDER BY created_at DESC",
    )
    .bind(principal_id.as_uuid())
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_co_steward).collect())
}

pub async fn co_steward_count(pool: &PgPool, principal_id: PrincipalId) -> Result<i64, DbError> {
    let row = sqlx::query(
        "SELECT COUNT(*)::BIGINT AS n FROM co_steward
          WHERE principal_id = $1 AND revoked_at IS NULL",
    )
    .bind(principal_id.as_uuid())
    .fetch_one(pool)
    .await?;
    Ok(row.get::<i64, _>("n"))
}

pub async fn fetch_co_steward(
    pool: &PgPool,
    co_steward_id: beacon_core::CoStewardId,
) -> Result<beacon_core::CoSteward, DbError> {
    let row = sqlx::query(
        "SELECT id, principal_id, display_name, email, confirmed_at,
                revoked_at, last_viewed_at, created_at
           FROM co_steward WHERE id = $1",
    )
    .bind(co_steward_id.as_uuid())
    .fetch_optional(pool)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(row_to_co_steward(&row))
}

pub async fn revoke_co_steward(
    pool: &PgPool,
    co_steward_id: beacon_core::CoStewardId,
) -> Result<(), DbError> {
    sqlx::query("UPDATE co_steward SET revoked_at = now() WHERE id = $1")
        .bind(co_steward_id.as_uuid())
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn touch_co_steward_view(
    pool: &PgPool,
    co_steward_id: beacon_core::CoStewardId,
) -> Result<(), DbError> {
    sqlx::query("UPDATE co_steward SET last_viewed_at = now() WHERE id = $1")
        .bind(co_steward_id.as_uuid())
        .execute(pool)
        .await?;
    Ok(())
}

/// Create a session for a confirmed Co-Steward. Identical to
/// [`create_session`] except the session is tagged as CO_STEWARD and
/// points at the co_steward row instead of a principal.
pub async fn create_co_steward_session(
    pool: &PgPool,
    co_steward_id: beacon_core::CoStewardId,
    principal_id: PrincipalId,
    token_hash: &[u8],
    ttl_hours: i64,
) -> Result<(), DbError> {
    let expires_at = Utc::now() + ChronoDuration::hours(ttl_hours);
    sqlx::query(
        "INSERT INTO session
            (token_hash, principal_id, co_steward_id, auth_role, expires_at)
         VALUES ($1, $2, $3, 'CO_STEWARD', $4)",
    )
    .bind(token_hash)
    .bind(principal_id.as_uuid())
    .bind(co_steward_id.as_uuid())
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// If the session token belongs to a Co-Steward, return their id +
/// the Principal they have read-only access to.
pub async fn co_steward_for_session(
    pool: &PgPool,
    token_hash: &[u8],
) -> Result<Option<(beacon_core::CoStewardId, PrincipalId)>, DbError> {
    let row = sqlx::query(
        "SELECT co_steward_id, principal_id FROM session
          WHERE token_hash = $1
            AND auth_role = 'CO_STEWARD'
            AND co_steward_id IS NOT NULL
            AND revoked_at IS NULL
            AND expires_at > now()",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| {
        (
            beacon_core::CoStewardId(r.get("co_steward_id")),
            PrincipalId(r.get("principal_id")),
        )
    }))
}

fn row_to_co_steward(row: &PgRow) -> beacon_core::CoSteward {
    beacon_core::CoSteward {
        id: beacon_core::CoStewardId(row.get("id")),
        principal_id: PrincipalId(row.get("principal_id")),
        display_name: row.try_get("display_name").ok(),
        email: row.get("email"),
        confirmed_at: row.try_get("confirmed_at").ok(),
        revoked_at: row.try_get("revoked_at").ok(),
        last_viewed_at: row.try_get("last_viewed_at").ok(),
        created_at: row.get("created_at"),
    }
}

// ----------------------------------------------------------------------------
// Signal subscriptions + observations
// ----------------------------------------------------------------------------

pub async fn upsert_signal_subscription(
    pool: &PgPool,
    vault_id: VaultId,
    source: SignalSource,
    weight: f32,
    enabled: bool,
) -> Result<SignalSubscription, DbError> {
    let row = sqlx::query(
        "INSERT INTO signal_subscription (vault_id, source, weight, enabled)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (vault_id, source) DO UPDATE
            SET weight = EXCLUDED.weight,
                enabled = EXCLUDED.enabled
         RETURNING id, vault_id, source, weight, enabled, config",
    )
    .bind(vault_id.as_uuid())
    .bind(source.as_db_str())
    .bind(weight)
    .bind(enabled)
    .fetch_one(pool)
    .await?;
    row_to_signal_subscription(&row)
}

pub async fn list_signal_subscriptions(
    pool: &PgPool,
    vault_id: VaultId,
) -> Result<Vec<SignalSubscription>, DbError> {
    let rows = sqlx::query(
        "SELECT id, vault_id, source, weight, enabled, config
           FROM signal_subscription WHERE vault_id = $1",
    )
    .bind(vault_id.as_uuid())
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_signal_subscription).collect()
}

fn row_to_signal_subscription(row: &PgRow) -> Result<SignalSubscription, DbError> {
    Ok(SignalSubscription {
        id: SignalSubscriptionId(row.get("id")),
        vault_id: VaultId(row.get("vault_id")),
        source: SignalSource::from_db_str(row.get("source"))?,
        weight: row.get("weight"),
        enabled: row.get("enabled"),
        config: row.try_get("config").unwrap_or(serde_json::Value::Null),
    })
}

pub async fn record_signal(
    pool: &PgPool,
    vault_id: VaultId,
    source: SignalSource,
    contribution: f32,
    evidence: serde_json::Value,
) -> Result<Signal, DbError> {
    let row = sqlx::query(
        "INSERT INTO signal (vault_id, source, contribution, evidence)
         VALUES ($1, $2, $3, $4)
         RETURNING id, vault_id, source, observed_at, contribution, evidence",
    )
    .bind(vault_id.as_uuid())
    .bind(source.as_db_str())
    .bind(contribution)
    .bind(evidence)
    .fetch_one(pool)
    .await?;
    Ok(Signal {
        id: SignalId(row.get("id")),
        vault_id: VaultId(row.get("vault_id")),
        source,
        observed_at: row.get("observed_at"),
        contribution: row.get("contribution"),
        evidence: row.try_get("evidence").unwrap_or(serde_json::Value::Null),
    })
}

pub async fn list_recent_signals(
    pool: &PgPool,
    vault_id: VaultId,
    since: DateTime<Utc>,
) -> Result<Vec<Signal>, DbError> {
    let rows = sqlx::query(
        "SELECT id, vault_id, source, observed_at, contribution, evidence
           FROM signal
          WHERE vault_id = $1 AND observed_at >= $2
          ORDER BY observed_at DESC",
    )
    .bind(vault_id.as_uuid())
    .bind(since)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|r| {
            Ok(Signal {
                id: SignalId(r.get("id")),
                vault_id: VaultId(r.get("vault_id")),
                source: SignalSource::from_db_str(r.get("source"))?,
                observed_at: r.get("observed_at"),
                contribution: r.get("contribution"),
                evidence: r.try_get("evidence").unwrap_or(serde_json::Value::Null),
            })
        })
        .collect()
}

// ----------------------------------------------------------------------------
// Apple Shortcut subscriptions
// ----------------------------------------------------------------------------

pub async fn create_apple_shortcut_sub(
    pool: &PgPool,
    principal_id: PrincipalId,
    installation_id: &str,
    secret_key: &[u8],
) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO apple_shortcut_subscription
            (principal_id, installation_id, secret_hmac_key)
         VALUES ($1, $2, $3)",
    )
    .bind(principal_id.as_uuid())
    .bind(installation_id)
    .bind(secret_key)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn fetch_apple_shortcut_sub(
    pool: &PgPool,
    installation_id: &str,
) -> Result<Option<(PrincipalId, Vec<u8>)>, DbError> {
    let row = sqlx::query(
        "SELECT principal_id, secret_hmac_key
           FROM apple_shortcut_subscription WHERE installation_id = $1",
    )
    .bind(installation_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| (PrincipalId(r.get("principal_id")), r.get("secret_hmac_key"))))
}

/// Total transformed-attachment bytes across every Vault owned by `principal_id`.
/// Used for plan-quota enforcement.
pub async fn storage_used(pool: &PgPool, principal_id: PrincipalId) -> Result<i64, DbError> {
    let row = sqlx::query(
        "SELECT COALESCE(SUM(a.transformed_size), 0)::BIGINT AS total
           FROM attachment a
           JOIN letter l ON l.id = a.letter_id
           JOIN vault v ON v.id = l.vault_id
          WHERE v.principal_id = $1",
    )
    .bind(principal_id.as_uuid())
    .fetch_one(pool)
    .await?;
    Ok(row.get::<i64, _>("total"))
}

/// Vault count for the principal, for the max_vaults check.
pub async fn vault_count(pool: &PgPool, principal_id: PrincipalId) -> Result<i64, DbError> {
    let row = sqlx::query("SELECT COUNT(*)::BIGINT AS n FROM vault WHERE principal_id = $1")
        .bind(principal_id.as_uuid())
        .fetch_one(pool)
        .await?;
    Ok(row.get::<i64, _>("n"))
}

/// Letter count under a Vault, for the max_letters_per_vault check.
pub async fn letter_count(pool: &PgPool, vault_id: VaultId) -> Result<i64, DbError> {
    let row = sqlx::query("SELECT COUNT(*)::BIGINT AS n FROM letter WHERE vault_id = $1")
        .bind(vault_id.as_uuid())
        .fetch_one(pool)
        .await?;
    Ok(row.get::<i64, _>("n"))
}

pub async fn touch_apple_shortcut_sub(pool: &PgPool, installation_id: &str) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE apple_shortcut_subscription
            SET last_ping_at = now()
          WHERE installation_id = $1",
    )
    .bind(installation_id)
    .execute(pool)
    .await?;
    Ok(())
}

// ----------------------------------------------------------------------------
// Terms of Service
// ----------------------------------------------------------------------------

pub async fn set_tos_accepted(pool: &PgPool, principal_id: PrincipalId) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE principal SET tos_accepted_at = COALESCE(tos_accepted_at, now())
          WHERE id = $1",
    )
    .bind(principal_id.as_uuid())
    .execute(pool)
    .await?;
    Ok(())
}

// ----------------------------------------------------------------------------
// Vault contacts
// ----------------------------------------------------------------------------

pub struct VaultContact {
    pub id: Uuid,
    pub vault_id: VaultId,
    pub display_name: String,
    pub email: String,
    pub birthday: Option<String>,
    pub release_note: Option<String>,
    pub created_at: DateTime<Utc>,
}

fn row_to_vault_contact(row: &PgRow) -> VaultContact {
    let birthday: Option<chrono::NaiveDate> = row.try_get("birthday").ok().flatten();
    VaultContact {
        id: row.get("id"),
        vault_id: VaultId(row.get("vault_id")),
        display_name: row.get("display_name"),
        email: row.get("email"),
        birthday: birthday.map(|d| d.format("%Y-%m-%d").to_string()),
        release_note: row.try_get("release_note").ok().flatten(),
        created_at: row.get("created_at"),
    }
}

pub async fn list_vault_contacts(
    pool: &PgPool,
    vault_id: VaultId,
) -> Result<Vec<VaultContact>, DbError> {
    let rows = sqlx::query(
        "SELECT id, vault_id, display_name, email, birthday, release_note, created_at
           FROM vault_contact WHERE vault_id = $1 ORDER BY created_at",
    )
    .bind(vault_id.as_uuid())
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_vault_contact).collect())
}

pub async fn create_vault_contact(
    pool: &PgPool,
    vault_id: VaultId,
    display_name: &str,
    email: &str,
    birthday: Option<&str>,
    release_note: Option<&str>,
) -> Result<VaultContact, DbError> {
    let bd: Option<chrono::NaiveDate> =
        birthday.and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
    let row = sqlx::query(
        "INSERT INTO vault_contact (vault_id, display_name, email, birthday, release_note)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (vault_id, email) DO UPDATE
            SET display_name = EXCLUDED.display_name,
                birthday     = EXCLUDED.birthday,
                release_note = EXCLUDED.release_note
         RETURNING id, vault_id, display_name, email, birthday, release_note, created_at",
    )
    .bind(vault_id.as_uuid())
    .bind(display_name)
    .bind(email)
    .bind(bd)
    .bind(release_note)
    .fetch_one(pool)
    .await?;
    Ok(row_to_vault_contact(&row))
}

pub async fn update_vault_contact(
    pool: &PgPool,
    contact_id: Uuid,
    vault_id: VaultId,
    display_name: &str,
    email: &str,
    birthday: Option<&str>,
    release_note: Option<&str>,
) -> Result<VaultContact, DbError> {
    let bd: Option<chrono::NaiveDate> =
        birthday.and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
    let row = sqlx::query(
        "UPDATE vault_contact
            SET display_name = $1, email = $2, birthday = $3, release_note = $4
          WHERE id = $5 AND vault_id = $6
         RETURNING id, vault_id, display_name, email, birthday, release_note, created_at",
    )
    .bind(display_name)
    .bind(email)
    .bind(bd)
    .bind(release_note)
    .bind(contact_id)
    .bind(vault_id.as_uuid())
    .fetch_optional(pool)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(row_to_vault_contact(&row))
}

pub async fn delete_vault_contact(
    pool: &PgPool,
    contact_id: Uuid,
    vault_id: VaultId,
) -> Result<(), DbError> {
    sqlx::query("DELETE FROM vault_contact WHERE id = $1 AND vault_id = $2")
        .bind(contact_id)
        .bind(vault_id.as_uuid())
        .execute(pool)
        .await?;
    Ok(())
}

// ----------------------------------------------------------------------------
// Bank dormancy subscriptions
// ----------------------------------------------------------------------------

pub struct BankDormancyRow {
    pub id: Uuid,
    pub principal_id: PrincipalId,
    pub webhook_id: Uuid,
    pub last_webhook_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

fn row_to_bank_dormancy(row: &PgRow) -> BankDormancyRow {
    BankDormancyRow {
        id: row.get("id"),
        principal_id: PrincipalId(row.get("principal_id")),
        webhook_id: row.get("webhook_id"),
        last_webhook_at: row.try_get("last_webhook_at").ok().flatten(),
        created_at: row.get("created_at"),
    }
}

pub async fn create_bank_dormancy_sub(
    pool: &PgPool,
    principal_id: PrincipalId,
    webhook_id: Uuid,
    secret_hmac_key: &[u8],
) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO bank_dormancy_subscription (principal_id, webhook_id, secret_hmac_key)
         VALUES ($1, $2, $3)
         ON CONFLICT (principal_id) DO NOTHING",
    )
    .bind(principal_id.as_uuid())
    .bind(webhook_id)
    .bind(secret_hmac_key)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn fetch_bank_dormancy_sub(
    pool: &PgPool,
    principal_id: PrincipalId,
) -> Result<Option<BankDormancyRow>, DbError> {
    let row = sqlx::query(
        "SELECT id, principal_id, webhook_id, last_webhook_at, created_at
           FROM bank_dormancy_subscription WHERE principal_id = $1",
    )
    .bind(principal_id.as_uuid())
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_bank_dormancy))
}

pub async fn fetch_bank_dormancy_sub_by_webhook(
    pool: &PgPool,
    webhook_id: Uuid,
) -> Result<Option<BankDormancyRow>, DbError> {
    let row = sqlx::query(
        "SELECT id, principal_id, webhook_id, last_webhook_at, created_at
           FROM bank_dormancy_subscription WHERE webhook_id = $1",
    )
    .bind(webhook_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_bank_dormancy))
}

pub async fn touch_bank_dormancy_sub(
    pool: &PgPool,
    webhook_id: Uuid,
    now: DateTime<Utc>,
) -> Result<(), DbError> {
    sqlx::query("UPDATE bank_dormancy_subscription SET last_webhook_at = $1 WHERE webhook_id = $2")
        .bind(now)
        .bind(webhook_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_bank_dormancy_sub(
    pool: &PgPool,
    principal_id: PrincipalId,
) -> Result<(), DbError> {
    sqlx::query("DELETE FROM bank_dormancy_subscription WHERE principal_id = $1")
        .bind(principal_id.as_uuid())
        .execute(pool)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Duress signal — covert, user-armed panic webhook (migration 0022).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct DuressSignalRow {
    pub principal_id: PrincipalId,
    pub webhook_id: Uuid,
    pub alert_email: Option<String>,
    pub armed_at: DateTime<Utc>,
    pub triggered_at: Option<DateTime<Utc>>,
    /// "freeze" (default) or "release".
    pub panic_mode: String,
}

fn row_to_duress(row: &PgRow) -> DuressSignalRow {
    DuressSignalRow {
        principal_id: PrincipalId(row.get("principal_id")),
        webhook_id: row.get("webhook_id"),
        alert_email: row.try_get("alert_email").ok().flatten(),
        armed_at: row.get("armed_at"),
        triggered_at: row.try_get("triggered_at").ok().flatten(),
        panic_mode: row
            .try_get("panic_mode")
            .unwrap_or_else(|_| "freeze".into()),
    }
}

/// Arm (or re-arm) the duress signal. Re-arming rotates the webhook and clears
/// any prior trigger.
pub async fn arm_duress(
    pool: &PgPool,
    principal_id: PrincipalId,
    webhook_id: Uuid,
    alert_email: Option<&str>,
    panic_mode: &str,
) -> Result<DuressSignalRow, DbError> {
    let row = sqlx::query(
        "INSERT INTO duress_signal (principal_id, webhook_id, alert_email, panic_mode)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (principal_id) DO UPDATE
            SET webhook_id = EXCLUDED.webhook_id,
                alert_email = EXCLUDED.alert_email,
                panic_mode = EXCLUDED.panic_mode,
                armed_at = now(),
                triggered_at = NULL,
                last_alert_at = NULL
         RETURNING principal_id, webhook_id, alert_email, armed_at, triggered_at, panic_mode",
    )
    .bind(principal_id.as_uuid())
    .bind(webhook_id)
    .bind(alert_email)
    .bind(panic_mode)
    .fetch_one(pool)
    .await?;
    Ok(row_to_duress(&row))
}

pub async fn fetch_duress(
    pool: &PgPool,
    principal_id: PrincipalId,
) -> Result<Option<DuressSignalRow>, DbError> {
    let row = sqlx::query(
        "SELECT principal_id, webhook_id, alert_email, armed_at, triggered_at, panic_mode
           FROM duress_signal WHERE principal_id = $1",
    )
    .bind(principal_id.as_uuid())
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_duress))
}

pub async fn delete_duress(pool: &PgPool, principal_id: PrincipalId) -> Result<(), DbError> {
    sqlx::query("DELETE FROM duress_signal WHERE principal_id = $1")
        .bind(principal_id.as_uuid())
        .execute(pool)
        .await?;
    Ok(())
}

/// Fire the duress signal by its opaque webhook id. Idempotently sets
/// triggered_at (keeps the first trigger time) and records the alert time.
/// Returns the row (for the silent contact alert) when the webhook is valid.
pub async fn trigger_duress(
    pool: &PgPool,
    webhook_id: Uuid,
    now: DateTime<Utc>,
) -> Result<Option<DuressSignalRow>, DbError> {
    let row = sqlx::query(
        "UPDATE duress_signal
            SET triggered_at = COALESCE(triggered_at, $1),
                last_alert_at = $1
          WHERE webhook_id = $2
        RETURNING principal_id, webhook_id, alert_email, armed_at, triggered_at, panic_mode",
    )
    .bind(now)
    .bind(webhook_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_duress))
}

/// True while the principal has a fired duress signal — the dead-man's switch is
/// frozen until they disarm.
pub async fn principal_in_duress(
    pool: &PgPool,
    principal_id: PrincipalId,
) -> Result<bool, DbError> {
    let hit: Option<bool> = sqlx::query_scalar(
        "SELECT true FROM duress_signal
          WHERE principal_id = $1 AND triggered_at IS NOT NULL",
    )
    .bind(principal_id.as_uuid())
    .fetch_optional(pool)
    .await?;
    Ok(hit.unwrap_or(false))
}

pub async fn list_bank_dormancy_subs_with_recent_ping(
    pool: &PgPool,
    principal_id: PrincipalId,
    since: DateTime<Utc>,
) -> Result<Vec<Uuid>, DbError> {
    let rows = sqlx::query(
        "SELECT webhook_id FROM bank_dormancy_subscription
          WHERE principal_id = $1 AND last_webhook_at > $2",
    )
    .bind(principal_id.as_uuid())
    .bind(since)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(|r| r.get("webhook_id")).collect())
}

// ----------------------------------------------------------------------------
// Attachments
// ----------------------------------------------------------------------------

pub struct AttachmentInput<'a> {
    pub letter_id: LetterId,
    pub original_filename: &'a str,
    pub original_mime: &'a str,
    pub original_size: i64,
    pub transformed_mime: &'a str,
    pub transformed_size: i64,
    pub sha256: &'a [u8],
    pub transformer_notes: serde_json::Value,
    pub storage_key: &'a str,
    pub nonce: &'a [u8],
}

pub async fn create_attachment(
    pool: &PgPool,
    input: AttachmentInput<'_>,
) -> Result<Attachment, DbError> {
    let row = sqlx::query(
        "INSERT INTO attachment
            (letter_id, original_filename, original_mime, original_size,
             transformed_mime, transformed_size, sha256, transformer_notes,
             storage_key, nonce)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         RETURNING id, letter_id, original_filename, original_mime, original_size,
                   transformed_mime, transformed_size, sha256, transformer_notes,
                   storage_key, sealed_at",
    )
    .bind(input.letter_id.as_uuid())
    .bind(input.original_filename)
    .bind(input.original_mime)
    .bind(input.original_size)
    .bind(input.transformed_mime)
    .bind(input.transformed_size)
    .bind(input.sha256)
    .bind(input.transformer_notes)
    .bind(input.storage_key)
    .bind(input.nonce)
    .fetch_one(pool)
    .await?;
    row_to_attachment(&row)
}

pub async fn list_attachments(
    pool: &PgPool,
    letter_id: LetterId,
) -> Result<Vec<Attachment>, DbError> {
    let rows = sqlx::query(
        "SELECT id, letter_id, original_filename, original_mime, original_size,
                transformed_mime, transformed_size, sha256, transformer_notes,
                storage_key, sealed_at
           FROM attachment WHERE letter_id = $1 ORDER BY sealed_at",
    )
    .bind(letter_id.as_uuid())
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_attachment).collect()
}

pub async fn fetch_attachment(
    pool: &PgPool,
    id: AttachmentId,
) -> Result<(Attachment, Vec<u8>), DbError> {
    // Returns (attachment, nonce) so callers can fetch ciphertext from BlobStore.
    let row = sqlx::query(
        "SELECT id, letter_id, original_filename, original_mime, original_size,
                transformed_mime, transformed_size, sha256, transformer_notes,
                storage_key, sealed_at, nonce
           FROM attachment WHERE id = $1",
    )
    .bind(id.as_uuid())
    .fetch_optional(pool)
    .await?
    .ok_or(DbError::NotFound)?;
    let nonce: Vec<u8> = row.get("nonce");
    let attachment = row_to_attachment(&row)?;
    Ok((attachment, nonce))
}

pub async fn purge_attachment(pool: &PgPool, id: AttachmentId) -> Result<Option<String>, DbError> {
    // Returns the storage_key so the caller can also delete from the BlobStore.
    let row = sqlx::query("DELETE FROM attachment WHERE id = $1 RETURNING storage_key")
        .bind(id.as_uuid())
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.get::<String, _>("storage_key")))
}

/// Every attachment under a vault (across all its letters), oldest first.
/// Used by the move-region flow to relocate each blob in turn.
pub async fn list_attachments_for_vault(
    pool: &PgPool,
    vault_id: VaultId,
) -> Result<Vec<Attachment>, DbError> {
    let rows = sqlx::query(
        "SELECT a.id, a.letter_id, a.original_filename, a.original_mime, a.original_size,
                a.transformed_mime, a.transformed_size, a.sha256, a.transformer_notes,
                a.storage_key, a.sealed_at
           FROM attachment a
           JOIN letter l ON l.id = a.letter_id
          WHERE l.vault_id = $1
          ORDER BY a.sealed_at",
    )
    .bind(vault_id.as_uuid())
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_attachment).collect()
}

/// Repoint an attachment at a relocated blob. The new key encodes the
/// destination region. Idempotent: re-running with the same key is a no-op.
pub async fn update_attachment_storage_key(
    pool: &PgPool,
    id: AttachmentId,
    new_storage_key: &str,
) -> Result<(), DbError> {
    sqlx::query("UPDATE attachment SET storage_key = $1 WHERE id = $2")
        .bind(new_storage_key)
        .bind(id.as_uuid())
        .execute(pool)
        .await?;
    Ok(())
}

fn row_to_attachment(row: &PgRow) -> Result<Attachment, DbError> {
    let sha256: Vec<u8> = row.get("sha256");
    Ok(Attachment {
        id: AttachmentId(row.get("id")),
        letter_id: LetterId(row.get("letter_id")),
        original_filename: row.get("original_filename"),
        original_mime: row.get("original_mime"),
        original_size: row.get("original_size"),
        transformed_mime: row.get("transformed_mime"),
        transformed_size: row.get("transformed_size"),
        sha256_hex: hex::encode(&sha256),
        transformer_notes: row
            .try_get("transformer_notes")
            .unwrap_or(serde_json::Value::Null),
        storage_key: row.get("storage_key"),
        sealed_at: row.get("sealed_at"),
    })
}

// ----------------------------------------------------------------------------
// Attachment release claims
// ----------------------------------------------------------------------------

pub async fn issue_attachment_claim(
    pool: &PgPool,
    release_event_id: ReleaseEventId,
    attachment_id: AttachmentId,
    recipient_email: &str,
    token_hash: &[u8],
    ttl_days: i64,
) -> Result<(), DbError> {
    let expires_at = Utc::now() + ChronoDuration::days(ttl_days);
    sqlx::query(
        "INSERT INTO release_attachment_claim
            (token_hash, release_event_id, attachment_id, recipient_email, expires_at)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(token_hash)
    .bind(release_event_id.as_uuid())
    .bind(attachment_id.as_uuid())
    .bind(recipient_email)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Returns (attachment_id, recipient_email, is_drill) if the claim is valid.
pub async fn consume_attachment_claim(
    pool: &PgPool,
    token_hash: &[u8],
) -> Result<Option<(AttachmentId, String, bool)>, DbError> {
    let row = sqlx::query(
        "WITH consumed AS (
            UPDATE release_attachment_claim
               SET claimed_at = now()
             WHERE token_hash = $1
               AND claimed_at IS NULL
               AND expires_at > now()
           RETURNING attachment_id, recipient_email, release_event_id
         )
         SELECT c.attachment_id, c.recipient_email, re.is_drill
           FROM consumed c
           JOIN release_event re ON re.id = c.release_event_id",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| {
        let id: Uuid = r.get("attachment_id");
        (
            AttachmentId(id),
            r.get("recipient_email"),
            r.get("is_drill"),
        )
    }))
}

// ----------------------------------------------------------------------------
// Principal lookup by email (passkey sign-in + recovery)
// ----------------------------------------------------------------------------

pub async fn fetch_principal_by_email(
    pool: &PgPool,
    email: &str,
) -> Result<Option<Principal>, DbError> {
    let row = sqlx::query(
        "SELECT id, display_name, primary_email, created_at FROM principal WHERE primary_email = $1",
    )
    .bind(email)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| row_to_principal(&r)))
}

// WebAuthn — challenges
// ----------------------------------------------------------------------------

pub async fn create_webauthn_challenge(
    pool: &PgPool,
    principal_id: Option<Uuid>,
    state_json: &str,
    ceremony: &str,
) -> Result<Uuid, DbError> {
    let row = sqlx::query(
        "INSERT INTO webauthn_challenge (principal_id, state_json, ceremony)
         VALUES ($1, $2, $3)
         RETURNING id",
    )
    .bind(principal_id)
    .bind(state_json)
    .bind(ceremony)
    .fetch_one(pool)
    .await?;
    Ok(row.get("id"))
}

/// Atomically consume a challenge (fetch + delete). Returns None if expired or absent.
/// Returns `(state_json, principal_id)`.
pub async fn consume_webauthn_challenge(
    pool: &PgPool,
    challenge_id: Uuid,
    ceremony: &str,
) -> Result<Option<(String, Option<Uuid>)>, DbError> {
    let row = sqlx::query(
        "DELETE FROM webauthn_challenge
          WHERE id = $1
            AND ceremony = $2
            AND expires_at > now()
         RETURNING state_json, principal_id",
    )
    .bind(challenge_id)
    .bind(ceremony)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| {
        let state_json: String = r.get("state_json");
        let principal_id: Option<Uuid> = r.try_get("principal_id").ok().flatten();
        (state_json, principal_id)
    }))
}

pub async fn expire_webauthn_challenges(pool: &PgPool) -> Result<u64, DbError> {
    let result = sqlx::query("DELETE FROM webauthn_challenge WHERE expires_at <= now()")
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

// ----------------------------------------------------------------------------
// WebAuthn — passkeys
// ----------------------------------------------------------------------------

pub struct PasskeyRow {
    pub id: Uuid,
    pub principal_id: Uuid,
    pub credential_id: Vec<u8>,
    pub passkey_json: String,
    pub backed_up: bool,
    pub transports: Vec<String>,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

fn row_to_passkey(row: &PgRow) -> PasskeyRow {
    PasskeyRow {
        id: row.get("id"),
        principal_id: row.get("principal_id"),
        credential_id: row.get("credential_id"),
        passkey_json: row.get("passkey_json"),
        backed_up: row.get("backed_up"),
        transports: row.get("transports"),
        name: row.get("name"),
        created_at: row.get("created_at"),
        last_used_at: row.try_get("last_used_at").ok().flatten(),
    }
}

pub async fn store_passkey(
    pool: &PgPool,
    principal_id: Uuid,
    credential_id: &[u8],
    passkey_json: &str,
    backed_up: bool,
    transports: &[String],
    name: &str,
) -> Result<Uuid, DbError> {
    let row = sqlx::query(
        "INSERT INTO principal_passkey
            (principal_id, credential_id, passkey_json, backed_up, transports, name)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id",
    )
    .bind(principal_id)
    .bind(credential_id)
    .bind(passkey_json)
    .bind(backed_up)
    .bind(transports)
    .bind(name)
    .fetch_one(pool)
    .await?;
    Ok(row.get("id"))
}

pub async fn list_passkeys_for_principal(
    pool: &PgPool,
    principal_id: Uuid,
) -> Result<Vec<PasskeyRow>, DbError> {
    let rows = sqlx::query(
        "SELECT id, principal_id, credential_id, passkey_json, backed_up,
                transports, name, created_at, last_used_at
           FROM principal_passkey
          WHERE principal_id = $1
          ORDER BY created_at ASC",
    )
    .bind(principal_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_passkey).collect())
}

pub async fn get_passkey_by_credential_id(
    pool: &PgPool,
    credential_id: &[u8],
) -> Result<Option<PasskeyRow>, DbError> {
    let row = sqlx::query(
        "SELECT id, principal_id, credential_id, passkey_json, backed_up,
                transports, name, created_at, last_used_at
           FROM principal_passkey
          WHERE credential_id = $1",
    )
    .bind(credential_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_passkey))
}

pub async fn get_passkeys_for_authentication(
    pool: &PgPool,
    principal_id: Uuid,
) -> Result<Vec<PasskeyRow>, DbError> {
    let rows = sqlx::query(
        "SELECT id, principal_id, credential_id, passkey_json, backed_up,
                transports, name, created_at, last_used_at
           FROM principal_passkey
          WHERE principal_id = $1
          ORDER BY created_at ASC",
    )
    .bind(principal_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_passkey).collect())
}

pub async fn update_passkey_after_auth(
    pool: &PgPool,
    passkey_id: Uuid,
    passkey_json: &str,
    last_used_at: DateTime<Utc>,
) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE principal_passkey
            SET passkey_json = $2, last_used_at = $3
          WHERE id = $1",
    )
    .bind(passkey_id)
    .bind(passkey_json)
    .bind(last_used_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Returns true if the row existed and was deleted.
pub async fn delete_passkey(
    pool: &PgPool,
    passkey_id: Uuid,
    principal_id: Uuid,
) -> Result<bool, DbError> {
    let result = sqlx::query("DELETE FROM principal_passkey WHERE id = $1 AND principal_id = $2")
        .bind(passkey_id)
        .bind(principal_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

// ----------------------------------------------------------------------------
// WebAuthn — ZK envelopes
// ----------------------------------------------------------------------------

pub struct ZkEnvelopeRow {
    pub id: Uuid,
    pub vault_id: Uuid,
    pub passkey_id: Uuid,
    pub ciphertext: Vec<u8>,
    pub nonce: Vec<u8>,
    pub created_at: DateTime<Utc>,
}

pub async fn upsert_zk_envelope(
    pool: &PgPool,
    vault_id: Uuid,
    passkey_id: Uuid,
    ciphertext: &[u8],
    nonce: &[u8],
) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO zk_key_envelope (vault_id, passkey_id, ciphertext, nonce)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (vault_id, passkey_id)
         DO UPDATE SET ciphertext = EXCLUDED.ciphertext, nonce = EXCLUDED.nonce",
    )
    .bind(vault_id)
    .bind(passkey_id)
    .bind(ciphertext)
    .bind(nonce)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_zk_envelopes_for_vault(
    pool: &PgPool,
    vault_id: Uuid,
) -> Result<Vec<ZkEnvelopeRow>, DbError> {
    let rows = sqlx::query(
        "SELECT id, vault_id, passkey_id, ciphertext, nonce, created_at
           FROM zk_key_envelope
          WHERE vault_id = $1",
    )
    .bind(vault_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| ZkEnvelopeRow {
            id: r.get("id"),
            vault_id: r.get("vault_id"),
            passkey_id: r.get("passkey_id"),
            ciphertext: r.get("ciphertext"),
            nonce: r.get("nonce"),
            created_at: r.get("created_at"),
        })
        .collect())
}

pub struct ZkRecoveryEnvelopeRow {
    pub id: Uuid,
    pub vault_id: Uuid,
    pub principal_id: Uuid,
    pub code_salt: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub nonce: Vec<u8>,
    pub created_at: DateTime<Utc>,
}

pub async fn upsert_zk_recovery_envelope(
    pool: &PgPool,
    vault_id: Uuid,
    principal_id: Uuid,
    code_salt: &[u8],
    ciphertext: &[u8],
    nonce: &[u8],
) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO zk_recovery_envelope
            (vault_id, principal_id, code_salt, ciphertext, nonce)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (vault_id, principal_id)
         DO UPDATE SET
            code_salt = EXCLUDED.code_salt,
            ciphertext = EXCLUDED.ciphertext,
            nonce = EXCLUDED.nonce",
    )
    .bind(vault_id)
    .bind(principal_id)
    .bind(code_salt)
    .bind(ciphertext)
    .bind(nonce)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_zk_recovery_envelope(
    pool: &PgPool,
    vault_id: Uuid,
) -> Result<Option<ZkRecoveryEnvelopeRow>, DbError> {
    let row = sqlx::query(
        "SELECT id, vault_id, principal_id, code_salt, ciphertext, nonce, created_at
           FROM zk_recovery_envelope
          WHERE vault_id = $1",
    )
    .bind(vault_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| ZkRecoveryEnvelopeRow {
        id: r.get("id"),
        vault_id: r.get("vault_id"),
        principal_id: r.get("principal_id"),
        code_salt: r.get("code_salt"),
        ciphertext: r.get("ciphertext"),
        nonce: r.get("nonce"),
        created_at: r.get("created_at"),
    }))
}

// ----------------------------------------------------------------------------
// WebAuthn — recovery codes
// ----------------------------------------------------------------------------

pub struct RecoveryRow {
    pub principal_id: Uuid,
    pub code_salt: Vec<u8>,
    pub code_verifier: Vec<u8>,
    pub created_at: DateTime<Utc>,
}

pub async fn upsert_principal_recovery(
    pool: &PgPool,
    principal_id: Uuid,
    code_salt: &[u8],
    code_verifier: &[u8],
) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO principal_recovery (principal_id, code_salt, code_verifier)
         VALUES ($1, $2, $3)
         ON CONFLICT (principal_id)
         DO UPDATE SET
            code_salt = EXCLUDED.code_salt,
            code_verifier = EXCLUDED.code_verifier,
            created_at = now()",
    )
    .bind(principal_id)
    .bind(code_salt)
    .bind(code_verifier)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_principal_recovery(
    pool: &PgPool,
    principal_id: Uuid,
) -> Result<Option<RecoveryRow>, DbError> {
    let row = sqlx::query(
        "SELECT principal_id, code_salt, code_verifier, created_at
           FROM principal_recovery
          WHERE principal_id = $1",
    )
    .bind(principal_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| RecoveryRow {
        principal_id: r.get("principal_id"),
        code_salt: r.get("code_salt"),
        code_verifier: r.get("code_verifier"),
        created_at: r.get("created_at"),
    }))
}

pub async fn get_principal_recovery_by_email(
    pool: &PgPool,
    email: &str,
) -> Result<Option<(Uuid, RecoveryRow)>, DbError> {
    let row = sqlx::query(
        "SELECT pr.principal_id, pr.code_salt, pr.code_verifier, pr.created_at
           FROM principal_recovery pr
           JOIN principal p ON p.id = pr.principal_id
          WHERE p.primary_email = $1",
    )
    .bind(email)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| {
        let pid: Uuid = r.get("principal_id");
        let recovery = RecoveryRow {
            principal_id: pid,
            code_salt: r.get("code_salt"),
            code_verifier: r.get("code_verifier"),
            created_at: r.get("created_at"),
        };
        (pid, recovery)
    }))
}

/// Count how many passkeys a principal has.
pub async fn count_passkeys_for_principal(
    pool: &PgPool,
    principal_id: Uuid,
) -> Result<i64, DbError> {
    let row = sqlx::query("SELECT COUNT(*) FROM principal_passkey WHERE principal_id = $1")
        .bind(principal_id)
        .fetch_one(pool)
        .await?;
    Ok(row.get::<i64, _>(0))
}

/// Count how many ZK vaults a principal has (for warning on last-passkey delete).
pub async fn count_zk_vaults_for_principal(
    pool: &PgPool,
    principal_id: Uuid,
) -> Result<i64, DbError> {
    let row = sqlx::query(
        "SELECT COUNT(*) FROM vault WHERE principal_id = $1 AND tier = 'ZERO_KNOWLEDGE'",
    )
    .bind(principal_id)
    .fetch_one(pool)
    .await?;
    Ok(row.get::<i64, _>(0))
}

// ----------------------------------------------------------------------------
// Principal-level Recovery Key (PRK)
// ----------------------------------------------------------------------------

pub struct PrincipalRecoveryKeyRow {
    pub principal_id: Uuid,
    pub code_salt: Vec<u8>,
    pub prk_code_ct: Vec<u8>,
    pub prk_code_nonce: Vec<u8>,
    pub prk_prf_ct: Vec<u8>,
    pub prk_prf_nonce: Vec<u8>,
}

/// Store the principal's Recovery Key wraps. Insert-once (`DO NOTHING`): the PRK
/// is fixed for the account's life, so re-running never orphans existing Vault
/// recovery envelopes.
pub async fn upsert_principal_recovery_key(
    pool: &PgPool,
    principal_id: Uuid,
    code_salt: &[u8],
    prk_code_ct: &[u8],
    prk_code_nonce: &[u8],
    prk_prf_ct: &[u8],
    prk_prf_nonce: &[u8],
) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO principal_recovery_key
            (principal_id, code_salt, prk_code_ct, prk_code_nonce, prk_prf_ct, prk_prf_nonce)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (principal_id) DO NOTHING",
    )
    .bind(principal_id)
    .bind(code_salt)
    .bind(prk_code_ct)
    .bind(prk_code_nonce)
    .bind(prk_prf_ct)
    .bind(prk_prf_nonce)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_principal_recovery_key(
    pool: &PgPool,
    principal_id: Uuid,
) -> Result<Option<PrincipalRecoveryKeyRow>, DbError> {
    let row = sqlx::query(
        "SELECT principal_id, code_salt, prk_code_ct, prk_code_nonce, prk_prf_ct, prk_prf_nonce
           FROM principal_recovery_key WHERE principal_id = $1",
    )
    .bind(principal_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| PrincipalRecoveryKeyRow {
        principal_id: r.get("principal_id"),
        code_salt: r.get("code_salt"),
        prk_code_ct: r.get("prk_code_ct"),
        prk_code_nonce: r.get("prk_code_nonce"),
        prk_prf_ct: r.get("prk_prf_ct"),
        prk_prf_nonce: r.get("prk_prf_nonce"),
    }))
}

pub async fn get_principal_recovery_key_by_email(
    pool: &PgPool,
    email: &str,
) -> Result<Option<PrincipalRecoveryKeyRow>, DbError> {
    let row = sqlx::query(
        "SELECT prk.principal_id, prk.code_salt, prk.prk_code_ct, prk.prk_code_nonce,
                prk.prk_prf_ct, prk.prk_prf_nonce
           FROM principal_recovery_key prk
           JOIN principal p ON p.id = prk.principal_id
          WHERE p.primary_email = $1",
    )
    .bind(email)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| PrincipalRecoveryKeyRow {
        principal_id: r.get("principal_id"),
        code_salt: r.get("code_salt"),
        prk_code_ct: r.get("prk_code_ct"),
        prk_code_nonce: r.get("prk_code_nonce"),
        prk_prf_ct: r.get("prk_prf_ct"),
        prk_prf_nonce: r.get("prk_prf_nonce"),
    }))
}

/// Upsert a Private Vault's DEK-wrapped-under-PRK recovery envelope.
pub async fn upsert_vault_prk_envelope(
    pool: &PgPool,
    vault_id: Uuid,
    principal_id: Uuid,
    ciphertext: &[u8],
    nonce: &[u8],
) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO vault_prk_envelope (vault_id, principal_id, ciphertext, nonce)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (vault_id) DO UPDATE SET
            ciphertext = EXCLUDED.ciphertext,
            nonce = EXCLUDED.nonce,
            created_at = now()",
    )
    .bind(vault_id)
    .bind(principal_id)
    .bind(ciphertext)
    .bind(nonce)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_vault_prk_envelope(
    pool: &PgPool,
    vault_id: Uuid,
) -> Result<Option<(Vec<u8>, Vec<u8>)>, DbError> {
    let row = sqlx::query("SELECT ciphertext, nonce FROM vault_prk_envelope WHERE vault_id = $1")
        .bind(vault_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| (r.get("ciphertext"), r.get("nonce"))))
}

// ----------------------------------------------------------------------------
// Private Letter heir envelope (body sealed under a recipient passphrase)
// ----------------------------------------------------------------------------

pub struct HeirEnvelopeRow {
    pub ciphertext: Vec<u8>,
    pub nonce: Vec<u8>,
    /// 'manual' | 'split' | 'operator'.
    pub mode: String,
    /// PBKDF2 salt — manual mode only.
    pub salt: Option<Vec<u8>>,
    /// Operator's half (split) or the whole key (operator) — released at unseal.
    pub release_secret: Option<Vec<u8>>,
}

pub async fn upsert_zk_letter_heir_envelope(
    pool: &PgPool,
    letter_id: LetterId,
    ciphertext: &[u8],
    nonce: &[u8],
    mode: &str,
    salt: Option<&[u8]>,
    release_secret: Option<&[u8]>,
) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO zk_letter_heir_envelope
            (letter_id, ciphertext, nonce, mode, salt, release_secret)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (letter_id) DO UPDATE SET
            ciphertext = EXCLUDED.ciphertext,
            nonce = EXCLUDED.nonce,
            mode = EXCLUDED.mode,
            salt = EXCLUDED.salt,
            release_secret = EXCLUDED.release_secret,
            created_at = now()",
    )
    .bind(letter_id.as_uuid())
    .bind(ciphertext)
    .bind(nonce)
    .bind(mode)
    .bind(salt)
    .bind(release_secret)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_zk_letter_heir_envelope(
    pool: &PgPool,
    letter_id: LetterId,
) -> Result<Option<HeirEnvelopeRow>, DbError> {
    let row = sqlx::query(
        "SELECT ciphertext, nonce, mode, salt, release_secret
           FROM zk_letter_heir_envelope WHERE letter_id = $1",
    )
    .bind(letter_id.as_uuid())
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| HeirEnvelopeRow {
        ciphertext: r.get("ciphertext"),
        nonce: r.get("nonce"),
        mode: r.get("mode"),
        salt: r.try_get("salt").ok().flatten(),
        release_secret: r.try_get("release_secret").ok().flatten(),
    }))
}
