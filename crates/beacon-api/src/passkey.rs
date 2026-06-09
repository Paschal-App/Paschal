//! WebAuthn passkey auth and ZK envelope routes.

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use beacon_core::{PrincipalId, Tier, VaultId};
use beacon_db as db;
use chrono::Utc;
use crypto_stub::{hash_token, random_token};
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use uuid::Uuid;
use webauthn_rs::prelude::*;

use crate::{
    auth::current_principal,
    errors::{ApiError, ApiResult},
    routes::parse_plan_param,
    state::AppState,
};

// ---------------------------------------------------------------------------
// Registration — options (unauthenticated)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct PasskeyRegisterOptionsReq {
    pub email: String,
    #[serde(default)]
    pub plan: Option<String>,
    #[serde(default)]
    pub tos_accepted: Option<bool>,
}

#[derive(Serialize)]
pub struct PasskeyRegisterOptionsResp {
    pub challenge_id: Uuid,
    pub options: serde_json::Value,
}

pub async fn passkey_register_options(
    State(state): State<AppState>,
    Json(body): Json<PasskeyRegisterOptionsReq>,
) -> ApiResult<Json<PasskeyRegisterOptionsResp>> {
    if !body.email.contains('@') {
        return Err(ApiError::BadRequest("email looks invalid".into()));
    }

    let (principal, created) = db::upsert_principal_by_email(&state.pool, &body.email).await?;

    if created {
        if body.tos_accepted != Some(true) {
            return Err(ApiError::BadRequest(
                "You must accept the Terms of Service to create an account.".into(),
            ));
        }
        let _ = db::set_tos_accepted(&state.pool, principal.id).await;
        let plan = parse_plan_param(body.plan.as_deref())?;
        db::create_subscription(&state.pool, principal.id, plan, state.config.trial_days).await?;
    }

    // Collect existing credentials to exclude (prevents re-registering same device).
    let existing = db::list_passkeys_for_principal(&state.pool, principal.id.as_uuid()).await?;
    let exclude: Vec<CredentialID> = existing
        .iter()
        .map(|pk| CredentialID::from(pk.credential_id.clone()))
        .collect();
    let exclude_opt = if exclude.is_empty() {
        None
    } else {
        Some(exclude)
    };

    let (ccr, reg_state) = state
        .webauthn
        .start_passkey_registration(
            principal.id.as_uuid(),
            &body.email,
            &body.email,
            exclude_opt,
        )
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("webauthn register start: {e}")))?;

    let state_json = serde_json::to_string(&reg_state)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("serialize reg state: {e}")))?;

    let challenge_id = db::create_webauthn_challenge(
        &state.pool,
        Some(principal.id.as_uuid()),
        &state_json,
        "REGISTRATION",
    )
    .await?;

    let options_json = serde_json::to_value(&ccr)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("serialize ccr: {e}")))?;

    Ok(Json(PasskeyRegisterOptionsResp {
        challenge_id,
        options: options_json,
    }))
}

// ---------------------------------------------------------------------------
// Registration — verify (unauthenticated)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct PasskeyRegisterVerifyReq {
    pub challenge_id: Uuid,
    pub credential: serde_json::Value,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Serialize)]
pub struct PasskeyRegisterVerifyResp {
    pub principal_id: PrincipalId,
    pub session_token: String,
    /// 32 hex chars (128-bit). Present only for the FIRST passkey on an account
    /// (shown once — keep with estate documents); absent on later adds, since
    /// regenerating it would invalidate the Recovery Key wraps tied to it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_code: Option<String>,
}

pub async fn passkey_register_verify(
    State(state): State<AppState>,
    Json(body): Json<PasskeyRegisterVerifyReq>,
) -> ApiResult<Json<PasskeyRegisterVerifyResp>> {
    let (state_json, principal_id_opt) =
        db::consume_webauthn_challenge(&state.pool, body.challenge_id, "REGISTRATION")
            .await?
            .ok_or_else(|| {
                ApiError::BadRequest("Challenge not found, expired, or already used".into())
            })?;

    let principal_uuid = principal_id_opt
        .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("Challenge missing principal_id")))?;

    let reg_state: PasskeyRegistration = serde_json::from_str(&state_json)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("deserialize reg state: {e}")))?;

    let reg_response: RegisterPublicKeyCredential = serde_json::from_value(body.credential)
        .map_err(|e| ApiError::BadRequest(format!("invalid credential: {e}")))?;

    let passkey = state
        .webauthn
        .finish_passkey_registration(&reg_response, &reg_state)
        .map_err(|e| ApiError::BadRequest(format!("passkey registration failed: {e}")))?;

    let cred_id: Vec<u8> = Vec::from(passkey.cred_id().clone());

    if db::get_passkey_by_credential_id(&state.pool, &cred_id)
        .await?
        .is_some()
    {
        return Err(ApiError::Conflict(
            "This passkey is already registered".into(),
        ));
    }

    let passkey_json = serde_json::to_string(&passkey)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("serialize passkey: {e}")))?;

    // webauthn-rs::Passkey doesn't expose backup_state/transports publicly;
    // extract them from the serialized JSON (structure: {"cred": {...}}).
    let passkey_value: serde_json::Value =
        serde_json::from_str(&passkey_json).unwrap_or(serde_json::Value::Null);
    let backed_up = passkey_value["cred"]["backup_state"]
        .as_bool()
        .unwrap_or(false);
    let transports: Vec<String> = passkey_value["cred"]["transports"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let name = body.name.unwrap_or_else(|| "Passkey".to_string());

    db::store_passkey(
        &state.pool,
        principal_uuid,
        &cred_id,
        &passkey_json,
        backed_up,
        &transports,
        &name,
    )
    .await?;

    // Mint a recovery code only for the FIRST passkey on the account.
    // Regenerating it on every add would orphan the Recovery Key wraps (and
    // therefore every Private Vault's recovery envelope) tied to the old code.
    let recovery_code = if db::get_principal_recovery(&state.pool, principal_uuid)
        .await?
        .is_some()
    {
        None
    } else {
        let code = generate_recovery_code();
        let code_salt = random_bytes(32);
        let code_verifier = hmac_sha256(&code_salt, code.as_bytes());
        db::upsert_principal_recovery(&state.pool, principal_uuid, &code_salt, &code_verifier)
            .await?;
        Some(code)
    };

    let principal_id = PrincipalId(principal_uuid);
    let session = random_token();
    db::create_session(&state.pool, principal_id, &hash_token(&session), 720).await?;

    Ok(Json(PasskeyRegisterVerifyResp {
        principal_id,
        session_token: session,
        recovery_code,
    }))
}

// ---------------------------------------------------------------------------
// Authentication — options (unauthenticated)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct PasskeyAuthOptionsReq {
    pub email: String,
}

#[derive(Serialize)]
pub struct PasskeyAuthOptionsResp {
    pub challenge_id: Uuid,
    pub options: serde_json::Value,
}

pub async fn passkey_authenticate_options(
    State(state): State<AppState>,
    Json(body): Json<PasskeyAuthOptionsReq>,
) -> ApiResult<Json<PasskeyAuthOptionsResp>> {
    let principal = db::fetch_principal_by_email(&state.pool, &body.email)
        .await?
        .ok_or(ApiError::Unauthorised)?;

    let passkey_rows =
        db::get_passkeys_for_authentication(&state.pool, principal.id.as_uuid()).await?;
    if passkey_rows.is_empty() {
        return Err(ApiError::NotFound);
    }

    let passkeys: Vec<Passkey> = passkey_rows
        .iter()
        .filter_map(|row| serde_json::from_str(&row.passkey_json).ok())
        .collect();

    let (rcr, auth_state) = state
        .webauthn
        .start_passkey_authentication(&passkeys)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("webauthn auth start: {e}")))?;

    let state_json = serde_json::to_string(&auth_state)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("serialize auth state: {e}")))?;

    let challenge_id = db::create_webauthn_challenge(
        &state.pool,
        Some(principal.id.as_uuid()),
        &state_json,
        "AUTHENTICATION",
    )
    .await?;

    let options_json = serde_json::to_value(&rcr)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("serialize rcr: {e}")))?;

    Ok(Json(PasskeyAuthOptionsResp {
        challenge_id,
        options: options_json,
    }))
}

// ---------------------------------------------------------------------------
// Authentication — verify (unauthenticated)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct PasskeyAuthVerifyReq {
    pub challenge_id: Uuid,
    pub assertion: serde_json::Value,
}

#[derive(Serialize)]
pub struct PasskeyAuthVerifyResp {
    pub principal_id: PrincipalId,
    pub session_token: String,
}

pub async fn passkey_authenticate_verify(
    State(state): State<AppState>,
    Json(body): Json<PasskeyAuthVerifyReq>,
) -> ApiResult<Json<PasskeyAuthVerifyResp>> {
    let (state_json, _) =
        db::consume_webauthn_challenge(&state.pool, body.challenge_id, "AUTHENTICATION")
            .await?
            .ok_or_else(|| {
                ApiError::BadRequest("Challenge not found, expired, or already used".into())
            })?;

    let auth_state: PasskeyAuthentication = serde_json::from_str(&state_json)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("deserialize auth state: {e}")))?;

    let auth_response: PublicKeyCredential = serde_json::from_value(body.assertion)
        .map_err(|e| ApiError::BadRequest(format!("invalid assertion: {e}")))?;

    let auth_result = state
        .webauthn
        .finish_passkey_authentication(&auth_response, &auth_state)
        .map_err(|e| ApiError::BadRequest(format!("passkey authentication failed: {e}")))?;

    let cred_id: Vec<u8> = Vec::from(auth_result.cred_id().clone());
    let passkey_row = db::get_passkey_by_credential_id(&state.pool, &cred_id)
        .await?
        .ok_or(ApiError::Unauthorised)?;

    // Update the stored passkey JSON (sign_count, backup state).
    let mut passkey: Passkey = serde_json::from_str(&passkey_row.passkey_json)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("deserialize passkey: {e}")))?;
    passkey.update_credential(&auth_result);
    let updated_json = serde_json::to_string(&passkey)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("serialize passkey: {e}")))?;

    db::update_passkey_after_auth(&state.pool, passkey_row.id, &updated_json, Utc::now()).await?;

    let principal_id = PrincipalId(passkey_row.principal_id);
    let session = random_token();
    db::create_session(&state.pool, principal_id, &hash_token(&session), 720).await?;

    Ok(Json(PasskeyAuthVerifyResp {
        principal_id,
        session_token: session,
    }))
}

// ---------------------------------------------------------------------------
// Recovery code validation (unauthenticated)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct RecoveryValidateReq {
    pub email: String,
    pub code: String,
}

#[derive(Serialize)]
pub struct RecoveryValidateResp {
    pub code_salt: String,
}

pub async fn passkey_recovery_validate(
    State(state): State<AppState>,
    Json(body): Json<RecoveryValidateReq>,
) -> ApiResult<Json<RecoveryValidateResp>> {
    let row = db::get_principal_recovery_by_email(&state.pool, &body.email)
        .await?
        .ok_or_else(|| ApiError::BadRequest("No recovery code set for this account".into()))?;

    let (_pid, recovery) = row;
    let expected = hmac_sha256(&recovery.code_salt, body.code.as_bytes());

    use subtle::ConstantTimeEq;
    if expected.ct_eq(&recovery.code_verifier).unwrap_u8() == 0 {
        return Err(ApiError::BadRequest("Invalid recovery code".into()));
    }

    Ok(Json(RecoveryValidateResp {
        code_salt: hex::encode(&recovery.code_salt),
    }))
}

// ---------------------------------------------------------------------------
// Principal-level Recovery Key (PRK)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct StoreRecoveryKeyReq {
    /// Hex-encoded PBKDF2 salt for the recovery-code KEK.
    pub code_salt: String,
    /// PRK wrapped under PBKDF2(recovery_code), base64url.
    pub prk_code_ct: String,
    pub prk_code_nonce: String,
    /// PRK wrapped under the passkey PRF-derived key, base64url.
    pub prk_prf_ct: String,
    pub prk_prf_nonce: String,
}

/// Store the principal's Recovery Key wraps (insert-once). Called once, in the
/// browser, right after the first passkey is registered.
pub async fn store_recovery_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<StoreRecoveryKeyReq>,
) -> ApiResult<StatusCode> {
    let pid = current_principal(&state, &headers).await?;
    let code_salt = hex::decode(&body.code_salt)
        .map_err(|_| ApiError::BadRequest("code_salt must be hex".into()))?;
    let prk_code_ct = b64url_decode(&body.prk_code_ct)?;
    let prk_code_nonce = b64url_decode(&body.prk_code_nonce)?;
    let prk_prf_ct = b64url_decode(&body.prk_prf_ct)?;
    let prk_prf_nonce = b64url_decode(&body.prk_prf_nonce)?;

    db::upsert_principal_recovery_key(
        &state.pool,
        pid.as_uuid(),
        &code_salt,
        &prk_code_ct,
        &prk_code_nonce,
        &prk_prf_ct,
        &prk_prf_nonce,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub struct RecoveryKeyView {
    pub configured: bool,
    /// PRK wrapped under the passkey PRF key (base64url), if configured — lets a
    /// logged-in client recover the PRK to wrap a new Vault DEK.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prk_prf_ct: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prk_prf_nonce: Option<String>,
}

pub async fn get_recovery_key(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<RecoveryKeyView>> {
    let pid = current_principal(&state, &headers).await?;
    let row = db::get_principal_recovery_key(&state.pool, pid.as_uuid()).await?;
    Ok(Json(match row {
        Some(r) => RecoveryKeyView {
            configured: true,
            prk_prf_ct: Some(b64url_encode(&r.prk_prf_ct)),
            prk_prf_nonce: Some(b64url_encode(&r.prk_prf_nonce)),
        },
        None => RecoveryKeyView {
            configured: false,
            prk_prf_ct: None,
            prk_prf_nonce: None,
        },
    }))
}

#[derive(Deserialize)]
pub struct StoreVaultPrkEnvelopeReq {
    /// Vault DEK wrapped under the PRK, base64url.
    pub ciphertext: String,
    pub nonce: String,
}

pub async fn store_vault_prk_envelope(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(vault_id): Path<Uuid>,
    Json(body): Json<StoreVaultPrkEnvelopeReq>,
) -> ApiResult<StatusCode> {
    let pid = current_principal(&state, &headers).await?;
    let vault = db::fetch_vault(&state.pool, VaultId(vault_id)).await?;
    if vault.principal_id != pid {
        return Err(ApiError::NotFound);
    }
    let ct = b64url_decode(&body.ciphertext)?;
    let nonce = b64url_decode(&body.nonce)?;
    db::upsert_vault_prk_envelope(&state.pool, vault_id, pid.as_uuid(), &ct, &nonce).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub struct VaultPrkEnvelopeView {
    pub ciphertext: String,
    pub nonce: String,
}

pub async fn get_vault_prk_envelope(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(vault_id): Path<Uuid>,
) -> ApiResult<Json<VaultPrkEnvelopeView>> {
    let pid = current_principal(&state, &headers).await?;
    let vault = db::fetch_vault(&state.pool, VaultId(vault_id)).await?;
    if vault.principal_id != pid {
        return Err(ApiError::NotFound);
    }
    let (ciphertext, nonce) = db::get_vault_prk_envelope(&state.pool, vault_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(VaultPrkEnvelopeView {
        ciphertext: b64url_encode(&ciphertext),
        nonce: b64url_encode(&nonce),
    }))
}

// ---------------------------------------------------------------------------
// Recovery redemption (unauthenticated; gated by the recovery code)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct RecoveryRedeemReq {
    pub email: String,
    pub code: String,
}

#[derive(Serialize)]
pub struct RecoveryLetterView {
    pub id: Uuid,
    pub title: String,
    pub recipient_email: String,
    pub ciphertext: String,
    pub nonce: String,
}

#[derive(Serialize)]
pub struct RecoveryVaultView {
    pub vault_id: Uuid,
    pub name: String,
    /// Vault DEK wrapped under the PRK (base64url).
    pub dek_prk_ct: String,
    pub dek_prk_nonce: String,
    pub letters: Vec<RecoveryLetterView>,
}

#[derive(Serialize)]
pub struct RecoveryRedeemResp {
    /// Hex PBKDF2 salt; with the recovery code, derives the KEK that unwraps PRK.
    pub code_salt: String,
    /// PRK wrapped under PBKDF2(recovery_code), base64url.
    pub prk_code_ct: String,
    pub prk_code_nonce: String,
    pub vaults: Vec<RecoveryVaultView>,
}

/// Redeem a recovery code. The code is the recovery credential; on a
/// constant-time match we return the ciphertext bundle for every Private Vault
/// — all of it wrapped under the PRK, so the Operator returns only bytes it
/// cannot read. The client derives the PRK from the code and decrypts locally.
/// No session is minted: the code grants letter recovery, not account control.
pub async fn recovery_redeem(
    State(state): State<AppState>,
    Json(body): Json<RecoveryRedeemReq>,
) -> ApiResult<Json<RecoveryRedeemResp>> {
    let (pid, recovery) = db::get_principal_recovery_by_email(&state.pool, &body.email)
        .await?
        .ok_or_else(|| ApiError::BadRequest("No recovery available for this account".into()))?;

    let expected = hmac_sha256(&recovery.code_salt, body.code.as_bytes());
    use subtle::ConstantTimeEq;
    if expected.ct_eq(&recovery.code_verifier).unwrap_u8() == 0 {
        return Err(ApiError::BadRequest("Invalid recovery code".into()));
    }

    let prk = db::get_principal_recovery_key_by_email(&state.pool, &body.email)
        .await?
        .ok_or_else(|| ApiError::BadRequest("No Recovery Key set up for this account".into()))?;

    let mut vaults = Vec::new();
    for vault in db::list_vaults(&state.pool, PrincipalId(pid)).await? {
        if vault.tier != Tier::ZeroKnowledge {
            continue;
        }
        let Some((dek_ct, dek_nonce)) =
            db::get_vault_prk_envelope(&state.pool, vault.id.as_uuid()).await?
        else {
            continue; // no recovery envelope was ever stored for this Vault
        };
        let mut letters = Vec::new();
        for lm in db::list_letters(&state.pool, vault.id).await? {
            let full = db::fetch_letter_full(&state.pool, lm.id).await?;
            if full.ciphertext.is_empty() {
                continue;
            }
            letters.push(RecoveryLetterView {
                id: lm.id.as_uuid(),
                title: full.title,
                recipient_email: full.recipient_email,
                ciphertext: b64url_encode(&full.ciphertext),
                nonce: b64url_encode(&full.nonce),
            });
        }
        vaults.push(RecoveryVaultView {
            vault_id: vault.id.as_uuid(),
            name: vault.name,
            dek_prk_ct: b64url_encode(&dek_ct),
            dek_prk_nonce: b64url_encode(&dek_nonce),
            letters,
        });
    }

    Ok(Json(RecoveryRedeemResp {
        // The PBKDF2 KEK salt from the Recovery Key (NOT the HMAC verifier salt
        // in `recovery`): the client re-derives the KEK from (code, this salt)
        // to unwrap the PRK.
        code_salt: hex::encode(&prk.code_salt),
        prk_code_ct: b64url_encode(&prk.prk_code_ct),
        prk_code_nonce: b64url_encode(&prk.prk_code_nonce),
        vaults,
    }))
}

// ---------------------------------------------------------------------------
// Passkey management (authenticated)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct PasskeyInfoResp {
    pub id: Uuid,
    pub name: String,
    pub backed_up: bool,
    pub transports: Vec<String>,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

pub async fn list_passkeys(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<PasskeyInfoResp>>> {
    let pid = current_principal(&state, &headers).await?;
    let rows = db::list_passkeys_for_principal(&state.pool, pid.as_uuid()).await?;
    let resp = rows
        .iter()
        .map(|r| PasskeyInfoResp {
            id: r.id,
            name: r.name.clone(),
            backed_up: r.backed_up,
            transports: r.transports.clone(),
            created_at: r.created_at.to_rfc3339(),
            last_used_at: r.last_used_at.map(|t| t.to_rfc3339()),
        })
        .collect();
    Ok(Json(resp))
}

pub async fn delete_passkey(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let pid = current_principal(&state, &headers).await?;

    let count = db::count_passkeys_for_principal(&state.pool, pid.as_uuid()).await?;
    if count <= 1 {
        let zk_count = db::count_zk_vaults_for_principal(&state.pool, pid.as_uuid()).await?;
        if zk_count > 0 {
            return Err(ApiError::Conflict(
                "Cannot delete your only passkey while Private Vaults exist. \
                 Add another passkey or delete your Private Vaults first."
                    .into(),
            ));
        }
    }

    let deleted = db::delete_passkey(&state.pool, id, pid.as_uuid()).await?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

// ---------------------------------------------------------------------------
// ZK envelopes (authenticated)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct StoreZkEnvelopeReq {
    pub passkey_id: Uuid,
    /// Base64url-encoded AES-256-GCM wrapped vault DEK.
    pub ciphertext: String,
    /// Base64url-encoded 12-byte GCM nonce.
    pub nonce: String,
    #[serde(default)]
    pub recovery_ciphertext: Option<String>,
    #[serde(default)]
    pub recovery_nonce: Option<String>,
    /// Hex-encoded 32-byte PBKDF2 salt.
    #[serde(default)]
    pub recovery_salt: Option<String>,
}

#[derive(Serialize)]
pub struct ZkEnvelopeResp {
    pub passkey_id: Uuid,
    pub ciphertext: String,
    pub nonce: String,
    pub created_at: String,
}

pub async fn store_zk_envelope(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(vault_id): Path<Uuid>,
    Json(body): Json<StoreZkEnvelopeReq>,
) -> ApiResult<StatusCode> {
    let pid = current_principal(&state, &headers).await?;

    // Verify vault ownership.
    let vault = db::fetch_vault(&state.pool, VaultId(vault_id)).await?;
    if vault.principal_id != pid {
        return Err(ApiError::NotFound);
    }

    let ciphertext = b64url_decode(&body.ciphertext)?;
    let nonce = b64url_decode(&body.nonce)?;

    db::upsert_zk_envelope(&state.pool, vault_id, body.passkey_id, &ciphertext, &nonce).await?;

    if let (Some(rc), Some(rn), Some(rs)) = (
        &body.recovery_ciphertext,
        &body.recovery_nonce,
        &body.recovery_salt,
    ) {
        let rc_bytes = b64url_decode(rc)?;
        let rn_bytes = b64url_decode(rn)?;
        let rs_bytes = hex::decode(rs)
            .map_err(|_| ApiError::BadRequest("recovery_salt must be hex-encoded".into()))?;

        db::upsert_zk_recovery_envelope(
            &state.pool,
            vault_id,
            pid.as_uuid(),
            &rs_bytes,
            &rc_bytes,
            &rn_bytes,
        )
        .await?;
    }

    Ok(StatusCode::NO_CONTENT)
}

pub async fn get_zk_envelopes(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(vault_id): Path<Uuid>,
) -> ApiResult<Json<Vec<ZkEnvelopeResp>>> {
    let pid = current_principal(&state, &headers).await?;

    let vault = db::fetch_vault(&state.pool, VaultId(vault_id)).await?;
    if vault.principal_id != pid {
        return Err(ApiError::NotFound);
    }

    let rows = db::get_zk_envelopes_for_vault(&state.pool, vault_id).await?;
    let resp = rows
        .iter()
        .map(|r| ZkEnvelopeResp {
            passkey_id: r.passkey_id,
            ciphertext: b64url_encode(&r.ciphertext),
            nonce: b64url_encode(&r.nonce),
            created_at: r.created_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(resp))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn generate_recovery_code() -> String {
    // OsRng (the OS CSPRNG) for long-lived key material — the most defensible
    // source and removes any reseeding argument (ZK crypto audit L4).
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn random_bytes(n: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; n];
    rand::rng().fill_bytes(&mut bytes);
    bytes
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key size");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn b64url_decode(s: &str) -> Result<Vec<u8>, ApiError> {
    URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|_| ApiError::BadRequest("invalid base64url encoding".into()))
}

fn b64url_encode(b: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(b)
}
