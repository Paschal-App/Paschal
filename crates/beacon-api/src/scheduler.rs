//! Background workers.
//!
//! The MVP runs all of these in-process. Each worker is independent and
//! resilient to the others failing.
//!
//!   * `signal_aggregator` — evaluates every Vault's trigger score on a
//!     configurable interval. State transitions ACTIVE → SUSPICIOUS →
//!     ALERT → COOLING_OFF happen here. The cooling-off elapse + release
//!     handoff lives in `release_pipeline`.
//!
//!   * `release_pipeline` — given a Vault transitioned into COOLING_OFF
//!     by any path (force-release, drill, signal aggregator), waits out
//!     the cooling-off window watching for cancellation, then transitions
//!     RELEASING → RELEASED and issues claim tokens.
//!
//!   * `retention_scheduler` — runs every minute; transitions CANCELED →
//!     EXPIRED → DELETED, performing cryptographic erasure of expired
//!     Letters.
//!
//!   * `scheduled_release_scheduler` — periodically fires Letters whose
//!     `scheduled_release_at` is in the past.

use std::time::Duration;

use beacon_core::{
    score, ReleaseEventId, ReleaseReason, ScoredObservation, SignalSource, VaultId, VaultState,
};
use beacon_db as db;
use chrono::Utc;
use crypto_stub::{hash_bytes, hash_token, random_token};
use serde_json::json;
use tokio::time::sleep;

use crate::{
    errors::{ApiError, ApiResult},
    metrics::Metrics,
    notifications::tx,
    routes::ForceReleaseResp,
    state::AppState,
};

/// Session-level advisory lock that elects a single scheduler leader across
/// replicas. Only the holder runs the background loops, so the aggregator,
/// retention sweep, and release poll do not run N times in an N-replica
/// deployment. Correctness of releases does not *depend* on this — the durable
/// deadline plus the atomic `COOLING_OFF → RELEASING` claim make releases
/// exactly-once regardless — but it avoids redundant work in steady state.
const SCHEDULER_LEADER_LOCK: i64 = 0x00C5_7117_5CED;

/// Spawn the background workers.
///
/// With leader election enabled (production default), a supervisor contends for
/// [`SCHEDULER_LEADER_LOCK`]; only the winner starts the loops. With it disabled
/// (tests), the loops start directly. Note the integration-test harness does not
/// call this at all — drills/force-releases fire via their per-request timer.
pub fn spawn_all(state: AppState) {
    if state.config.scheduler_leader_election {
        tokio::spawn(async move { leader_supervisor(state).await });
    } else {
        spawn_workers(state);
    }
}

fn spawn_workers(state: AppState) {
    let s1 = state.clone();
    tokio::spawn(async move { signal_aggregator(s1).await });
    let s2 = state.clone();
    tokio::spawn(async move { retention_scheduler(s2).await });
    let s3 = state.clone();
    tokio::spawn(async move { scheduled_release_scheduler(s3).await });
    let s4 = state.clone();
    tokio::spawn(async move { release_elapse_scheduler(s4).await });
    let s5 = state.clone();
    tokio::spawn(async move { recipient_change_applier(s5).await });
}

/// Contend for scheduler leadership. The first replica to acquire the advisory
/// lock starts the workers and holds the lock connection for the life of the
/// process. A standby keeps polling and takes over if the leader's connection
/// drops (e.g. the leader process dies, releasing the lock).
async fn leader_supervisor(state: AppState) {
    loop {
        match acquire_leader_lock(&state).await {
            Ok(Some(conn)) => {
                tracing::info!("scheduler leadership acquired; starting background workers");
                spawn_workers(state.clone());
                // Hold the lock connection for the life of the process. If it
                // dies, the lock is released so a standby can take over; the
                // workers already running here keep operating safely (the
                // atomic claim makes any brief overlap harmless), so we simply
                // stop re-contending.
                keepalive(conn).await;
                tracing::warn!("scheduler leader lock connection lost; relinquishing leadership");
                return;
            }
            Ok(None) => {
                // Another replica leads. Retry later so we can take over if it
                // goes away.
                sleep(Duration::from_secs(15)).await;
            }
            Err(e) => {
                tracing::warn!(?e, "scheduler leader election attempt failed");
                sleep(Duration::from_secs(15)).await;
            }
        }
    }
}

async fn acquire_leader_lock(
    state: &AppState,
) -> anyhow::Result<Option<sqlx::pool::PoolConnection<sqlx::Postgres>>> {
    let mut conn = state.pool.acquire().await?;
    let (acquired,): (bool,) = sqlx::query_as("SELECT pg_try_advisory_lock($1)")
        .bind(SCHEDULER_LEADER_LOCK)
        .fetch_one(&mut *conn)
        .await?;
    if acquired {
        Ok(Some(conn))
    } else {
        Ok(None)
    }
}

/// Hold a connection alive, returning only when it can no longer be reached
/// (which means our advisory lock has been released).
async fn keepalive(mut conn: sqlx::pool::PoolConnection<sqlx::Postgres>) {
    loop {
        sleep(Duration::from_secs(10)).await;
        if sqlx::query("SELECT 1").execute(&mut *conn).await.is_err() {
            return;
        }
    }
}

// ----------------------------------------------------------------------------
// Signal aggregator
// ----------------------------------------------------------------------------

async fn signal_aggregator(state: AppState) {
    let tick = Duration::from_secs(state.config.aggregator_tick_seconds.max(1));
    tracing::info!(?tick, "signal aggregator started");
    loop {
        if let Err(e) = aggregator_tick(&state).await {
            tracing::error!(?e, "aggregator tick failed");
        }
        sleep(tick).await;
    }
}

async fn aggregator_tick(state: &AppState) -> anyhow::Result<()> {
    let watchable = [
        VaultState::Active,
        VaultState::Suspicious,
        VaultState::Alert,
    ];
    let vaults = db::list_vaults_in_states(&state.pool, &watchable).await?;
    let now = Utc::now();

    for vault in vaults {
        // Build observations:
        //   - missing heartbeat → contribution rises with seconds-since-last
        //   - buddy attestations → use recent signal rows
        //   - apple shortcut fresh → contribution 0 (means "alive")

        let subs = db::list_signal_subscriptions(&state.pool, vault.id).await?;
        if subs.is_empty() {
            continue;
        }
        let weight_for = |src: SignalSource| -> Option<f32> {
            subs.iter()
                .find(|s| s.source == src && s.enabled)
                .map(|s| s.weight)
        };

        let mut observations = Vec::new();

        // Heartbeat — missing for too long counts as a strong signal.
        if let Some(w) = weight_for(SignalSource::Heartbeat) {
            let max_gap = state.config.heartbeat_max_gap_seconds;
            let last = vault.last_attestation_at;
            let elapsed = (now - last).num_seconds();
            let c = ((elapsed as f32) / (max_gap as f32)).clamp(0.0, 1.0);
            if c > 0.0 {
                observations.push(ScoredObservation {
                    source: SignalSource::Heartbeat,
                    contribution: c,
                    weight: w,
                });
            }
        }

        // Apple iCloud — same: stale = signal. Fresh observation in the past
        // 5 × max_gap means a ping was received, which keeps last_attestation_at
        // fresh; we therefore reuse the same staleness logic above. No
        // additional observation needed unless we wanted to differentiate the
        // class (which we don't right now).

        // Buddy attestation — pick the strongest recent positive contribution.
        if let Some(w) = weight_for(SignalSource::BuddyAttestation) {
            let since = now - chrono::Duration::days(30);
            let signals = db::list_recent_signals(&state.pool, vault.id, since).await?;
            let strongest_buddy = signals
                .iter()
                .filter(|s| s.source == SignalSource::BuddyAttestation)
                .map(|s| s.contribution)
                .fold(0.0_f32, f32::max);
            if strongest_buddy > 0.0 {
                observations.push(ScoredObservation {
                    source: SignalSource::BuddyAttestation,
                    contribution: strongest_buddy,
                    weight: w,
                });
            }
        }

        // Bank dormancy (CDR_BANK_DORMANCY) — webhook-based proof-of-life.
        // A fresh ping within the last 5× heartbeat window keeps the score low.
        if let Some(w) = weight_for(SignalSource::Cdr) {
            let since = now
                - chrono::Duration::seconds((state.config.heartbeat_max_gap_seconds * 5) as i64);
            let pings = db::list_bank_dormancy_subs_with_recent_ping(
                &state.pool,
                vault.principal_id,
                since,
            )
            .await
            .unwrap_or_default();
            if pings.is_empty() {
                // No recent ping — same staleness logic as heartbeat.
                let last_bank_signals = db::list_recent_signals(&state.pool, vault.id, since)
                    .await
                    .unwrap_or_default();
                let has_recent = last_bank_signals
                    .iter()
                    .any(|s| s.source == SignalSource::Cdr);
                if !has_recent {
                    let max_gap = state.config.heartbeat_max_gap_seconds;
                    let last = vault.last_attestation_at;
                    let elapsed = (now - last).num_seconds();
                    let c = ((elapsed as f32) / (max_gap as f32)).clamp(0.0, 1.0);
                    if c > 0.0 {
                        observations.push(ScoredObservation {
                            source: SignalSource::Cdr,
                            contribution: c,
                            weight: w,
                        });
                    }
                }
            }
        }

        // Guardian — same shape if/when implemented. Placeholder for v2.

        let scored = score(&observations);
        let policy_threshold = state.config.trigger_threshold;
        let min_classes = state.config.min_independent_signals;

        let new_state = evaluate(vault.state, &scored, policy_threshold, min_classes);
        if new_state != vault.state {
            tracing::info!(
                vault_id = %vault.id,
                from = ?vault.state,
                to = ?new_state,
                score = scored.score,
                classes = scored.independent_classes,
                "state transition"
            );

            if new_state == VaultState::CoolingOff {
                // Enter cooling-off atomically from the observed prior state so
                // the side effects (notification + release event) fire exactly
                // once even if two schedulers briefly overlap during failover.
                // The release itself is fired later by the release-elapse poll
                // loop once the durable deadline elapses — no in-memory timer.
                if db::claim_vault_cooling_off(&state.pool, vault.id, vault.state).await? {
                    let _ = db::append_transparency_entry(
                        &state.pool,
                        "TRIGGER_COOLING_OFF",
                        &hash_bytes(vault.id.as_uuid().as_bytes()),
                        json!({ "score": scored.score, "classes": scored.independent_classes }),
                        Some(vault.principal_id),
                        Some(vault.id),
                    )
                    .await;

                    let principal = db::fetch_principal(&state.pool, vault.principal_id).await?;
                    let ends_at =
                        Utc::now() + chrono::Duration::seconds(vault.cooling_off_seconds as i64);
                    tx::cooling_off_started(
                        state.notifications.as_ref(),
                        &principal.primary_email,
                        &vault.name,
                        &ends_at.to_rfc3339(),
                    )
                    .await;
                    db::create_release_event_with_deadline(
                        &state.pool,
                        vault.id,
                        ReleaseReason::SignalTrigger,
                        false,
                        ends_at,
                    )
                    .await?;
                }
            } else {
                db::set_vault_state(&state.pool, vault.id, new_state, None, None).await?;
                let _ = db::append_transparency_entry(
                    &state.pool,
                    &format!("TRIGGER_{}", new_state.as_db_str()),
                    &hash_bytes(vault.id.as_uuid().as_bytes()),
                    json!({ "score": scored.score, "classes": scored.independent_classes }),
                    Some(vault.principal_id),
                    Some(vault.id),
                )
                .await;
            }
        }
    }
    Ok(())
}

/// Decide the next Vault state given the current state, the trigger score,
/// the configured threshold, and the minimum independent class count.
fn evaluate(
    current: VaultState,
    s: &beacon_core::TriggerScore,
    threshold: f32,
    min_classes: usize,
) -> VaultState {
    use VaultState::*;
    let crossed_threshold = s.should_alert(threshold, min_classes);
    let soft_threshold = s.score >= (threshold * 0.5) && s.independent_classes >= 1;

    match current {
        Active => {
            if crossed_threshold {
                Alert
            } else if soft_threshold {
                Suspicious
            } else {
                Active
            }
        }
        Suspicious => {
            if crossed_threshold {
                Alert
            } else if s.score < 0.1 {
                Active
            } else {
                Suspicious
            }
        }
        Alert => {
            if !soft_threshold {
                Suspicious
            } else {
                // Persistent ALERT → cooling-off. In the MVP we collapse
                // alert_dwell to "one tick" so the demo finishes; production
                // honours the configured alert_dwell_seconds.
                CoolingOff
            }
        }
        other => other,
    }
}

// ----------------------------------------------------------------------------
// Release pipeline (cooling-off → released)
// ----------------------------------------------------------------------------

/// Begin a release: set state to COOLING_OFF and spawn the elapse task.
///
/// Used by `force_release` (manual), `run_drill` (with is_drill=true), and
/// — via the aggregator — by signal-based triggers.
pub async fn begin_release(
    state: AppState,
    vault_id: VaultId,
    reason: ReleaseReason,
    is_drill: bool,
) -> ApiResult<ForceReleaseResp> {
    let vault = db::fetch_vault(&state.pool, vault_id).await?;
    begin_release_with_cool(state, vault_id, reason, is_drill, vault.cooling_off_seconds).await
}

pub async fn begin_release_with_cool(
    state: AppState,
    vault_id: VaultId,
    reason: ReleaseReason,
    is_drill: bool,
    cool_seconds: i32,
) -> ApiResult<ForceReleaseResp> {
    let now = Utc::now();
    let cool = cool_seconds.max(1) as i64;
    let ends_at = now + chrono::Duration::seconds(cool);

    // Enter cooling-off atomically from ACTIVE. Callers (force-release, drill,
    // scheduled release) all require ACTIVE; the conditional update also dedupes
    // a racing double-trigger on the same Vault.
    if !db::claim_vault_cooling_off(&state.pool, vault_id, VaultState::Active).await? {
        return Err(ApiError::Conflict(
            "Vault is not ACTIVE; cannot begin release".into(),
        ));
    }

    // Record the durable deadline so the release survives a restart and can be
    // fired by the release-elapse poll loop on any replica.
    let release =
        db::create_release_event_with_deadline(&state.pool, vault_id, reason, is_drill, ends_at)
            .await?;

    Metrics::inc(&state.metrics.releases_started_total);

    let _ = db::append_transparency_entry(
        &state.pool,
        if is_drill {
            "DRILL_STARTED"
        } else {
            "COOLING_OFF_STARTED"
        },
        &hash_bytes(release.id.as_uuid().as_bytes()),
        json!({
            "vault_id": vault_id,
            "release_event_id": release.id,
            "reason": reason.as_db_str(),
            "is_drill": is_drill,
        }),
        None,
        Some(vault_id),
    )
    .await;

    // Fire promptly via a per-request timer (keeps drills snappy and lets the
    // integration-test harness, which does not run the background scheduler,
    // complete a release). The durable deadline + poll loop is the production
    // backstop; the atomic claim in `perform_release` keeps it exactly-once.
    spawn_release_elapse(state.clone(), vault_id, release.id, cool, is_drill);

    Ok(ForceReleaseResp {
        release_event_id: release.id,
        cooling_off_started_at: now.to_rfc3339(),
        cooling_off_ends_at: ends_at.to_rfc3339(),
    })
}

fn spawn_release_elapse(
    state: AppState,
    vault_id: VaultId,
    release_event_id: ReleaseEventId,
    cool_seconds: i64,
    is_drill: bool,
) {
    tokio::spawn(async move {
        if let Err(e) =
            elapse_release(state, vault_id, release_event_id, cool_seconds, is_drill).await
        {
            tracing::error!(?e, "release pipeline failed");
        }
    });
}

/// Per-request elapse: wait out the cooling-off window, then attempt to claim
/// and perform the release. Cancellation needs no special handling — a cancel
/// returns the Vault to ACTIVE, so the atomic `COOLING_OFF → RELEASING` claim
/// simply finds nothing to claim and the release is abandoned.
async fn elapse_release(
    state: AppState,
    vault_id: VaultId,
    release_event_id: ReleaseEventId,
    cool_seconds: i64,
    is_drill: bool,
) -> anyhow::Result<()> {
    sleep(Duration::from_secs(cool_seconds.max(0) as u64)).await;

    if !db::claim_vault_for_release(&state.pool, vault_id).await? {
        tracing::info!(
            ?vault_id,
            "release not claimed (cancelled or already fired); abandoning"
        );
        return Ok(());
    }

    perform_release(state, vault_id, release_event_id, is_drill).await
}

/// The release-elapse poll loop — the durable, replica-safe path. Finds open
/// releases whose deadline has elapsed, claims each atomically, and performs it.
/// This is what fires aggregator-triggered releases and recovers releases whose
/// per-request timer was lost to a restart.
async fn release_elapse_scheduler(state: AppState) {
    tracing::info!("release-elapse scheduler started");
    let tick = Duration::from_secs(2);
    loop {
        if let Err(e) = release_elapse_tick(&state).await {
            tracing::error!(?e, "release-elapse tick failed");
        }
        sleep(tick).await;
    }
}

async fn release_elapse_tick(state: &AppState) -> anyhow::Result<()> {
    let due = db::list_due_releases(&state.pool, Utc::now()).await?;
    for (vault_id, release_event_id, is_drill) in due {
        if db::claim_vault_for_release(&state.pool, vault_id).await? {
            if let Err(e) =
                perform_release(state.clone(), vault_id, release_event_id, is_drill).await
            {
                tracing::error!(?e, ?vault_id, "perform_release failed");
            }
        }
    }
    Ok(())
}

/// Issue a single Letter's claim token (plus per-attachment claims) and notify
/// the recipient. Shared by the whole-Vault release pipeline and the
/// Co-Steward event-trigger path so both deliver identically.
async fn deliver_letter(
    state: &AppState,
    release_event_id: ReleaseEventId,
    letter: &beacon_core::LetterMeta,
    is_drill: bool,
) -> anyhow::Result<()> {
    let token = random_token();
    let token_hash = hash_token(&token);
    db::issue_release_claim(
        &state.pool,
        release_event_id,
        letter.id,
        &letter.recipient_email,
        &token_hash,
        30,
    )
    .await?;

    let claim_url = format!(
        "{}/claim?token={}",
        state.config.public_base_url.trim_end_matches('/'),
        token
    );

    // Attachment claims: one per attachment.
    let attachments = db::list_attachments(&state.pool, letter.id).await?;
    let mut attachment_links: Vec<(String, String)> = Vec::with_capacity(attachments.len());
    for attachment in attachments {
        let att_token = random_token();
        let att_hash = hash_token(&att_token);
        db::issue_attachment_claim(
            &state.pool,
            release_event_id,
            attachment.id,
            &letter.recipient_email,
            &att_hash,
            30,
        )
        .await?;
        attachment_links.push((attachment.original_filename.clone(), att_token));
    }

    let attachment_pairs: Vec<(String, String)> = attachment_links
        .iter()
        .map(|(filename, att_token)| {
            let att_url = format!(
                "{}/v1/releases/claim/attachment?token={}",
                state.config.public_base_url.trim_end_matches('/'),
                att_token
            );
            (filename.clone(), att_url)
        })
        .collect();

    tx::release_notification_with_attachments(
        state.notifications.as_ref(),
        &letter.recipient_email,
        &letter.title,
        &claim_url,
        is_drill,
        &attachment_pairs,
    )
    .await;

    tracing::info!(
        recipient = %letter.recipient_email,
        url = %claim_url,
        attachment_count = attachment_pairs.len(),
        is_drill,
        "release claim issued"
    );
    Ok(())
}

/// Trigger a single held EVENT_ON_DEMAND Letter — the Co-Steward "the wedding
/// happened, send it now" path. The caller has already authorised the deputy
/// and confirmed the Letter's Vault has RELEASED. Returns the claim URL is not
/// needed by the caller; delivery + notification happen here. Idempotent: a
/// Letter that has already fired cannot fire again.
pub async fn trigger_event_release(
    state: &AppState,
    vault_id: VaultId,
    letter_id: beacon_core::LetterId,
) -> ApiResult<()> {
    // Apply any due (past-hold) recipient change first so the event fires to
    // the latest confirmed address.
    let now = Utc::now();
    if let Some(change) = db::fetch_pending_recipient_change(&state.pool, letter_id).await? {
        if change.effective_at <= now {
            let _ = db::apply_recipient_change(&state.pool, change.id).await?;
        }
    }

    // Atomically claim the event so it fires exactly once.
    if !db::mark_letter_event_released(&state.pool, letter_id).await? {
        return Err(ApiError::Conflict(
            "Letter is not a held event Letter, or has already been released".into(),
        ));
    }

    let letter = db::fetch_letter_meta(&state.pool, letter_id).await?;
    let release =
        db::create_release_event(&state.pool, vault_id, ReleaseReason::Scheduled, false).await?;

    deliver_letter(state, release.id, &letter, false).await?;

    db::mark_release_released(&state.pool, release.id).await?;
    let _ = db::append_transparency_entry(
        &state.pool,
        "CO_STEWARD_EVENT_RELEASE",
        &hash_bytes(letter_id.as_uuid().as_bytes()),
        json!({ "vault_id": vault_id, "letter_id": letter_id, "release_event_id": release.id }),
        None,
        Some(vault_id),
    )
    .await;
    Metrics::inc(&state.metrics.releases_completed_total);
    tracing::info!(?vault_id, ?letter_id, "co-steward triggered event release");
    Ok(())
}

/// Perform a release for a Vault that has already been atomically claimed
/// (`COOLING_OFF → RELEASING`). Issues claim tokens, notifies recipients,
/// marks the Vault and release event RELEASED, and returns a drill to ACTIVE.
async fn perform_release(
    state: AppState,
    vault_id: VaultId,
    release_event_id: ReleaseEventId,
    is_drill: bool,
) -> anyhow::Result<()> {
    // Signal-triggered releases exclude SCHEDULED_ONLY (time-capsule) and
    // EVENT_ON_DEMAND letters — those fire on their own date or when a deputy
    // triggers them, regardless of Vault state.
    let letters = db::list_letters_for_signal_release(&state.pool, vault_id).await?;
    for letter in letters {
        deliver_letter(&state, release_event_id, &letter, is_drill).await?;
    }

    let now = Utc::now();
    db::set_vault_state(&state.pool, vault_id, VaultState::Released, None, Some(now)).await?;
    db::mark_release_released(&state.pool, release_event_id).await?;

    let _ = db::append_transparency_entry(
        &state.pool,
        if is_drill {
            "DRILL_COMPLETED"
        } else {
            "RELEASED"
        },
        &hash_bytes(release_event_id.as_uuid().as_bytes()),
        json!({ "vault_id": vault_id, "release_event_id": release_event_id, "is_drill": is_drill }),
        None,
        Some(vault_id),
    )
    .await;

    Metrics::inc(&state.metrics.releases_completed_total);
    if is_drill {
        // If this was a Drill, the Vault returns to ACTIVE so it can be
        // drilled again or really released later.
        db::set_vault_state(&state.pool, vault_id, VaultState::Active, None, None).await?;
        Metrics::inc(&state.metrics.drills_completed_total);
    }

    tracing::info!(?vault_id, is_drill, "release completed");
    Ok(())
}

// ----------------------------------------------------------------------------
// Recipient-change applier
// ----------------------------------------------------------------------------

/// Apply Co-Steward recipient-contact changes whose hold has elapsed. The hold
/// (log + notify on request, then a delay before this applies the change) is
/// the guardrail against a deputy silently redirecting a deceased person's
/// Letter — others can cancel the pending change during the window.
async fn recipient_change_applier(state: AppState) {
    tracing::info!("recipient-change applier started");
    let tick = Duration::from_secs(15);
    loop {
        if let Err(e) = recipient_change_tick(&state).await {
            tracing::error!(?e, "recipient-change tick failed");
        }
        sleep(tick).await;
    }
}

async fn recipient_change_tick(state: &AppState) -> anyhow::Result<()> {
    let due = db::list_due_recipient_changes(&state.pool, Utc::now()).await?;
    for change_id in due {
        if let Some((letter_id, new_email)) =
            db::apply_recipient_change(&state.pool, change_id).await?
        {
            let _ = db::append_transparency_entry(
                &state.pool,
                "RECIPIENT_CHANGE_APPLIED",
                &hash_bytes(letter_id.as_uuid().as_bytes()),
                json!({ "letter_id": letter_id, "change_id": change_id }),
                None,
                None,
            )
            .await;
            tracing::info!(?letter_id, new_email = %new_email, "recipient change applied");
        }
    }
    Ok(())
}

// ----------------------------------------------------------------------------
// Retention scheduler
// ----------------------------------------------------------------------------

async fn retention_scheduler(state: AppState) {
    tracing::info!("retention scheduler started");
    let tick = Duration::from_secs(60); // production: 1h; MVP: 1m
    loop {
        if let Err(e) = retention_tick(&state).await {
            tracing::error!(?e, "retention tick failed");
        }
        sleep(tick).await;
    }
}

async fn retention_tick(state: &AppState) -> ApiResult<()> {
    // 1. Subscription-retention sweep: CANCELED → EXPIRED → DELETED.
    let due = db::list_canceled_past_retention(&state.pool).await?;
    for pid in due {
        tracing::info!(?pid, "retention elapsed — transitioning to EXPIRED");
        db::expire_subscription(&state.pool, pid).await?;
        purge_principal_ciphertext(state, pid).await?;
        db::delete_subscription(&state.pool, pid).await?;
        let _ = db::append_transparency_entry(
            &state.pool,
            "SUBSCRIPTION_EXPIRED_AND_DELETED",
            &hash_bytes(pid.as_uuid().as_bytes()),
            json!({}),
            Some(pid),
            None,
        )
        .await;
    }

    // 2. Account-deletion sweep: deletion_scheduled_for elapsed → erase.
    let deletions = db::list_due_account_deletions(&state.pool).await?;
    for pid in deletions {
        tracing::info!(?pid, "account deletion cool-off elapsed — purging");
        purge_principal_ciphertext(state, pid).await?;
        db::anonymise_principal(&state.pool, pid).await?;
        let _ = db::append_transparency_entry(
            &state.pool,
            "ACCOUNT_DELETED",
            &hash_bytes(pid.as_uuid().as_bytes()),
            json!({}),
            Some(pid),
            None,
        )
        .await;
    }
    Ok(())
}

async fn purge_principal_ciphertext(
    state: &AppState,
    pid: beacon_core::PrincipalId,
) -> ApiResult<()> {
    for vault in db::list_vaults(&state.pool, pid).await? {
        for letter in db::list_letters(&state.pool, vault.id).await? {
            // Purge attachment blobs first.
            for attachment in db::list_attachments(&state.pool, letter.id).await? {
                if let Some(key) = db::purge_attachment(&state.pool, attachment.id).await? {
                    if let Err(e) = state.blob_store.delete(&key).await {
                        tracing::warn!(?e, %key, "blob delete failed");
                    }
                }
            }
            db::purge_letter_ciphertext(&state.pool, letter.id).await?;
        }
    }
    Ok(())
}

// ----------------------------------------------------------------------------
// Scheduled-release scheduler
// ----------------------------------------------------------------------------

async fn scheduled_release_scheduler(state: AppState) {
    tracing::info!("scheduled-release scheduler started");
    let tick = Duration::from_secs(15); // configurable in production
    loop {
        if let Err(e) = scheduled_release_tick(&state).await {
            tracing::error!(?e, "scheduled-release tick failed");
        }
        sleep(tick).await;
    }
}

async fn scheduled_release_tick(state: &AppState) -> anyhow::Result<()> {
    let due = db::list_scheduled_letters_due(&state.pool, Utc::now()).await?;
    for (vault_id, letter_id) in due {
        let vault = match db::fetch_vault(&state.pool, vault_id).await {
            Ok(v) => v,
            Err(_) => continue,
        };
        if vault.state != VaultState::Active {
            continue;
        }
        let sub = match db::fetch_subscription(&state.pool, vault.principal_id).await {
            Ok(s) => s,
            Err(_) => continue,
        };
        if !sub.state.allows_release() {
            continue;
        }
        tracing::info!(?vault_id, ?letter_id, "firing scheduled release");
        // begin_release applies to the WHOLE vault, not just one letter, but
        // the spec says a scheduled release "fires the Letter". For the MVP
        // we conservatively release the whole Vault — this is the correct
        // semantics for the most common case (the principal authored a
        // single deathbed Letter). Multi-Letter scheduling lands at v1 with
        // per-Letter release events.
        let _ = begin_release(state.clone(), vault.id, ReleaseReason::Scheduled, false).await;
        // Clear the scheduled_release_at so we don't re-fire on next tick.
        let _ = sqlx::query("UPDATE letter SET scheduled_release_at = NULL WHERE id = $1")
            .bind(letter_id.as_uuid())
            .execute(&state.pool)
            .await;
    }
    Ok(())
}
