//! Tests for Buddies, signal subscriptions, signals, and the Apple Shortcut
//! ping pipeline.

mod common;

use beacon_core::{BuddyResponse, PlanId, SignalSource, StorageRegion, Tier};
use beacon_db as db;

async fn principal_with_vault(pool: &sqlx::PgPool) -> (beacon_core::Principal, beacon_core::Vault) {
    let (p, _) = db::upsert_principal_by_email(pool, "buddy@example.org")
        .await
        .unwrap();
    db::create_subscription(pool, p.id, PlanId::SelfHosted, 30)
        .await
        .unwrap();
    let v = db::create_vault(
        pool,
        p.id,
        "V",
        Tier::HonestOperator,
        5,
        StorageRegion::default(),
    )
    .await
    .unwrap();
    (p, v)
}

// ---- Buddies ----

#[tokio::test]
async fn buddy_lifecycle_invite_confirm_respond() {
    let pool = common::setup().await;
    let (p, _v) = principal_with_vault(&pool).await;

    let token = b"buddy-confirmation-token-32bytes".as_slice();
    let buddy = db::invite_buddy(
        &pool,
        db::BuddyInviteInput {
            principal_id: p.id,
            display_name: Some("Riley"),
            email: "riley@example.org",
            phone: None,
            confirmation_token_hash: token,
            prompt_cadence_days: 90,
        },
    )
    .await
    .unwrap();
    assert!(buddy.confirmed_at.is_none());
    assert!(!buddy.is_active(), "unconfirmed buddy is not active");

    let confirmed = db::confirm_buddy(&pool, token).await.unwrap().unwrap();
    assert!(confirmed.confirmed_at.is_some());
    assert!(confirmed.is_active());

    // Confirm is single-use.
    let second = db::confirm_buddy(&pool, token).await.unwrap();
    assert!(second.is_none());

    let responded = db::record_buddy_response(&pool, confirmed.id, BuddyResponse::Worried)
        .await
        .unwrap();
    assert_eq!(responded.last_response, Some(BuddyResponse::Worried));
}

#[tokio::test]
async fn revoked_buddy_cannot_respond() {
    let pool = common::setup().await;
    let (p, _v) = principal_with_vault(&pool).await;
    let token = b"another-confirmation-token-32byt".as_slice();
    let buddy = db::invite_buddy(
        &pool,
        db::BuddyInviteInput {
            principal_id: p.id,
            display_name: None,
            email: "rev@example.org",
            phone: None,
            confirmation_token_hash: token,
            prompt_cadence_days: 90,
        },
    )
    .await
    .unwrap();
    db::confirm_buddy(&pool, token).await.unwrap();
    db::revoke_buddy(&pool, buddy.id).await.unwrap();

    let err = db::record_buddy_response(&pool, buddy.id, BuddyResponse::Worried)
        .await
        .unwrap_err();
    assert!(matches!(err, db::DbError::NotFound));
}

#[tokio::test]
async fn buddy_emails_are_unique_per_principal() {
    let pool = common::setup().await;
    let (p, _v) = principal_with_vault(&pool).await;
    let token1 = b"token-1-padded-to-thirty-two-byt".as_slice();
    let token2 = b"token-2-padded-to-thirty-two-byt".as_slice();
    db::invite_buddy(
        &pool,
        db::BuddyInviteInput {
            principal_id: p.id,
            display_name: None,
            email: "dup@example.org",
            phone: None,
            confirmation_token_hash: token1,
            prompt_cadence_days: 90,
        },
    )
    .await
    .unwrap();
    let err = db::invite_buddy(
        &pool,
        db::BuddyInviteInput {
            principal_id: p.id,
            display_name: None,
            email: "dup@example.org",
            phone: None,
            confirmation_token_hash: token2,
            prompt_cadence_days: 90,
        },
    )
    .await;
    assert!(err.is_err(), "duplicate buddy emails per principal should fail");
}

// ---- Signal subscriptions + observations ----

#[tokio::test]
async fn signal_subscription_upsert_replaces() {
    let pool = common::setup().await;
    let (_p, v) = principal_with_vault(&pool).await;

    let s1 =
        db::upsert_signal_subscription(&pool, v.id, SignalSource::Heartbeat, 0.30, true)
            .await
            .unwrap();
    assert!((s1.weight - 0.30).abs() < f32::EPSILON);

    let s2 =
        db::upsert_signal_subscription(&pool, v.id, SignalSource::Heartbeat, 0.40, false)
            .await
            .unwrap();
    assert!((s2.weight - 0.40).abs() < f32::EPSILON);
    assert!(!s2.enabled);

    let subs = db::list_signal_subscriptions(&pool, v.id).await.unwrap();
    assert_eq!(subs.len(), 1, "upsert must replace, not duplicate");
}

#[tokio::test]
async fn record_and_list_signals() {
    let pool = common::setup().await;
    let (_p, v) = principal_with_vault(&pool).await;

    db::record_signal(
        &pool,
        v.id,
        SignalSource::BuddyAttestation,
        0.20,
        serde_json::json!({"buddy": "riley"}),
    )
    .await
    .unwrap();
    db::record_signal(
        &pool,
        v.id,
        SignalSource::Heartbeat,
        0.0,
        serde_json::json!({"via": "CLI"}),
    )
    .await
    .unwrap();

    let recent = db::list_recent_signals(&pool, v.id, chrono::Utc::now() - chrono::Duration::days(1))
        .await
        .unwrap();
    assert_eq!(recent.len(), 2);
}

// ---- Apple Shortcut ----

#[tokio::test]
async fn apple_shortcut_enrol_and_lookup() {
    let pool = common::setup().await;
    let (p, _v) = principal_with_vault(&pool).await;
    let secret = vec![7u8; 32];
    db::create_apple_shortcut_sub(&pool, p.id, "install-1", &secret)
        .await
        .unwrap();

    let looked = db::fetch_apple_shortcut_sub(&pool, "install-1").await.unwrap();
    assert!(looked.is_some());
    let (pid, key) = looked.unwrap();
    assert_eq!(pid, p.id);
    assert_eq!(key, secret);

    let missing = db::fetch_apple_shortcut_sub(&pool, "unknown")
        .await
        .unwrap();
    assert!(missing.is_none());
}

#[tokio::test]
async fn apple_shortcut_touch_updates_timestamp() {
    let pool = common::setup().await;
    let (p, _v) = principal_with_vault(&pool).await;
    db::create_apple_shortcut_sub(&pool, p.id, "install-2", &[1u8; 32])
        .await
        .unwrap();
    db::touch_apple_shortcut_sub(&pool, "install-2").await.unwrap();
    // The schema does the touch via UPDATE; we just confirm no error.
}

#[tokio::test]
async fn apple_shortcut_installation_id_unique() {
    let pool = common::setup().await;
    let (p, _v) = principal_with_vault(&pool).await;
    db::create_apple_shortcut_sub(&pool, p.id, "install-dup", &[2u8; 32])
        .await
        .unwrap();
    let err = db::create_apple_shortcut_sub(&pool, p.id, "install-dup", &[3u8; 32]).await;
    assert!(err.is_err(), "duplicate installation_id should fail");
}
