use axum::http::{header, HeaderMap};
use beacon_core::{CoStewardId, PrincipalId};
use beacon_db as db;
use crypto_stub::hash_token;

use crate::{errors::ApiError, state::AppState};

/// Resolve the principal from a `Authorization: Bearer <session>` header.
///
/// A Co-Steward session is NOT accepted here — Co-Stewards can never act as
/// the principal. Use [`current_co_steward`] for the read-only side.
pub async fn current_principal(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<PrincipalId, ApiError> {
    let token = bearer(headers)?;
    let hash = hash_token(token);
    let pid = db::principal_for_session(&state.pool, &hash)
        .await?
        .ok_or(ApiError::Unauthorised)?;
    Ok(pid)
}

/// Resolve a Co-Steward from the bearer token.
///
/// Returns `(co_steward_id, principal_id_they_steward_for)`. The Co-Steward
/// is always allowed to read the principal's dashboard state; they are
/// never allowed to write anything.
pub async fn current_co_steward(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(CoStewardId, PrincipalId), ApiError> {
    let token = bearer(headers)?;
    let hash = hash_token(token);
    let pair = db::co_steward_for_session(&state.pool, &hash)
        .await?
        .ok_or(ApiError::Unauthorised)?;
    Ok(pair)
}

fn bearer(headers: &HeaderMap) -> Result<&str, ApiError> {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .ok_or(ApiError::Unauthorised)?;
    auth.strip_prefix("Bearer ").ok_or(ApiError::Unauthorised)
}
