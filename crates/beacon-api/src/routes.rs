//! HTTP route handlers.
//!
//! Per-route handlers are kept in one file for the MVP. As surface area
//! grows, the natural next step is to split per resource: `routes/auth.rs`,
//! `routes/vaults.rs`, etc.

use axum::{
    body::Body,
    extract::{Multipart, Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::Response,
    Json,
};
use beacon_core::{
    can_transition, plan_features, public_catalog, AttachmentId, BuddyId, BuddyResponse,
    CoStewardId, LetterId, PlanId, PrincipalId, ReleaseReason, Tier, VaultId, VaultState,
};
use beacon_db as db;
use chrono::Utc;
use crypto_stub::{hash_bytes, hash_passphrase, hash_token, random_salt, random_token, Sealed};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    auth::{current_co_steward, current_principal},
    errors::{ApiError, ApiResult},
    metrics::Metrics,
    notifications::tx,
    state::AppState,
};

// ----------------------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------------------

#[derive(Serialize)]
pub struct VaultView {
    pub id: VaultId,
    pub name: String,
    pub tier: String,
    pub state: String,
    pub cooling_off_seconds: i32,
    /// AWS region code where this vault's attachment blobs live.
    pub storage_region: String,
    /// Human-readable location, e.g. "Frankfurt, Germany".
    pub storage_region_label: String,
    pub last_attestation_at: String,
    pub cooling_off_started_at: Option<String>,
    pub released_at: Option<String>,
}

fn vault_view(v: &beacon_core::Vault) -> VaultView {
    VaultView {
        id: v.id,
        name: v.name.clone(),
        tier: v.tier.as_db_str().to_string(),
        state: v.state.as_db_str().to_string(),
        cooling_off_seconds: v.cooling_off_seconds,
        storage_region: v.storage_region.as_aws_str().to_string(),
        storage_region_label: v.storage_region.display_name().to_string(),
        last_attestation_at: v.last_attestation_at.to_rfc3339(),
        cooling_off_started_at: v.cooling_off_started_at.map(|t| t.to_rfc3339()),
        released_at: v.released_at.map(|t| t.to_rfc3339()),
    }
}

#[derive(Serialize)]
pub struct LetterMetaView {
    pub id: LetterId,
    pub title: String,
    pub recipient_email: String,
    pub sealed_at: String,
    pub scheduled_release_at: Option<String>,
}

#[derive(Serialize)]
pub struct SubscriptionView {
    pub state: String,
    pub plan_id: String,
    pub started_at: String,
    pub trial_end_at: Option<String>,
    pub current_period_end: Option<String>,
    pub canceled_at: Option<String>,
    pub retention_until: Option<String>,
}

fn subscription_view(s: &beacon_core::Subscription) -> SubscriptionView {
    SubscriptionView {
        state: s.state.as_db_str().to_string(),
        plan_id: s.plan_id.as_db_str().to_string(),
        started_at: s.started_at.to_rfc3339(),
        trial_end_at: s.trial_end_at.map(|t| t.to_rfc3339()),
        current_period_end: s.current_period_end.map(|t| t.to_rfc3339()),
        canceled_at: s.canceled_at.map(|t| t.to_rfc3339()),
        retention_until: s.retention_until.map(|t| t.to_rfc3339()),
    }
}

#[derive(Serialize)]
pub struct BuddyView {
    pub id: BuddyId,
    pub display_name: Option<String>,
    pub email: String,
    pub phone: Option<String>,
    pub prompt_cadence_days: i32,
    pub last_response: Option<String>,
    pub last_response_at: Option<String>,
    pub confirmed: bool,
    pub revoked: bool,
}

fn buddy_view(b: &beacon_core::Buddy) -> BuddyView {
    BuddyView {
        id: b.id,
        display_name: b.display_name.clone(),
        email: b.email.clone(),
        phone: b.phone.clone(),
        prompt_cadence_days: b.prompt_cadence_days,
        last_response: b.last_response.map(|r| r.as_db_str().to_string()),
        last_response_at: b.last_response_at.map(|t| t.to_rfc3339()),
        confirmed: b.confirmed_at.is_some(),
        revoked: b.revoked_at.is_some(),
    }
}

// ----------------------------------------------------------------------------
// Signup
// ----------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct SignupReq {
    pub email: String,
    #[serde(default)]
    pub display_name: Option<String>,
    pub plan: Option<String>,
    #[serde(default)]
    pub tos_accepted: Option<bool>,
}

#[derive(Serialize)]
pub struct SignupResp {
    pub principal_id: PrincipalId,
    pub session_token: String,
    pub subscription_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trial_end_at: Option<String>,
    /// Dev-only: returned in debug builds so the CLI can complete the flow
    /// without an SMTP server. Never serialised in release.
    #[serde(
        rename = "magic_token_DEV_ONLY",
        skip_serializing_if = "Option::is_none"
    )]
    pub magic_token_dev_only: Option<String>,
}

pub async fn signup(
    State(state): State<AppState>,
    Json(body): Json<SignupReq>,
) -> ApiResult<Json<SignupResp>> {
    if !body.email.contains('@') {
        return Err(ApiError::BadRequest("email looks invalid".into()));
    }
    if body.tos_accepted != Some(true) {
        return Err(ApiError::BadRequest(
            "You must accept the Terms of Service to create an account.".into(),
        ));
    }
    let _ = body.display_name; // recorded by the principal-update endpoint later

    let (principal, created) = db::upsert_principal_by_email(&state.pool, &body.email).await?;
    if !created {
        // The account already exists: never hand a session to an unverified
        // caller (knowing the email is not proof of ownership). Email a one-time
        // sign-in link to the real owner instead, and return a conflict.
        let magic = random_token();
        db::create_magic_link(&state.pool, principal.id, &hash_token(&magic), 15).await?;
        let link = format!(
            "{}/app/auth/verify?token={magic}",
            state.config.public_base_url.trim_end_matches('/'),
        );
        tx::magic_link(
            state.notifications.as_ref(),
            &principal.primary_email,
            &link,
        )
        .await;
        return Err(ApiError::Conflict(
            "An account with this email already exists. We've emailed you a sign-in link.".into(),
        ));
    }
    let _ = db::set_tos_accepted(&state.pool, principal.id).await;

    let sub = match db::fetch_subscription(&state.pool, principal.id).await {
        Ok(s) => s,
        Err(db::DbError::NotFound) => {
            let plan = parse_plan_param(body.plan.as_deref())?;
            db::create_subscription(&state.pool, principal.id, plan, state.config.trial_days)
                .await?
        }
        Err(e) => return Err(e.into()),
    };

    let magic = random_token();
    db::create_magic_link(&state.pool, principal.id, &hash_token(&magic), 15).await?;
    let session = random_token();
    db::create_session(&state.pool, principal.id, &hash_token(&session), 24).await?;

    let _ = db::append_transparency_entry(
        &state.pool,
        "PRINCIPAL_SIGNED_UP",
        &hash_bytes(principal.primary_email.as_bytes()),
        json!({ "email_hash": hex::encode(hash_bytes(principal.primary_email.as_bytes())) }),
        Some(principal.id),
        None,
    )
    .await;

    if let Some(end) = sub.trial_end_at {
        tx::welcome(
            state.notifications.as_ref(),
            &principal.primary_email,
            &end.to_rfc3339(),
        )
        .await;
    }

    Metrics::inc(&state.metrics.signups_total);

    Ok(Json(SignupResp {
        principal_id: principal.id,
        session_token: session,
        subscription_state: sub.state.as_db_str().to_string(),
        trial_end_at: sub.trial_end_at.map(|t| t.to_rfc3339()),
        magic_token_dev_only: if cfg!(debug_assertions) {
            Some(magic)
        } else {
            None
        },
    }))
}

/// The self-hosted edition has a single plan; the sign-up `plan` parameter is
/// accepted for API compatibility but ignored.
pub fn parse_plan_param(_s: Option<&str>) -> ApiResult<PlanId> {
    Ok(PlanId::SelfHosted)
}

// ----------------------------------------------------------------------------
// Plan catalog (public — no auth required)
// ----------------------------------------------------------------------------

#[derive(Serialize)]
pub struct PlanView {
    pub plan_id: String,
    pub storage_bytes: u64,
    pub retention_days: i64,
    pub scheduled_horizon_days: i64,
    pub max_vaults: u32,
    pub max_letters_per_vault: u32,
    pub max_trustees: u32,
    pub allowed_signals: Vec<String>,
    pub multi_region: bool,
    pub notes: Vec<String>,
}

fn plan_view(f: beacon_core::PlanFeatures) -> PlanView {
    PlanView {
        plan_id: f.plan_id.as_db_str().to_string(),
        storage_bytes: f.storage_bytes,
        retention_days: f.retention_days,
        scheduled_horizon_days: f.scheduled_horizon_days,
        max_vaults: f.max_vaults,
        max_letters_per_vault: f.max_letters_per_vault,
        max_trustees: f.max_trustees,
        allowed_signals: f
            .allowed_signals
            .iter()
            .map(|s| s.as_db_str().to_string())
            .collect(),
        multi_region: f.multi_region,
        notes: f.notes.iter().map(|s| (*s).to_string()).collect(),
    }
}

pub async fn list_plans() -> Json<Vec<PlanView>> {
    Json(public_catalog().into_iter().map(plan_view).collect())
}

// ----------------------------------------------------------------------------
// Subscription
// ----------------------------------------------------------------------------

pub async fn get_subscription(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<SubscriptionView>> {
    let pid = current_principal(&state, &headers).await?;
    let sub = db::fetch_subscription(&state.pool, pid).await?;
    Ok(Json(subscription_view(&sub)))
}

// ----------------------------------------------------------------------------
// Usage (storage meter, vault/letter counts vs plan quotas)
// ----------------------------------------------------------------------------

#[derive(Serialize)]
pub struct UsageView {
    pub plan_id: String,
    pub storage_used_bytes: i64,
    pub storage_quota_bytes: u64,
    pub storage_pct: f32,
    pub vaults_used: i64,
    pub vaults_quota: u32,
    pub letters_quota_per_vault: u32,
    pub co_stewards_quota: u32,
    pub retention_days: i64,
    pub scheduled_horizon_days: i64,
}

pub async fn get_usage(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<UsageView>> {
    let pid = current_principal(&state, &headers).await?;
    let sub = db::fetch_subscription(&state.pool, pid).await?;
    let plan = plan_features(sub.plan_id);

    let storage_used = db::storage_used(&state.pool, pid).await?;
    let vault_count = db::vault_count(&state.pool, pid).await?;
    let effective = sub.effective_storage_bytes();

    let storage_pct = if effective == 0 {
        0.0
    } else {
        (storage_used as f64 / effective as f64 * 100.0).min(999.0) as f32
    };

    Ok(Json(UsageView {
        plan_id: sub.plan_id.as_db_str().to_string(),
        storage_used_bytes: storage_used,
        storage_quota_bytes: effective,
        storage_pct,
        vaults_used: vault_count,
        vaults_quota: plan.max_vaults,
        letters_quota_per_vault: plan.max_letters_per_vault,
        co_stewards_quota: plan.max_co_stewards,
        retention_days: plan.retention_days,
        scheduled_horizon_days: plan.scheduled_horizon_days,
    }))
}

// ----------------------------------------------------------------------------
// Vaults
// ----------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct CreateVaultReq {
    pub name: String,
    pub tier: Option<String>,
    pub cooling_off_seconds: Option<i32>,
    /// AWS region code for this vault's attachment storage (e.g.
    /// `eu-central-1`). Omitted or the default region needs no entitlement;
    /// any other region requires a multi-region plan (Estate+ / Legacy).
    #[serde(default)]
    pub storage_region: Option<String>,
}

/// Resolve a requested region string against the principal's plan. `None` →
/// the default region (always allowed). A non-default region requires the
/// plan's `multi_region` entitlement.
fn resolve_storage_region(
    requested: Option<&str>,
    plan: &beacon_core::PlanFeatures,
) -> Result<beacon_core::StorageRegion, ApiError> {
    let region = match requested {
        None => return Ok(beacon_core::StorageRegion::default()),
        Some(s) => beacon_core::StorageRegion::from_aws_str(s)
            .map_err(|e| ApiError::BadRequest(e.to_string()))?,
    };
    if region != beacon_core::StorageRegion::default() && !plan.multi_region {
        return Err(ApiError::Forbidden(format!(
            "Storing a Vault in {} requires multi-region storage, which is not enabled on this deployment (default region: {}).",
            region.display_name(),
            beacon_core::StorageRegion::default().display_name(),
        )));
    }
    Ok(region)
}

pub async fn create_vault(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateVaultReq>,
) -> ApiResult<Json<VaultView>> {
    let pid = current_principal(&state, &headers).await?;

    let tier = match body.tier.as_deref() {
        Some("ZERO_KNOWLEDGE") | Some("zero_knowledge") => Tier::ZeroKnowledge,
        _ => Tier::HonestOperator,
    };

    let sub = db::fetch_subscription(&state.pool, pid).await?;
    if !sub.state.allows_authoring() {
        return Err(ApiError::Forbidden(format!(
            "Subscription state {:?} does not permit authoring",
            sub.state
        )));
    }

    // Plan-quota check: max_vaults.
    let plan = plan_features(sub.plan_id);
    let current_vaults = db::vault_count(&state.pool, pid).await? as u32;
    if current_vaults >= plan.max_vaults {
        return Err(ApiError::Forbidden(format!(
            "Vault limit reached ({} of {} allowed). Adjust MAX_VAULTS in your deployment configuration to increase it.",
            current_vaults, plan.max_vaults
        )));
    }

    let region = resolve_storage_region(body.storage_region.as_deref(), &plan)?;

    let cool = body
        .cooling_off_seconds
        .unwrap_or(state.config.default_cooling_off_seconds);
    let vault = db::create_vault(&state.pool, pid, &body.name, tier, cool, region).await?;

    // Default signal subscriptions: Heartbeat + Buddy attestation + Apple
    // iCloud Shortcut. The principal can add Microsoft, Calendar, Email later
    // via the signal-subscriptions endpoint.
    for source in [
        beacon_core::SignalSource::Heartbeat,
        beacon_core::SignalSource::BuddyAttestation,
        beacon_core::SignalSource::AppleIcloudShortcut,
    ] {
        let _ = db::upsert_signal_subscription(
            &state.pool,
            vault.id,
            source,
            source.default_weight(),
            true,
        )
        .await;
    }

    let _ = db::append_transparency_entry(
        &state.pool,
        "VAULT_CREATED",
        &hash_bytes(vault.id.as_uuid().as_bytes()),
        json!({ "vault_id": vault.id, "tier": tier.as_db_str(), "storage_region": vault.storage_region.as_aws_str() }),
        Some(pid),
        Some(vault.id),
    )
    .await;

    Metrics::inc(&state.metrics.vaults_created_total);
    Ok(Json(vault_view(&vault)))
}

pub async fn list_vaults(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<VaultView>>> {
    let pid = current_principal(&state, &headers).await?;
    let vaults = db::list_vaults(&state.pool, pid).await?;
    Ok(Json(vaults.iter().map(vault_view).collect()))
}

pub async fn get_vault(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(vault_id): Path<Uuid>,
) -> ApiResult<Json<VaultView>> {
    let pid = current_principal(&state, &headers).await?;
    let vault = db::fetch_vault(&state.pool, VaultId(vault_id)).await?;
    if vault.principal_id != pid {
        return Err(ApiError::NotFound);
    }
    Ok(Json(vault_view(&vault)))
}

#[derive(Deserialize)]
pub struct MoveVaultRegionReq {
    pub storage_region: String,
}

/// Move a Vault's sealed attachment blobs to another storage region (Estate+ /
/// Legacy). Per attachment: read from its current region, write a fresh object
/// in the target region, repoint the DB row at the new key, then delete the old
/// object. Crash-safe and idempotent — the DB is repointed before the old blob
/// is deleted (so a crash never strands a row at a deleted blob), and a re-run
/// skips attachments already encoding the target region.
pub async fn move_vault_region(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(vault_id): Path<Uuid>,
    Json(body): Json<MoveVaultRegionReq>,
) -> ApiResult<Json<VaultView>> {
    let pid = current_principal(&state, &headers).await?;
    let vault = db::fetch_vault(&state.pool, VaultId(vault_id)).await?;
    if vault.principal_id != pid {
        return Err(ApiError::NotFound);
    }

    let sub = db::fetch_subscription(&state.pool, pid).await?;
    let plan = plan_features(sub.plan_id);
    if !plan.multi_region {
        return Err(ApiError::Forbidden(
            "Moving a Vault's storage region is not enabled for this deployment.".into(),
        ));
    }

    let target = beacon_core::StorageRegion::from_aws_str(&body.storage_region)
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    if target == vault.storage_region {
        return Ok(Json(vault_view(&vault)));
    }

    let attachments = db::list_attachments_for_vault(&state.pool, vault.id).await?;
    for att in &attachments {
        let (cur_region, _id) = blob_store::parse_key(&att.storage_key);
        if cur_region == Some(target.as_aws_str()) {
            continue; // already moved on a prior (crashed) run
        }
        let bytes = state
            .blob_store
            .get(&att.storage_key)
            .await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("blob get: {e}")))?;
        let new_key = state
            .blob_store
            .put(target.as_aws_str(), &bytes)
            .await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("blob put: {e}")))?;
        db::update_attachment_storage_key(&state.pool, att.id, &new_key).await?;
        // Best-effort delete of the now-orphaned source object. A leftover is
        // reclaimed by a later move or a sweep — never a correctness problem.
        let _ = state.blob_store.delete(&att.storage_key).await;
    }

    let updated = db::set_vault_storage_region(&state.pool, vault.id, target).await?;

    let _ = db::append_transparency_entry(
        &state.pool,
        "VAULT_REGION_MOVED",
        &hash_bytes(vault.id.as_uuid().as_bytes()),
        json!({
            "vault_id": vault.id,
            "from": vault.storage_region.as_aws_str(),
            "to": target.as_aws_str(),
            "attachments": attachments.len(),
        }),
        Some(pid),
        Some(vault.id),
    )
    .await;

    Ok(Json(vault_view(&updated)))
}

// ----------------------------------------------------------------------------
// Letters
// ----------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct SealLetterReq {
    pub title: String,
    pub recipient_email: String,
    pub body: String,
    /// Optional drill body — separate plaintext rehearsed during Drills.
    #[serde(default)]
    pub drill_body: Option<String>,
    /// Optional scheduled release date (RFC 3339).
    #[serde(default)]
    pub scheduled_release_at: Option<chrono::DateTime<Utc>>,
    /// Server-side Letter classification. One of MESSAGE, CREDENTIAL_BUNDLE,
    /// FILE_ARCHIVE, ACTION, WILL_LOCATOR, VIDEO_MESSAGE, AUDIO_MESSAGE.
    /// Defaults to MESSAGE.
    #[serde(default)]
    pub kind: Option<String>,
    /// Template identifier (e.g. "will_locator", "letter_to_children").
    /// Informational only; the recipient never sees it.
    #[serde(default)]
    pub category: Option<String>,
    /// SIGNAL_OR_SCHEDULED (default), SCHEDULED_ONLY (time-capsule), or
    /// SIGNAL_ONLY. SCHEDULED_ONLY requires scheduled_release_at.
    #[serde(default)]
    pub release_mode: Option<String>,
}

/// The set of Letter kinds the schema accepts. Mirrors the CHECK constraint
/// in migration 0006_letter_categories.sql.
const VALID_LETTER_KINDS: &[&str] = &[
    "MESSAGE",
    "CREDENTIAL_BUNDLE",
    "FILE_ARCHIVE",
    "ACTION",
    "WILL_LOCATOR",
    "VIDEO_MESSAGE",
    "AUDIO_MESSAGE",
    "MEDIA_PLAYLIST",
];

fn validate_letter_kind(kind: Option<&str>) -> ApiResult<()> {
    match kind {
        Some(k) if !VALID_LETTER_KINDS.contains(&k) => Err(ApiError::BadRequest(format!(
            "unknown letter kind '{k}'; allowed: {}",
            VALID_LETTER_KINDS.join(", ")
        ))),
        _ => Ok(()),
    }
}

const VALID_RELEASE_MODES: &[&str] = &[
    "SIGNAL_OR_SCHEDULED",
    "SCHEDULED_ONLY",
    "SIGNAL_ONLY",
    "EVENT_ON_DEMAND",
];

fn validate_release_mode(
    mode: Option<&str>,
    scheduled_release_at: Option<&chrono::DateTime<Utc>>,
) -> ApiResult<()> {
    let mode = mode.unwrap_or("SIGNAL_OR_SCHEDULED");
    if !VALID_RELEASE_MODES.contains(&mode) {
        return Err(ApiError::BadRequest(format!(
            "unknown release_mode '{mode}'; allowed: {}",
            VALID_RELEASE_MODES.join(", ")
        )));
    }
    if mode == "SCHEDULED_ONLY" && scheduled_release_at.is_none() {
        return Err(ApiError::BadRequest(
            "SCHEDULED_ONLY requires scheduled_release_at — a time-capsule \
             needs a date to open on."
                .into(),
        ));
    }
    Ok(())
}

pub async fn seal_letter(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(vault_id): Path<Uuid>,
    Json(body): Json<SealLetterReq>,
) -> ApiResult<Json<LetterMetaView>> {
    let pid = current_principal(&state, &headers).await?;
    let vault = db::fetch_vault(&state.pool, VaultId(vault_id)).await?;
    if vault.principal_id != pid {
        return Err(ApiError::NotFound);
    }

    validate_letter_kind(body.kind.as_deref())?;
    validate_release_mode(
        body.release_mode.as_deref(),
        body.scheduled_release_at.as_ref(),
    )?;

    let sub = db::fetch_subscription(&state.pool, pid).await?;
    if !sub.state.allows_authoring() {
        return Err(ApiError::Forbidden(format!(
            "Subscription state {:?} does not permit authoring",
            sub.state
        )));
    }
    if vault.state != VaultState::Active {
        return Err(ApiError::Conflict(format!(
            "Vault state {:?} does not permit sealing new Letters",
            vault.state
        )));
    }

    // Plan checks: letter count + scheduled-release horizon.
    let plan = plan_features(sub.plan_id);
    let current_letters = db::letter_count(&state.pool, vault.id).await? as u32;
    if current_letters >= plan.max_letters_per_vault {
        return Err(ApiError::Forbidden(format!(
            "Letter limit reached ({} of {} allowed). Adjust MAX_LETTERS_PER_VAULT in your deployment configuration.",
            current_letters, plan.max_letters_per_vault
        )));
    }

    if let Some(t) = body.scheduled_release_at {
        if t < Utc::now() {
            return Err(ApiError::BadRequest(
                "scheduled_release_at must be in the future".into(),
            ));
        }
        let horizon = Utc::now() + chrono::Duration::days(plan.scheduled_horizon_days);
        if t > horizon {
            return Err(ApiError::BadRequest(format!(
                "Scheduled release is {} days in the future, which exceeds the {} day horizon for this deployment.",
                (t - Utc::now()).num_days(),
                plan.scheduled_horizon_days
            )));
        }
    }

    let sealed = crypto_stub::seal(state.kms.as_ref(), body.body.as_bytes()).await?;
    let drill_sealed = if let Some(dbody) = body.drill_body.as_deref() {
        Some(crypto_stub::seal(state.kms.as_ref(), dbody.as_bytes()).await?)
    } else {
        None
    };

    let drill_ct_ref = drill_sealed.as_ref().map(|s| s.ciphertext.as_slice());
    let drill_nonce_ref = drill_sealed.as_ref().map(|s| s.nonce.as_slice());

    let letter = db::seal_letter(
        &state.pool,
        db::LetterSealInput {
            vault_id: vault.id,
            title: &body.title,
            recipient_email: &body.recipient_email,
            ciphertext: &sealed.ciphertext,
            nonce: &sealed.nonce,
            drill_ciphertext: drill_ct_ref,
            drill_nonce: drill_nonce_ref,
            scheduled_release_at: body.scheduled_release_at,
            kind: body.kind.as_deref(),
            category: body.category.as_deref(),
            release_mode: body.release_mode.as_deref(),
        },
    )
    .await?;

    let _ = db::append_transparency_entry(
        &state.pool,
        "LETTER_SEALED",
        &hash_bytes(letter.id.as_uuid().as_bytes()),
        json!({
            "letter_id": letter.id,
            "vault_id": vault.id,
            "title_hash": hex::encode(hash_bytes(body.title.as_bytes())),
            "has_drill_payload": drill_sealed.is_some(),
            "scheduled_release_at": body.scheduled_release_at,
        }),
        Some(pid),
        Some(vault.id),
    )
    .await;

    Metrics::inc(&state.metrics.letters_sealed_total);
    Ok(Json(LetterMetaView {
        id: letter.id,
        title: letter.title,
        recipient_email: letter.recipient_email,
        sealed_at: letter.sealed_at.to_rfc3339(),
        scheduled_release_at: body.scheduled_release_at.map(|t| t.to_rfc3339()),
    }))
}

pub async fn list_letters(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(vault_id): Path<Uuid>,
) -> ApiResult<Json<Vec<LetterMetaView>>> {
    let pid = current_principal(&state, &headers).await?;
    let vault = db::fetch_vault(&state.pool, VaultId(vault_id)).await?;
    if vault.principal_id != pid {
        return Err(ApiError::NotFound);
    }
    let letters = db::list_letters(&state.pool, vault.id).await?;
    Ok(Json(
        letters
            .into_iter()
            .map(|l| LetterMetaView {
                id: l.id,
                title: l.title,
                recipient_email: l.recipient_email,
                sealed_at: l.sealed_at.to_rfc3339(),
                scheduled_release_at: None,
            })
            .collect(),
    ))
}

// ----------------------------------------------------------------------------
// Heartbeat
// ----------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct HeartbeatReq {
    pub via: Option<String>,
}

#[derive(Serialize)]
pub struct HeartbeatResp {
    pub received_at: String,
}

pub async fn post_heartbeat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<HeartbeatReq>,
) -> ApiResult<Json<HeartbeatResp>> {
    let pid = current_principal(&state, &headers).await?;
    let via = body.via.as_deref().unwrap_or("CLI").to_uppercase();
    if !["CLI", "WEB", "EMAIL", "SMS", "PUSH"].contains(&via.as_str()) {
        return Err(ApiError::BadRequest("invalid via value".into()));
    }
    let ts = db::record_heartbeat(&state.pool, pid, &via).await?;
    Metrics::inc(&state.metrics.heartbeats_total);

    // Also record signal observations for every active Vault so the
    // aggregator can see a fresh Heartbeat. Contribution 0 means "alive".
    for vault in db::list_vaults(&state.pool, pid).await? {
        let _ = db::record_signal(
            &state.pool,
            vault.id,
            beacon_core::SignalSource::Heartbeat,
            0.0,
            json!({ "kind": "fresh", "via": via }),
        )
        .await;
    }

    Ok(Json(HeartbeatResp {
        received_at: ts.to_rfc3339(),
    }))
}

// ----------------------------------------------------------------------------
// Signal subscriptions
// ----------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct UpsertSignalSubReq {
    pub source: String,
    pub weight: Option<f32>,
    pub enabled: Option<bool>,
}

#[derive(Serialize)]
pub struct SignalSubView {
    pub source: String,
    pub weight: f32,
    pub enabled: bool,
}

pub async fn upsert_signal_subscription(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(vault_id): Path<Uuid>,
    Json(body): Json<UpsertSignalSubReq>,
) -> ApiResult<Json<SignalSubView>> {
    let pid = current_principal(&state, &headers).await?;
    let vault = db::fetch_vault(&state.pool, VaultId(vault_id)).await?;
    if vault.principal_id != pid {
        return Err(ApiError::NotFound);
    }
    let source = beacon_core::SignalSource::from_db_str(&body.source.to_uppercase())
        .map_err(|e| ApiError::BadRequest(format!("unknown source: {}", e)))?;
    let weight = body.weight.unwrap_or_else(|| source.default_weight());
    let enabled = body.enabled.unwrap_or(true);
    let sub =
        db::upsert_signal_subscription(&state.pool, vault.id, source, weight, enabled).await?;
    Ok(Json(SignalSubView {
        source: sub.source.as_db_str().to_string(),
        weight: sub.weight,
        enabled: sub.enabled,
    }))
}

pub async fn list_signal_subscriptions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(vault_id): Path<Uuid>,
) -> ApiResult<Json<Vec<SignalSubView>>> {
    let pid = current_principal(&state, &headers).await?;
    let vault = db::fetch_vault(&state.pool, VaultId(vault_id)).await?;
    if vault.principal_id != pid {
        return Err(ApiError::NotFound);
    }
    let subs = db::list_signal_subscriptions(&state.pool, vault.id).await?;
    Ok(Json(
        subs.into_iter()
            .map(|s| SignalSubView {
                source: s.source.as_db_str().to_string(),
                weight: s.weight,
                enabled: s.enabled,
            })
            .collect(),
    ))
}

// ----------------------------------------------------------------------------
// Apple iCloud Shortcut signal
// ----------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct AppleEnrolReq {
    /// Stable per-installation identifier the Shortcut generates.
    pub installation_id: String,
}

#[derive(Serialize)]
pub struct AppleEnrolResp {
    pub installation_id: String,
    /// Base64-encoded HMAC secret. The Shortcut stores this and signs each
    /// ping with HMAC-SHA256.
    pub secret_base64: String,
    pub ping_url: String,
}

pub async fn apple_icloud_enrol(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<AppleEnrolReq>,
) -> ApiResult<Json<AppleEnrolResp>> {
    let pid = current_principal(&state, &headers).await?;
    if body.installation_id.is_empty() || body.installation_id.len() > 128 {
        return Err(ApiError::BadRequest(
            "installation_id length invalid".into(),
        ));
    }

    // Generate a 32-byte secret. The Shortcut will store this and HMAC each ping.
    let mut secret = [0u8; 32];
    use rand::RngCore;
    rand::rng().fill_bytes(&mut secret);

    db::create_apple_shortcut_sub(&state.pool, pid, &body.installation_id, &secret).await?;

    use base64::{engine::general_purpose::STANDARD, Engine as _};
    Ok(Json(AppleEnrolResp {
        installation_id: body.installation_id,
        secret_base64: STANDARD.encode(secret),
        ping_url: format!(
            "{}/v1/signals/apple-icloud/ping",
            state.config.public_base_url.trim_end_matches('/')
        ),
    }))
}

#[derive(Deserialize)]
pub struct ApplePingReq {
    pub installation_id: String,
    pub timestamp: i64,
    /// Base64-encoded HMAC-SHA256 of `{installation_id}|{timestamp}`.
    pub signature: String,
}

pub async fn apple_icloud_ping(
    State(state): State<AppState>,
    Json(body): Json<ApplePingReq>,
) -> ApiResult<Json<serde_json::Value>> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    // Reject anything older than 5 minutes or > 5 minutes in the future.
    let now = Utc::now().timestamp();
    let drift = (now - body.timestamp).abs();
    if drift > 300 {
        return Err(ApiError::BadRequest(format!(
            "timestamp drift {drift}s outside acceptable window"
        )));
    }

    let (principal_id, secret) = db::fetch_apple_shortcut_sub(&state.pool, &body.installation_id)
        .await?
        .ok_or(ApiError::NotFound)?;

    let provided = STANDARD
        .decode(body.signature.as_bytes())
        .map_err(|_| ApiError::BadRequest("signature must be base64".into()))?;
    let expected = hmac_sha256(
        &secret,
        &format!("{}|{}", body.installation_id, body.timestamp),
    );
    if !constant_time_eq(&provided, &expected) {
        return Err(ApiError::Unauthorised);
    }

    db::touch_apple_shortcut_sub(&state.pool, &body.installation_id).await?;
    Metrics::inc(&state.metrics.apple_pings_total);

    // Record a signal observation against every active Vault for this principal.
    // contribution 0.0 = fresh proof of life.
    for vault in db::list_vaults(&state.pool, principal_id).await? {
        let _ = db::record_signal(
            &state.pool,
            vault.id,
            beacon_core::SignalSource::AppleIcloudShortcut,
            0.0,
            json!({ "installation_id": body.installation_id, "timestamp": body.timestamp }),
        )
        .await;
    }

    Ok(Json(json!({ "received_at": Utc::now().to_rfc3339() })))
}

fn hmac_sha256(key: &[u8], msg: &str) -> Vec<u8> {
    // Paschal-MVP HMAC-SHA256 via the sha2 crate. Production code uses the
    // dedicated `hmac` crate; for the skeleton we inline RFC 2104 to avoid
    // an extra dep.
    const BLOCK_SIZE: usize = 64;
    let mut k = if key.len() > BLOCK_SIZE {
        Sha256::digest(key).to_vec()
    } else {
        key.to_vec()
    };
    k.resize(BLOCK_SIZE, 0);
    let ipad: Vec<u8> = k.iter().map(|b| b ^ 0x36).collect();
    let opad: Vec<u8> = k.iter().map(|b| b ^ 0x5c).collect();

    let mut inner = Sha256::new();
    inner.update(&ipad);
    inner.update(msg.as_bytes());
    let inner_hash = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(&opad);
    outer.update(inner_hash);
    outer.finalize().to_vec()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

// ----------------------------------------------------------------------------
// Buddies
// ----------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct InviteBuddyReq {
    pub email: String,
    pub display_name: Option<String>,
    pub phone: Option<String>,
    pub prompt_cadence_days: Option<i32>,
}

#[derive(Serialize)]
pub struct InviteBuddyResp {
    pub buddy: BuddyView,
    /// In production the confirmation link is emailed. Returned here for the
    /// MVP demo flow.
    #[serde(rename = "confirmation_token_DEV_ONLY")]
    pub confirmation_token_dev_only: String,
}

pub async fn invite_buddy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<InviteBuddyReq>,
) -> ApiResult<Json<InviteBuddyResp>> {
    let pid = current_principal(&state, &headers).await?;
    if !body.email.contains('@') {
        return Err(ApiError::BadRequest("email looks invalid".into()));
    }
    let token = random_token();
    let cadence = body.prompt_cadence_days.unwrap_or(90).clamp(7, 365);
    let buddy = db::invite_buddy(
        &state.pool,
        db::BuddyInviteInput {
            principal_id: pid,
            display_name: body.display_name.as_deref(),
            email: &body.email,
            phone: body.phone.as_deref(),
            confirmation_token_hash: &hash_token(&token),
            prompt_cadence_days: cadence,
        },
    )
    .await?;

    let principal = db::fetch_principal(&state.pool, pid).await?;
    let confirmation_url = format!(
        "{}/buddies/confirm?token={}",
        state.config.public_base_url.trim_end_matches('/'),
        token
    );
    tx::buddy_invite(
        state.notifications.as_ref(),
        &body.email,
        &principal.primary_email,
        &confirmation_url,
    )
    .await;

    Ok(Json(InviteBuddyResp {
        buddy: buddy_view(&buddy),
        confirmation_token_dev_only: token,
    }))
}

pub async fn list_buddies(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<BuddyView>>> {
    let pid = current_principal(&state, &headers).await?;
    let buddies = db::list_buddies(&state.pool, pid).await?;
    Ok(Json(buddies.iter().map(buddy_view).collect()))
}

pub async fn revoke_buddy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(buddy_id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let pid = current_principal(&state, &headers).await?;
    let buddy = db::fetch_buddy(&state.pool, BuddyId(buddy_id)).await?;
    if buddy.principal_id != pid {
        return Err(ApiError::NotFound);
    }
    db::revoke_buddy(&state.pool, buddy.id).await?;
    Ok(Json(json!({ "revoked": true })))
}

#[derive(Deserialize)]
pub struct ConfirmBuddyReq {
    pub token: String,
}

pub async fn confirm_buddy(
    State(state): State<AppState>,
    Json(body): Json<ConfirmBuddyReq>,
) -> ApiResult<Json<BuddyView>> {
    let hash = hash_token(&body.token);
    let buddy = db::confirm_buddy(&state.pool, &hash)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(buddy_view(&buddy)))
}

#[derive(Deserialize)]
pub struct BuddyResponseReq {
    /// "WELL", "WORRIED", or "UNABLE_TO_REACH"
    pub response: String,
    /// Token that proves this responder is the named Buddy. For the MVP
    /// we use the confirmation flow's identity — Buddy id alone is enough
    /// while we have not yet built a Buddy auth flow; production gates
    /// this with a passphrase or email-magic link.
    pub buddy_token: Option<String>,
}

pub async fn respond_buddy(
    State(state): State<AppState>,
    Path(buddy_id): Path<Uuid>,
    Json(body): Json<BuddyResponseReq>,
) -> ApiResult<Json<BuddyView>> {
    let response = BuddyResponse::from_db_str(&body.response.to_uppercase())
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    // MVP: respond is open to anyone holding the Buddy ID (which is sent only
    // to the confirmed Buddy via email). Production: require a per-Buddy
    // session token derived from confirmation.
    let _ = body.buddy_token;
    let buddy = db::record_buddy_response(&state.pool, BuddyId(buddy_id), response).await?;
    Metrics::inc(&state.metrics.buddy_responses_total);

    // Record a signal observation on every active Vault for the principal.
    if response.release_contribution() > 0.0 {
        for vault in db::list_vaults(&state.pool, buddy.principal_id).await? {
            let _ = db::record_signal(
                &state.pool,
                vault.id,
                beacon_core::SignalSource::BuddyAttestation,
                response.release_contribution(),
                json!({ "buddy_id": buddy.id, "response": response.as_db_str() }),
            )
            .await;
        }
    } else {
        // WELL response — bump last_attestation_at to push back the clock.
        let _ = sqlx::query("UPDATE vault SET last_attestation_at = now() WHERE principal_id = $1")
            .bind(buddy.principal_id.as_uuid())
            .execute(&state.pool)
            .await;
    }

    Ok(Json(buddy_view(&buddy)))
}

// ----------------------------------------------------------------------------
// Release: force-release, cancel, drill
// ----------------------------------------------------------------------------

#[derive(Serialize)]
pub struct ForceReleaseResp {
    pub release_event_id: beacon_core::ReleaseEventId,
    pub cooling_off_started_at: String,
    pub cooling_off_ends_at: String,
}

pub async fn force_release(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(vault_id): Path<Uuid>,
) -> ApiResult<Json<ForceReleaseResp>> {
    let pid = current_principal(&state, &headers).await?;
    let vault = db::fetch_vault(&state.pool, VaultId(vault_id)).await?;
    if vault.principal_id != pid {
        return Err(ApiError::NotFound);
    }

    let sub = db::fetch_subscription(&state.pool, pid).await?;
    if !sub.state.allows_release() {
        return Err(ApiError::Forbidden(format!(
            "Subscription state {:?} does not permit release",
            sub.state
        )));
    }

    if vault.state != VaultState::Active {
        return Err(ApiError::Conflict(format!(
            "Vault state is {:?}; force-release requires ACTIVE",
            vault.state
        )));
    }
    if !can_transition(vault.state, VaultState::CoolingOff) {
        return Err(ApiError::Conflict("cannot enter cooling-off".into()));
    }

    let resp = crate::scheduler::begin_release(
        state.clone(),
        vault.id,
        ReleaseReason::ManualPrincipalRelease,
        false,
    )
    .await?;
    Ok(Json(resp))
}

pub async fn cancel_release(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(vault_id): Path<Uuid>,
) -> ApiResult<Json<VaultView>> {
    let pid = current_principal(&state, &headers).await?;
    let vault = db::fetch_vault(&state.pool, VaultId(vault_id)).await?;
    if vault.principal_id != pid {
        return Err(ApiError::NotFound);
    }
    if vault.state != VaultState::CoolingOff {
        return Err(ApiError::Conflict(format!(
            "Vault is in state {:?}; nothing to cancel",
            vault.state
        )));
    }
    let updated =
        db::set_vault_state(&state.pool, vault.id, VaultState::Active, None, None).await?;
    // Close the open release event so the durable poll loop won't reconsider it.
    if let Ok(Some(open)) = db::fetch_open_release(&state.pool, vault.id).await {
        let _ = db::mark_release_cancelled(&state.pool, open.id).await;
    }
    let _ = db::append_transparency_entry(
        &state.pool,
        "COOLING_OFF_CANCELLED",
        &hash_bytes(vault.id.as_uuid().as_bytes()),
        json!({ "vault_id": vault.id }),
        Some(pid),
        Some(vault.id),
    )
    .await;

    let principal = db::fetch_principal(&state.pool, pid).await?;
    tx::cooling_off_cancelled(state.notifications.as_ref(), &principal.primary_email).await;
    Ok(Json(vault_view(&updated)))
}

pub async fn run_drill(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(vault_id): Path<Uuid>,
) -> ApiResult<Json<ForceReleaseResp>> {
    let pid = current_principal(&state, &headers).await?;
    let vault = db::fetch_vault(&state.pool, VaultId(vault_id)).await?;
    if vault.principal_id != pid {
        return Err(ApiError::NotFound);
    }
    let sub = db::fetch_subscription(&state.pool, pid).await?;
    if !sub.state.allows_release() {
        return Err(ApiError::Forbidden(format!(
            "Subscription state {:?} does not permit drills",
            sub.state
        )));
    }
    if vault.state != VaultState::Active {
        return Err(ApiError::Conflict(format!(
            "Drill requires Vault state ACTIVE; currently {:?}",
            vault.state
        )));
    }

    // A drill uses a short cooling-off so it completes quickly.
    let resp = crate::scheduler::begin_release_with_cool(
        state.clone(),
        vault.id,
        ReleaseReason::Drill,
        true,
        3, // 3-second cooling-off for drills in the MVP
    )
    .await?;
    Ok(Json(resp))
}

// ----------------------------------------------------------------------------
// Recipient claim
// ----------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ClaimQuery {
    pub token: String,
}

#[derive(Serialize)]
pub struct ClaimView {
    pub title: String,
    pub recipient_email: String,
    pub body: String,
    pub is_drill: bool,
    /// True for a Private (Zero-Knowledge) Letter: `body` is empty and the
    /// recipient decrypts the heir envelope below.
    pub zk: bool,
    /// 'manual' | 'split' | 'operator' — how the recipient obtains the key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heir_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heir_ciphertext: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heir_nonce: Option<String>,
    /// manual mode: PBKDF2 salt (hex).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heir_salt: Option<String>,
    /// split: operator's half of the key (base64url). operator: the whole key.
    /// Released only here, at claim time (after the Vault unseals).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heir_release_secret: Option<String>,
}

// ----------------------------------------------------------------------------
// Multipart upload with content transformation
// ----------------------------------------------------------------------------

#[derive(Serialize)]
pub struct LetterWithAttachmentsView {
    pub id: LetterId,
    pub title: String,
    pub recipient_email: String,
    pub sealed_at: String,
    pub attachments: Vec<AttachmentView>,
}

#[derive(Serialize)]
pub struct AttachmentView {
    pub id: AttachmentId,
    pub original_filename: String,
    pub original_mime: String,
    pub original_size: i64,
    pub transformed_mime: String,
    pub transformed_size: i64,
    pub sha256_hex: String,
    pub transformer_notes: serde_json::Value,
}

fn attachment_view(a: &beacon_core::Attachment) -> AttachmentView {
    AttachmentView {
        id: a.id,
        original_filename: a.original_filename.clone(),
        original_mime: a.original_mime.clone(),
        original_size: a.original_size,
        transformed_mime: a.transformed_mime.clone(),
        transformed_size: a.transformed_size,
        sha256_hex: a.sha256_hex.clone(),
        transformer_notes: a.transformer_notes.clone(),
    }
}

/// `POST /v1/vaults/:id/letters/multipart`
///
/// multipart/form-data fields:
///   * title (text, required)
///   * recipient_email (text, required)
///   * body (text, optional — if absent, the Letter is attachment-only)
///   * file (binary, repeatable — each becomes an Attachment)
///
/// Each `file` part is run through the Transformer pipeline before sealing.
/// Files larger than `MAX_UPLOAD_BYTES` are rejected.
pub async fn seal_letter_multipart(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(vault_id): Path<Uuid>,
    mut multipart: Multipart,
) -> ApiResult<Json<LetterWithAttachmentsView>> {
    let pid = current_principal(&state, &headers).await?;
    let vault = db::fetch_vault(&state.pool, VaultId(vault_id)).await?;
    if vault.principal_id != pid {
        return Err(ApiError::NotFound);
    }
    let sub = db::fetch_subscription(&state.pool, pid).await?;
    if !sub.state.allows_authoring() {
        return Err(ApiError::Forbidden(format!(
            "Subscription state {:?} does not permit authoring",
            sub.state
        )));
    }
    if vault.state != VaultState::Active {
        return Err(ApiError::Conflict(format!(
            "Vault state {:?} does not permit sealing new Letters",
            vault.state
        )));
    }

    let mut title: Option<String> = None;
    let mut recipient_email: Option<String> = None;
    let mut body: Option<String> = None;
    let mut kind: Option<String> = None;
    let mut category: Option<String> = None;
    let mut release_mode: Option<String> = None;
    let mut scheduled_release_at: Option<chrono::DateTime<Utc>> = None;
    let mut files: Vec<(String, String, Vec<u8>)> = Vec::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("multipart: {e}")))?
    {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "title" => {
                title = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| ApiError::BadRequest(format!("title: {e}")))?,
                );
            }
            "recipient_email" => {
                recipient_email = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| ApiError::BadRequest(format!("recipient_email: {e}")))?,
                );
            }
            "body" => {
                body = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| ApiError::BadRequest(format!("body: {e}")))?,
                );
            }
            "kind" => {
                kind = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| ApiError::BadRequest(format!("kind: {e}")))?,
                );
            }
            "category" => {
                category = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| ApiError::BadRequest(format!("category: {e}")))?,
                );
            }
            "release_mode" => {
                release_mode = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| ApiError::BadRequest(format!("release_mode: {e}")))?,
                );
            }
            "scheduled_release_at" => {
                let s = field
                    .text()
                    .await
                    .map_err(|e| ApiError::BadRequest(format!("scheduled_release_at: {e}")))?;
                scheduled_release_at = Some(
                    chrono::DateTime::parse_from_rfc3339(&s)
                        .map_err(|e| ApiError::BadRequest(format!("scheduled_release_at: {e}")))?
                        .with_timezone(&Utc),
                );
            }
            "file" => {
                let filename = field
                    .file_name()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "unnamed".into());
                let mime = field
                    .content_type()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "application/octet-stream".into());
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| ApiError::BadRequest(format!("file body: {e}")))?
                    .to_vec();
                if bytes.len() > state.config.max_upload_bytes {
                    return Err(ApiError::BadRequest(format!(
                        "file '{filename}' exceeds MAX_UPLOAD_BYTES ({} > {})",
                        bytes.len(),
                        state.config.max_upload_bytes
                    )));
                }
                files.push((filename, mime, bytes));
            }
            other => {
                tracing::warn!(field = %other, "ignoring unknown multipart field");
            }
        }
    }

    let title = title.ok_or_else(|| ApiError::BadRequest("title is required".into()))?;
    let recipient_email = recipient_email
        .ok_or_else(|| ApiError::BadRequest("recipient_email is required".into()))?;
    let body = body.unwrap_or_default();
    if body.is_empty() && files.is_empty() {
        return Err(ApiError::BadRequest(
            "either body or at least one file must be supplied".into(),
        ));
    }

    // Plan-quota: letter count + total storage. Storage uses transformed
    // sizes which is what gets sealed; we approximate via the original
    // sizes since transformers can shrink (text/image) but rarely grow.
    let plan = plan_features(sub.plan_id);
    let current_letters = db::letter_count(&state.pool, vault.id).await? as u32;
    if current_letters >= plan.max_letters_per_vault {
        return Err(ApiError::Forbidden(format!(
            "Letter limit reached ({} of {} allowed). Adjust MAX_LETTERS_PER_VAULT in your deployment configuration.",
            current_letters, plan.max_letters_per_vault
        )));
    }
    let upload_total: u64 = files.iter().map(|(_, _, b)| b.len() as u64).sum();
    let current_storage = db::storage_used(&state.pool, pid).await? as u64;
    let effective_quota = sub.effective_storage_bytes();
    if current_storage + upload_total > effective_quota {
        return Err(ApiError::Forbidden(format!(
            "Storage quota exceeded: {} bytes used of {} allowed; this upload would add {} bytes.",
            current_storage, effective_quota, upload_total
        )));
    }

    validate_letter_kind(kind.as_deref())?;
    validate_release_mode(release_mode.as_deref(), scheduled_release_at.as_ref())?;

    // Enforce the scheduled-release horizon on multipart sealing too.
    if let Some(t) = scheduled_release_at {
        if t < Utc::now() {
            return Err(ApiError::BadRequest(
                "scheduled_release_at must be in the future".into(),
            ));
        }
        let horizon = Utc::now() + chrono::Duration::days(plan.scheduled_horizon_days);
        if t > horizon {
            return Err(ApiError::BadRequest(format!(
                "Scheduled release is {} days in the future, which exceeds the {} day horizon for this deployment.",
                (t - Utc::now()).num_days(),
                plan.scheduled_horizon_days
            )));
        }
    }

    // Seal the body.
    let sealed = crypto_stub::seal(state.kms.as_ref(), body.as_bytes()).await?;
    let letter = db::seal_letter(
        &state.pool,
        db::LetterSealInput {
            vault_id: vault.id,
            title: &title,
            recipient_email: &recipient_email,
            ciphertext: &sealed.ciphertext,
            nonce: &sealed.nonce,
            drill_ciphertext: None,
            drill_nonce: None,
            scheduled_release_at,
            kind: kind.as_deref(),
            category: category.as_deref(),
            release_mode: release_mode.as_deref(),
        },
    )
    .await?;

    // Process each file: transform → seal → store ciphertext in BlobStore →
    // persist attachment row.
    let mut attachment_views = Vec::with_capacity(files.len());
    for (filename, original_mime, bytes) in files {
        let original_size = bytes.len() as i64;

        let transformed = paschal_transform::apply(
            state.transformers.as_ref(),
            &bytes,
            &original_mime,
            state.config.max_upload_bytes,
        )
        .await
        .map_err(|e| ApiError::BadRequest(format!("transform failed for '{filename}': {e}")))?;

        let attachment_sealed = crypto_stub::seal(state.kms.as_ref(), &transformed.bytes).await?;
        let storage_key = state
            .blob_store
            .put(
                vault.storage_region.as_aws_str(),
                &attachment_sealed.ciphertext,
            )
            .await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("blob put: {e}")))?;

        let sha256_bytes = hex::decode(&transformed.sha256_hex)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("sha256 decode: {e}")))?;

        let notes_json = serde_json::json!({
            "notes": transformed.notes,
            "transformed_sha256_hex": transformed.sha256_hex,
        });

        let attachment = db::create_attachment(
            &state.pool,
            db::AttachmentInput {
                letter_id: letter.id,
                original_filename: &filename,
                original_mime: &original_mime,
                original_size,
                transformed_mime: &transformed.mime,
                transformed_size: transformed.bytes.len() as i64,
                sha256: &sha256_bytes,
                transformer_notes: notes_json,
                storage_key: &storage_key,
                nonce: &attachment_sealed.nonce,
            },
        )
        .await?;

        let _ = db::append_transparency_entry(
            &state.pool,
            "ATTACHMENT_SEALED",
            &hash_bytes(attachment.id.as_uuid().as_bytes()),
            json!({
                "attachment_id": attachment.id,
                "letter_id": letter.id,
                "original_mime": original_mime,
                "transformed_mime": attachment.transformed_mime,
                "transformer_notes": attachment.transformer_notes,
            }),
            Some(pid),
            Some(vault.id),
        )
        .await;

        Metrics::inc(&state.metrics.attachments_sealed_total);
        Metrics::add(
            &state.metrics.attachments_bytes_total,
            attachment.transformed_size as u64,
        );

        attachment_views.push(attachment_view(&attachment));
    }
    Metrics::inc(&state.metrics.letters_sealed_total);

    let _ = db::append_transparency_entry(
        &state.pool,
        "LETTER_SEALED",
        &hash_bytes(letter.id.as_uuid().as_bytes()),
        json!({
            "letter_id": letter.id,
            "vault_id": vault.id,
            "title_hash": hex::encode(hash_bytes(title.as_bytes())),
            "attachment_count": attachment_views.len(),
        }),
        Some(pid),
        Some(vault.id),
    )
    .await;

    Ok(Json(LetterWithAttachmentsView {
        id: letter.id,
        title: letter.title,
        recipient_email: letter.recipient_email,
        sealed_at: letter.sealed_at.to_rfc3339(),
        attachments: attachment_views,
    }))
}

/// `GET /v1/releases/claim/attachment?token=<token>`
///
/// Returns the decrypted attachment bytes with the original filename in
/// Content-Disposition. Single-use — like `claim_release`, the token is
/// consumed atomically.
pub async fn claim_attachment(
    State(state): State<AppState>,
    Query(q): Query<ClaimQuery>,
) -> Result<Response, ApiError> {
    let hash = hash_token(&q.token);
    let (attachment_id, _rcpt, _is_drill) = db::consume_attachment_claim(&state.pool, &hash)
        .await?
        .ok_or(ApiError::NotFound)?;

    let (attachment, nonce) = db::fetch_attachment(&state.pool, attachment_id).await?;
    let ciphertext = state
        .blob_store
        .get(&attachment.storage_key)
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("blob get: {e}")))?;

    if nonce.len() != 12 {
        return Err(ApiError::Internal(anyhow::anyhow!("nonce length")));
    }
    let mut nbuf = [0u8; 12];
    nbuf.copy_from_slice(&nonce);
    let sealed = Sealed {
        nonce: nbuf,
        ciphertext,
    };
    let plaintext = crypto_stub::open(state.kms.as_ref(), &sealed).await?;

    let safe_filename = attachment
        .original_filename
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-' || *c == '_')
        .collect::<String>();
    let disposition = format!("attachment; filename=\"{safe_filename}\"");

    Response::builder()
        .status(200)
        .header(header::CONTENT_TYPE, &attachment.transformed_mime)
        .header(header::CONTENT_DISPOSITION, disposition)
        .header("X-Paschal-Sha256", &attachment.sha256_hex)
        .header(
            "X-Paschal-Transformer-Notes",
            serde_json::to_string(&attachment.transformer_notes).unwrap_or_default(),
        )
        .body(Body::from(plaintext))
        .map_err(|e| ApiError::Internal(anyhow::anyhow!(e)))
}

// ----------------------------------------------------------------------------
// Microsoft Account signal — manual / scripted observation
// ----------------------------------------------------------------------------
//
// Production polls the Microsoft Graph API on a schedule with the
// principal's OAuth token. For the MVP we expose a write-only endpoint that
// records an observation. The principal's own automation (e.g. a Logic
// App, a Power Automate flow, or a scheduled script) can push to it.

#[derive(Deserialize)]
pub struct MicrosoftObserveReq {
    pub last_sign_in_at: chrono::DateTime<Utc>,
}

pub async fn microsoft_observe(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<MicrosoftObserveReq>,
) -> ApiResult<Json<serde_json::Value>> {
    let pid = current_principal(&state, &headers).await?;
    if body.last_sign_in_at > Utc::now() + chrono::Duration::minutes(5) {
        return Err(ApiError::BadRequest(
            "last_sign_in_at is in the future".into(),
        ));
    }
    let elapsed = (Utc::now() - body.last_sign_in_at).num_seconds().max(0);
    let max_gap = 14 * 86_400_i64; // 14 days
    let contribution = ((elapsed as f32) / (max_gap as f32)).clamp(0.0, 1.0);

    let vaults = db::list_vaults(&state.pool, pid).await?;
    let mut affected = 0;
    for v in vaults {
        let _ = db::record_signal(
            &state.pool,
            v.id,
            beacon_core::SignalSource::MicrosoftAccount,
            contribution,
            json!({ "last_sign_in_at": body.last_sign_in_at.to_rfc3339() }),
        )
        .await;
        affected += 1;
    }
    Ok(Json(json!({
        "recorded_at": Utc::now().to_rfc3339(),
        "contribution": contribution,
        "vaults_affected": affected,
    })))
}

// ----------------------------------------------------------------------------
// Account deletion (separate from Subscription cancellation)
// ----------------------------------------------------------------------------

#[derive(Serialize)]
pub struct DeletionRequestedView {
    pub deletion_scheduled_for: String,
    pub cancel_until: String,
    pub explanation: &'static str,
}

pub async fn request_account_deletion(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<(axum::http::StatusCode, Json<DeletionRequestedView>)> {
    let pid = current_principal(&state, &headers).await?;
    let scheduled = db::request_account_deletion(&state.pool, pid, 30).await?;

    let _ = db::append_transparency_entry(
        &state.pool,
        "ACCOUNT_DELETION_REQUESTED",
        &hash_bytes(pid.as_uuid().as_bytes()),
        json!({ "scheduled_for": scheduled.to_rfc3339() }),
        Some(pid),
        None,
    )
    .await;

    let principal = db::fetch_principal(&state.pool, pid).await?;
    state
        .notifications
        .send(crate::notifications::OutboundMessage {
            channel: crate::notifications::Channel::Email,
            to: principal.primary_email.clone(),
            subject: Some("Account deletion requested".into()),
            body: format!(
                "We have received your request to delete your Paschal account. \
                 If you do nothing, all your Vaults, Letters, and Attachments will \
                 be permanently destroyed at {scheduled}. To cancel and keep your \
                 account, sign in and reverse this request within 30 days.",
                scheduled = scheduled.to_rfc3339()
            ),
        })
        .await;

    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(DeletionRequestedView {
            deletion_scheduled_for: scheduled.to_rfc3339(),
            cancel_until: scheduled.to_rfc3339(),
            explanation: "Your account will be deleted at the scheduled time unless you cancel. \
                 This is different from cancelling your Subscription. \
                 See docs/USER_GUIDE.md.",
        }),
    ))
}

pub async fn cancel_account_deletion(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<serde_json::Value>> {
    let pid = current_principal(&state, &headers).await?;
    db::cancel_account_deletion(&state.pool, pid).await?;
    let _ = db::append_transparency_entry(
        &state.pool,
        "ACCOUNT_DELETION_CANCELLED",
        &hash_bytes(pid.as_uuid().as_bytes()),
        json!({}),
        Some(pid),
        None,
    )
    .await;
    Ok(Json(json!({ "cancelled": true })))
}

pub async fn claim_release(
    State(state): State<AppState>,
    Query(q): Query<ClaimQuery>,
) -> ApiResult<Json<ClaimView>> {
    let hash = hash_token(&q.token);
    let (letter_id, _rcpt, is_drill) = db::consume_release_claim(&state.pool, &hash)
        .await?
        .ok_or(ApiError::NotFound)?;
    let letter = db::fetch_letter_full(&state.pool, letter_id).await?;
    let vault = db::fetch_vault(&state.pool, letter.vault_id).await?;

    // Private (Zero-Knowledge) Letters: the operator holds no key. Hand the
    // recipient the heir envelope to open with the passphrase the Principal
    // shared out-of-band (see specs/14 §4). No server-side decryption.
    if vault.tier == Tier::ZeroKnowledge {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine as _;
        let heir = db::get_zk_letter_heir_envelope(&state.pool, letter_id).await?;
        let (heir_mode, heir_ciphertext, heir_nonce, heir_salt, heir_release_secret) = match heir {
            Some(h) => (
                Some(h.mode),
                Some(URL_SAFE_NO_PAD.encode(&h.ciphertext)),
                Some(URL_SAFE_NO_PAD.encode(&h.nonce)),
                h.salt.as_deref().map(hex::encode),
                h.release_secret
                    .as_deref()
                    .map(|s| URL_SAFE_NO_PAD.encode(s)),
            ),
            None => (None, None, None, None, None),
        };
        return Ok(Json(ClaimView {
            title: letter.title,
            recipient_email: letter.recipient_email,
            body: String::new(),
            is_drill,
            zk: true,
            heir_mode,
            heir_ciphertext,
            heir_nonce,
            heir_salt,
            heir_release_secret,
        }));
    }

    // Standard Letters: operator KMS-decrypts (drill payload when applicable).
    let (title, rcpt, ciphertext, nonce) =
        db::fetch_letter_ciphertext(&state.pool, letter_id, is_drill).await?;

    if nonce.len() != 12 {
        return Err(ApiError::Internal(anyhow::anyhow!("nonce length")));
    }
    let mut nbuf = [0u8; 12];
    nbuf.copy_from_slice(&nonce);
    let sealed = Sealed {
        nonce: nbuf,
        ciphertext,
    };
    let plaintext = crypto_stub::open(state.kms.as_ref(), &sealed).await?;
    let body = String::from_utf8(plaintext)
        .unwrap_or_else(|_| "(binary Letter — viewer not yet implemented)".into());

    Ok(Json(ClaimView {
        title,
        recipient_email: rcpt,
        body,
        is_drill,
        zk: false,
        heir_mode: None,
        heir_ciphertext: None,
        heir_nonce: None,
        heir_salt: None,
        heir_release_secret: None,
    }))
}

// ----------------------------------------------------------------------------
// Letter export bundles
// ----------------------------------------------------------------------------
//
// An export bundle is a self-contained, passphrase-encrypted copy of a
// sealed Letter. The Principal can store it on a USB stick, print the
// base64 onto paper, or hand it to a trusted friend. Together with the
// passphrase, the bundle can be unsealed without Paschal ever being
// online.
//
// The bundle is JSON for human-readability; a CLI helper for offline
// unseal lands in paschal-cli at v1.

#[derive(Deserialize)]
pub struct ExportLetterReq {
    /// Passphrase that gates the offline bundle. Must be at least 12
    /// characters. The Principal is responsible for not losing it; there
    /// is no recovery.
    pub passphrase: String,
}

#[derive(Serialize)]
pub struct LetterExportBundle {
    pub format_version: u8,
    pub letter_id: LetterId,
    pub vault_id: VaultId,
    pub title: String,
    pub recipient_email: String,
    pub sealed_at: String,
    pub scheduled_release_at: Option<String>,
    pub kind: String,
    pub category: Option<String>,
    pub release_mode: String,
    /// Base64-URL-safe-no-pad of the passphrase-sealed body ciphertext.
    pub ciphertext_b64: String,
    /// Base64 of the AES-GCM nonce.
    pub nonce_b64: String,
    /// Base64 of the salt used to derive the KEK from the passphrase.
    pub salt_b64: String,
    /// The KDF identifier. The MVP ships "sha256-prefix-v1"; v1 swaps to
    /// "argon2id-v1" per specs/14 §2.
    pub kdf: String,
    /// Note for the future reader — short, human-readable.
    pub note: String,
}

pub async fn export_letter(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((vault_id, letter_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<ExportLetterReq>,
) -> ApiResult<Json<LetterExportBundle>> {
    let pid = current_principal(&state, &headers).await?;

    if body.passphrase.chars().count() < 12 {
        return Err(ApiError::BadRequest(
            "passphrase must be at least 12 characters — there is no recovery if it's lost".into(),
        ));
    }

    let vault = db::fetch_vault(&state.pool, VaultId(vault_id)).await?;
    if vault.principal_id != pid {
        return Err(ApiError::NotFound);
    }

    // Private (ZK) letters are sealed in the browser under the vault DEK — the
    // operator holds no key and cannot re-encrypt them here. Decrypt and save
    // them from the Vault page instead.
    if vault.tier == Tier::ZeroKnowledge {
        return Err(ApiError::Conflict(
            "Private (Zero-Knowledge) Vault letters are encrypted in your browser and \
             can't be exported server-side. Open and save them from the Vault page."
                .into(),
        ));
    }

    let letter = db::fetch_letter_full(&state.pool, LetterId(letter_id)).await?;
    if letter.vault_id != vault.id {
        return Err(ApiError::NotFound);
    }

    // Decrypt with the operator KMS, re-encrypt under the passphrase.
    let kms_sealed = Sealed {
        nonce: {
            let mut n = [0u8; 12];
            if letter.nonce.len() != 12 {
                return Err(ApiError::BadRequest(
                    "letter is missing its ciphertext; was it crypto-erased?".into(),
                ));
            }
            n.copy_from_slice(&letter.nonce);
            n
        },
        ciphertext: letter.ciphertext,
    };
    if kms_sealed.ciphertext.is_empty() {
        return Err(ApiError::Conflict(
            "letter has been crypto-erased and can no longer be exported".into(),
        ));
    }
    let plaintext = crypto_stub::open(state.kms.as_ref(), &kms_sealed).await?;
    let exported = crypto_stub::seal_with_passphrase(&plaintext, &body.passphrase)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("export seal: {e}")))?;

    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;

    let _ = db::append_transparency_entry(
        &state.pool,
        "LETTER_EXPORTED",
        &hash_bytes(letter.id.as_uuid().as_bytes()),
        json!({
            "letter_id": letter.id,
            "vault_id": vault.id,
            "format_version": 1,
        }),
        Some(pid),
        Some(vault.id),
    )
    .await;

    let escaped_title = letter.title.replace('\'', "\\'");
    Ok(Json(LetterExportBundle {
        format_version: 1,
        letter_id: letter.id,
        vault_id: letter.vault_id,
        title: letter.title,
        recipient_email: letter.recipient_email,
        sealed_at: letter.sealed_at.to_rfc3339(),
        scheduled_release_at: letter.scheduled_release_at.map(|t| t.to_rfc3339()),
        kind: letter.kind,
        category: letter.category,
        release_mode: letter.release_mode,
        ciphertext_b64: URL_SAFE_NO_PAD.encode(&exported.ciphertext),
        nonce_b64: URL_SAFE_NO_PAD.encode(exported.nonce),
        salt_b64: URL_SAFE_NO_PAD.encode(exported.salt),
        kdf: "sha256-prefix-v1".into(),
        note: format!(
            "Paschal Letter export. To open: decrypt ciphertext with AES-256-GCM \
             using nonce_b64 and a 32-byte key derived from your passphrase as \
             SHA-256('paschal/passphrase-kek/v1' || salt_b64 || passphrase). \
             The plaintext is the body of '{escaped_title}'."
        ),
    }))
}

// ----------------------------------------------------------------------------
// Co-Stewards (read-only family deputies)
// ----------------------------------------------------------------------------

#[derive(Serialize)]
pub struct CoStewardView {
    pub id: CoStewardId,
    pub display_name: Option<String>,
    pub email: String,
    pub confirmed: bool,
    pub revoked: bool,
    pub last_viewed_at: Option<String>,
    pub created_at: String,
}

fn co_steward_view(c: &beacon_core::CoSteward) -> CoStewardView {
    CoStewardView {
        id: c.id,
        display_name: c.display_name.clone(),
        email: c.email.clone(),
        confirmed: c.confirmed_at.is_some(),
        revoked: c.revoked_at.is_some(),
        last_viewed_at: c.last_viewed_at.map(|t| t.to_rfc3339()),
        created_at: c.created_at.to_rfc3339(),
    }
}

#[derive(Deserialize)]
pub struct InviteCoStewardReq {
    pub email: String,
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Serialize)]
pub struct InviteCoStewardResp {
    pub co_steward: CoStewardView,
    /// In production this is emailed, not returned. The MVP returns it so
    /// the demo can complete without an SMTP server.
    #[serde(rename = "confirmation_token_DEV_ONLY")]
    pub confirmation_token_dev_only: String,
}

pub async fn invite_co_steward(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<InviteCoStewardReq>,
) -> ApiResult<Json<InviteCoStewardResp>> {
    let pid = current_principal(&state, &headers).await?;
    if !body.email.contains('@') {
        return Err(ApiError::BadRequest("email looks invalid".into()));
    }

    // Plan-gate: max_co_stewards.
    let sub = db::fetch_subscription(&state.pool, pid).await?;
    let plan = plan_features(sub.plan_id);
    if plan.max_co_stewards == 0 {
        return Err(ApiError::Forbidden(
            "Co-Stewards are not enabled on this deployment.".into(),
        ));
    }
    let current = db::co_steward_count(&state.pool, pid).await? as u32;
    if current >= plan.max_co_stewards {
        return Err(ApiError::Forbidden(format!(
            "Co-Steward limit reached ({} of {} allowed). Adjust MAX_CO_STEWARDS in your deployment configuration.",
            current, plan.max_co_stewards
        )));
    }

    let token = random_token();
    let co = db::invite_co_steward(
        &state.pool,
        db::CoStewardInviteInput {
            principal_id: pid,
            display_name: body.display_name.as_deref(),
            email: &body.email,
            confirmation_token_hash: &hash_token(&token),
        },
    )
    .await?;

    let principal = db::fetch_principal(&state.pool, pid).await?;
    let confirmation_url = format!(
        "{}/app/co-steward/confirm?token={}",
        state.config.public_base_url.trim_end_matches('/'),
        token
    );
    tx::co_steward_invite(
        state.notifications.as_ref(),
        &body.email,
        &principal.primary_email,
        &confirmation_url,
    )
    .await;

    let _ = db::append_transparency_entry(
        &state.pool,
        "CO_STEWARD_INVITED",
        &hash_bytes(co.id.as_uuid().as_bytes()),
        json!({ "email_hash": hex::encode(hash_bytes(body.email.as_bytes())) }),
        Some(pid),
        None,
    )
    .await;

    Ok(Json(InviteCoStewardResp {
        co_steward: co_steward_view(&co),
        confirmation_token_dev_only: token,
    }))
}

pub async fn list_co_stewards(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<CoStewardView>>> {
    let pid = current_principal(&state, &headers).await?;
    let rows = db::list_co_stewards(&state.pool, pid).await?;
    Ok(Json(rows.iter().map(co_steward_view).collect()))
}

pub async fn revoke_co_steward(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(co_steward_id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let pid = current_principal(&state, &headers).await?;
    let co = db::fetch_co_steward(&state.pool, CoStewardId(co_steward_id)).await?;
    if co.principal_id != pid {
        return Err(ApiError::NotFound);
    }
    db::revoke_co_steward(&state.pool, co.id).await?;
    let _ = db::append_transparency_entry(
        &state.pool,
        "CO_STEWARD_REVOKED",
        &hash_bytes(co.id.as_uuid().as_bytes()),
        json!({}),
        Some(pid),
        None,
    )
    .await;
    Ok(Json(json!({ "revoked": true })))
}

#[derive(Deserialize)]
pub struct ConfirmCoStewardReq {
    pub token: String,
    /// The passphrase the Co-Steward chooses at confirmation. Used to
    /// authenticate them on subsequent sign-ins.
    pub passphrase: String,
}

#[derive(Serialize)]
pub struct ConfirmCoStewardResp {
    pub co_steward_id: CoStewardId,
    pub session_token: String,
}

pub async fn confirm_co_steward(
    State(state): State<AppState>,
    Json(body): Json<ConfirmCoStewardReq>,
) -> ApiResult<Json<ConfirmCoStewardResp>> {
    if body.passphrase.chars().count() < 10 {
        return Err(ApiError::BadRequest(
            "passphrase must be at least 10 characters".into(),
        ));
    }
    let salt = random_salt();
    let hash = hash_passphrase(&body.passphrase, &salt);
    let token_hash = hash_token(&body.token);
    let co = db::confirm_co_steward(&state.pool, &token_hash, &hash, &salt)
        .await?
        .ok_or_else(|| ApiError::BadRequest("invalid or expired token".into()))?;

    // Issue a session for the Co-Steward.
    let session = random_token();
    db::create_co_steward_session(
        &state.pool,
        co.id,
        co.principal_id,
        &hash_token(&session),
        24,
    )
    .await?;

    let _ = db::append_transparency_entry(
        &state.pool,
        "CO_STEWARD_CONFIRMED",
        &hash_bytes(co.id.as_uuid().as_bytes()),
        json!({}),
        Some(co.principal_id),
        None,
    )
    .await;

    Ok(Json(ConfirmCoStewardResp {
        co_steward_id: co.id,
        session_token: session,
    }))
}

#[derive(Deserialize)]
pub struct CoStewardSignInReq {
    pub email: String,
    pub passphrase: String,
}

pub async fn sign_in_co_steward(
    State(state): State<AppState>,
    Json(body): Json<CoStewardSignInReq>,
) -> ApiResult<Json<ConfirmCoStewardResp>> {
    let salt = db::lookup_co_steward_salt(&state.pool, &body.email)
        .await?
        .ok_or(ApiError::Unauthorised)?;
    let hash = hash_passphrase(&body.passphrase, &salt);
    let co = db::sign_in_co_steward(&state.pool, &body.email, &hash)
        .await?
        .ok_or(ApiError::Unauthorised)?;

    let session = random_token();
    db::create_co_steward_session(
        &state.pool,
        co.id,
        co.principal_id,
        &hash_token(&session),
        24,
    )
    .await?;

    Ok(Json(ConfirmCoStewardResp {
        co_steward_id: co.id,
        session_token: session,
    }))
}

/// Read-only dashboard view for a Co-Steward. Returns the Principal's
/// Vaults (metadata only — no Letter content, no recipient emails),
/// subscription state, usage meter, and Buddy roster.
#[derive(Serialize)]
pub struct CoStewardDashboardView {
    pub principal_email: String,
    pub subscription: SubscriptionView,
    pub vaults: Vec<VaultView>,
    pub buddy_count: usize,
    pub letter_counts: Vec<LetterCountView>,
}

#[derive(Serialize)]
pub struct LetterCountView {
    pub vault_id: VaultId,
    pub letters: i64,
}

pub async fn co_steward_dashboard(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<CoStewardDashboardView>> {
    let (co_id, pid) = current_co_steward(&state, &headers).await?;
    db::touch_co_steward_view(&state.pool, co_id).await?;

    let principal = db::fetch_principal(&state.pool, pid).await?;
    let sub = db::fetch_subscription(&state.pool, pid).await?;
    let vaults = db::list_vaults(&state.pool, pid).await?;
    let buddies = db::list_buddies(&state.pool, pid).await?;

    let mut letter_counts = Vec::with_capacity(vaults.len());
    for v in &vaults {
        let n = db::letter_count(&state.pool, v.id).await?;
        letter_counts.push(LetterCountView {
            vault_id: v.id,
            letters: n,
        });
    }

    Ok(Json(CoStewardDashboardView {
        principal_email: principal.primary_email,
        subscription: subscription_view(&sub),
        vaults: vaults.iter().map(vault_view).collect(),
        buddy_count: buddies.iter().filter(|b| b.is_active()).count(),
        letter_counts,
    }))
}

// ----------------------------------------------------------------------------
// Co-Steward post-mortem administration.
//
// A Co-Steward is read-only while the principal is alive. These endpoints give
// them two narrow write powers — only for Vaults that have RELEASED — so a
// surviving deputy can keep automated deliveries reaching the right place and
// fire event Letters (a wedding) when the moment comes.
// ----------------------------------------------------------------------------

/// Authorise a Co-Steward to administer a specific Vault: the session must
/// belong to a live (confirmed, non-revoked) Co-Steward of this principal, and
/// the Vault must have RELEASED (the principal is verifiably gone).
async fn authorise_postmortem(
    state: &AppState,
    co_id: CoStewardId,
    pid: PrincipalId,
    vault_id: VaultId,
) -> ApiResult<()> {
    let co = db::fetch_co_steward(&state.pool, co_id).await?;
    if !co.is_active() || co.principal_id != pid {
        return Err(ApiError::Unauthorised);
    }
    let vault = db::fetch_vault(&state.pool, vault_id).await?;
    if vault.principal_id != pid {
        return Err(ApiError::NotFound);
    }
    if vault.state != VaultState::Released {
        return Err(ApiError::Conflict(
            "Available only after the Vault has released".into(),
        ));
    }
    Ok(())
}

#[derive(Serialize)]
pub struct PendingChangeView {
    pub change_id: Uuid,
    pub new_email: String,
    pub effective_at: String,
}

#[derive(Serialize)]
pub struct AdminLetterView {
    pub letter_id: LetterId,
    pub vault_id: VaultId,
    pub title: String,
    pub recipient_email: String,
    pub release_mode: String,
    /// A held event Letter still awaiting a deputy's trigger.
    pub awaiting_event_trigger: bool,
    pub event_released: bool,
    pub pending_recipient_change: Option<PendingChangeView>,
}

/// List the Letters a Co-Steward may administer: every Letter in the
/// principal's RELEASED Vaults, with its current delivery contact, whether it
/// is a held event Letter, and any pending recipient change.
pub async fn co_steward_admin_letters(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<AdminLetterView>>> {
    let (co_id, pid) = current_co_steward(&state, &headers).await?;
    let co = db::fetch_co_steward(&state.pool, co_id).await?;
    if !co.is_active() {
        return Err(ApiError::Unauthorised);
    }
    db::touch_co_steward_view(&state.pool, co_id).await?;

    let mut out = Vec::new();
    for vault in db::list_vaults(&state.pool, pid).await? {
        if vault.state != VaultState::Released {
            continue;
        }
        for la in db::list_letters_admin(&state.pool, vault.id).await? {
            let pending = db::fetch_pending_recipient_change(&state.pool, la.id).await?;
            let awaiting_event_trigger =
                la.release_mode == "EVENT_ON_DEMAND" && la.event_released_at.is_none();
            out.push(AdminLetterView {
                letter_id: la.id,
                vault_id: la.vault_id,
                title: la.title,
                recipient_email: la.recipient_email,
                release_mode: la.release_mode,
                awaiting_event_trigger,
                event_released: la.event_released_at.is_some(),
                pending_recipient_change: pending.map(|c| PendingChangeView {
                    change_id: c.id,
                    new_email: c.new_email,
                    effective_at: c.effective_at.to_rfc3339(),
                }),
            });
        }
    }
    Ok(Json(out))
}

#[derive(Deserialize)]
pub struct UpdateRecipientReq {
    pub new_email: String,
}

#[derive(Serialize)]
pub struct UpdateRecipientResp {
    pub change_id: Uuid,
    pub effective_at: String,
}

/// Request a change to where a held Letter is delivered. The change is logged
/// and the *current* address is notified, then it takes effect after a hold
/// during which any Co-Steward can cancel it.
pub async fn co_steward_update_recipient(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(letter_id): Path<Uuid>,
    Json(body): Json<UpdateRecipientReq>,
) -> ApiResult<Json<UpdateRecipientResp>> {
    let (co_id, pid) = current_co_steward(&state, &headers).await?;
    let letter = db::fetch_letter_admin(&state.pool, LetterId(letter_id)).await?;
    authorise_postmortem(&state, co_id, pid, letter.vault_id).await?;

    let new_email = body.new_email.trim();
    if !new_email.contains('@') || new_email.len() > 320 {
        return Err(ApiError::BadRequest("invalid email address".into()));
    }

    // Only Letters with a future automated delivery can have their contact
    // changed: a held event Letter not yet fired, or a date-scheduled Letter
    // whose date is still ahead. Anything already delivered is immutable.
    let now = Utc::now();
    let editable = (letter.release_mode == "EVENT_ON_DEMAND" && letter.event_released_at.is_none())
        || letter
            .scheduled_release_at
            .map(|d| d > now)
            .unwrap_or(false);
    if !editable {
        return Err(ApiError::Conflict(
            "This Letter has no pending automated delivery to redirect".into(),
        ));
    }

    let hold = chrono::Duration::seconds(state.config.recipient_change_hold_seconds.max(0));
    let effective_at = now + hold;
    let change = db::request_recipient_change(
        &state.pool,
        LetterId(letter_id),
        co_id,
        &letter.recipient_email,
        new_email,
        effective_at,
    )
    .await?;

    let _ = db::append_transparency_entry(
        &state.pool,
        "RECIPIENT_CHANGE_REQUESTED",
        &hash_bytes(letter_id.as_bytes()),
        json!({
            "letter_id": letter_id,
            "change_id": change.id,
            "co_steward_id": co_id,
            "effective_at": effective_at.to_rfc3339(),
        }),
        Some(pid),
        Some(letter.vault_id),
    )
    .await;

    // Notify the *current* address so a redirect can't happen silently.
    tx::recipient_change_requested(
        state.notifications.as_ref(),
        &letter.recipient_email,
        &letter.title,
        &effective_at.to_rfc3339(),
    )
    .await;

    Ok(Json(UpdateRecipientResp {
        change_id: change.id,
        effective_at: effective_at.to_rfc3339(),
    }))
}

/// Cancel a pending recipient change during its hold window.
pub async fn co_steward_cancel_recipient_change(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(change_id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let (co_id, pid) = current_co_steward(&state, &headers).await?;
    let change = db::fetch_recipient_change(&state.pool, change_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let letter = db::fetch_letter_admin(&state.pool, change.letter_id).await?;
    authorise_postmortem(&state, co_id, pid, letter.vault_id).await?;

    if !db::cancel_recipient_change(&state.pool, change_id).await? {
        return Err(ApiError::Conflict(
            "Change is no longer pending (already applied or cancelled)".into(),
        ));
    }
    let _ = db::append_transparency_entry(
        &state.pool,
        "RECIPIENT_CHANGE_CANCELLED",
        &hash_bytes(change.letter_id.as_uuid().as_bytes()),
        json!({ "letter_id": change.letter_id, "change_id": change_id }),
        Some(pid),
        Some(letter.vault_id),
    )
    .await;
    Ok(Json(json!({ "cancelled": true })))
}

/// Trigger a held EVENT_ON_DEMAND Letter — "the wedding happened, send it now".
pub async fn co_steward_release_event_letter(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(letter_id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let (co_id, pid) = current_co_steward(&state, &headers).await?;
    let letter = db::fetch_letter_admin(&state.pool, LetterId(letter_id)).await?;
    authorise_postmortem(&state, co_id, pid, letter.vault_id).await?;

    if letter.release_mode != "EVENT_ON_DEMAND" {
        return Err(ApiError::Conflict(
            "Only held event Letters can be triggered this way".into(),
        ));
    }

    crate::scheduler::trigger_event_release(&state, letter.vault_id, LetterId(letter_id)).await?;
    Ok(Json(json!({ "released": true })))
}

// ----------------------------------------------------------------------------
// Vault contacts
// ----------------------------------------------------------------------------

#[derive(Serialize)]
pub struct VaultContactView {
    pub id: Uuid,
    pub display_name: String,
    pub email: String,
    pub birthday: Option<String>,
    pub release_note: Option<String>,
    pub created_at: String,
}

#[derive(Deserialize)]
pub struct CreateVaultContactReq {
    pub display_name: String,
    pub email: String,
    pub birthday: Option<String>,
    pub release_note: Option<String>,
}

pub async fn list_vault_contacts(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(vault_id): Path<Uuid>,
) -> ApiResult<Json<Vec<VaultContactView>>> {
    let pid = current_principal(&state, &headers).await?;
    let vault = db::fetch_vault(&state.pool, VaultId(vault_id)).await?;
    if vault.principal_id != pid {
        return Err(ApiError::NotFound);
    }
    let contacts = db::list_vault_contacts(&state.pool, vault.id).await?;
    Ok(Json(
        contacts
            .into_iter()
            .map(|c| VaultContactView {
                id: c.id,
                display_name: c.display_name,
                email: c.email,
                birthday: c.birthday,
                release_note: c.release_note,
                created_at: c.created_at.to_rfc3339(),
            })
            .collect(),
    ))
}

pub async fn create_vault_contact(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(vault_id): Path<Uuid>,
    Json(body): Json<CreateVaultContactReq>,
) -> ApiResult<Json<VaultContactView>> {
    let pid = current_principal(&state, &headers).await?;
    let vault = db::fetch_vault(&state.pool, VaultId(vault_id)).await?;
    if vault.principal_id != pid {
        return Err(ApiError::NotFound);
    }
    if !body.email.contains('@') {
        return Err(ApiError::BadRequest("email looks invalid".into()));
    }
    let c = db::create_vault_contact(
        &state.pool,
        vault.id,
        &body.display_name,
        &body.email,
        body.birthday.as_deref(),
        body.release_note.as_deref(),
    )
    .await?;
    Ok(Json(VaultContactView {
        id: c.id,
        display_name: c.display_name,
        email: c.email,
        birthday: c.birthday,
        release_note: c.release_note,
        created_at: c.created_at.to_rfc3339(),
    }))
}

pub async fn update_vault_contact(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((vault_id, contact_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<CreateVaultContactReq>,
) -> ApiResult<Json<VaultContactView>> {
    let pid = current_principal(&state, &headers).await?;
    let vault = db::fetch_vault(&state.pool, VaultId(vault_id)).await?;
    if vault.principal_id != pid {
        return Err(ApiError::NotFound);
    }
    let c = db::update_vault_contact(
        &state.pool,
        contact_id,
        vault.id,
        &body.display_name,
        &body.email,
        body.birthday.as_deref(),
        body.release_note.as_deref(),
    )
    .await?;
    Ok(Json(VaultContactView {
        id: c.id,
        display_name: c.display_name,
        email: c.email,
        birthday: c.birthday,
        release_note: c.release_note,
        created_at: c.created_at.to_rfc3339(),
    }))
}

pub async fn delete_vault_contact(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((vault_id, contact_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<axum::http::StatusCode> {
    let pid = current_principal(&state, &headers).await?;
    let vault = db::fetch_vault(&state.pool, VaultId(vault_id)).await?;
    if vault.principal_id != pid {
        return Err(ApiError::NotFound);
    }
    db::delete_vault_contact(&state.pool, contact_id, vault.id).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

// ----------------------------------------------------------------------------
// Bank dormancy signal
// ----------------------------------------------------------------------------

#[derive(Serialize)]
pub struct BankSignalView {
    pub webhook_url: String,
    pub last_webhook_at: Option<String>,
}

pub async fn bank_dormancy_enrol(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<BankSignalView>> {
    let pid = current_principal(&state, &headers).await?;

    // Idempotent: return existing subscription if present.
    if let Some(row) = db::fetch_bank_dormancy_sub(&state.pool, pid).await? {
        let url = format!(
            "{}/v1/signals/bank-dormancy/{}",
            state.config.public_base_url.trim_end_matches('/'),
            row.webhook_id
        );
        return Ok(Json(BankSignalView {
            webhook_url: url,
            last_webhook_at: row.last_webhook_at.map(|t| t.to_rfc3339()),
        }));
    }

    let webhook_id = Uuid::new_v4();
    let mut secret = [0u8; 32];
    use rand::RngCore;
    rand::rng().fill_bytes(&mut secret);

    db::create_bank_dormancy_sub(&state.pool, pid, webhook_id, &secret).await?;
    let url = format!(
        "{}/v1/signals/bank-dormancy/{}",
        state.config.public_base_url.trim_end_matches('/'),
        webhook_id
    );
    Ok(Json(BankSignalView {
        webhook_url: url,
        last_webhook_at: None,
    }))
}

pub async fn get_bank_signal_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<BankSignalView>> {
    let pid = current_principal(&state, &headers).await?;
    let row = db::fetch_bank_dormancy_sub(&state.pool, pid)
        .await?
        .ok_or(ApiError::NotFound)?;
    let url = format!(
        "{}/v1/signals/bank-dormancy/{}",
        state.config.public_base_url.trim_end_matches('/'),
        row.webhook_id
    );
    Ok(Json(BankSignalView {
        webhook_url: url,
        last_webhook_at: row.last_webhook_at.map(|t| t.to_rfc3339()),
    }))
}

pub async fn revoke_bank_signal(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<axum::http::StatusCode> {
    let pid = current_principal(&state, &headers).await?;
    db::delete_bank_dormancy_sub(&state.pool, pid).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub async fn bank_dormancy_webhook(
    State(state): State<AppState>,
    Path(webhook_id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let row = db::fetch_bank_dormancy_sub_by_webhook(&state.pool, webhook_id)
        .await?
        .ok_or(ApiError::NotFound)?;

    db::touch_bank_dormancy_sub(&state.pool, webhook_id, Utc::now()).await?;

    for vault in db::list_vaults(&state.pool, row.principal_id).await? {
        let _ = db::record_signal(
            &state.pool,
            vault.id,
            beacon_core::SignalSource::Cdr,
            0.0,
            json!({ "source": "bank_dormancy_webhook" }),
        )
        .await;
    }

    Ok(Json(json!({ "ok": true })))
}

// ----------------------------------------------------------------------------
// Public trust endpoints (no auth)
// ----------------------------------------------------------------------------

pub async fn get_warrant_canary() -> Json<serde_json::Value> {
    Json(json!({
        "statement": "As of this statement, the operator of this Paschal instance has not received any secret government order, search warrant, gag order, or national-security letter requiring them to compromise the security or integrity of any user's data, and has not been compelled to install any backdoor.",
        "issued_at": "2026-06-01T00:00:00Z",
        "next_update_by": "2026-09-01T00:00:00Z"
    }))
}

pub async fn list_transparency_log(
    State(state): State<AppState>,
) -> ApiResult<Json<Vec<serde_json::Value>>> {
    let entries = db::list_transparency_entries(&state.pool, 200).await?;
    let resp: Vec<serde_json::Value> = entries
        .into_iter()
        .map(|e| {
            json!({
                "id": e.id,
                "kind": e.kind,
                "ts": e.ts.to_rfc3339(),
                "payload": e.payload_json,
            })
        })
        .collect();
    Ok(Json(resp))
}

// ---------------------------------------------------------------------------
// Duress signal — covert, user-armed panic webhook that freezes release.
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
pub struct ArmDuressReq {
    pub alert_email: Option<String>,
    /// "freeze" (default) or "release".
    pub panic_mode: Option<String>,
}

#[derive(serde::Serialize)]
pub struct DuressView {
    pub armed: bool,
    pub triggered_at: Option<String>,
    pub webhook_url: Option<String>,
    pub alert_email: Option<String>,
    pub panic_mode: String,
}

fn duress_view(state: &AppState, row: Option<db::DuressSignalRow>) -> DuressView {
    match row {
        Some(r) => DuressView {
            armed: true,
            triggered_at: r.triggered_at.map(|t| t.to_rfc3339()),
            webhook_url: Some(format!(
                "{}/v1/signals/duress/{}",
                state.config.public_base_url.trim_end_matches('/'),
                r.webhook_id
            )),
            alert_email: r.alert_email,
            panic_mode: r.panic_mode,
        },
        None => DuressView {
            armed: false,
            triggered_at: None,
            webhook_url: None,
            alert_email: None,
            panic_mode: "freeze".into(),
        },
    }
}

pub async fn arm_duress(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ArmDuressReq>,
) -> ApiResult<Json<DuressView>> {
    let pid = current_principal(&state, &headers).await?;
    let alert_email = body
        .alert_email
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(e) = alert_email {
        if !e.contains('@') {
            return Err(ApiError::BadRequest("alert_email looks invalid".into()));
        }
    }
    let panic_mode = body
        .panic_mode
        .as_deref()
        .map(str::trim)
        .unwrap_or("freeze");
    if !matches!(panic_mode, "freeze" | "release") {
        return Err(ApiError::BadRequest(
            "panic_mode must be 'freeze' or 'release'".into(),
        ));
    }
    let row = db::arm_duress(&state.pool, pid, Uuid::new_v4(), alert_email, panic_mode).await?;
    Ok(Json(duress_view(&state, Some(row))))
}

pub async fn get_duress(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<DuressView>> {
    let pid = current_principal(&state, &headers).await?;
    let row = db::fetch_duress(&state.pool, pid).await?;
    Ok(Json(duress_view(&state, row)))
}

pub async fn revoke_duress(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<axum::http::StatusCode> {
    let pid = current_principal(&state, &headers).await?;
    db::delete_duress(&state.pool, pid).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Public covert trigger. Always returns 200 (even for an unknown webhook) so an
/// observer cannot probe it. When valid it freezes the dead-man's switch (via
/// `triggered_at`, checked in the aggregator) and silently alerts the contact.
pub async fn duress_webhook(
    State(state): State<AppState>,
    Path(webhook_id): Path<Uuid>,
) -> axum::http::StatusCode {
    if let Ok(Some(row)) = db::trigger_duress(&state.pool, webhook_id, Utc::now()).await {
        if let Some(email) = row.alert_email.as_deref() {
            state
                .notifications
                .send(crate::notifications::OutboundMessage {
                    channel: crate::notifications::Channel::Email,
                    to: email.to_string(),
                    subject: Some("Wellbeing check — please reach out".into()),
                    body: "Someone who trusts you has triggered a private safety alert \
                           through Paschal. Please check on them directly and discreetly. \
                           This message was sent on their prior instruction."
                        .into(),
                })
                .await;
        }
        let _ = db::append_transparency_entry(
            &state.pool,
            "DURESS_TRIGGERED",
            &hash_bytes(row.principal_id.as_uuid().as_bytes()),
            json!({ "triggered_at": row.triggered_at }),
            Some(row.principal_id),
            None,
        )
        .await;

        // "release" panic mode: start cooling-off on every watchable Vault
        // immediately, so the principal's wishes are carried out without delay.
        // (The default "freeze" mode is handled by the aggregator pausing while
        // `triggered_at` is set — see scheduler.rs.)
        if row.panic_mode == "release" {
            let watchable = [
                VaultState::Active,
                VaultState::Suspicious,
                VaultState::Alert,
            ];
            if let Ok(vaults) = db::list_vaults(&state.pool, row.principal_id).await {
                for vault in vaults.into_iter().filter(|v| watchable.contains(&v.state)) {
                    if db::claim_vault_cooling_off(&state.pool, vault.id, vault.state)
                        .await
                        .unwrap_or(false)
                    {
                        let ends_at = Utc::now()
                            + chrono::Duration::seconds(vault.cooling_off_seconds as i64);
                        if let Ok(principal) =
                            db::fetch_principal(&state.pool, vault.principal_id).await
                        {
                            crate::notifications::tx::cooling_off_started(
                                state.notifications.as_ref(),
                                &principal.primary_email,
                                &vault.name,
                                &ends_at.to_rfc3339(),
                            )
                            .await;
                        }
                        let _ = db::create_release_event_with_deadline(
                            &state.pool,
                            vault.id,
                            ReleaseReason::SignalTrigger,
                            false,
                            ends_at,
                        )
                        .await;
                        let _ = db::append_transparency_entry(
                            &state.pool,
                            "DURESS_RELEASE_TRIGGERED",
                            &hash_bytes(vault.id.as_uuid().as_bytes()),
                            json!({ "panic_mode": "release" }),
                            Some(vault.principal_id),
                            Some(vault.id),
                        )
                        .await;
                    }
                }
            }
        }
    }
    axum::http::StatusCode::OK
}

// ----------------------------------------------------------------------------
// Passwordless sign-in (email)
// ----------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct SigninReq {
    pub email: String,
}

#[derive(Serialize)]
pub struct SigninResp {
    pub status: String,
    #[serde(
        rename = "magic_token_DEV_ONLY",
        skip_serializing_if = "Option::is_none"
    )]
    pub magic_token_dev_only: Option<String>,
}

/// Email sign-in, step 1: send a one-time magic link.
///
/// Always returns 200 with the same body whether or not an account exists, so
/// the endpoint cannot be used to enumerate accounts, and — critically — never
/// issues a session just because someone knows an email address. The session is
/// only minted by [`magic_link_verify`] after the emailed token is redeemed.
pub async fn signin(
    State(state): State<AppState>,
    Json(body): Json<SigninReq>,
) -> ApiResult<Json<SigninResp>> {
    if !body.email.contains('@') {
        return Err(ApiError::BadRequest("email looks invalid".into()));
    }
    let mut dev_token = None;
    if let Some(principal) = db::fetch_principal_by_email(&state.pool, &body.email).await? {
        let magic = random_token();
        db::create_magic_link(&state.pool, principal.id, &hash_token(&magic), 15).await?;
        let link = format!(
            "{}/app/auth/verify?token={magic}",
            state.config.public_base_url.trim_end_matches('/'),
        );
        tx::magic_link(
            state.notifications.as_ref(),
            &principal.primary_email,
            &link,
        )
        .await;
        if cfg!(debug_assertions) {
            dev_token = Some(magic);
        }
    }
    Ok(Json(SigninResp {
        status: "sent".into(),
        magic_token_dev_only: dev_token,
    }))
}

#[derive(Deserialize)]
pub struct MagicVerifyReq {
    pub token: String,
}

#[derive(Serialize)]
pub struct MagicVerifyResp {
    pub principal_id: PrincipalId,
    pub session_token: String,
    pub email: String,
}

/// Email sign-in, step 2: redeem the magic link for a session.
///
/// The token is single-use (consumed atomically) and expires 15 minutes after
/// issue. A valid redemption proves control of the inbox.
pub async fn magic_link_verify(
    State(state): State<AppState>,
    Json(body): Json<MagicVerifyReq>,
) -> ApiResult<Json<MagicVerifyResp>> {
    let pid = db::consume_magic_link(&state.pool, &hash_token(&body.token))
        .await?
        .ok_or(ApiError::Unauthorised)?;
    let principal = db::fetch_principal(&state.pool, pid).await?;
    let session = random_token();
    db::create_session(&state.pool, pid, &hash_token(&session), 24 * 30).await?;
    Ok(Json(MagicVerifyResp {
        principal_id: pid,
        session_token: session,
        email: principal.primary_email,
    }))
}

/// Revoke the caller's session server-side (so a stolen token stops working).
pub async fn signout(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<StatusCode> {
    if let Some(token) = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
    {
        let hash = hash_token(token);
        let _ = db::revoke_session(&state.pool, &hash).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Map the `plan` query parameter from sign-up into a concrete PlanId.
/// Accepts both the short tier names ("draft", "monthly", "annual",
/// "plus", "plus_annual", "legacy", "legacy_annual") and the explicit
/// underscore IDs ("estate_monthly_v2", etc).

// Zero-Knowledge letters (browser-encrypted; operator stores ciphertext only)
// ----------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct SealZkLetterReq {
    pub title: String,
    pub recipient_email: String,
    /// Base64url AES-256-GCM ciphertext of the letter body, encrypted in the
    /// browser under the vault DEK. The operator never receives the plaintext.
    pub ciphertext: String,
    /// Base64url 12-byte GCM nonce.
    pub nonce: String,
    #[serde(default)]
    pub scheduled_release_at: Option<chrono::DateTime<Utc>>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub release_mode: Option<String>,
    /// Optional heir envelope: the body re-sealed under a key the recipient can
    /// obtain after release. `heir_mode` selects how (default 'manual').
    #[serde(default)]
    pub heir_ciphertext: Option<String>,
    #[serde(default)]
    pub heir_nonce: Option<String>,
    /// 'manual' (default) | 'split' | 'operator'.
    #[serde(default)]
    pub heir_mode: Option<String>,
    /// manual mode: PBKDF2 salt (hex).
    #[serde(default)]
    pub heir_salt: Option<String>,
    /// split: the operator's half of the key (base64url). operator: the whole
    /// key. Stored, released to the recipient at claim.
    #[serde(default)]
    pub heir_release_secret: Option<String>,
    /// split only: the recipient's half (base64url). Emailed to the recipient at
    /// creation and NEVER stored — so the operator never holds both halves.
    #[serde(default)]
    pub heir_recipient_share: Option<String>,
}

/// Seal a Letter into a Private (Zero-Knowledge) Vault. The body arrives
/// already encrypted under the vault DEK; we store the ciphertext verbatim and
/// run NO operator-side KMS seal. Mirrors `seal_letter`'s authoring guards.
pub async fn seal_zk_letter(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(vault_id): Path<Uuid>,
    Json(body): Json<SealZkLetterReq>,
) -> ApiResult<Json<LetterMetaView>> {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;

    let pid = current_principal(&state, &headers).await?;
    let vault = db::fetch_vault(&state.pool, VaultId(vault_id)).await?;
    if vault.principal_id != pid {
        return Err(ApiError::NotFound);
    }
    if vault.tier != Tier::ZeroKnowledge {
        return Err(ApiError::BadRequest(
            "This endpoint is for Private (Zero-Knowledge) Vaults. Use /letters to \
             seal in a Standard Vault."
                .into(),
        ));
    }

    validate_letter_kind(body.kind.as_deref())?;
    validate_release_mode(
        body.release_mode.as_deref(),
        body.scheduled_release_at.as_ref(),
    )?;

    let sub = db::fetch_subscription(&state.pool, pid).await?;
    if !sub.state.allows_authoring() {
        return Err(ApiError::Forbidden(format!(
            "Subscription state {:?} does not permit authoring",
            sub.state
        )));
    }
    if vault.state != VaultState::Active {
        return Err(ApiError::Conflict(format!(
            "Vault state {:?} does not permit sealing new Letters",
            vault.state
        )));
    }

    let plan = plan_features(sub.plan_id);
    let current_letters = db::letter_count(&state.pool, vault.id).await? as u32;
    if current_letters >= plan.max_letters_per_vault {
        return Err(ApiError::Forbidden(format!(
            "Letter limit reached ({} of {} per Vault). Adjust the plan limits in your deployment configuration to increase it.",
            current_letters, plan.max_letters_per_vault
        )));
    }

    if let Some(t) = body.scheduled_release_at {
        if t < Utc::now() {
            return Err(ApiError::BadRequest(
                "scheduled_release_at must be in the future".into(),
            ));
        }
        let horizon = Utc::now() + chrono::Duration::days(plan.scheduled_horizon_days);
        if t > horizon {
            return Err(ApiError::BadRequest(format!(
                "Scheduled releases are limited to {} days in the future. \
                 Adjust the scheduled-release horizon in your deployment configuration to extend it.",
                plan.scheduled_horizon_days
            )));
        }
    }

    let ciphertext = URL_SAFE_NO_PAD
        .decode(body.ciphertext.as_bytes())
        .map_err(|_| ApiError::BadRequest("ciphertext must be base64url".into()))?;
    let nonce = URL_SAFE_NO_PAD
        .decode(body.nonce.as_bytes())
        .map_err(|_| ApiError::BadRequest("nonce must be base64url".into()))?;
    if ciphertext.is_empty() {
        return Err(ApiError::BadRequest("ciphertext is empty".into()));
    }
    if nonce.len() != 12 {
        return Err(ApiError::BadRequest("nonce must be 12 bytes".into()));
    }

    let letter = db::seal_letter(
        &state.pool,
        db::LetterSealInput {
            vault_id: vault.id,
            title: &body.title,
            recipient_email: &body.recipient_email,
            ciphertext: &ciphertext,
            nonce: &nonce,
            drill_ciphertext: None,
            drill_nonce: None,
            scheduled_release_at: body.scheduled_release_at,
            kind: body.kind.as_deref(),
            category: body.category.as_deref(),
            release_mode: body.release_mode.as_deref(),
        },
    )
    .await?;

    // Optional heir envelope: the same body re-sealed under a key the recipient
    // can obtain after release. The mode decides how the key reaches them.
    let heir_mode = body.heir_mode.as_deref().unwrap_or("manual");
    let has_heir = if let (Some(hc), Some(hn)) = (&body.heir_ciphertext, &body.heir_nonce) {
        let ciphertext = URL_SAFE_NO_PAD
            .decode(hc.as_bytes())
            .map_err(|_| ApiError::BadRequest("heir_ciphertext must be base64url".into()))?;
        let nonce = URL_SAFE_NO_PAD
            .decode(hn.as_bytes())
            .map_err(|_| ApiError::BadRequest("heir_nonce must be base64url".into()))?;
        if nonce.len() != 12 {
            return Err(ApiError::BadRequest("heir_nonce must be 12 bytes".into()));
        }
        match heir_mode {
            "manual" => {
                let salt = body.heir_salt.as_deref().ok_or_else(|| {
                    ApiError::BadRequest("manual heir mode needs heir_salt".into())
                })?;
                let salt = hex::decode(salt)
                    .map_err(|_| ApiError::BadRequest("heir_salt must be hex".into()))?;
                db::upsert_zk_letter_heir_envelope(
                    &state.pool,
                    letter.id,
                    &ciphertext,
                    &nonce,
                    "manual",
                    Some(&salt),
                    None,
                )
                .await?;
            }
            "split" => {
                let op_share = body.heir_release_secret.as_deref().ok_or_else(|| {
                    ApiError::BadRequest("split heir mode needs heir_release_secret".into())
                })?;
                let op_share = URL_SAFE_NO_PAD.decode(op_share.as_bytes()).map_err(|_| {
                    ApiError::BadRequest("heir_release_secret must be base64url".into())
                })?;
                let rcpt_share = body.heir_recipient_share.as_deref().ok_or_else(|| {
                    ApiError::BadRequest("split heir mode needs heir_recipient_share".into())
                })?;
                // Store ONLY the operator's half — never the recipient's. Email
                // the recipient theirs now, then discard it.
                db::upsert_zk_letter_heir_envelope(
                    &state.pool,
                    letter.id,
                    &ciphertext,
                    &nonce,
                    "split",
                    None,
                    Some(&op_share),
                )
                .await?;
                tx::heir_share(
                    state.notifications.as_ref(),
                    &body.recipient_email,
                    rcpt_share,
                )
                .await;
            }
            "operator" => {
                let key = body.heir_release_secret.as_deref().ok_or_else(|| {
                    ApiError::BadRequest("operator heir mode needs heir_release_secret".into())
                })?;
                let key = URL_SAFE_NO_PAD.decode(key.as_bytes()).map_err(|_| {
                    ApiError::BadRequest("heir_release_secret must be base64url".into())
                })?;
                db::upsert_zk_letter_heir_envelope(
                    &state.pool,
                    letter.id,
                    &ciphertext,
                    &nonce,
                    "operator",
                    None,
                    Some(&key),
                )
                .await?;
            }
            other => {
                return Err(ApiError::BadRequest(format!("unknown heir_mode '{other}'")));
            }
        }
        true
    } else {
        false
    };

    let _ = db::append_transparency_entry(
        &state.pool,
        "LETTER_SEALED",
        &hash_bytes(letter.id.as_uuid().as_bytes()),
        json!({
            "letter_id": letter.id,
            "vault_id": vault.id,
            "title_hash": hex::encode(hash_bytes(body.title.as_bytes())),
            "zero_knowledge": true,
            "has_heir_envelope": has_heir,
            "scheduled_release_at": body.scheduled_release_at,
        }),
        Some(pid),
        Some(vault.id),
    )
    .await;

    Metrics::inc(&state.metrics.letters_sealed_total);
    Ok(Json(LetterMetaView {
        id: letter.id,
        title: letter.title,
        recipient_email: letter.recipient_email,
        sealed_at: letter.sealed_at.to_rfc3339(),
        scheduled_release_at: body.scheduled_release_at.map(|t| t.to_rfc3339()),
    }))
}

#[derive(Serialize)]
pub struct ZkLetterCiphertextView {
    /// Base64url AES-256-GCM ciphertext, decryptable only with the vault DEK.
    pub ciphertext: String,
    /// Base64url 12-byte GCM nonce.
    pub nonce: String,
}

/// Return a Private Letter's stored ciphertext so the author can decrypt it in
/// the browser. The operator returns bytes it cannot read.
pub async fn get_zk_letter_ciphertext(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((vault_id, letter_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ZkLetterCiphertextView>> {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;

    let pid = current_principal(&state, &headers).await?;
    let vault = db::fetch_vault(&state.pool, VaultId(vault_id)).await?;
    if vault.principal_id != pid {
        return Err(ApiError::NotFound);
    }
    if vault.tier != Tier::ZeroKnowledge {
        return Err(ApiError::BadRequest(
            "This endpoint is for Private (Zero-Knowledge) Vaults.".into(),
        ));
    }

    let letter = db::fetch_letter_full(&state.pool, LetterId(letter_id)).await?;
    if letter.vault_id != vault.id {
        return Err(ApiError::NotFound);
    }
    if letter.ciphertext.is_empty() {
        return Err(ApiError::Conflict(
            "letter has been crypto-erased and can no longer be opened".into(),
        ));
    }

    Ok(Json(ZkLetterCiphertextView {
        ciphertext: URL_SAFE_NO_PAD.encode(&letter.ciphertext),
        nonce: URL_SAFE_NO_PAD.encode(&letter.nonce),
    }))
}
