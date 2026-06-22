use std::time::Duration;

use axum::{
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    middleware::{from_fn, from_fn_with_state},
    response::Html,
    routing::{get, post},
    Json, Router,
};
use serde_json::json;
use tower::ServiceBuilder;
use tower_http::{
    cors::CorsLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    timeout::TimeoutLayer,
    trace::TraceLayer,
};

use crate::{passkey, rate_limit::RateLimiter, routes, security_headers, state::AppState};

const REQUEST_ID_HEADER: &str = "x-request-id";

pub fn build_router(state: AppState) -> Router {
    let max_request_bytes = state.config.max_request_bytes;
    let limiter = RateLimiter::new(state.config.rate_limit_per_minute);

    // Email-auth routes get a tighter per-IP limit (10/min by default) on top of
    // the global limit, to slow credential-stuffing and magic-link spamming.
    let auth_limiter = RateLimiter::new(state.config.auth_rate_limit_per_minute);
    let auth_routes = {
        let al = auth_limiter.clone();
        Router::new()
            .route("/v1/auth/signup", post(routes::signup))
            .route("/v1/auth/signin", post(routes::signin))
            .route("/v1/auth/signout", post(routes::signout))
            .route(
                "/v1/auth/magic-link/verify",
                post(routes::magic_link_verify),
            )
            .route("/v1/auth/magic-link/poll", post(routes::magic_link_poll))
            .route_layer(from_fn(move |req, next| {
                let al = al.clone();
                async move { crate::rate_limit::apply(&al, req, next).await }
            }))
    };

    Router::new()
        // Health endpoints — no auth.
        .route("/health", get(health))
        .route("/livez", get(livez))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics_endpoint))
        // Auth — email (merged with the dedicated auth rate limit above)
        .merge(auth_routes)
        // Auth — passwordless passkeys (unauthenticated ceremonies)
        .route(
            "/v1/auth/passkey/register/options",
            post(passkey::passkey_register_options),
        )
        .route(
            "/v1/auth/passkey/register/verify",
            post(passkey::passkey_register_verify),
        )
        .route(
            "/v1/auth/passkey/authenticate/options",
            post(passkey::passkey_authenticate_options),
        )
        .route(
            "/v1/auth/passkey/authenticate/verify",
            post(passkey::passkey_authenticate_verify),
        )
        .route(
            "/v1/auth/passkey/recovery/validate",
            post(passkey::passkey_recovery_validate),
        )
        .route(
            "/v1/auth/passkey/recovery/redeem",
            post(passkey::recovery_redeem),
        )
        // Public plan catalog
        .route("/v1/plans", get(routes::list_plans))
        // Public trust endpoints (no auth)
        .route("/v1/public/warrant-canary", get(routes::get_warrant_canary))
        .route(
            "/v1/public/transparency-log",
            get(routes::list_transparency_log),
        )
        // Vaults
        .route(
            "/v1/vaults",
            get(routes::list_vaults).post(routes::create_vault),
        )
        .route(
            "/v1/vaults/:id",
            get(routes::get_vault).patch(routes::update_vault),
        )
        .route("/v1/vaults/:id/region", post(routes::move_vault_region))
        .route(
            "/v1/vaults/:id/letters",
            get(routes::list_letters).post(routes::seal_letter),
        )
        .route(
            "/v1/vaults/:id/letters/multipart",
            post(routes::seal_letter_multipart),
        )
        .route(
            "/v1/vaults/:vault_id/letters/:letter_id/export",
            post(routes::export_letter),
        )
        // Letter detail / edit / delete
        .route(
            "/v1/vaults/:vault_id/letters/:letter_id",
            get(routes::get_letter)
                .patch(routes::update_letter)
                .delete(routes::delete_letter_handler),
        )
        // Private (Zero-Knowledge) letters: browser-encrypted seal + read-back.
        .route("/v1/vaults/:id/letters/zk", post(routes::seal_zk_letter))
        .route(
            "/v1/vaults/:vault_id/letters/:letter_id/ciphertext",
            get(routes::get_zk_letter_ciphertext),
        )
        // ZK key envelopes (passkey-PRF-wrapped vault DEK) + per-vault PRK envelope
        .route(
            "/v1/vaults/:id/zk-envelope",
            get(passkey::get_zk_envelopes).post(passkey::store_zk_envelope),
        )
        .route(
            "/v1/vaults/:id/prk-envelope",
            get(passkey::get_vault_prk_envelope).post(passkey::store_vault_prk_envelope),
        )
        .route("/v1/vaults/:id/force-release", post(routes::force_release))
        .route("/v1/vaults/:id/cancel", post(routes::cancel_release))
        .route("/v1/vaults/:id/drills", post(routes::run_drill))
        .route(
            "/v1/vaults/:id/signal-subscriptions",
            get(routes::list_signal_subscriptions).post(routes::upsert_signal_subscription),
        )
        // Heartbeat
        .route("/v1/heartbeats", post(routes::post_heartbeat))
        // Apple iCloud Shortcut signal
        .route(
            "/v1/signals/apple-icloud/enrol",
            post(routes::apple_icloud_enrol),
        )
        .route(
            "/v1/signals/apple-icloud/ping",
            post(routes::apple_icloud_ping),
        )
        // Microsoft Account signal
        .route(
            "/v1/signals/microsoft/observe",
            post(routes::microsoft_observe),
        )
        // Co-Stewards (read-only family deputies)
        .route(
            "/v1/principals/me/co-stewards",
            get(routes::list_co_stewards).post(routes::invite_co_steward),
        )
        .route(
            "/v1/co-stewards/:id",
            axum::routing::delete(routes::revoke_co_steward),
        )
        .route("/v1/co-stewards/confirm", post(routes::confirm_co_steward))
        .route("/v1/co-stewards/sign-in", post(routes::sign_in_co_steward))
        .route(
            "/v1/co-stewards/me/dashboard",
            get(routes::co_steward_dashboard),
        )
        .route(
            "/v1/co-stewards/me/letters",
            get(routes::co_steward_admin_letters),
        )
        .route(
            "/v1/co-stewards/me/letters/:id/recipient",
            post(routes::co_steward_update_recipient),
        )
        .route(
            "/v1/co-stewards/me/letters/:id/release",
            post(routes::co_steward_release_event_letter),
        )
        .route(
            "/v1/co-stewards/me/recipient-changes/:id/cancel",
            post(routes::co_steward_cancel_recipient_change),
        )
        // Buddies
        .route(
            "/v1/principals/me/buddies",
            get(routes::list_buddies).post(routes::invite_buddy),
        )
        .route(
            "/v1/buddies/:id",
            axum::routing::delete(routes::revoke_buddy),
        )
        .route("/v1/buddies/confirm", post(routes::confirm_buddy))
        .route("/v1/buddies/:id/responses", post(routes::respond_buddy))
        // Subscription (read-only in the self-hosted edition — no billing)
        .route(
            "/v1/principals/me/subscription",
            get(routes::get_subscription),
        )
        .route("/v1/principals/me/usage", get(routes::get_usage))
        .route("/v1/principals/me/activity", get(routes::get_activity))
        // Passkeys (manage registered devices) + per-principal Recovery Key
        .route("/v1/principals/me/passkeys", get(passkey::list_passkeys))
        .route(
            "/v1/principals/me/passkeys/:id",
            axum::routing::delete(passkey::delete_passkey),
        )
        .route(
            "/v1/principals/me/recovery-key",
            get(passkey::get_recovery_key).post(passkey::store_recovery_key),
        )
        // Account deletion
        .route(
            "/v1/principals/me",
            axum::routing::delete(routes::request_account_deletion),
        )
        .route(
            "/v1/principals/me/cancel-deletion",
            post(routes::cancel_account_deletion),
        )
        // Releases — recipient claim
        .route("/v1/releases/claim", get(routes::claim_release))
        .route(
            "/v1/releases/claim/attachment",
            get(routes::claim_attachment),
        )
        // Vault contacts
        .route(
            "/v1/vaults/:id/contacts",
            get(routes::list_vault_contacts).post(routes::create_vault_contact),
        )
        .route(
            "/v1/vaults/:id/contacts/:cid",
            axum::routing::put(routes::update_vault_contact).delete(routes::delete_vault_contact),
        )
        // Bank dormancy signal
        .route(
            "/v1/principals/me/bank-signal",
            get(routes::get_bank_signal_status)
                .post(routes::bank_dormancy_enrol)
                .delete(routes::revoke_bank_signal),
        )
        .route(
            "/v1/signals/bank-dormancy/:webhook_id",
            post(routes::bank_dormancy_webhook),
        )
        // Duress signal (covert panic webhook + arm/disarm)
        .route(
            "/v1/principals/me/duress",
            get(routes::get_duress)
                .post(routes::arm_duress)
                .delete(routes::revoke_duress),
        )
        .route(
            "/v1/signals/duress/:webhook_id",
            post(routes::duress_webhook),
        )
        // Heir UI page (recipient claim)
        .route("/claim", get(heir_page))
        // Principal SPA (SvelteKit). Mounted at /app/*.
        .route("/", get(root_redirect))
        .route("/app", get(crate::spa::serve))
        .route("/app/", get(crate::spa::serve))
        .route("/app/*rest", get(crate::spa::serve_with_path))
        .layer(
            ServiceBuilder::new()
                .layer(SetRequestIdLayer::new(
                    REQUEST_ID_HEADER.parse().unwrap(),
                    MakeRequestUuid,
                ))
                .layer(TraceLayer::new_for_http())
                .layer(PropagateRequestIdLayer::new(
                    REQUEST_ID_HEADER.parse().unwrap(),
                ))
                .layer(TimeoutLayer::new(Duration::from_secs(60)))
                .layer(CorsLayer::permissive())
                .layer(DefaultBodyLimit::max(max_request_bytes))
                .layer(from_fn(security_headers::middleware))
                .layer(from_fn_with_state(limiter, crate::rate_limit::middleware)),
        )
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({
        "status": "ok",
        "service": "paschal-beacon",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn livez() -> StatusCode {
    StatusCode::OK
}

async fn readyz(State(state): State<AppState>) -> (StatusCode, Json<serde_json::Value>) {
    match sqlx::query("SELECT 1").fetch_one(&state.pool).await {
        Ok(_) => (
            StatusCode::OK,
            Json(json!({ "ready": true, "checks": { "db": "ok" } })),
        ),
        Err(e) => {
            tracing::warn!(error = ?e, "readyz: DB check failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "ready": false, "checks": { "db": "error" } })),
            )
        }
    }
}

async fn heir_page() -> Html<&'static str> {
    Html(include_str!("../../../apps/heir/claim.html"))
}

async fn root_redirect() -> axum::response::Redirect {
    axum::response::Redirect::permanent("/app/")
}

async fn metrics_endpoint(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    let body = state.metrics.prometheus();
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
}
