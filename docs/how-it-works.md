# How Paschal works

Paschal is a dead-man's switch for your digital estate. This document explains the full lifecycle from setup to release, without assuming technical knowledge.

---

## The core idea

You seal letters inside encrypted vaults. As long as you keep checking in, the vaults stay locked. When you stop — because you are incapacitated, or die, or simply forget — a cooling-off window opens. If nobody cancels it in time, the switch trips and your recipients get what you left them.

The principle is that **the system only acts on silence**, not on a positive confirmation of death. This is deliberately conservative: a missed check-in is far more common than a death, so the cooling-off period is a safety valve for false alarms.

---

## Step by step

### 1. Sign up and create a vault

A **Principal** is anyone who creates an account. When you sign up, a **Subscription** is created automatically (in self-hosted mode, it is always active and free).

A **Vault** is a container for your letters. You can have multiple vaults for different purposes — one for your family, one for your executor, one for a friend. Each vault has its own:

- **Recipient list** — who gets the letters when it releases.
- **Cooling-off window** — how long the grace period lasts before release (default 14 days, configurable per vault).
- **Signal subscriptions** — which signals keep the vault alive.

### 2. Seal letters

A **Letter** is a piece of content sealed inside a vault. It can be:

- A text message.
- A credential bundle (passwords, recovery codes).
- A file archive with attachments (documents, photos, recordings).
- A will locator (where to find the physical will).
- A time-capsule message with a scheduled future release date.

Letters are encrypted with AES-256-GCM using a key held only on your server. Neither the operator nor Paschal can read them.

### 3. Check in

**Check-ins** (heartbeats) tell the system you are alive. You can check in via:

- The web app (one click on the dashboard).
- The `paschal-cli` command-line tool.
- An **Apple iCloud Shortcut** running on your phone (runs automatically in the background).
- A **bank dormancy webhook** — your bank notifies Paschal when you make a transaction (requires your bank to support CDR/open banking webhooks).
- A **Microsoft Account** last-sign-in observation (manual or automated).
- **Buddies** — trusted contacts who confirm you are well when prompted.

Each check-in resets the countdown. The vault stays ACTIVE as long as check-ins arrive within the configured `HEARTBEAT_MAX_GAP_SECONDS` window (default 7 days).

### 4. The switch trips

When the last check-in is too old, the **signal aggregator** (a background task) starts escalating:

```
ACTIVE → SUSPICIOUS → ALERT → COOLING_OFF → RELEASING → RELEASED
```

The transitions from SUSPICIOUS to ALERT and from ALERT to COOLING_OFF are driven by the **trigger score** — a weighted combination of signal observations from multiple independent sources. A single stale heartbeat is not enough to trip the switch; typically two or more independent signal classes must confirm silence.

### 5. Cooling-off window

When a vault enters COOLING_OFF, three things happen:

1. The principal receives a warning email.
2. Any active **Co-Stewards** (trusted deputies) can see the vault status.
3. A countdown starts. By default it runs for 14 days.

During this window, **anyone with access** can cancel:
- The principal can log in and cancel from the dashboard.
- A Co-Steward can cancel on the principal's behalf.

If nothing happens, the vault moves to RELEASING.

### 6. Release

When the cooling-off window expires without cancellation:

1. The vault moves to RELEASING.
2. The scheduler generates a **single-use claim token** for each letter.
3. Recipients receive an email with a link to the claim page.
4. The recipient visits `/claim`, enters the token, and can read the letter and download attachments.
5. The vault moves to RELEASED.

Claim tokens are single-use. After the content is claimed, the token is invalidated.

---

## Drills

You can run a **drill** at any time. A drill:

1. Triggers a simulated cooling-off (with a very short window — seconds in test mode).
2. Sends the drill body of each letter (if configured — a separate, rehearsal-safe message) to recipients.
3. Returns the vault to ACTIVE when complete.

Drills let you verify the full release path — email delivery, claim page, attachment downloads — without committing real content to your recipients.

---

## Co-Stewards

A **Co-Steward** is a trusted deputy (a sibling, a solicitor, a close friend) who can:

- View your vault status, signal score, and scheduled release dates.
- Cancel a cooling-off that is in progress.
- After release: change a recipient's email address if needed.
- Trigger event-on-demand letters (e.g. a letter you sealed for "when my daughter graduates", which only fires when the Co-Steward confirms the event happened).

Co-Stewards authenticate with a passphrase set at confirmation time. They never see letter bodies.

---

## Buddies

**Buddies** are contacts who receive periodic check-in requests from Paschal. When prompted, they respond:

- **WELL** — the principal is fine. This resets any buddy-sourced signals.
- **WORRIED** — the principal seems unreachable; contributes a release signal.
- **UNABLE_TO_REACH** — the buddy has tried and cannot make contact; stronger signal.

Buddy responses are one input into the trigger score alongside other signal sources.

---

## Encryption

- Letter bodies and attachment blobs are encrypted at rest with AES-256-GCM.
- The encryption key (`kms.key`) lives in the `beacon_data` Docker volume on your server.
- **Back up this key.** If it is lost, all letters become permanently unrecoverable.
- In cloud deployments, an AWS KMS-wrapped data key can replace the local file key (see [`self-hosting.md`](self-hosting.md)).

---

## Transparency log

Every significant event (signup, vault creation, letter seal, release) is appended to an append-only transparency log. Entries contain only hashed identifiers — no plaintext content — so the log can be audited without exposing private data.
