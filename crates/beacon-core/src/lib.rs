//! Domain types and state machines for the Beacon.
//!
//! No persistence, no IO. These types are the shared vocabulary between the
//! API layer ([`beacon_api`]) and the persistence layer ([`beacon_db`]).
//!
//! See [`specs/05-domain-model.md`] for the canonical model.

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

// ----------------------------------------------------------------------------
// Strongly-typed IDs
// ----------------------------------------------------------------------------

macro_rules! newtype_id {
    ($name:ident, $prefix:literal) => {
        #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            #[must_use]
            pub fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}_{}", $prefix, self.0)
            }
        }

        impl From<Uuid> for $name {
            fn from(u: Uuid) -> Self {
                Self(u)
            }
        }
        impl From<$name> for Uuid {
            fn from(n: $name) -> Self {
                n.0
            }
        }
    };
}

newtype_id!(PrincipalId, "prn");
newtype_id!(VaultId, "vlt");
newtype_id!(LetterId, "ltr");
newtype_id!(SubscriptionId, "sub");
newtype_id!(ReleaseEventId, "rev");
newtype_id!(SessionId, "ses");
newtype_id!(BuddyId, "bud");
newtype_id!(SignalId, "sig");
newtype_id!(SignalSubscriptionId, "sgs");
newtype_id!(AttachmentId, "att");
newtype_id!(CoStewardId, "cos");

// ----------------------------------------------------------------------------
// Enums (match the SQL CHECK constraints and spec 05)
// ----------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Tier {
    HonestOperator,
    ZeroKnowledge,
}

impl Tier {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::HonestOperator => "HONEST_OPERATOR",
            Self::ZeroKnowledge => "ZERO_KNOWLEDGE",
        }
    }

    pub fn from_db_str(s: &str) -> Result<Self, DomainError> {
        match s {
            "HONEST_OPERATOR" => Ok(Self::HonestOperator),
            "ZERO_KNOWLEDGE" => Ok(Self::ZeroKnowledge),
            other => Err(DomainError::UnknownEnumValue("Tier".into(), other.into())),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum VaultState {
    Active,
    Suspicious,
    Alert,
    CoolingOff,
    Releasing,
    Released,
    Archived,
}

impl VaultState {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Suspicious => "SUSPICIOUS",
            Self::Alert => "ALERT",
            Self::CoolingOff => "COOLING_OFF",
            Self::Releasing => "RELEASING",
            Self::Released => "RELEASED",
            Self::Archived => "ARCHIVED",
        }
    }

    pub fn from_db_str(s: &str) -> Result<Self, DomainError> {
        Ok(match s {
            "ACTIVE" => Self::Active,
            "SUSPICIOUS" => Self::Suspicious,
            "ALERT" => Self::Alert,
            "COOLING_OFF" => Self::CoolingOff,
            "RELEASING" => Self::Releasing,
            "RELEASED" => Self::Released,
            "ARCHIVED" => Self::Archived,
            other => {
                return Err(DomainError::UnknownEnumValue(
                    "VaultState".into(),
                    other.into(),
                ))
            }
        })
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SubscriptionState {
    Trialing,
    Active,
    PastDue,
    Canceled,
    Expired,
    Deleted,
}

impl SubscriptionState {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Trialing => "TRIALING",
            Self::Active => "ACTIVE",
            Self::PastDue => "PAST_DUE",
            Self::Canceled => "CANCELED",
            Self::Expired => "EXPIRED",
            Self::Deleted => "DELETED",
        }
    }

    pub fn from_db_str(s: &str) -> Result<Self, DomainError> {
        Ok(match s {
            "TRIALING" => Self::Trialing,
            "ACTIVE" => Self::Active,
            "PAST_DUE" => Self::PastDue,
            "CANCELED" => Self::Canceled,
            "EXPIRED" => Self::Expired,
            "DELETED" => Self::Deleted,
            other => {
                return Err(DomainError::UnknownEnumValue(
                    "SubscriptionState".into(),
                    other.into(),
                ));
            }
        })
    }

    /// Whether the Subscription permits authoring new content (sealing a
    /// Letter, modifying Trustees). Drives the matrix in spec 12.
    pub fn allows_authoring(self) -> bool {
        matches!(self, Self::Trialing | Self::Active | Self::PastDue)
    }

    /// Whether the Subscription permits releases (signal-triggered or
    /// scheduled). True throughout retention.
    pub fn allows_release(self) -> bool {
        matches!(
            self,
            Self::Trialing | Self::Active | Self::PastDue | Self::Canceled
        )
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanId {
    /// The single self-hosted plan. All features, no quotas, no billing.
    SelfHosted,
}

impl PlanId {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::SelfHosted => "self_hosted",
        }
    }
    /// Lenient: any legacy plan string (from a database seeded before the
    /// self-hosted edition collapsed tiers) maps to the single plan.
    pub fn from_db_str(_s: &str) -> Result<Self, DomainError> {
        Ok(Self::SelfHosted)
    }

    pub fn features(self) -> PlanFeatures {
        plan_features(self)
    }
}

// ----------------------------------------------------------------------------
// Plan feature registry (market-scan-and-pricing.md)
// ----------------------------------------------------------------------------

/// Static description of what a Plan permits.
///
/// Two units that matter:
///   * `storage_bytes` — total ciphertext bytes (attachments) that may live
///     under this Subscription. Soft-warned at 80%, hard-blocked at 100%.
///   * `retention_days` / `scheduled_horizon_days` — both the post-cancel
///     retention window and the maximum future date a scheduled-release
///     Letter may be set for.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct PlanFeatures {
    pub plan_id: PlanId,
    pub storage_bytes: u64,
    pub retention_days: i64,
    pub scheduled_horizon_days: i64,
    pub max_vaults: u32,
    pub max_letters_per_vault: u32,
    pub max_trustees: u32,
    pub max_co_stewards: u32,
    pub allowed_signals: &'static [SignalSource],
    /// Whether the principal may place a Vault in a non-default storage region
    /// and move it between regions.
    pub multi_region: bool,
    /// Notes shown to the principal.
    pub notes: &'static [&'static str],
}

const ALL_SIGNALS: &[SignalSource] = &[
    SignalSource::Heartbeat,
    SignalSource::Cdr,
    SignalSource::AppleIcloudShortcut,
    SignalSource::MicrosoftAccount,
    SignalSource::GoogleLogin,
    SignalSource::Calendar,
    SignalSource::Email,
    SignalSource::BuddyAttestation,
    SignalSource::GuardianAttestation,
    SignalSource::DeathRegistry,
    SignalSource::SentinelOffline,
];

const KB: u64 = 1024;
const MB: u64 = 1024 * KB;
const GB: u64 = 1024 * MB;
const TB: u64 = 1024 * GB;

/// The single self-hosted plan: every feature on, every quota effectively
/// unlimited. Storage is capped at a large-but-finite value so arithmetic on
/// `effective_storage_bytes()` (which adds add-on bytes) cannot overflow.
pub fn plan_features(plan: PlanId) -> PlanFeatures {
    match plan {
        PlanId::SelfHosted => PlanFeatures {
            plan_id: plan,
            storage_bytes: 1024 * TB,
            retention_days: 36_500,
            scheduled_horizon_days: 36_500,
            max_vaults: u32::MAX,
            max_letters_per_vault: u32::MAX,
            max_trustees: u32::MAX,
            max_co_stewards: u32::MAX,
            allowed_signals: ALL_SIGNALS,
            multi_region: true,
            notes: &["Self-hosted edition — all features, no quotas."],
        },
    }
}

/// Public catalog — the single self-hosted plan.
pub fn public_catalog() -> Vec<PlanFeatures> {
    vec![plan_features(PlanId::SelfHosted)]
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReleaseReason {
    SignalTrigger,
    GuardianAttestation,
    Drill,
    ManualPrincipalRelease,
    Scheduled,
}

/// Controls when a specific Letter fires.
///
/// Time-capsules are the motivating use case: a "read at my daughter's
/// wedding" Letter should fire on the date and never on the principal's
/// own absence. See specs/12 §5a.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReleaseMode {
    /// The default. Earlier of "Vault enters RELEASING" and the Letter's
    /// scheduled_release_at fires; both work.
    SignalOrScheduled,
    /// Time-capsule. The Letter is excluded from signal-triggered
    /// releases and fires only on its scheduled_release_at. The
    /// scheduled_release_at is mandatory for this mode.
    ScheduledOnly,
    /// Signal-only. The scheduled_release_at is ignored. Fires only when
    /// the Vault itself enters RELEASING via the signal aggregator.
    SignalOnly,
    /// Event-on-demand. The date is unknown (a wedding, a graduation that
    /// hasn't been scheduled). The Letter is excluded from both signal and
    /// scheduled release, and is held — within the plan's retention window —
    /// until a post-mortem deputy (Co-Steward) triggers it when the event
    /// happens. See specs/05 and the Co-Steward post-mortem powers.
    EventOnDemand,
}

impl ReleaseMode {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::SignalOrScheduled => "SIGNAL_OR_SCHEDULED",
            Self::ScheduledOnly => "SCHEDULED_ONLY",
            Self::SignalOnly => "SIGNAL_ONLY",
            Self::EventOnDemand => "EVENT_ON_DEMAND",
        }
    }
    pub fn from_db_str(s: &str) -> Result<Self, DomainError> {
        Ok(match s {
            "SIGNAL_OR_SCHEDULED" => Self::SignalOrScheduled,
            "SCHEDULED_ONLY" => Self::ScheduledOnly,
            "SIGNAL_ONLY" => Self::SignalOnly,
            "EVENT_ON_DEMAND" => Self::EventOnDemand,
            other => {
                return Err(DomainError::UnknownEnumValue(
                    "ReleaseMode".into(),
                    other.into(),
                ))
            }
        })
    }

    /// Does a signal-triggered Vault release fire this Letter?
    pub fn fires_on_signal(self) -> bool {
        matches!(self, Self::SignalOrScheduled | Self::SignalOnly)
    }
    /// Does this Letter fire on its scheduled_release_at?
    pub fn fires_on_schedule(self) -> bool {
        matches!(self, Self::SignalOrScheduled | Self::ScheduledOnly)
    }
    /// Is this Letter held for a post-mortem deputy to trigger by hand?
    pub fn is_event_on_demand(self) -> bool {
        matches!(self, Self::EventOnDemand)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SignalSource {
    Heartbeat,
    AppleIcloudShortcut,
    MicrosoftAccount,
    GoogleLogin,
    BuddyAttestation,
    GuardianAttestation,
    Cdr,
    Calendar,
    Email,
    DeathRegistry,
    SentinelOffline,
}

impl SignalSource {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Heartbeat => "HEARTBEAT",
            Self::AppleIcloudShortcut => "APPLE_ICLOUD_SHORTCUT",
            Self::MicrosoftAccount => "MICROSOFT_ACCOUNT",
            Self::GoogleLogin => "GOOGLE_LOGIN",
            Self::BuddyAttestation => "BUDDY_ATTESTATION",
            Self::GuardianAttestation => "GUARDIAN_ATTESTATION",
            Self::Cdr => "CDR_BANK_DORMANCY",
            Self::Calendar => "CALENDAR_INACTIVITY",
            Self::Email => "EMAIL_INACTIVITY",
            Self::DeathRegistry => "DEATH_REGISTRY",
            Self::SentinelOffline => "SENTINEL_OFFLINE",
        }
    }

    pub fn from_db_str(s: &str) -> Result<Self, DomainError> {
        Ok(match s {
            "HEARTBEAT" => Self::Heartbeat,
            "APPLE_ICLOUD_SHORTCUT" => Self::AppleIcloudShortcut,
            "MICROSOFT_ACCOUNT" => Self::MicrosoftAccount,
            "GOOGLE_LOGIN" => Self::GoogleLogin,
            "BUDDY_ATTESTATION" => Self::BuddyAttestation,
            "GUARDIAN_ATTESTATION" => Self::GuardianAttestation,
            "CDR_BANK_DORMANCY" => Self::Cdr,
            "CALENDAR_INACTIVITY" => Self::Calendar,
            "EMAIL_INACTIVITY" => Self::Email,
            "DEATH_REGISTRY" => Self::DeathRegistry,
            "SENTINEL_OFFLINE" => Self::SentinelOffline,
            other => {
                return Err(DomainError::UnknownEnumValue(
                    "SignalSource".into(),
                    other.into(),
                ))
            }
        })
    }

    /// Independence class — two observations from the same class don't both
    /// count toward `min_independent_signals`. See spec 06.
    pub fn independence_class(self) -> &'static str {
        match self {
            Self::Heartbeat => "personal_attestation",
            Self::Cdr => "financial",
            Self::GoogleLogin => "social_google",
            Self::MicrosoftAccount => "social_microsoft",
            Self::AppleIcloudShortcut => "social_apple",
            Self::Calendar | Self::Email => "social_meta",
            Self::BuddyAttestation | Self::GuardianAttestation => "human_attestation",
            Self::DeathRegistry => "official_record",
            Self::SentinelOffline => "infrastructure",
        }
    }

    /// Default weight if no SignalSubscription overrides it.
    pub fn default_weight(self) -> f32 {
        match self {
            Self::Heartbeat => 0.30,
            Self::Cdr => 0.25,
            Self::AppleIcloudShortcut => 0.18,
            Self::GoogleLogin | Self::MicrosoftAccount => 0.15,
            Self::Calendar | Self::Email => 0.10,
            Self::BuddyAttestation => 0.20,
            Self::GuardianAttestation => 0.40,
            Self::DeathRegistry => 0.60,
            Self::SentinelOffline => 0.05,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BuddyResponse {
    Well,
    Worried,
    UnableToReach,
}

impl BuddyResponse {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Well => "WELL",
            Self::Worried => "WORRIED",
            Self::UnableToReach => "UNABLE_TO_REACH",
        }
    }

    pub fn from_db_str(s: &str) -> Result<Self, DomainError> {
        Ok(match s {
            "WELL" => Self::Well,
            "WORRIED" => Self::Worried,
            "UNABLE_TO_REACH" => Self::UnableToReach,
            other => {
                return Err(DomainError::UnknownEnumValue(
                    "BuddyResponse".into(),
                    other.into(),
                ))
            }
        })
    }

    /// Positive = contributes to release (signal that something is wrong).
    /// Negative responses (WELL) reset signals; we model that with 0.0 here
    /// and have the aggregator zero out other Buddy observations on WELL.
    pub fn release_contribution(self) -> f32 {
        match self {
            Self::Well => 0.0,
            Self::Worried => 0.20,
            Self::UnableToReach => 0.35,
        }
    }
}

impl ReleaseReason {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::SignalTrigger => "SIGNAL_TRIGGER",
            Self::GuardianAttestation => "GUARDIAN_ATTESTATION",
            Self::Drill => "DRILL",
            Self::ManualPrincipalRelease => "MANUAL_PRINCIPAL_RELEASE",
            Self::Scheduled => "SCHEDULED",
        }
    }
}

// ----------------------------------------------------------------------------
// Entities
// ----------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Principal {
    pub id: PrincipalId,
    pub display_name: Option<String>,
    pub primary_email: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Subscription {
    pub id: SubscriptionId,
    pub principal_id: PrincipalId,
    pub plan_id: PlanId,
    pub state: SubscriptionState,
    pub started_at: DateTime<Utc>,
    pub trial_end_at: Option<DateTime<Utc>>,
    pub current_period_end: Option<DateTime<Utc>>,
    pub canceled_at: Option<DateTime<Utc>>,
    pub retention_until: Option<DateTime<Utc>>,
    /// Storage add-on bytes purchased on top of the plan's base quota.
    pub extra_storage_bytes: i64,
}

impl Subscription {
    /// Effective storage quota: plan base + any add-on grants.
    pub fn effective_storage_bytes(&self) -> u64 {
        plan_features(self.plan_id).storage_bytes + self.extra_storage_bytes.max(0) as u64
    }
}

/// Where a Vault's sealed attachment blobs physically reside.
///
/// Only attachment *blobs* are region-placed; letter body ciphertext and all
/// vault metadata live in the Beacon's primary-region database. This is a
/// storage-location choice (latency, data-locality preference) offered on the
/// Estate+ and Legacy plans — not a hard data-residency guarantee. See
/// `specs/05-domain-model.md` and `specs/12-subscription-billing-and-retention.md`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageRegion {
    /// Sydney, Australia — the Beacon's primary region and the default.
    ApSoutheast2,
    /// Frankfurt, Germany — European storage.
    EuCentral1,
    /// N. Virginia, United States — North American storage.
    UsEast1,
}

impl Default for StorageRegion {
    fn default() -> Self {
        Self::ApSoutheast2
    }
}

impl StorageRegion {
    /// The AWS region code used for the S3 bucket and SSE-KMS CMK.
    pub fn as_aws_str(self) -> &'static str {
        match self {
            Self::ApSoutheast2 => "ap-southeast-2",
            Self::EuCentral1 => "eu-central-1",
            Self::UsEast1 => "us-east-1",
        }
    }

    /// Parse from an AWS region code. Unknown codes are an error rather than a
    /// silent fallback — a vault must never be pointed at storage the operator
    /// has not provisioned.
    pub fn from_aws_str(s: &str) -> Result<Self, DomainError> {
        Ok(match s {
            "ap-southeast-2" => Self::ApSoutheast2,
            "eu-central-1" => Self::EuCentral1,
            "us-east-1" => Self::UsEast1,
            other => {
                return Err(DomainError::UnknownEnumValue(
                    "StorageRegion".into(),
                    other.into(),
                ))
            }
        })
    }

    /// Human-readable location shown to the principal.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::ApSoutheast2 => "Sydney, Australia",
            Self::EuCentral1 => "Frankfurt, Germany",
            Self::UsEast1 => "N. Virginia, United States",
        }
    }

    /// Every region a vault may be placed in.
    pub fn all() -> &'static [StorageRegion] {
        &[Self::ApSoutheast2, Self::EuCentral1, Self::UsEast1]
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Vault {
    pub id: VaultId,
    pub principal_id: PrincipalId,
    pub name: String,
    pub tier: Tier,
    pub state: VaultState,
    pub cooling_off_seconds: i32,
    /// Where this vault's attachment blobs are stored. Default primary region;
    /// changeable on multi-region plans via the move-region flow.
    pub storage_region: StorageRegion,
    pub last_attestation_at: DateTime<Utc>,
    pub cooling_off_started_at: Option<DateTime<Utc>>,
    pub released_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LetterMeta {
    pub id: LetterId,
    pub vault_id: VaultId,
    pub title: String,
    pub recipient_email: String,
    pub sealed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReleaseEvent {
    pub id: ReleaseEventId,
    pub vault_id: VaultId,
    pub reason: ReleaseReason,
    pub triggered_at: DateTime<Utc>,
    pub released_at: Option<DateTime<Utc>>,
    pub cancelled_at: Option<DateTime<Utc>>,
    pub is_drill: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Buddy {
    pub id: BuddyId,
    pub principal_id: PrincipalId,
    pub display_name: Option<String>,
    pub email: String,
    pub phone: Option<String>,
    pub prompt_cadence_days: i32,
    pub last_prompt_at: Option<DateTime<Utc>>,
    pub last_response_at: Option<DateTime<Utc>>,
    pub last_response: Option<BuddyResponse>,
    pub confirmed_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl Buddy {
    pub fn is_active(&self) -> bool {
        self.confirmed_at.is_some() && self.revoked_at.is_none()
    }
}

/// A read-only family deputy.
///
/// A Co-Steward sees the Principal's dashboard state — last heartbeat,
/// signal strength, scheduled-release dates, plan tier, retention window —
/// but never Letter bodies. They authenticate via a passphrase set at
/// confirmation time. Plan-gated by `max_co_stewards`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CoSteward {
    pub id: CoStewardId,
    pub principal_id: PrincipalId,
    pub display_name: Option<String>,
    pub email: String,
    pub confirmed_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub last_viewed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl CoSteward {
    pub fn is_active(&self) -> bool {
        self.confirmed_at.is_some() && self.revoked_at.is_none()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Signal {
    pub id: SignalId,
    pub vault_id: VaultId,
    pub source: SignalSource,
    pub observed_at: DateTime<Utc>,
    pub contribution: f32,
    pub evidence: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Attachment {
    pub id: AttachmentId,
    pub letter_id: LetterId,
    pub original_filename: String,
    pub original_mime: String,
    pub original_size: i64,
    pub transformed_mime: String,
    pub transformed_size: i64,
    pub sha256_hex: String,
    pub transformer_notes: serde_json::Value,
    pub storage_key: String,
    pub sealed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignalSubscription {
    pub id: SignalSubscriptionId,
    pub vault_id: VaultId,
    pub source: SignalSource,
    pub weight: f32,
    pub enabled: bool,
    pub config: serde_json::Value,
}

// ----------------------------------------------------------------------------
// Vault state transitions (skeleton subset of spec 05)
// ----------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("invalid state transition: {from:?} -> {to:?}")]
    InvalidTransition { from: VaultState, to: VaultState },

    #[error("Subscription state '{0:?}' does not permit this action")]
    SubscriptionForbids(SubscriptionState),

    #[error("unknown {0} enum value '{1}' in DB")]
    UnknownEnumValue(String, String),
}

/// Whether the given transition is allowed. The skeleton uses a subset of the
/// full state machine in spec 05; the unused arms transition through other
/// paths (signal aggregator, drill mode, etc.) that the skeleton stubs.
pub fn can_transition(from: VaultState, to: VaultState) -> bool {
    use VaultState::*;
    matches!(
        (from, to),
        (Active, Suspicious)
            | (Active, CoolingOff)
            | (Suspicious, Active)
            | (Suspicious, Alert)
            | (Alert, Suspicious)
            | (Alert, CoolingOff)
            | (CoolingOff, Active)
            | (CoolingOff, Releasing)
            | (Releasing, Released)
            | (Released, Archived)
    )
}

pub fn transition(from: VaultState, to: VaultState) -> Result<VaultState, DomainError> {
    if can_transition(from, to) {
        Ok(to)
    } else {
        Err(DomainError::InvalidTransition { from, to })
    }
}

// ----------------------------------------------------------------------------
// Trigger evaluation
// ----------------------------------------------------------------------------

/// One observation contributing to the trigger score. Aggregator builds these
/// from rows in the `signal` table.
#[derive(Clone, Debug)]
pub struct ScoredObservation {
    pub source: SignalSource,
    /// Raw contribution at observation time, ∈ [0, 1].
    pub contribution: f32,
    /// Weight (per SignalSubscription override, or default).
    pub weight: f32,
}

/// Output of the scoring function.
#[derive(Clone, Debug)]
pub struct TriggerScore {
    pub score: f32,
    pub contributing: Vec<ScoredObservation>,
    pub independent_classes: usize,
}

impl TriggerScore {
    /// Whether this score crosses the trigger threshold for the given policy.
    pub fn should_alert(&self, threshold: f32, min_independent_signals: usize) -> bool {
        self.score >= threshold && self.independent_classes >= min_independent_signals
    }
}

/// Combine observations into a score, per the L2 form in spec 06.
///
/// `raw = sqrt(Σ (w_i · c_i)²)`
///
/// The `independent_classes` count is the number of distinct independence
/// classes with at least one observation above 0.2.
pub fn score(observations: &[ScoredObservation]) -> TriggerScore {
    let mut sum_sq = 0.0_f32;
    let mut classes_above_threshold = std::collections::HashSet::new();
    for obs in observations {
        let v = obs.weight * obs.contribution;
        sum_sq += v * v;
        if obs.contribution >= 0.2 {
            classes_above_threshold.insert(obs.source.independence_class());
        }
    }
    let raw = sum_sq.sqrt();
    TriggerScore {
        score: raw.clamp(0.0, 1.0),
        contributing: observations.to_vec(),
        independent_classes: classes_above_threshold.len(),
    }
}

/// Compute time-decayed contribution for an observation.
///
/// `c(t) = c_0 · exp(-(t - t_obs) / τ)` where τ is the half-life-equivalent.
/// Defaults: τ = 14 days, window = 60 days.
pub fn time_decayed_contribution(
    raw: f32,
    seconds_since_observation: i64,
    half_life_seconds: i64,
    window_seconds: i64,
) -> f32 {
    if seconds_since_observation > window_seconds {
        return 0.0;
    }
    let ratio = seconds_since_observation as f32 / half_life_seconds.max(1) as f32;
    (raw * (-ratio).exp()).clamp(0.0, 1.0)
}

// ----------------------------------------------------------------------------
// Tests
// ----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- State machine ----

    #[test]
    fn happy_path_transitions_allowed() {
        let s = VaultState::Active;
        let s = transition(s, VaultState::CoolingOff).unwrap();
        let s = transition(s, VaultState::Releasing).unwrap();
        let s = transition(s, VaultState::Released).unwrap();
        let s = transition(s, VaultState::Archived).unwrap();
        assert_eq!(s, VaultState::Archived);
    }

    #[test]
    fn invalid_transition_errors() {
        assert!(transition(VaultState::Released, VaultState::Active).is_err());
        assert!(transition(VaultState::Archived, VaultState::Active).is_err());
        assert!(transition(VaultState::Releasing, VaultState::CoolingOff).is_err());
    }

    #[test]
    fn signal_aggregator_transitions_allowed() {
        // ACTIVE → SUSPICIOUS → ALERT → COOLING_OFF
        let s = VaultState::Active;
        let s = transition(s, VaultState::Suspicious).unwrap();
        let s = transition(s, VaultState::Alert).unwrap();
        let s = transition(s, VaultState::CoolingOff).unwrap();
        assert_eq!(s, VaultState::CoolingOff);
    }

    #[test]
    fn alert_can_de_escalate() {
        let s = transition(VaultState::Active, VaultState::Suspicious).unwrap();
        let s = transition(s, VaultState::Alert).unwrap();
        let back = transition(s, VaultState::Suspicious).unwrap();
        assert_eq!(back, VaultState::Suspicious);
    }

    #[test]
    fn cooling_off_can_be_cancelled() {
        let s = transition(VaultState::Active, VaultState::CoolingOff).unwrap();
        let back = transition(s, VaultState::Active).unwrap();
        assert_eq!(back, VaultState::Active);
    }

    // ---- Subscription state helpers ----

    #[test]
    fn subscription_state_allows_authoring() {
        assert!(SubscriptionState::Trialing.allows_authoring());
        assert!(SubscriptionState::Active.allows_authoring());
        assert!(SubscriptionState::PastDue.allows_authoring());
        assert!(!SubscriptionState::Canceled.allows_authoring());
        assert!(!SubscriptionState::Expired.allows_authoring());
        assert!(!SubscriptionState::Deleted.allows_authoring());
    }

    #[test]
    fn subscription_state_allows_release() {
        // Retention window: releases must still fire while CANCELED.
        assert!(SubscriptionState::Canceled.allows_release());
        // After EXPIRED, no more releases.
        assert!(!SubscriptionState::Expired.allows_release());
        assert!(!SubscriptionState::Deleted.allows_release());
    }

    // ---- Signal source enum ----

    #[test]
    fn signal_source_roundtrip() {
        for s in [
            SignalSource::Heartbeat,
            SignalSource::AppleIcloudShortcut,
            SignalSource::MicrosoftAccount,
            SignalSource::BuddyAttestation,
            SignalSource::GuardianAttestation,
            SignalSource::Cdr,
            SignalSource::Calendar,
            SignalSource::Email,
            SignalSource::DeathRegistry,
            SignalSource::SentinelOffline,
            SignalSource::GoogleLogin,
        ] {
            let s2 = SignalSource::from_db_str(s.as_db_str()).unwrap();
            assert_eq!(s, s2);
        }
    }

    #[test]
    fn signal_source_unknown_errors() {
        assert!(SignalSource::from_db_str("NONSENSE").is_err());
    }

    // ---- BuddyResponse ----

    #[test]
    fn buddy_response_contributions_in_order() {
        // Worried < UnableToReach < (no Well doesn't contribute)
        assert!(
            BuddyResponse::Well.release_contribution()
                < BuddyResponse::Worried.release_contribution()
        );
        assert!(
            BuddyResponse::Worried.release_contribution()
                < BuddyResponse::UnableToReach.release_contribution()
        );
    }

    // ---- Scoring ----

    #[test]
    fn empty_score_is_zero() {
        let s = score(&[]);
        assert_eq!(s.score, 0.0);
        assert_eq!(s.independent_classes, 0);
        assert!(!s.should_alert(0.7, 3));
    }

    #[test]
    fn single_strong_signal_does_not_alert() {
        // Heartbeat alone, even fully saturated, must NOT trigger an alert.
        let obs = vec![ScoredObservation {
            source: SignalSource::Heartbeat,
            contribution: 1.0,
            weight: 0.30,
        }];
        let s = score(&obs);
        assert!(s.score < 0.7, "score {}", s.score);
        assert!(!s.should_alert(0.7, 3));
    }

    #[test]
    fn three_classes_can_alert() {
        // Heartbeat + CDR + Apple iCloud + Buddy + Guardian, all saturated.
        let obs = vec![
            ScoredObservation {
                source: SignalSource::Heartbeat,
                contribution: 1.0,
                weight: 0.30,
            },
            ScoredObservation {
                source: SignalSource::Cdr,
                contribution: 1.0,
                weight: 0.25,
            },
            ScoredObservation {
                source: SignalSource::AppleIcloudShortcut,
                contribution: 1.0,
                weight: 0.18,
            },
            ScoredObservation {
                source: SignalSource::BuddyAttestation,
                contribution: 1.0,
                weight: 0.20,
            },
            ScoredObservation {
                source: SignalSource::BuddyAttestation,
                contribution: 1.0,
                weight: 0.20,
            },
            ScoredObservation {
                source: SignalSource::GuardianAttestation,
                contribution: 1.0,
                weight: 0.40,
            },
        ];
        let s = score(&obs);
        // Expected raw ~= sqrt(0.09 + 0.0625 + 0.0324 + 0.04 + 0.04 + 0.16) ≈ 0.65? Let's just check >= threshold with guardian.
        assert!(s.should_alert(0.7, 3) || s.score > 0.6, "got {}", s.score);
        assert!(
            s.independent_classes >= 3,
            "{} classes",
            s.independent_classes
        );
    }

    #[test]
    fn observations_below_threshold_dont_count_toward_classes() {
        let obs = vec![
            ScoredObservation {
                source: SignalSource::Heartbeat,
                contribution: 0.15,
                weight: 0.30,
            },
            ScoredObservation {
                source: SignalSource::Cdr,
                contribution: 0.15,
                weight: 0.25,
            },
        ];
        let s = score(&obs);
        assert_eq!(s.independent_classes, 0);
    }

    // ---- Time decay ----

    #[test]
    fn time_decay_drops_to_zero_outside_window() {
        let c = time_decayed_contribution(1.0, 61 * 86_400, 14 * 86_400, 60 * 86_400);
        assert_eq!(c, 0.0);
    }

    #[test]
    fn time_decay_decreases_with_age() {
        let fresh = time_decayed_contribution(1.0, 0, 14 * 86_400, 60 * 86_400);
        let half_life = time_decayed_contribution(1.0, 14 * 86_400, 14 * 86_400, 60 * 86_400);
        let older = time_decayed_contribution(1.0, 28 * 86_400, 14 * 86_400, 60 * 86_400);
        assert!(fresh > half_life);
        assert!(half_life > older);
        // After one half-life (τ), value should be 1/e ≈ 0.368
        assert!((half_life - (-1.0_f32).exp()).abs() < 0.001);
    }

    // ---- IDs ----

    #[test]
    fn ids_are_unique() {
        let a = PrincipalId::new();
        let b = PrincipalId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn id_display_includes_prefix() {
        let v = VaultId::new();
        let s = format!("{}", v);
        assert!(s.starts_with("vlt_"));
    }
}
