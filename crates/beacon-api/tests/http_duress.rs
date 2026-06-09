//! Duress signal — covert, user-armed panic webhook that freezes release.

mod common;

use axum::http::StatusCode;
use serde_json::json;

#[tokio::test]
async fn arm_then_trigger_freezes_and_is_covert() {
    let app = common::setup().await;
    let (_, token) = common::signup_and_get_token(&app.router, "duress1@example.com").await;

    // Arm with a trusted-contact alert email.
    let (status, view) = common::send(
        &app.router,
        common::req_post(
            "/v1/principals/me/duress",
            json!({ "alert_email": "trusted@example.com" }),
            Some(&token),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{view:?}");
    assert_eq!(view["armed"], true);
    assert!(view["triggered_at"].is_null(), "not yet triggered");
    let url = view["webhook_url"].as_str().unwrap().to_string();
    assert!(url.contains("/v1/signals/duress/"));
    let webhook_id = url.rsplit('/').next().unwrap().to_string();

    // Public covert trigger — no auth, always 200.
    let (status, _) = common::send(
        &app.router,
        common::req_post(&format!("/v1/signals/duress/{webhook_id}"), json!({}), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Now armed + triggered: the dead-man's switch is frozen until disarmed.
    let (_, view) = common::send(
        &app.router,
        common::req_get("/v1/principals/me/duress", Some(&token)),
    )
    .await;
    assert_eq!(view["armed"], true);
    // triggered_at set == the flag the aggregator reads to freeze release.
    assert!(
        view["triggered_at"].is_string(),
        "should be frozen: {view:?}"
    );
}

#[tokio::test]
async fn unknown_webhook_is_covert_200() {
    let app = common::setup().await;
    // A random webhook id must still return 200 so an observer can't probe it.
    let (status, _) = common::send(
        &app.router,
        common::req_post(
            "/v1/signals/duress/00000000-0000-0000-0000-000000000000",
            json!({}),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn disarm_clears_duress() {
    let app = common::setup().await;
    let (_, token) = common::signup_and_get_token(&app.router, "duress2@example.com").await;
    common::send(
        &app.router,
        common::req_post("/v1/principals/me/duress", json!({}), Some(&token)),
    )
    .await;
    let (status, _) = common::send(
        &app.router,
        common::req_delete("/v1/principals/me/duress", Some(&token)),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, view) = common::send(
        &app.router,
        common::req_get("/v1/principals/me/duress", Some(&token)),
    )
    .await;
    assert_eq!(view["armed"], false);
}
