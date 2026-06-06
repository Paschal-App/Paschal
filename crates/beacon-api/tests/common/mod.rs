#![allow(dead_code)] // each test binary uses a different subset of these helpers

//! Test harness for beacon-api integration tests.
//!
//! Builds an in-process `Router` against a clean test database. Each test
//! gets its own temporary KMS-key file so encryption is hermetic.

use std::sync::Arc;

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use beacon_api::{app, state::AppState};
use crypto_stub::LocalFileKms;
use sqlx::PgPool;
use tempfile::TempDir;
use tower::ServiceExt;

pub fn test_database_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://paschal:paschal@localhost:5432/paschal_test".into())
}

pub struct TestApp {
    pub router: Router,
    pub pool: PgPool,
    pub _kms_dir: TempDir,
}

pub async fn setup() -> TestApp {
    let pool = beacon_db::connect(&test_database_url())
        .await
        .expect("connect");
    truncate_all(&pool).await;

    let kms_dir = TempDir::new().expect("tempdir");
    let kms = Arc::new(LocalFileKms::new(kms_dir.path().join("kms.key")));

    let state = AppState::for_tests(pool.clone(), kms);
    let router = app::build_router(state);
    TestApp {
        router,
        pool,
        _kms_dir: kms_dir,
    }
}

pub async fn truncate_all(pool: &PgPool) {
    sqlx::query(
        "TRUNCATE TABLE
            release_claim,
            release_event,
            signal,
            signal_subscription,
            apple_shortcut_subscription,
            heartbeat,
            letter,
            vault,
            transparency_entry,
            magic_link,
            session,
            auth_identity,
            buddy,
            subscription,
            principal
         RESTART IDENTITY CASCADE",
    )
    .execute(pool)
    .await
    .expect("truncate all");
}

pub async fn send(router: &Router, req: Request<Body>) -> (StatusCode, serde_json::Value) {
    let resp = router.clone().oneshot(req).await.expect("oneshot");
    let status = resp.status();
    let body_bytes = to_bytes(resp.into_body(), 4 * 1024 * 1024)
        .await
        .expect("collect body");
    let body = if body_bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&body_bytes).unwrap_or_else(
            |_| serde_json::json!({ "raw": String::from_utf8_lossy(&body_bytes).to_string() }),
        )
    };
    (status, body)
}

pub fn req_post(path: &str, body: serde_json::Value, token: Option<&str>) -> Request<Body> {
    let mut b = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json");
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    b.body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

pub fn req_get(path: &str, token: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().method("GET").uri(path);
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    b.body(Body::empty()).unwrap()
}

pub fn req_delete(path: &str, token: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().method("DELETE").uri(path);
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    b.body(Body::empty()).unwrap()
}

/// Sign up a fresh principal and return the bearer token.
pub async fn signup_and_get_token(router: &Router, email: &str) -> (serde_json::Value, String) {
    let (status, body) = send(
        router,
        req_post(
            "/v1/auth/signup",
            serde_json::json!({ "email": email, "plan": "monthly" }),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "signup failed: {body:?}");
    let token = body["session_token"].as_str().unwrap().to_string();
    (body, token)
}
