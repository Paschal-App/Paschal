# API reference

The Beacon exposes a JSON REST API under `/v1`. All authenticated endpoints require a `Bearer` token in the `Authorization` header, obtained from the signup or magic-link flow.

Error responses use [`application/problem+json`](https://www.rfc-editor.org/rfc/rfc7807) with fields `type`, `title`, `status`, and `detail`.

---

## Authentication

### Sign up

```
POST /v1/auth/signup
Content-Type: application/json
```

```json
{
  "email": "you@example.com",
  "tos_accepted": true
}
```

Response `200` — **new account** (`status: "active"`): a session is attached.

```json
{
  "status": "active",
  "principal_id": "prn_<uuid>",
  "session_token": "<token>",
  "subscription_state": "TRIALING",
  "trial_end_at": "2024-03-01T00:00:00Z",
  "magic_token_DEV_ONLY": "<token>"
}
```

Response `200` — **account already exists** (`status: "verification_sent"`): no
session is issued. A one-time sign-in link is emailed to the owner instead, and a
`poll_id` is returned so the waiting tab can auto-complete (see *Poll the
magic link* below).

```json
{
  "status": "verification_sent",
  "poll_id": "<uuid>",
  "magic_token_DEV_ONLY": "<token>"
}
```

Use `session_token` as the `Bearer` token for authenticated requests.

> In production, `magic_token_DEV_ONLY` is not returned. The magic link is emailed to the address. In development (no SMTP configured), it is returned in the response for convenience.

### Sign in (request a magic link)

```
POST /v1/auth/signin
Content-Type: application/json
```

```json
{ "email": "you@example.com" }
```

Response `200` — always identical whether or not the account exists, so the
endpoint cannot be used to enumerate accounts:

```json
{ "status": "sent", "poll_id": "<uuid>", "magic_token_DEV_ONLY": "<token>" }
```

### Verify a magic link

The emailed link points at `/app/auth/verify#token=<token>` — the token is in the
URL **fragment** so it never reaches server logs or `Referer` headers. The web app
reads it and calls:

```
POST /v1/auth/magic-link/verify
Content-Type: application/json
```

```json
{ "token": "<token>" }
```

Response `200`: `{ "principal_id": "...", "session_token": "...", "email": "..." }`.
The token is single-use and expires 15 minutes after issue.

### Poll the magic link

While the "check your email" screen is showing, the originating tab polls with the
`poll_id` so it signs in automatically the moment the emailed link is clicked
(possibly on another device):

```
POST /v1/auth/magic-link/poll
Content-Type: application/json
```

```json
{ "poll_id": "<uuid>" }
```

Response `200` while waiting: `{ "status": "pending" }`. Once the link is clicked:
`{ "status": "ready", "session_token": "...", "principal_id": "...", "email": "..." }`.
The poll is single-use — after the first `ready` (or for an unknown `poll_id`) it
returns `404`.

> The email-auth endpoints (signup, signin, signout, magic-link verify/poll) are
> rate-limited per IP by `AUTH_RATE_LIMIT_PER_MINUTE` (default 10/min) on top of
> the global `RATE_LIMIT_PER_MINUTE`.

---

## Plan catalog

### List plans

```
GET /v1/plans
```

Returns the single self-hosted plan. No authentication required.

```json
[
  {
    "plan_id": "self_hosted",
    "storage_bytes": 1125899906842624,
    "retention_days": 36500,
    "scheduled_horizon_days": 36500,
    "max_vaults": 4294967295,
    "max_letters_per_vault": 4294967295,
    "max_trustees": 4294967295,
    "allowed_signals": ["HEARTBEAT", "CDR_BANK_DORMANCY", "..."],
    "multi_region": true,
    "notes": ["Self-hosted edition — all features, no quotas."]
  }
]
```

---

## Subscription

### Get subscription

```
GET /v1/principals/me/subscription
Authorization: Bearer <token>
```

```json
{
  "state": "TRIALING",
  "plan_id": "self_hosted",
  "started_at": "2024-01-01T00:00:00Z",
  "trial_end_at": "2024-01-31T00:00:00Z",
  "current_period_end": null,
  "canceled_at": null,
  "retention_until": null
}
```

Subscription states: `TRIALING`, `ACTIVE`, `PAST_DUE`, `CANCELED`, `EXPIRED`, `DELETED`.

In self-hosted mode the state is informational. All features are available regardless of state.

### Get usage

```
GET /v1/principals/me/usage
Authorization: Bearer <token>
```

```json
{
  "plan_id": "self_hosted",
  "storage_used_bytes": 1048576,
  "storage_quota_bytes": 1125899906842624,
  "storage_pct": 0.0,
  "vaults_used": 2,
  "vaults_quota": 4294967295,
  "letters_quota_per_vault": 4294967295,
  "co_stewards_quota": 4294967295,
  "retention_days": 36500,
  "scheduled_horizon_days": 36500
}
```

---

## Vaults

### List vaults

```
GET /v1/vaults
Authorization: Bearer <token>
```

### Create vault

```
POST /v1/vaults
Authorization: Bearer <token>
Content-Type: application/json
```

```json
{
  "name": "Family vault",
  "cooling_off_seconds": 1209600,
  "storage_region": "ap-southeast-2"
}
```

`storage_region` is optional; omit for the default region (`ap-southeast-2`).

### Get vault

```
GET /v1/vaults/:id
Authorization: Bearer <token>
```

Vault states: `ACTIVE`, `SUSPICIOUS`, `ALERT`, `COOLING_OFF`, `RELEASING`, `RELEASED`, `ARCHIVED`.

### Move vault storage region

```
POST /v1/vaults/:id/region
Authorization: Bearer <token>
Content-Type: application/json
```

```json
{ "storage_region": "eu-central-1" }
```

Migrates all attachment blobs to the new region. Idempotent and crash-safe. Available regions: `ap-southeast-2`, `eu-central-1`, `us-east-1`.

---

## Letters

### List letters

```
GET /v1/vaults/:vault_id/letters
Authorization: Bearer <token>
```

### Seal a text letter

```
POST /v1/vaults/:vault_id/letters
Authorization: Bearer <token>
Content-Type: application/json
```

```json
{
  "title": "To my family",
  "recipient_email": "family@example.com",
  "body": "If you're reading this…",
  "drill_body": "This is a rehearsal. Ignore it.",
  "kind": "MESSAGE",
  "release_mode": "SIGNAL_OR_SCHEDULED",
  "scheduled_release_at": null
}
```

`kind` values: `MESSAGE`, `CREDENTIAL_BUNDLE`, `FILE_ARCHIVE`, `ACTION`, `WILL_LOCATOR`, `VIDEO_MESSAGE`, `AUDIO_MESSAGE`, `MEDIA_PLAYLIST`.

`release_mode` values:
- `SIGNAL_OR_SCHEDULED` (default) — fires on signal trigger or scheduled date, whichever comes first.
- `SCHEDULED_ONLY` — time-capsule; only fires on `scheduled_release_at`. Requires the date.
- `SIGNAL_ONLY` — fires only on vault signal trigger, ignores any schedule.
- `EVENT_ON_DEMAND` — held until a Co-Steward triggers it manually.

### Upload a letter with attachments

```
POST /v1/vaults/:vault_id/letters/multipart
Authorization: Bearer <token>
Content-Type: multipart/form-data
```

Form fields: `title`, `recipient_email`, `body` (optional), `kind`, `release_mode`, `scheduled_release_at`, `file` (repeatable).

Attachments are automatically transcoded to archive-safe formats (video → WebM/VP9+Opus, audio → Opus/Ogg).

### Export a letter (offline bundle)

```
POST /v1/vaults/:vault_id/letters/:letter_id/export
Authorization: Bearer <token>
Content-Type: application/json
```

```json
{ "passphrase": "my-strong-passphrase" }
```

Returns a self-contained JSON bundle with the letter ciphertext re-encrypted under the passphrase using Argon2id KDF. Can be decrypted offline without the server.

---

## Heartbeat

```
POST /v1/heartbeats
Authorization: Bearer <token>
Content-Type: application/json
```

```json
{ "via": "WEB" }
```

`via` values: `CLI`, `WEB`, `EMAIL`, `SMS`, `PUSH`.

Resets the heartbeat timer across all vaults and records a `HEARTBEAT` signal observation with contribution `0.0` (proof of life).

---

## Signal subscriptions

Each vault has a set of signal sources with individual weights. Defaults are set when the vault is created.

### List signal subscriptions

```
GET /v1/vaults/:vault_id/signal-subscriptions
Authorization: Bearer <token>
```

### Update a signal subscription

```
POST /v1/vaults/:vault_id/signal-subscriptions
Authorization: Bearer <token>
Content-Type: application/json
```

```json
{
  "source": "BUDDY_ATTESTATION",
  "weight": 0.25,
  "enabled": true
}
```

---

## Apple iCloud Shortcut signal

### Enrol

```
POST /v1/signals/apple-icloud/enrol
Authorization: Bearer <token>
Content-Type: application/json
```

```json
{ "installation_id": "<stable UUID from Shortcut>" }
```

Returns `secret_base64` and `ping_url`. Configure the Shortcut to POST to `ping_url` with an HMAC-SHA256 signature.

### Ping (called by the Shortcut)

```
POST /v1/signals/apple-icloud/ping
Content-Type: application/json
```

```json
{
  "installation_id": "<uuid>",
  "timestamp": 1700000000,
  "signature": "<base64 HMAC-SHA256>"
}
```

---

## Bank dormancy signal

### Enrol

```
POST /v1/principals/me/bank-signal
Authorization: Bearer <token>
```

Returns a `webhook_url`. Configure your bank or open-banking provider to call this URL on each transaction.

### Get status

```
GET /v1/principals/me/bank-signal
Authorization: Bearer <token>
```

### Revoke

```
DELETE /v1/principals/me/bank-signal
Authorization: Bearer <token>
```

---

## Buddies

### List buddies

```
GET /v1/principals/me/buddies
Authorization: Bearer <token>
```

### Invite a buddy

```
POST /v1/principals/me/buddies
Authorization: Bearer <token>
Content-Type: application/json
```

```json
{
  "email": "friend@example.com",
  "display_name": "Alex",
  "phone": "+61400000000",
  "prompt_cadence_days": 30
}
```

Returns `confirmation_token_DEV_ONLY` in development; in production the confirmation link is emailed.

### Confirm a buddy invitation (no auth)

```
POST /v1/buddies/confirm
Content-Type: application/json
```

```json
{ "token": "<confirmation_token>" }
```

### Respond to a buddy check-in (no auth)

```
POST /v1/buddies/:buddy_id/responses
Content-Type: application/json
```

```json
{ "response": "WELL" }
```

Response values: `WELL`, `WORRIED`, `UNABLE_TO_REACH`.

### Revoke a buddy

```
DELETE /v1/buddies/:buddy_id
Authorization: Bearer <token>
```

---

## Co-Stewards

### List co-stewards

```
GET /v1/principals/me/co-stewards
Authorization: Bearer <token>
```

### Invite a co-steward

```
POST /v1/principals/me/co-stewards
Authorization: Bearer <token>
Content-Type: application/json
```

```json
{
  "email": "sibling@example.com",
  "display_name": "Jordan"
}
```

### Confirm co-steward invitation (no auth)

```
POST /v1/co-stewards/confirm
Content-Type: application/json
```

```json
{
  "token": "<confirmation_token>",
  "passphrase": "my-co-steward-passphrase"
}
```

### Co-steward sign in

```
POST /v1/co-stewards/sign-in
Content-Type: application/json
```

```json
{
  "email": "sibling@example.com",
  "passphrase": "my-co-steward-passphrase"
}
```

### Co-steward dashboard

```
GET /v1/co-stewards/me/dashboard
Authorization: Bearer <co_steward_token>
```

Returns principal vault states, signal scores, and scheduled-release dates. Never returns letter bodies.

### Revoke co-steward

```
DELETE /v1/co-stewards/:id
Authorization: Bearer <token>
```

---

## Release control

### Force-release a vault

```
POST /v1/vaults/:id/force-release
Authorization: Bearer <token>
```

Immediately transitions the vault to COOLING_OFF. The cooling-off window still applies; this skips the signal-aggregator escalation path.

### Cancel cooling-off

```
POST /v1/vaults/:id/cancel
Authorization: Bearer <token>
```

Returns the vault to ACTIVE. Only valid while in COOLING_OFF.

### Run a drill

```
POST /v1/vaults/:id/drills
Authorization: Bearer <token>
```

Executes a full release rehearsal. Sends `drill_body` content (not the real letter body) to recipients. The vault returns to ACTIVE after the drill cooling-off window.

---

## Recipient claim

### Claim a released letter (no auth)

```
GET /v1/releases/claim?token=<claim_token>
```

Returns the decrypted letter body, title, and attachment list. The token is single-use.

### Download a claim attachment (no auth)

```
GET /v1/releases/claim/attachment?token=<claim_token>&attachment_id=<id>
```

Streams the decrypted attachment file. Same single-use token.

---

## Account

### Request account deletion

```
DELETE /v1/principals/me
Authorization: Bearer <token>
```

Schedules permanent deletion after 30 days.

### Cancel pending deletion

```
POST /v1/principals/me/cancel-deletion
Authorization: Bearer <token>
```

---

## Health

```
GET /health     → { "status": "ok", "service": "paschal-beacon", "version": "..." }
GET /livez      → 200 OK  (process is alive)
GET /readyz     → 200 OK or 503  (database reachable check)
GET /metrics    → Prometheus text format
```
