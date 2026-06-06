//! In-process metrics for the Beacon.
//!
//! The MVP exposes a `/metrics` endpoint in Prometheus text format. Counters
//! are atomic; the formatter renders a snapshot. For production-scale
//! deployments swap to `metrics-exporter-prometheus` and instrument with the
//! `metrics` macros instead — the architecture allows it without route
//! changes.

use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;

/// Global metrics counters. Cheap to read, lock-free to update.
pub struct Metrics {
    pub signups_total: AtomicU64,
    pub vaults_created_total: AtomicU64,
    pub letters_sealed_total: AtomicU64,
    pub attachments_sealed_total: AtomicU64,
    pub attachments_bytes_total: AtomicU64,
    pub heartbeats_total: AtomicU64,
    pub releases_started_total: AtomicU64,
    pub releases_completed_total: AtomicU64,
    pub drills_completed_total: AtomicU64,
    pub buddy_responses_total: AtomicU64,
    pub apple_pings_total: AtomicU64,
    pub subscription_canceled_total: AtomicU64,
    pub subscription_reactivated_total: AtomicU64,
}

impl Metrics {
    pub const fn new() -> Self {
        Self {
            signups_total: AtomicU64::new(0),
            vaults_created_total: AtomicU64::new(0),
            letters_sealed_total: AtomicU64::new(0),
            attachments_sealed_total: AtomicU64::new(0),
            attachments_bytes_total: AtomicU64::new(0),
            heartbeats_total: AtomicU64::new(0),
            releases_started_total: AtomicU64::new(0),
            releases_completed_total: AtomicU64::new(0),
            drills_completed_total: AtomicU64::new(0),
            buddy_responses_total: AtomicU64::new(0),
            apple_pings_total: AtomicU64::new(0),
            subscription_canceled_total: AtomicU64::new(0),
            subscription_reactivated_total: AtomicU64::new(0),
        }
    }

    pub fn inc(c: &AtomicU64) {
        c.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add(c: &AtomicU64, by: u64) {
        c.fetch_add(by, Ordering::Relaxed);
    }

    /// Snapshot the counter values for the `/metrics` endpoint.
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            signups_total: self.signups_total.load(Ordering::Relaxed),
            vaults_created_total: self.vaults_created_total.load(Ordering::Relaxed),
            letters_sealed_total: self.letters_sealed_total.load(Ordering::Relaxed),
            attachments_sealed_total: self
                .attachments_sealed_total
                .load(Ordering::Relaxed),
            attachments_bytes_total: self
                .attachments_bytes_total
                .load(Ordering::Relaxed),
            heartbeats_total: self.heartbeats_total.load(Ordering::Relaxed),
            releases_started_total: self.releases_started_total.load(Ordering::Relaxed),
            releases_completed_total: self
                .releases_completed_total
                .load(Ordering::Relaxed),
            drills_completed_total: self.drills_completed_total.load(Ordering::Relaxed),
            buddy_responses_total: self.buddy_responses_total.load(Ordering::Relaxed),
            apple_pings_total: self.apple_pings_total.load(Ordering::Relaxed),
            subscription_canceled_total: self
                .subscription_canceled_total
                .load(Ordering::Relaxed),
            subscription_reactivated_total: self
                .subscription_reactivated_total
                .load(Ordering::Relaxed),
        }
    }

    /// Format as Prometheus text. Each counter gets a `# TYPE` and
    /// `# HELP` line followed by the value.
    pub fn prometheus(&self) -> String {
        let s = self.snapshot();
        let mut out = String::new();
        let lines: &[(&str, &str, u64)] = &[
            ("paschal_signups_total", "Total principal sign-ups.", s.signups_total),
            (
                "paschal_vaults_created_total",
                "Total Vaults created.",
                s.vaults_created_total,
            ),
            (
                "paschal_letters_sealed_total",
                "Total Letters sealed.",
                s.letters_sealed_total,
            ),
            (
                "paschal_attachments_sealed_total",
                "Total Attachments sealed.",
                s.attachments_sealed_total,
            ),
            (
                "paschal_attachments_bytes_total",
                "Total Attachment bytes (post-transform).",
                s.attachments_bytes_total,
            ),
            (
                "paschal_heartbeats_total",
                "Total Heartbeats received.",
                s.heartbeats_total,
            ),
            (
                "paschal_releases_started_total",
                "Total release events started (any reason).",
                s.releases_started_total,
            ),
            (
                "paschal_releases_completed_total",
                "Total release events that reached RELEASED.",
                s.releases_completed_total,
            ),
            (
                "paschal_drills_completed_total",
                "Total Drills completed.",
                s.drills_completed_total,
            ),
            (
                "paschal_buddy_responses_total",
                "Total Buddy responses received.",
                s.buddy_responses_total,
            ),
            (
                "paschal_apple_pings_total",
                "Total Apple Shortcut pings verified.",
                s.apple_pings_total,
            ),
            (
                "paschal_subscription_canceled_total",
                "Total Subscriptions transitioned to CANCELED.",
                s.subscription_canceled_total,
            ),
            (
                "paschal_subscription_reactivated_total",
                "Total Subscriptions reactivated within retention.",
                s.subscription_reactivated_total,
            ),
        ];
        for (name, help, value) in lines {
            out.push_str(&format!("# HELP {name} {help}\n"));
            out.push_str(&format!("# TYPE {name} counter\n"));
            out.push_str(&format!("{name} {value}\n"));
        }
        out
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Serialize, Clone)]
pub struct MetricsSnapshot {
    pub signups_total: u64,
    pub vaults_created_total: u64,
    pub letters_sealed_total: u64,
    pub attachments_sealed_total: u64,
    pub attachments_bytes_total: u64,
    pub heartbeats_total: u64,
    pub releases_started_total: u64,
    pub releases_completed_total: u64,
    pub drills_completed_total: u64,
    pub buddy_responses_total: u64,
    pub apple_pings_total: u64,
    pub subscription_canceled_total: u64,
    pub subscription_reactivated_total: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_start_at_zero() {
        let m = Metrics::new();
        let s = m.snapshot();
        assert_eq!(s.signups_total, 0);
        assert_eq!(s.releases_completed_total, 0);
    }

    #[test]
    fn inc_increments() {
        let m = Metrics::new();
        Metrics::inc(&m.signups_total);
        Metrics::inc(&m.signups_total);
        assert_eq!(m.snapshot().signups_total, 2);
    }

    #[test]
    fn add_accumulates() {
        let m = Metrics::new();
        Metrics::add(&m.attachments_bytes_total, 1024);
        Metrics::add(&m.attachments_bytes_total, 2048);
        assert_eq!(m.snapshot().attachments_bytes_total, 3072);
    }

    #[test]
    fn prometheus_format_is_valid() {
        let m = Metrics::new();
        Metrics::inc(&m.signups_total);
        let text = m.prometheus();
        assert!(text.contains("# HELP paschal_signups_total"));
        assert!(text.contains("# TYPE paschal_signups_total counter"));
        assert!(text.contains("paschal_signups_total 1"));
    }
}
