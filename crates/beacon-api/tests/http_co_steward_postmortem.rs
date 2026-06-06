//! Co-Steward post-mortem powers: updating where a held Letter is delivered,
//! and triggering an EVENT_ON_DEMAND ("wedding") Letter — both gated to a
//! Vault that has RELEASED.

mod common;

use axum::http::StatusCode;
use serde_json::json;

/// Sign up a principal, create a released-able Vault with one normal Letter
/// and one held event Letter, and onboard a confirmed Co-Steward. Returns
/// (principal_token, co_steward_token, vault_id, event_letter_id).
async fn fixture(app: &common::TestApp) -> (String, String, String, String) {
    let (_, token) = common::signup_and_get_token(&app.router, "gran@example.org").await;

    let (_, vault) = common::send(
        &app.router,
        common::req_post(
            "/v1/vaults",
            json!({ "name": "Family", "cooling_off_seconds": 2 }),
            Some(&token),
        ),
    )
    .await;
    let vid = vault["id"].as_str().unwrap().to_string();

    // A normal Letter (delivered on death) ...
    common::send(
        &app.router,
        common::req_post(
            &format!("/v1/vaults/{vid}/letters"),
            json!({ "title": "Passwords", "recipient_email": "exec@example.org", "body": "secrets" }),
            Some(&token),
        ),
    )
    .await;
    // ... and a held event Letter (waits for a deputy to trigger).
    let (_, ev) = common::send(
        &app.router,
        common::req_post(
            &format!("/v1/vaults/{vid}/letters"),
            json!({
                "title": "Read at the wedding",
                "recipient_email": "bride@example.org",
                "body": "Dancing instructions",
                "release_mode": "EVENT_ON_DEMAND"
            }),
            Some(&token),
        ),
    )
    .await;
    let event_letter_id = ev["id"].as_str().unwrap().to_string();

    // Onboard a Co-Steward.
    let (status, invite) = common::send(
        &app.router,
        common::req_post(
            "/v1/principals/me/co-stewards",
            json!({ "email": "deputy@example.org", "display_name": "Daughter" }),
            Some(&token),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "invite failed: {invite:?}");
    let confirm_token = invite["confirmation_token_DEV_ONLY"].as_str().unwrap();
    let (status, confirmed) = common::send(
        &app.router,
        common::req_post(
            "/v1/co-stewards/confirm",
            json!({ "token": confirm_token, "passphrase": "correct horse battery" }),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "confirm failed: {confirmed:?}");
    let co_token = confirmed["session_token"].as_str().unwrap().to_string();

    (token, co_token, vid, event_letter_id)
}

async fn vault_state(app: &common::TestApp, vid: &str, token: &str) -> String {
    let (_, vault) = common::send(
        &app.router,
        common::req_get(&format!("/v1/vaults/{vid}"), Some(token)),
    )
    .await;
    vault["state"].as_str().unwrap_or("").to_string()
}

#[tokio::test]
async fn co_steward_powers_are_locked_until_release() {
    let app = common::setup().await;
    let (_token, co_token, _vid, event_letter_id) = fixture(&app).await;

    // While the principal is alive (Vault ACTIVE), the admin list is empty ...
    let (status, letters) = common::send(
        &app.router,
        common::req_get("/v1/co-stewards/me/letters", Some(&co_token)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(letters.as_array().unwrap().len(), 0, "no released vaults yet");

    // ... and the write powers are refused.
    let (status, _) = common::send(
        &app.router,
        common::req_post(
            &format!("/v1/co-stewards/me/letters/{event_letter_id}/recipient"),
            json!({ "new_email": "x@example.org" }),
            Some(&co_token),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "edit must be locked pre-release");

    let (status, _) = common::send(
        &app.router,
        common::req_post(
            &format!("/v1/co-stewards/me/letters/{event_letter_id}/release"),
            json!({}),
            Some(&co_token),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "trigger must be locked pre-release");
}

#[tokio::test]
async fn co_steward_updates_contact_then_triggers_event_after_release() {
    let app = common::setup().await;
    let (token, co_token, vid, event_letter_id) = fixture(&app).await;

    // The principal dies: force the release and wait it out (2s cooling-off).
    let (status, _) = common::send(
        &app.router,
        common::req_post(&format!("/v1/vaults/{vid}/force-release"), json!({}), Some(&token)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        if vault_state(&app, &vid, &token).await == "RELEASED" {
            break;
        }
    }
    assert_eq!(vault_state(&app, &vid, &token).await, "RELEASED");

    // The held event Letter survived the death release, awaiting a trigger.
    let (_, letters) = common::send(
        &app.router,
        common::req_get("/v1/co-stewards/me/letters", Some(&co_token)),
    )
    .await;
    let ev = letters
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["letter_id"] == event_letter_id)
        .expect("event letter present in admin list");
    assert_eq!(ev["awaiting_event_trigger"], true);
    assert_eq!(ev["event_released"], false);

    // Update where the wedding Letter will be delivered (hold = 1s in tests).
    let (status, change) = common::send(
        &app.router,
        common::req_post(
            &format!("/v1/co-stewards/me/letters/{event_letter_id}/recipient"),
            json!({ "new_email": "bride-married-name@example.org" }),
            Some(&co_token),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "contact update failed: {change:?}");
    assert!(change["change_id"].is_string());

    // Wait out the hold, then trigger the event. The trigger applies the due
    // change lazily, so delivery goes to the new address.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let (status, _) = common::send(
        &app.router,
        common::req_post(
            &format!("/v1/co-stewards/me/letters/{event_letter_id}/release"),
            json!({}),
            Some(&co_token),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "event trigger should succeed");

    // A claim was issued to the UPDATED address.
    let claim_email: (String,) = sqlx::query_as(
        "SELECT recipient_email FROM release_claim WHERE letter_id = $1 ORDER BY issued_at DESC LIMIT 1",
    )
    .bind(uuid::Uuid::parse_str(&event_letter_id).unwrap())
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(
        claim_email.0, "bride-married-name@example.org",
        "event delivery must use the updated contact"
    );

    // The event Letter cannot be triggered twice.
    let (status, _) = common::send(
        &app.router,
        common::req_post(
            &format!("/v1/co-stewards/me/letters/{event_letter_id}/release"),
            json!({}),
            Some(&co_token),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "double trigger must be refused");
}
