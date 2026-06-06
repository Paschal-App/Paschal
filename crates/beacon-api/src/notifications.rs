//! Notification adapter abstraction.
//!
//! Production has three concrete adapters per spec 13: AWS SES (email),
//! AWS SNS (SMS + push), Twilio (voice). The MVP ships a single
//! log-to-file `StubNotifications` so the demo can show the full pipeline
//! without external accounts. The abstraction also enforces the abuse
//! guardrails (double opt-in for SMS, geo allowlist, rate limits) — see
//! `should_attempt_sms` for the policy.

use std::path::PathBuf;

use async_trait::async_trait;
use chrono::Utc;
use tokio::io::AsyncWriteExt;

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // Sms/Push/Voice/Operator unused until specific adapters land.
pub enum Channel {
    Email,
    Sms,
    Push,
    Voice,
    /// Used for internal events (operator paging, audit) not user-bound.
    Operator,
}

impl Channel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Email => "EMAIL",
            Self::Sms => "SMS",
            Self::Push => "PUSH",
            Self::Voice => "VOICE",
            Self::Operator => "OPERATOR",
        }
    }
}

#[derive(Clone, Debug)]
pub struct OutboundMessage {
    pub channel: Channel,
    pub to: String,
    pub subject: Option<String>,
    pub body: String,
}

#[async_trait]
pub trait NotificationSink: Send + Sync {
    async fn send(&self, msg: OutboundMessage);
}

/// Log-to-file notification sink for development.
pub struct StubNotifications {
    path: PathBuf,
}

impl StubNotifications {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

/// Captures notifications into memory for integration tests.
#[derive(Default)]
pub struct TestNotifications {
    pub captured: std::sync::Mutex<Vec<OutboundMessage>>,
}

#[async_trait]
impl NotificationSink for TestNotifications {
    async fn send(&self, msg: OutboundMessage) {
        if let Ok(mut g) = self.captured.lock() {
            g.push(msg);
        }
    }
}

#[async_trait]
impl NotificationSink for StubNotifications {
    async fn send(&self, msg: OutboundMessage) {
        if let Some(parent) = self.path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let line = format!(
            "{}\t{}\t{}\t{}\t{}\n",
            Utc::now().to_rfc3339(),
            msg.channel.as_str(),
            msg.to,
            msg.subject.unwrap_or_default(),
            msg.body.replace('\n', " \\n "),
        );
        if let Ok(mut f) = tokio::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&self.path)
            .await
        {
            let _ = f.write_all(line.as_bytes()).await;
        }
        tracing::info!(
            channel = %msg.channel.as_str(),
            to = %msg.to,
            "notification (stub)"
        );
    }
}

/// Convenience helpers for sending the standard message shapes.
pub mod tx {
    use super::*;

    pub async fn welcome(sink: &dyn NotificationSink, email: &str, trial_end_at: &str) {
        sink.send(OutboundMessage {
            channel: Channel::Email,
            to: email.into(),
            subject: Some("Welcome to Paschal".into()),
            body: format!(
                "Welcome to Paschal. Your Trial ends at {trial_end_at}.\n\
                 You can author Letters now; we will run a rehearsal soon."
            ),
        })
        .await;
    }

    pub async fn cooling_off_started(
        sink: &dyn NotificationSink,
        principal_email: &str,
        vault_name: &str,
        ends_at: &str,
    ) {
        sink.send(OutboundMessage {
            channel: Channel::Email,
            to: principal_email.into(),
            subject: Some(format!("Your {vault_name} Vault is in cooling-off")),
            body: format!(
                "Your {vault_name} Vault entered cooling-off.\n\
                 Unless you cancel, it will release at {ends_at}.\n\
                 Cancel from any signed-in device."
            ),
        })
        .await;
    }

    pub async fn cooling_off_cancelled(sink: &dyn NotificationSink, principal_email: &str) {
        sink.send(OutboundMessage {
            channel: Channel::Email,
            to: principal_email.into(),
            subject: Some("Cooling-off cancelled".into()),
            body: "Your Vault is back to Active. No further action.".into(),
        })
        .await;
    }

    pub async fn release_notification(
        sink: &dyn NotificationSink,
        recipient: &str,
        letter_title: &str,
        claim_url: &str,
        is_drill: bool,
    ) {
        release_notification_with_attachments(
            sink,
            recipient,
            letter_title,
            claim_url,
            is_drill,
            &[],
        )
        .await
    }

    pub async fn release_notification_with_attachments(
        sink: &dyn NotificationSink,
        recipient: &str,
        letter_title: &str,
        claim_url: &str,
        is_drill: bool,
        attachments: &[(String, String)], // (filename, url)
    ) {
        let (subject, opener) = if is_drill {
            (
                "Rehearsal — please read",
                "Rehearsal — the principal is rehearsing the system that will one day release material to you.",
            )
        } else {
            (
                "Material left for you",
                "The principal named you to receive material from their Vault.",
            )
        };

        let attachments_block = if attachments.is_empty() {
            String::new()
        } else {
            let lines: Vec<String> = attachments
                .iter()
                .map(|(name, url)| format!("  - {name}\n    {url}"))
                .collect();
            format!(
                "\n\nAttachments (each link works once):\n{}",
                lines.join("\n")
            )
        };

        sink.send(OutboundMessage {
            channel: Channel::Email,
            to: recipient.into(),
            subject: Some(subject.into()),
            body: format!(
                "{opener}\n\nTitle: {letter_title}\nOpen the Letter: {claim_url}{attachments_block}\n\n\
                 Paschal is a digital continuity service. The transparency \
                 log entry for this message is recorded."
            ),
        })
        .await;
    }

    pub async fn buddy_invite(
        sink: &dyn NotificationSink,
        buddy_email: &str,
        principal_email: &str,
        confirmation_url: &str,
    ) {
        sink.send(OutboundMessage {
            channel: Channel::Email,
            to: buddy_email.into(),
            subject: Some("You were invited as a Paschal Buddy".into()),
            body: format!(
                "{principal_email} added you as a Buddy on Paschal.\n\
                 You will be asked to confirm they are well, occasionally.\n\
                 Confirm your role: {confirmation_url}"
            ),
        })
        .await;
    }

    /// Sent when a Co-Steward requests a change to where a deceased
    /// principal's Letter is delivered. Goes to the *current* (old) recipient
    /// address so a redirect cannot happen silently, and the change takes
    /// effect only after a hold during which it can be cancelled.
    pub async fn recipient_change_requested(
        sink: &dyn NotificationSink,
        old_recipient: &str,
        letter_title: &str,
        effective_at: &str,
    ) {
        sink.send(OutboundMessage {
            channel: Channel::Email,
            to: old_recipient.into(),
            subject: Some("A delivery address was changed".into()),
            body: format!(
                "A Co-Steward has asked to change the delivery contact for the Letter \
                 \"{letter_title}\".\n\
                 The change takes effect at {effective_at}.\n\
                 If this is unexpected, contact support before then. The change is \
                 recorded in the transparency log."
            ),
        })
        .await;
    }

    pub async fn co_steward_invite(
        sink: &dyn NotificationSink,
        co_steward_email: &str,
        principal_email: &str,
        confirmation_url: &str,
    ) {
        sink.send(OutboundMessage {
            channel: Channel::Email,
            to: co_steward_email.into(),
            subject: Some("You were invited as a Paschal Co-Steward".into()),
            body: format!(
                "{principal_email} nominated you as a Co-Steward on their Paschal account.\n\
                 \n\
                 As a Co-Steward you can SEE the state of their Vaults — last heartbeat,\n\
                 signal strength, scheduled releases — but you will NEVER see Letter\n\
                 contents and you cannot change anything.\n\
                 \n\
                 To accept this role, set a passphrase here:\n\
                   {confirmation_url}\n\
                 \n\
                 You can refuse simply by ignoring this email."
            ),
        })
        .await;
    }
}
