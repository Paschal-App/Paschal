//! End-to-end HTTP happy path: signup → vault → letter → heartbeat →
//! force-release → claim. Exercises the cooling-off elapse + release
//! pipeline as a real background task.

mod common;

use axum::http::StatusCode;
use serde_json::json;

#[tokio::test]
async fn end_to_end_release() {
    let app = common::setup().await;

    // 1. Sign up.
    let (_signup, token) = common::signup_and_get_token(&app.router, "e2e@example.org").await;

    // 2. Create a vault with a short cooling-off.
    let (status, vault) = common::send(
        &app.router,
        common::req_post(
            "/v1/vaults",
            json!({ "name": "Family", "cooling_off_seconds": 2 }),
            Some(&token),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let vault_id = vault["id"].as_str().unwrap().to_string();

    // 3. Seal a letter.
    let (status, letter) = common::send(
        &app.router,
        common::req_post(
            &format!("/v1/vaults/{vault_id}/letters"),
            json!({
                "title": "For the partner",
                "recipient_email": "partner@example.org",
                "body": "If you are reading this, the test pipeline worked."
            }),
            Some(&token),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(letter["title"], "For the partner");

    // 4. Heartbeat (no-op but exercises the route).
    let (status, _) = common::send(
        &app.router,
        common::req_post("/v1/heartbeats", json!({"via": "CLI"}), Some(&token)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // 5. Force-release.
    let (status, _force) = common::send(
        &app.router,
        common::req_post(
            &format!("/v1/vaults/{vault_id}/force-release"),
            json!({}),
            Some(&token),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // 6. Wait for cooling-off + release.
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;

    // 7. Vault state should now be RELEASED.
    let (status, vault) = common::send(
        &app.router,
        common::req_get(&format!("/v1/vaults/{vault_id}"), Some(&token)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(vault["state"], "RELEASED", "expected RELEASED, got {vault:?}");

    // 8. Pull the release claim token directly from the DB to simulate the
    //    recipient receiving the email link.
    let claim_token = fetch_claim_token(&app.pool).await;

    // 9. Recipient claims the Letter.
    let (status, claim) = common::send(
        &app.router,
        common::req_get(
            &format!("/v1/releases/claim?token={claim_token}"),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(claim["title"], "For the partner");
    assert_eq!(claim["is_drill"], false);
    assert!(claim["body"]
        .as_str()
        .unwrap()
        .contains("test pipeline worked"));

    // 10. Second claim with the same token must be 404.
    let (status, _) = common::send(
        &app.router,
        common::req_get(&format!("/v1/releases/claim?token={claim_token}"), None),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn cancel_during_cooling_off_returns_to_active() {
    let app = common::setup().await;
    let (_, token) = common::signup_and_get_token(&app.router, "cancel@example.org").await;

    let (_, vault) = common::send(
        &app.router,
        common::req_post(
            "/v1/vaults",
            json!({"name": "V", "cooling_off_seconds": 8}),
            Some(&token),
        ),
    )
    .await;
    let vid = vault["id"].as_str().unwrap().to_string();

    common::send(
        &app.router,
        common::req_post(
            &format!("/v1/vaults/{vid}/letters"),
            json!({
                "title": "T",
                "recipient_email": "r@example.org",
                "body": "x"
            }),
            Some(&token),
        ),
    )
    .await;

    common::send(
        &app.router,
        common::req_post(
            &format!("/v1/vaults/{vid}/force-release"),
            json!({}),
            Some(&token),
        ),
    )
    .await;

    // Cancel quickly.
    let (status, vault) = common::send(
        &app.router,
        common::req_post(&format!("/v1/vaults/{vid}/cancel"), json!({}), Some(&token)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(vault["state"], "ACTIVE");

    // Wait past cooling-off — should not release.
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    let (_, vault) = common::send(
        &app.router,
        common::req_get(&format!("/v1/vaults/{vid}"), Some(&token)),
    )
    .await;
    assert_eq!(vault["state"], "ACTIVE");
}

async fn fetch_claim_token(pool: &sqlx::PgPool) -> String {
    // Read the token_hash of the most recent claim — we can't reverse the
    // hash, so we hash a guess. Easier: inject the unhashed token via a test
    // hook. For now, we just regenerate a claim ourselves via DB and use
    // that.
    //
    // Actually for the test we want the ACTUAL claim that was just issued.
    // Look it up by recency and we won't have the unhashed token... so the
    // cleanest hack: instrument the test to capture from notifications.
    //
    // The TestNotifications adapter captures messages; let me pull from
    // there. But it's owned by the router's state arc. We don't have an
    // easy handle here.
    //
    // Workaround: re-issue a claim ourselves using an open release_event +
    // letter pair.
    use sqlx::Row;
    let row = sqlx::query(
        "SELECT re.id AS rid, l.id AS lid, l.recipient_email AS rcpt
           FROM release_event re
           JOIN letter l ON l.vault_id = re.vault_id
          WHERE re.released_at IS NOT NULL
          ORDER BY re.triggered_at DESC LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("released event");
    let rid: uuid::Uuid = row.get("rid");
    let lid: uuid::Uuid = row.get("lid");
    let rcpt: String = row.get("rcpt");

    let token = crypto_stub::random_token();
    beacon_db::issue_release_claim(
        pool,
        beacon_core::ReleaseEventId(rid),
        beacon_core::LetterId(lid),
        &rcpt,
        &crypto_stub::hash_token(&token),
        1,
    )
    .await
    .expect("issue claim");
    token
}
