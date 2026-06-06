//! Read-only subscription view + Drill mode end-to-end.
//!
//! The self-hosted edition has a single free plan and no billing, so the
//! cancel/reactivate/checkout/portal flows have been removed.

mod common;

use axum::http::StatusCode;
use serde_json::json;

#[tokio::test]
async fn subscription_is_readable() {
    let app = common::setup().await;
    let (_, token) = common::signup_and_get_token(&app.router, "sub@example.org").await;

    let (status, sub) = common::send(
        &app.router,
        common::req_get("/v1/principals/me/subscription", Some(&token)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(sub["plan_id"], "self_hosted");
}

#[tokio::test]
async fn drill_completes_and_returns_vault_to_active() {
    let app = common::setup().await;
    let (_, token) = common::signup_and_get_token(&app.router, "drill@example.org").await;

    let (_, vault) = common::send(
        &app.router,
        common::req_post(
            "/v1/vaults",
            json!({"name": "V", "cooling_off_seconds": 2}),
            Some(&token),
        ),
    )
    .await;
    let vid = vault["id"].as_str().unwrap().to_string();

    // Seal a letter with a drill body.
    common::send(
        &app.router,
        common::req_post(
            &format!("/v1/vaults/{vid}/letters"),
            json!({
                "title": "Letter",
                "recipient_email": "r@example.org",
                "body": "Real content",
                "drill_body": "Rehearsal content"
            }),
            Some(&token),
        ),
    )
    .await;

    // Trigger drill.
    let (status, _) = common::send(
        &app.router,
        common::req_post(&format!("/v1/vaults/{vid}/drills"), json!({}), Some(&token)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Wait out the drill cooling-off (3s in scheduler) + some.
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;

    // After a drill the vault must return to ACTIVE.
    let (_, vault) = common::send(
        &app.router,
        common::req_get(&format!("/v1/vaults/{vid}"), Some(&token)),
    )
    .await;
    assert_eq!(
        vault["state"], "ACTIVE",
        "drill should return vault to ACTIVE, got {vault:?}"
    );

    // A claim should have been issued — pull from DB and confirm is_drill=true.
    let row: (uuid::Uuid, bool) = sqlx::query_as(
        "SELECT re.id, re.is_drill FROM release_event re
          ORDER BY re.triggered_at DESC LIMIT 1",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert!(row.1, "expected most-recent release_event.is_drill = true");
}

#[tokio::test]
async fn buddy_lifecycle_via_http() {
    let app = common::setup().await;
    let (_, token) = common::signup_and_get_token(&app.router, "buddy-http@example.org").await;

    // Invite
    let (status, body) = common::send(
        &app.router,
        common::req_post(
            "/v1/principals/me/buddies",
            json!({
                "email": "riley@example.org",
                "display_name": "Riley",
                "prompt_cadence_days": 30
            }),
            Some(&token),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let buddy_id = body["buddy"]["id"].as_str().unwrap().to_string();
    let confirmation = body["confirmation_token_DEV_ONLY"]
        .as_str()
        .unwrap()
        .to_string();

    // List buddies
    let (status, list) = common::send(
        &app.router,
        common::req_get("/v1/principals/me/buddies", Some(&token)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 1);

    // Confirm
    let (status, buddy) = common::send(
        &app.router,
        common::req_post("/v1/buddies/confirm", json!({"token": confirmation}), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(buddy["confirmed"], true);

    // Respond WORRIED
    let (status, buddy) = common::send(
        &app.router,
        common::req_post(
            &format!("/v1/buddies/{buddy_id}/responses"),
            json!({"response": "WORRIED"}),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(buddy["last_response"], "WORRIED");

    // Revoke (auth required)
    let (status, _) = common::send(
        &app.router,
        common::req_delete(&format!("/v1/buddies/{buddy_id}"), Some(&token)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}
