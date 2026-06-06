//! Tests for per-vault attachment storage region (spec 05 "Attachment storage
//! region", spec 12 §1 gating). Only Estate+/Legacy may choose a non-default
//! region or move a Vault between regions.

mod common;

use axum::http::StatusCode;
use serde_json::json;

/// Force a principal's subscription onto a given plan. Signup created an Estate
/// subscription; we repoint it so we can exercise the multi-region feature gate.
async fn set_plan(pool: &sqlx::PgPool, email: &str, plan_db_str: &str) {
    sqlx::query(
        "UPDATE subscription s SET plan_id = $1
           FROM principal p
          WHERE p.id = s.principal_id AND p.email = $2",
    )
    .bind(plan_db_str)
    .bind(email)
    .execute(pool)
    .await
    .expect("set plan");
}

#[tokio::test]
async fn default_plan_cannot_choose_non_default_region() {
    let app = common::setup().await;
    let (_, token) = common::signup_and_get_token(&app.router, "region-gate@example.org").await;

    let (status, body) = common::send(
        &app.router,
        common::req_post(
            "/v1/vaults",
            json!({"name": "EU?", "cooling_off_seconds": 1, "storage_region": "eu-central-1"}),
            Some(&token),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "body: {body:?}");
}

#[tokio::test]
async fn default_region_is_allowed_on_any_plan() {
    let app = common::setup().await;
    let (_, token) = common::signup_and_get_token(&app.router, "region-default@example.org").await;

    let (status, body) = common::send(
        &app.router,
        common::req_post(
            "/v1/vaults",
            json!({"name": "AU", "cooling_off_seconds": 1, "storage_region": "ap-southeast-2"}),
            Some(&token),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    assert_eq!(body["storage_region"], "ap-southeast-2");
}

#[tokio::test]
async fn estate_plus_can_create_and_move_region() {
    let app = common::setup().await;
    let email = "region-plus@example.org";
    let (_, token) = common::signup_and_get_token(&app.router, email).await;
    set_plan(&app.pool, email, "estate_plus_monthly_v1").await;

    // Create directly in the EU.
    let (status, vault) = common::send(
        &app.router,
        common::req_post(
            "/v1/vaults",
            json!({"name": "EU vault", "cooling_off_seconds": 1, "storage_region": "eu-central-1"}),
            Some(&token),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {vault:?}");
    assert_eq!(vault["storage_region"], "eu-central-1");
    let vid = vault["id"].as_str().unwrap();

    // Move it to the US.
    let (status, moved) = common::send(
        &app.router,
        common::req_post(
            &format!("/v1/vaults/{vid}/region"),
            json!({"storage_region": "us-east-1"}),
            Some(&token),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {moved:?}");
    assert_eq!(moved["storage_region"], "us-east-1");
}

#[tokio::test]
async fn default_plan_cannot_move_region() {
    let app = common::setup().await;
    let (_, token) = common::signup_and_get_token(&app.router, "region-nomove@example.org").await;

    let (_, vault) = common::send(
        &app.router,
        common::req_post(
            "/v1/vaults",
            json!({"name": "AU vault", "cooling_off_seconds": 1}),
            Some(&token),
        ),
    )
    .await;
    let vid = vault["id"].as_str().unwrap();

    let (status, body) = common::send(
        &app.router,
        common::req_post(
            &format!("/v1/vaults/{vid}/region"),
            json!({"storage_region": "us-east-1"}),
            Some(&token),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "body: {body:?}");
}
