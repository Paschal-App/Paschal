//! HTTP-level checks for auth gating, input validation, and error envelope.

mod common;

use axum::http::StatusCode;
use serde_json::json;

#[tokio::test]
async fn missing_token_returns_401() {
    let app = common::setup().await;
    let (status, body) = common::send(
        &app.router,
        common::req_post("/v1/vaults", json!({"name": "x"}), None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["title"], "unauthorised");
}

#[tokio::test]
async fn bogus_token_returns_401() {
    let app = common::setup().await;
    let (status, _) = common::send(
        &app.router,
        common::req_post("/v1/vaults", json!({"name": "x"}), Some("not-a-real-token")),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn signup_rejects_invalid_email() {
    let app = common::setup().await;
    let (status, body) = common::send(
        &app.router,
        common::req_post("/v1/auth/signup", json!({"email": "no-at-sign"}), None),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["title"], "bad_request");
}

#[tokio::test]
async fn cannot_see_another_principals_vault() {
    let app = common::setup().await;
    let (_, t1) = common::signup_and_get_token(&app.router, "first@example.org").await;
    let (_, t2) = common::signup_and_get_token(&app.router, "second@example.org").await;

    let (_, vault) = common::send(
        &app.router,
        common::req_post("/v1/vaults", json!({"name": "Mine"}), Some(&t1)),
    )
    .await;
    let vid = vault["id"].as_str().unwrap().to_string();

    let (status, _) = common::send(
        &app.router,
        common::req_get(&format!("/v1/vaults/{vid}"), Some(&t2)),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "principal 2 must not see principal 1's vault"
    );
}

#[tokio::test]
async fn zero_knowledge_tier_is_rejected_in_mvp() {
    let app = common::setup().await;
    let (_, token) = common::signup_and_get_token(&app.router, "zk@example.org").await;
    let (status, body) = common::send(
        &app.router,
        common::req_post(
            "/v1/vaults",
            json!({"name": "Secret", "tier": "ZERO_KNOWLEDGE"}),
            Some(&token),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["detail"].as_str().unwrap().contains("Zero-Knowledge"));
}

#[tokio::test]
async fn problem_json_content_type() {
    let app = common::setup().await;
    let resp = tower::ServiceExt::oneshot(
        app.router.clone(),
        common::req_post("/v1/vaults", json!({"name": "x"}), None),
    )
    .await
    .unwrap();
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        content_type.starts_with("application/problem+json"),
        "expected problem+json, got {content_type}"
    );
}

#[tokio::test]
async fn health_endpoint() {
    let app = common::setup().await;
    let (status, body) = common::send(&app.router, common::req_get("/health", None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["service"], "paschal-beacon");
}

#[tokio::test]
async fn heartbeat_records_via_field() {
    let app = common::setup().await;
    let (_, token) = common::signup_and_get_token(&app.router, "hb@example.org").await;
    let (status, body) = common::send(
        &app.router,
        common::req_post("/v1/heartbeats", json!({"via": "WEB"}), Some(&token)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["received_at"].is_string());

    // Invalid via.
    let (status, _) = common::send(
        &app.router,
        common::req_post("/v1/heartbeats", json!({"via": "BOGUS"}), Some(&token)),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
