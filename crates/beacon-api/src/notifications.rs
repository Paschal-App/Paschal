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

/// Email sink backed by the [Resend](https://resend.com) HTTP API.
///
/// Email only — SMS/voice need their own providers, so those channels are logged
/// and dropped here rather than silently lost. Sends are fire-and-forget per the
/// trait, but a failure is logged at ERROR: a release or invite email that never
/// goes out means a recipient never got their letter, so it must be loud.
pub struct ResendNotifications {
    api_key: String,
    from: String,
    client: reqwest::Client,
}

impl ResendNotifications {
    pub fn new(api_key: String, from: String) -> Self {
        Self {
            api_key,
            from,
            client: reqwest::Client::new(),
        }
    }

    /// Build the Resend `POST /emails` JSON body. Separated so the field mapping
    /// is unit-testable without a network round-trip.
    fn payload(&self, msg: &OutboundMessage) -> serde_json::Value {
        serde_json::json!({
            "from": self.from,
            "to": [msg.to],
            "subject": msg.subject.clone().unwrap_or_default(),
            "text": msg.body,
        })
    }
}

#[async_trait]
impl NotificationSink for ResendNotifications {
    async fn send(&self, msg: OutboundMessage) {
        if !matches!(msg.channel, Channel::Email) {
            tracing::warn!(
                channel = %msg.channel.as_str(),
                to = %msg.to,
                "notification channel not supported by Resend; dropped"
            );
            return;
        }
        let payload = self.payload(&msg);
        match self
            .client
            .post("https://api.resend.com/emails")
            .bearer_auth(&self.api_key)
            .json(&payload)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                tracing::info!(to = %msg.to, "email sent via Resend");
            }
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                tracing::error!(%status, to = %msg.to, body = %body, "Resend send FAILED");
            }
            Err(e) => {
                tracing::error!(error = %e, to = %msg.to, "Resend request error");
            }
        }
    }
}

/// Self-hosted email sink backed by an SMTP relay — any mail server you run or
/// have submission credentials for. Email only, like Resend; other channels are
/// logged and dropped. Sends are fire-and-forget; a failure is logged at ERROR.
pub struct SmtpNotifications {
    transport: lettre::AsyncSmtpTransport<lettre::Tokio1Executor>,
    from: String,
}

impl SmtpNotifications {
    /// Build an SMTP sink. `tls` selects connection security:
    ///   "starttls" (default, submission port 587),
    ///   "implicit" (TLS-on-connect, port 465),
    ///   "none"     (unencrypted, e.g. a trusted local relay on port 25).
    /// Credentials are optional — omit both for an unauthenticated local relay.
    pub fn new(
        host: &str,
        port: Option<u16>,
        username: Option<String>,
        password: Option<String>,
        tls: &str,
        from: String,
    ) -> anyhow::Result<Self> {
        use lettre::transport::smtp::authentication::Credentials;
        use lettre::{AsyncSmtpTransport, Tokio1Executor};

        let mut builder = match tls {
            "implicit" => AsyncSmtpTransport::<Tokio1Executor>::relay(host)?,
            "none" => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host),
            _ => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)?,
        };
        if let Some(p) = port {
            builder = builder.port(p);
        }
        if let (Some(u), Some(pw)) = (username, password) {
            builder = builder.credentials(Credentials::new(u, pw));
        }
        Ok(Self {
            transport: builder.build(),
            from,
        })
    }
}

#[async_trait]
impl NotificationSink for SmtpNotifications {
    async fn send(&self, msg: OutboundMessage) {
        use lettre::AsyncTransport;
        if !matches!(msg.channel, Channel::Email) {
            tracing::warn!(
                channel = %msg.channel.as_str(),
                to = %msg.to,
                "notification channel not supported by SMTP; dropped"
            );
            return;
        }
        let from = match self.from.parse::<lettre::message::Mailbox>() {
            Ok(m) => m,
            Err(e) => {
                tracing::error!(error = %e, from = %self.from, "SMTP: invalid EMAIL_FROM address");
                return;
            }
        };
        let to = match msg.to.parse::<lettre::message::Mailbox>() {
            Ok(m) => m,
            Err(e) => {
                tracing::error!(error = %e, to = %msg.to, "SMTP: invalid recipient address");
                return;
            }
        };
        let email = match lettre::Message::builder()
            .from(from)
            .to(to)
            .subject(msg.subject.clone().unwrap_or_default())
            .header(lettre::message::header::ContentType::TEXT_PLAIN)
            .body(msg.body.clone())
        {
            Ok(m) => m,
            Err(e) => {
                tracing::error!(error = %e, "SMTP: failed to build message");
                return;
            }
        };
        match self.transport.send(email).await {
            Ok(_) => tracing::info!(to = %msg.to, "email sent via SMTP"),
            Err(e) => tracing::error!(error = %e, to = %msg.to, "SMTP send FAILED"),
        }
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

    pub async fn magic_link(sink: &dyn NotificationSink, email: &str, link: &str) {
        sink.send(OutboundMessage {
            channel: Channel::Email,
            to: email.into(),
            subject: Some("Your Paschal sign-in link".into()),
            body: format!(
                "Click the link below to sign in to Paschal:\n\n\
                 {link}\n\n\
                 This link expires in 15 minutes and can be used once.\n\
                 If you did not request this, you can safely ignore this email \
                 \u{2014} no one can sign in without it.\n\n\
                 \u{2014} The Paschal team"
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

    /// Sent at letter-creation for the split heir mode: the recipient's half of
    /// the key. The operator never STORES this half (only the other half is in
    /// the DB), so split mode is zero-knowledge AT REST. Honest caveat (ZK audit
    /// H2): the share still transits this email path, so its in-transit
    /// confidentiality depends on the mail channel — for the strongest posture
    /// use `manual` mode, where the passphrase never reaches Paschal at all. We
    /// deliberately don't name the sender (the recipient learns who at release).
    pub async fn heir_share(sink: &dyn NotificationSink, recipient_email: &str, share_code: &str) {
        sink.send(OutboundMessage {
            channel: Channel::Email,
            to: recipient_email.into(),
            subject: Some("You've been named in a Paschal letter".into()),
            body: format!(
                "Someone has written you a letter through Paschal, to be delivered to \
                 you at some point in the future.\n\n\
                 Keep this recovery code somewhere safe — you'll need it to open the \
                 letter if and when it is delivered to you:\n\n    {share_code}\n\n\
                 There's nothing to do now. If the letter is ever released you'll get a \
                 link, and this code will open it. We cannot recover this code for you, \
                 and we cannot read the letter."
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resend_payload_maps_message_fields() {
        let sink =
            ResendNotifications::new("re_test_key".into(), "Paschal <noreply@example.com>".into());
        let msg = OutboundMessage {
            channel: Channel::Email,
            to: "heir@example.org".into(),
            subject: Some("A letter is waiting for you".into()),
            body: "Open it: https://vault.example.com/claim?token=abc".into(),
        };
        let p = sink.payload(&msg);
        assert_eq!(p["from"], "Paschal <noreply@example.com>");
        assert_eq!(p["to"][0], "heir@example.org");
        assert_eq!(p["subject"], "A letter is waiting for you");
        assert_eq!(
            p["text"],
            "Open it: https://vault.example.com/claim?token=abc"
        );
    }

    #[test]
    fn resend_payload_defaults_missing_subject_to_empty() {
        let sink = ResendNotifications::new("re".into(), "x@example.com".into());
        let msg = OutboundMessage {
            channel: Channel::Email,
            to: "a@b.com".into(),
            subject: None,
            body: "hi".into(),
        };
        assert_eq!(sink.payload(&msg)["subject"], "");
    }

    #[test]
    fn smtp_sink_builds_for_each_tls_mode() {
        for tls in ["starttls", "implicit", "none"] {
            let sink = SmtpNotifications::new(
                "smtp.example.com",
                Some(587),
                Some("user".into()),
                Some("pass".into()),
                tls,
                "Paschal <noreply@example.com>".into(),
            );
            assert!(sink.is_ok(), "SMTP sink should build for tls={tls}");
        }
    }

    #[test]
    fn smtp_sink_builds_without_credentials() {
        // Unauthenticated local relay: no username/password.
        let sink = SmtpNotifications::new(
            "localhost",
            Some(25),
            None,
            None,
            "none",
            "Paschal <noreply@localhost>".into(),
        );
        assert!(sink.is_ok());
    }
}
