//! Tests for Principal, Subscription, and Session repositories.

mod common;

use beacon_core::PlanId;
use beacon_db as db;

#[tokio::test]
async fn upsert_principal_returns_existing_on_repeat() {
    let pool = common::setup().await;
    let (a, created_a) = db::upsert_principal_by_email(&pool, "sam@example.org")
        .await
        .unwrap();
    assert!(created_a);
    let (b, created_b) = db::upsert_principal_by_email(&pool, "sam@example.org")
        .await
        .unwrap();
    assert!(!created_b);
    assert_eq!(a.id, b.id);
}

#[tokio::test]
async fn fetch_principal_round_trips() {
    let pool = common::setup().await;
    let (p, _) = db::upsert_principal_by_email(&pool, "fetch@example.org")
        .await
        .unwrap();
    let got = db::fetch_principal(&pool, p.id).await.unwrap();
    assert_eq!(got.id, p.id);
    assert_eq!(got.primary_email, "fetch@example.org");
}

#[tokio::test]
async fn fetch_unknown_principal_is_not_found() {
    let pool = common::setup().await;
    let bogus = beacon_core::PrincipalId::new();
    let err = db::fetch_principal(&pool, bogus).await.unwrap_err();
    assert!(matches!(err, db::DbError::NotFound));
}

#[tokio::test]
async fn session_only_resolves_when_unrevoked_and_unexpired() {
    let pool = common::setup().await;
    let (p, _) = db::upsert_principal_by_email(&pool, "session@example.org")
        .await
        .unwrap();

    let hash = b"deadbeef-32-bytes-of-something-12";
    db::create_session(&pool, p.id, hash, 1).await.unwrap();

    let resolved = db::principal_for_session(&pool, hash).await.unwrap();
    assert_eq!(resolved, Some(p.id));

    // Unknown token returns None, not an error.
    let bogus = db::principal_for_session(&pool, b"nope").await.unwrap();
    assert!(bogus.is_none());
}

#[tokio::test]
async fn magic_link_is_single_use() {
    let pool = common::setup().await;
    let (p, _) = db::upsert_principal_by_email(&pool, "magic@example.org")
        .await
        .unwrap();
    let hash = b"x".repeat(32);
    db::create_magic_link(&pool, p.id, &hash, 15).await.unwrap();
    let first = db::consume_magic_link(&pool, &hash).await.unwrap();
    assert_eq!(first, Some(p.id));
    let second = db::consume_magic_link(&pool, &hash).await.unwrap();
    assert!(second.is_none(), "magic link must be single-use");
}

#[tokio::test]
async fn subscription_starts_trialing_and_can_be_cancelled() {
    let pool = common::setup().await;
    let (p, _) = db::upsert_principal_by_email(&pool, "sub@example.org")
        .await
        .unwrap();
    let sub = db::create_subscription(&pool, p.id, PlanId::SelfHosted, 30)
        .await
        .unwrap();
    assert_eq!(sub.state, beacon_core::SubscriptionState::Trialing);
    assert!(sub.trial_end_at.is_some());

    let cancelled = db::cancel_subscription(&pool, p.id, 1095).await.unwrap();
    assert_eq!(cancelled.state, beacon_core::SubscriptionState::Canceled);
    assert!(cancelled.retention_until.is_some());
}

#[tokio::test]
async fn cancel_then_reactivate_works_during_retention() {
    let pool = common::setup().await;
    let (p, _) = db::upsert_principal_by_email(&pool, "reactivate@example.org")
        .await
        .unwrap();
    db::create_subscription(&pool, p.id, PlanId::SelfHosted, 30)
        .await
        .unwrap();
    db::cancel_subscription(&pool, p.id, 1095).await.unwrap();

    let reactivated = db::reactivate_subscription(&pool, p.id).await.unwrap();
    assert_eq!(reactivated.state, beacon_core::SubscriptionState::Active);
    assert!(reactivated.canceled_at.is_none());
    assert!(reactivated.retention_until.is_none());
}

#[tokio::test]
async fn reactivate_fails_when_not_canceled() {
    let pool = common::setup().await;
    let (p, _) = db::upsert_principal_by_email(&pool, "noreactivate@example.org")
        .await
        .unwrap();
    db::create_subscription(&pool, p.id, PlanId::SelfHosted, 30)
        .await
        .unwrap();
    // No cancel.
    let err = db::reactivate_subscription(&pool, p.id).await.unwrap_err();
    assert!(matches!(err, db::DbError::NotFound));
}

#[tokio::test]
async fn retention_sweep_finds_due_subscriptions() {
    let pool = common::setup().await;
    let (p, _) = db::upsert_principal_by_email(&pool, "expired@example.org")
        .await
        .unwrap();
    db::create_subscription(&pool, p.id, PlanId::SelfHosted, 30)
        .await
        .unwrap();
    // Cancel with negative retention so the retention window is already past.
    db::cancel_subscription(&pool, p.id, -1).await.unwrap();

    let due = db::list_canceled_past_retention(&pool).await.unwrap();
    assert!(due.contains(&p.id), "expected {:?} to be due, got {:?}", p.id, due);

    db::expire_subscription(&pool, p.id).await.unwrap();
    let sub = db::fetch_subscription(&pool, p.id).await.unwrap();
    assert_eq!(sub.state, beacon_core::SubscriptionState::Expired);
}
