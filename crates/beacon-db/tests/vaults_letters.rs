//! Tests for Vault, Letter, and Heartbeat repositories.

mod common;

use beacon_core::{PlanId, ReleaseReason, StorageRegion, Tier, VaultState};
use beacon_db as db;
use chrono::{Duration as ChronoDuration, Utc};

async fn principal_with_vault(pool: &sqlx::PgPool) -> (beacon_core::Principal, beacon_core::Vault) {
    let (p, _) = db::upsert_principal_by_email(pool, "vault@example.org")
        .await
        .unwrap();
    db::create_subscription(pool, p.id, PlanId::SelfHosted, 30)
        .await
        .unwrap();
    let v = db::create_vault(
        pool,
        p.id,
        "My Vault",
        Tier::HonestOperator,
        5,
        StorageRegion::default(),
    )
    .await
    .unwrap();
    (p, v)
}

#[tokio::test]
async fn create_and_list_vaults() {
    let pool = common::setup().await;
    let (p, _v) = principal_with_vault(&pool).await;
    let vaults = db::list_vaults(&pool, p.id).await.unwrap();
    assert_eq!(vaults.len(), 1);
    assert_eq!(vaults[0].state, VaultState::Active);
    // Default region when none is chosen.
    assert_eq!(vaults[0].storage_region, StorageRegion::default());
}

#[tokio::test]
async fn storage_region_persists_and_moves() {
    let pool = common::setup().await;
    let (p, _) = db::upsert_principal_by_email(&pool, "region@example.org")
        .await
        .unwrap();
    db::create_subscription(&pool, p.id, PlanId::SelfHosted, 0)
        .await
        .unwrap();

    // Created in a non-default region — read back round-trips through the enum.
    let v = db::create_vault(
        &pool,
        p.id,
        "EU vault",
        Tier::HonestOperator,
        5,
        StorageRegion::EuCentral1,
    )
    .await
    .unwrap();
    assert_eq!(v.storage_region, StorageRegion::EuCentral1);
    let fetched = db::fetch_vault(&pool, v.id).await.unwrap();
    assert_eq!(fetched.storage_region, StorageRegion::EuCentral1);

    // Move to another region.
    let moved = db::set_vault_storage_region(&pool, v.id, StorageRegion::UsEast1)
        .await
        .unwrap();
    assert_eq!(moved.storage_region, StorageRegion::UsEast1);
    assert_eq!(
        db::fetch_vault(&pool, v.id).await.unwrap().storage_region,
        StorageRegion::UsEast1
    );
}

#[tokio::test]
async fn seal_letter_persists_ciphertext() {
    let pool = common::setup().await;
    let (_p, v) = principal_with_vault(&pool).await;

    let letter = db::seal_letter(
        &pool,
        db::LetterSealInput {
            vault_id: v.id,
            title: "Hello",
            recipient_email: "rcpt@example.org",
            ciphertext: b"sealed-bytes",
            nonce: b"012345678901",
            drill_ciphertext: None,
            drill_nonce: None,
            scheduled_release_at: None,
            kind: None,
            category: None,
            release_mode: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(letter.title, "Hello");
    assert_eq!(letter.recipient_email, "rcpt@example.org");

    let listed = db::list_letters(&pool, v.id).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, letter.id);
}

#[tokio::test]
async fn fetch_letter_ciphertext_returns_drill_when_requested() {
    let pool = common::setup().await;
    let (_p, v) = principal_with_vault(&pool).await;

    let letter = db::seal_letter(
        &pool,
        db::LetterSealInput {
            vault_id: v.id,
            title: "Hello",
            recipient_email: "rcpt@example.org",
            ciphertext: b"real-ciphertext-bytes",
            nonce: b"012345678901",
            drill_ciphertext: Some(b"drill-ciphertext-bytes"),
            drill_nonce: Some(b"abcdef012345"),
            scheduled_release_at: None,
            kind: None,
            category: None,
            release_mode: None,
        },
    )
    .await
    .unwrap();

    let (_t, _r, ct, _n) = db::fetch_letter_ciphertext(&pool, letter.id, false)
        .await
        .unwrap();
    assert_eq!(ct, b"real-ciphertext-bytes");

    let (_t, _r, dct, _n) = db::fetch_letter_ciphertext(&pool, letter.id, true)
        .await
        .unwrap();
    assert_eq!(dct, b"drill-ciphertext-bytes");
}

#[tokio::test]
async fn drill_falls_back_to_real_if_no_drill_payload() {
    let pool = common::setup().await;
    let (_p, v) = principal_with_vault(&pool).await;
    let letter = db::seal_letter(
        &pool,
        db::LetterSealInput {
            vault_id: v.id,
            title: "Hello",
            recipient_email: "rcpt@example.org",
            ciphertext: b"only-real",
            nonce: b"012345678901",
            drill_ciphertext: None,
            drill_nonce: None,
            scheduled_release_at: None,
            kind: None,
            category: None,
            release_mode: None,
        },
    )
    .await
    .unwrap();
    let (_t, _r, ct, _n) = db::fetch_letter_ciphertext(&pool, letter.id, true)
        .await
        .unwrap();
    assert_eq!(ct, b"only-real", "fallback should yield real payload");
}

#[tokio::test]
async fn heartbeat_bumps_vault_attestation() {
    let pool = common::setup().await;
    let (p, v) = principal_with_vault(&pool).await;

    let before = v.last_attestation_at;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    db::record_heartbeat(&pool, p.id, "CLI").await.unwrap();

    let vv = db::fetch_vault(&pool, v.id).await.unwrap();
    assert!(
        vv.last_attestation_at > before,
        "expected attestation to advance"
    );
}

#[tokio::test]
async fn state_transitions_persist() {
    let pool = common::setup().await;
    let (_p, v) = principal_with_vault(&pool).await;

    let v2 = db::set_vault_state(
        &pool,
        v.id,
        VaultState::CoolingOff,
        Some(chrono::Utc::now()),
        None,
    )
    .await
    .unwrap();
    assert_eq!(v2.state, VaultState::CoolingOff);
    assert!(v2.cooling_off_started_at.is_some());

    let v3 = db::set_vault_state(&pool, v.id, VaultState::Releasing, None, None)
        .await
        .unwrap();
    assert_eq!(v3.state, VaultState::Releasing);
}

#[tokio::test]
async fn release_event_lifecycle() {
    let pool = common::setup().await;
    let (_p, v) = principal_with_vault(&pool).await;

    let evt = db::create_release_event(
        &pool,
        v.id,
        ReleaseReason::ManualPrincipalRelease,
        false,
    )
    .await
    .unwrap();
    assert!(!evt.is_drill);
    assert!(evt.released_at.is_none());

    db::mark_release_released(&pool, evt.id).await.unwrap();

    // fetch_open_release should now return None since released_at is set.
    let still_open = db::fetch_open_release(&pool, v.id).await.unwrap();
    assert!(still_open.is_none());
}

/// Phase-0 durability: a release whose deadline has elapsed is selectable by
/// the poll loop, can be claimed exactly once (even with the per-request timer
/// and any number of replicas all attempting it), and is no longer selectable
/// once claimed.
#[tokio::test]
async fn durable_release_is_due_and_claimed_exactly_once() {
    let pool = common::setup().await;
    let (_p, v) = principal_with_vault(&pool).await;

    // Enter cooling-off and record a deadline that has already elapsed.
    db::set_vault_state(&pool, v.id, VaultState::CoolingOff, Some(Utc::now()), None)
        .await
        .unwrap();
    let past = Utc::now() - ChronoDuration::seconds(1);
    let evt = db::create_release_event_with_deadline(
        &pool,
        v.id,
        ReleaseReason::SignalTrigger,
        false,
        past,
    )
    .await
    .unwrap();

    // The poll loop sees it as due.
    let due = db::list_due_releases(&pool, Utc::now()).await.unwrap();
    assert!(
        due.iter().any(|(vid, rid, _)| *vid == v.id && *rid == evt.id),
        "elapsed release should be due"
    );

    // Exactly one claimant wins the COOLING_OFF -> RELEASING transition.
    let first = db::claim_vault_for_release(&pool, v.id).await.unwrap();
    let second = db::claim_vault_for_release(&pool, v.id).await.unwrap();
    assert!(first, "first claim must win");
    assert!(!second, "second claim must lose — release fires exactly once");

    // After the claim the Vault is RELEASING and the release is no longer due.
    assert_eq!(
        db::fetch_vault(&pool, v.id).await.unwrap().state,
        VaultState::Releasing
    );
    let due_after = db::list_due_releases(&pool, Utc::now()).await.unwrap();
    assert!(
        !due_after.iter().any(|(vid, _, _)| *vid == v.id),
        "claimed release must not be re-selected"
    );
}

async fn a_co_steward(pool: &sqlx::PgPool, pid: beacon_core::PrincipalId) -> beacon_core::CoStewardId {
    db::invite_co_steward(
        pool,
        db::CoStewardInviteInput {
            principal_id: pid,
            display_name: Some("Deputy"),
            email: "deputy@example.org",
            confirmation_token_hash: b"tokenhash-co-steward-0000000000",
        },
    )
    .await
    .unwrap()
    .id
}

/// EVENT_ON_DEMAND ("wedding") Letters are held: excluded from both the
/// signal-release set and the scheduled-due set, so they never fire on the
/// principal's death or on a date — only when a deputy triggers them.
#[tokio::test]
async fn event_letter_is_held_from_signal_and_schedule() {
    let pool = common::setup().await;
    let (_p, v) = principal_with_vault(&pool).await;
    let ev = db::seal_letter(
        &pool,
        db::LetterSealInput {
            vault_id: v.id,
            title: "Read at the wedding",
            recipient_email: "bride@example.org",
            ciphertext: b"x",
            nonce: b"123456789012",
            drill_ciphertext: None,
            drill_nonce: None,
            scheduled_release_at: None,
            kind: None,
            category: None,
            release_mode: Some("EVENT_ON_DEMAND"),
        },
    )
    .await
    .unwrap();

    let sig = db::list_letters_for_signal_release(&pool, v.id).await.unwrap();
    assert!(
        !sig.iter().any(|l| l.id == ev.id),
        "event letter must not fire on a signal release"
    );
    let far_future = Utc::now() + ChronoDuration::days(36500);
    let due = db::list_scheduled_letters_due(&pool, far_future).await.unwrap();
    assert!(
        !due.iter().any(|(_, lid)| *lid == ev.id),
        "event letter must never be scheduled-due"
    );

    // It can be triggered exactly once.
    assert!(db::mark_letter_event_released(&pool, ev.id).await.unwrap());
    assert!(
        !db::mark_letter_event_released(&pool, ev.id).await.unwrap(),
        "an event letter cannot fire twice"
    );
}

/// A post-mortem recipient change applies after its hold and is idempotent;
/// a cancelled change never applies.
#[tokio::test]
async fn recipient_change_applies_after_hold_and_cancels() {
    let pool = common::setup().await;
    let (p, v) = principal_with_vault(&pool).await;
    let co = a_co_steward(&pool, p.id).await;
    let letter = db::seal_letter(
        &pool,
        db::LetterSealInput {
            vault_id: v.id,
            title: "For my niece",
            recipient_email: "old@example.org",
            ciphertext: b"x",
            nonce: b"123456789012",
            drill_ciphertext: None,
            drill_nonce: None,
            scheduled_release_at: None,
            kind: None,
            category: None,
            release_mode: Some("EVENT_ON_DEMAND"),
        },
    )
    .await
    .unwrap();

    // Request a change already past its hold → due → apply updates the letter.
    let change = db::request_recipient_change(
        &pool,
        letter.id,
        co,
        "old@example.org",
        "new@example.org",
        Utc::now() - ChronoDuration::seconds(1),
    )
    .await
    .unwrap();
    assert!(db::list_due_recipient_changes(&pool, Utc::now())
        .await
        .unwrap()
        .contains(&change.id));
    let applied = db::apply_recipient_change(&pool, change.id).await.unwrap();
    assert_eq!(applied, Some((letter.id, "new@example.org".to_string())));
    // Idempotent: re-applying does nothing.
    assert!(db::apply_recipient_change(&pool, change.id).await.unwrap().is_none());
    assert_eq!(
        db::fetch_letter_admin(&pool, letter.id).await.unwrap().recipient_email,
        "new@example.org"
    );

    // A cancelled change is never due and never applies.
    let evil = db::request_recipient_change(
        &pool,
        letter.id,
        co,
        "new@example.org",
        "attacker@example.org",
        Utc::now() - ChronoDuration::seconds(1),
    )
    .await
    .unwrap();
    assert!(db::cancel_recipient_change(&pool, evil.id).await.unwrap());
    assert!(!db::list_due_recipient_changes(&pool, Utc::now())
        .await
        .unwrap()
        .contains(&evil.id));
    assert!(db::apply_recipient_change(&pool, evil.id).await.unwrap().is_none());
    assert_eq!(
        db::fetch_letter_admin(&pool, letter.id).await.unwrap().recipient_email,
        "new@example.org",
        "cancelled change must not touch the delivery address"
    );
}

/// A release whose deadline is still in the future is not yet due, and a
/// cancelled release is never due.
#[tokio::test]
async fn future_and_cancelled_releases_are_not_due() {
    let pool = common::setup().await;
    let (_p, v) = principal_with_vault(&pool).await;
    db::set_vault_state(&pool, v.id, VaultState::CoolingOff, Some(Utc::now()), None)
        .await
        .unwrap();

    // Future deadline → not due.
    let future = Utc::now() + ChronoDuration::seconds(3600);
    let evt = db::create_release_event_with_deadline(
        &pool,
        v.id,
        ReleaseReason::SignalTrigger,
        false,
        future,
    )
    .await
    .unwrap();
    let due = db::list_due_releases(&pool, Utc::now()).await.unwrap();
    assert!(
        !due.iter().any(|(vid, _, _)| *vid == v.id),
        "future-dated release must not be due yet"
    );

    // Cancel it (principal cancelled during cooling-off): never due, and the
    // claim finds nothing because the vault is no longer COOLING_OFF.
    db::mark_release_cancelled(&pool, evt.id).await.unwrap();
    db::set_vault_state(&pool, v.id, VaultState::Active, None, None)
        .await
        .unwrap();
    let due_now = db::list_due_releases(&pool, future + ChronoDuration::seconds(1))
        .await
        .unwrap();
    assert!(
        !due_now.iter().any(|(vid, _, _)| *vid == v.id),
        "cancelled release must never be due"
    );
    assert!(
        !db::claim_vault_for_release(&pool, v.id).await.unwrap(),
        "cancelled/active vault cannot be claimed for release"
    );
}

#[tokio::test]
async fn release_claim_is_single_use() {
    let pool = common::setup().await;
    let (_p, v) = principal_with_vault(&pool).await;
    let letter = db::seal_letter(
        &pool,
        db::LetterSealInput {
            vault_id: v.id,
            title: "Claim me",
            recipient_email: "rcpt@example.org",
            ciphertext: b"x",
            nonce: b"123456789012",
            drill_ciphertext: None,
            drill_nonce: None,
            scheduled_release_at: None,
            kind: None,
            category: None,
            release_mode: None,
        },
    )
    .await
    .unwrap();
    let evt = db::create_release_event(&pool, v.id, ReleaseReason::ManualPrincipalRelease, false)
        .await
        .unwrap();

    let token_hash = b"a".repeat(32);
    db::issue_release_claim(&pool, evt.id, letter.id, "rcpt@example.org", &token_hash, 1)
        .await
        .unwrap();

    let first = db::consume_release_claim(&pool, &token_hash).await.unwrap();
    assert!(first.is_some(), "first claim must succeed");
    let second = db::consume_release_claim(&pool, &token_hash).await.unwrap();
    assert!(second.is_none(), "claim must be single-use");
}

#[tokio::test]
async fn purge_letter_ciphertext_zeroises_payload() {
    let pool = common::setup().await;
    let (_p, v) = principal_with_vault(&pool).await;
    let letter = db::seal_letter(
        &pool,
        db::LetterSealInput {
            vault_id: v.id,
            title: "Bye",
            recipient_email: "rcpt@example.org",
            ciphertext: b"some-bytes-of-content",
            nonce: b"123456789012",
            drill_ciphertext: Some(b"drill-bytes"),
            drill_nonce: Some(b"abcdef123456"),
            scheduled_release_at: None,
            kind: None,
            category: None,
            release_mode: None,
        },
    )
    .await
    .unwrap();

    db::purge_letter_ciphertext(&pool, letter.id).await.unwrap();
    let (_t, _r, ct, _n) = db::fetch_letter_ciphertext(&pool, letter.id, false)
        .await
        .unwrap();
    assert!(ct.is_empty(), "ciphertext must be zeroised; got {:?}", ct);
}

#[tokio::test]
async fn scheduled_release_listing() {
    let pool = common::setup().await;
    let (_p, v) = principal_with_vault(&pool).await;

    let past = chrono::Utc::now() - chrono::Duration::seconds(60);
    let future = chrono::Utc::now() + chrono::Duration::days(1);

    let due = db::seal_letter(
        &pool,
        db::LetterSealInput {
            vault_id: v.id,
            title: "due",
            recipient_email: "r@example.org",
            ciphertext: b"x",
            nonce: b"123456789012",
            drill_ciphertext: None,
            drill_nonce: None,
            scheduled_release_at: Some(past),
            kind: None,
            category: None,
            release_mode: None,
        },
    )
    .await
    .unwrap();

    let _later = db::seal_letter(
        &pool,
        db::LetterSealInput {
            vault_id: v.id,
            title: "later",
            recipient_email: "r@example.org",
            ciphertext: b"y",
            nonce: b"123456789012",
            drill_ciphertext: None,
            drill_nonce: None,
            scheduled_release_at: Some(future),
            kind: None,
            category: None,
            release_mode: None,
        },
    )
    .await
    .unwrap();

    let listed = db::list_scheduled_letters_due(&pool, chrono::Utc::now())
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].1, due.id);
}

#[tokio::test]
async fn list_vaults_in_states_filters_correctly() {
    let pool = common::setup().await;
    let (_p, v) = principal_with_vault(&pool).await;
    let actives = db::list_vaults_in_states(&pool, &[VaultState::Active])
        .await
        .unwrap();
    assert!(actives.iter().any(|x| x.id == v.id));

    db::set_vault_state(&pool, v.id, VaultState::CoolingOff, None, None)
        .await
        .unwrap();
    let actives = db::list_vaults_in_states(&pool, &[VaultState::Active])
        .await
        .unwrap();
    assert!(!actives.iter().any(|x| x.id == v.id));
    let cooling = db::list_vaults_in_states(&pool, &[VaultState::CoolingOff])
        .await
        .unwrap();
    assert!(cooling.iter().any(|x| x.id == v.id));
}
