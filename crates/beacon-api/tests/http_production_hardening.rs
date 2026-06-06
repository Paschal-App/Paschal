//! Tests for production-hardening features: rate limiting, security headers,
//! account deletion, /readyz, /livez, /metrics, Microsoft observe.

mod common;

use axum::http::StatusCode;
use serde_json::json;

#[tokio::test]
async fn livez_always_ok() {
    let app = common::setup().await;
    let (status, _) = common::send(&app.router, common::req_get("/livez", None)).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn readyz_ok_when_db_up() {
    let app = common::setup().await;
    let (status, body) = common::send(&app.router, common::req_get("/readyz", None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ready"], true);
    assert_eq!(body["checks"]["db"], "ok");
}

#[tokio::test]
async fn metrics_endpoint_emits_prometheus_format() {
    let app = common::setup().await;
    let (_, _token) = common::signup_and_get_token(&app.router, "metrics@example.org").await;
    let resp = tower::ServiceExt::oneshot(
        app.router.clone(),
        common::req_get("/metrics", None),
    )
    .await
    .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let text = std::str::from_utf8(&body).unwrap();
    assert!(text.contains("# HELP paschal_signups_total"));
    assert!(text.contains("paschal_signups_total 1"));
}

#[tokio::test]
async fn security_headers_are_present() {
    let app = common::setup().await;
    let resp = tower::ServiceExt::oneshot(app.router.clone(), common::req_get("/health", None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let h = resp.headers();
    assert_eq!(h.get("x-content-type-options").unwrap(), "nosniff");
    assert_eq!(h.get("x-frame-options").unwrap(), "DENY");
    assert!(h.get("content-security-policy").is_some());
    assert!(h.get("permissions-policy").is_some());
    assert_eq!(h.get("referrer-policy").unwrap(), "no-referrer");
    assert!(h.get("strict-transport-security").is_some());
    assert!(h.get("cache-control").unwrap().to_str().unwrap().contains("no-store"));
}

#[tokio::test]
async fn account_deletion_full_lifecycle() {
    let app = common::setup().await;
    let (_, token) = common::signup_and_get_token(&app.router, "delete@example.org").await;

    // Request deletion.
    let resp = tower::ServiceExt::oneshot(
        app.router.clone(),
        common::req_delete("/v1/principals/me", Some(&token)),
    )
    .await
    .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    // Subsequent call to status should still work (account exists, just pending).
    let (status, _) = common::send(
        &app.router,
        common::req_get("/v1/vaults", Some(&token)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Cancel deletion.
    let (status, body) = common::send(
        &app.router,
        common::req_post(
            "/v1/principals/me/cancel-deletion",
            json!({}),
            Some(&token),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body:?}");
    assert_eq!(body["cancelled"], true);
}

#[tokio::test]
async fn cancel_deletion_404_when_no_pending_request() {
    let app = common::setup().await;
    let (_, token) = common::signup_and_get_token(&app.router, "nodel@example.org").await;
    let (status, _) = common::send(
        &app.router,
        common::req_post(
            "/v1/principals/me/cancel-deletion",
            json!({}),
            Some(&token),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn microsoft_observe_records_signal() {
    let app = common::setup().await;
    let (_, token) = common::signup_and_get_token(&app.router, "ms@example.org").await;

    // Need a vault for the signal to attach to.
    let (_, vault) = common::send(
        &app.router,
        common::req_post("/v1/vaults", json!({ "name": "V" }), Some(&token)),
    )
    .await;
    let _vid = vault["id"].as_str().unwrap();

    // Observe a fresh sign-in.
    let (status, body) = common::send(
        &app.router,
        common::req_post(
            "/v1/signals/microsoft/observe",
            json!({ "last_sign_in_at": chrono::Utc::now().to_rfc3339() }),
            Some(&token),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body:?}");
    assert_eq!(body["vaults_affected"], 1);
    // Fresh sign-in → contribution ~ 0.
    let c = body["contribution"].as_f64().unwrap();
    assert!(c < 0.01, "fresh observation should have ~0 contribution, got {c}");
}

#[tokio::test]
async fn microsoft_observe_rejects_future_timestamps() {
    let app = common::setup().await;
    let (_, token) = common::signup_and_get_token(&app.router, "msfuture@example.org").await;
    let future = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
    let (status, _) = common::send(
        &app.router,
        common::req_post(
            "/v1/signals/microsoft/observe",
            json!({ "last_sign_in_at": future }),
            Some(&token),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn openapi_yaml_served() {
    let app = common::setup().await;
    let resp =
        tower::ServiceExt::oneshot(app.router.clone(), common::req_get("/openapi.yaml", None))
            .await
            .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let text = std::str::from_utf8(&body).unwrap();
    assert!(text.contains("openapi: 3.1.0"));
    assert!(text.contains("Paschal Beacon API"));
}

#[tokio::test]
async fn docs_page_served() {
    let app = common::setup().await;
    let resp = tower::ServiceExt::oneshot(app.router.clone(), common::req_get("/docs", None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
