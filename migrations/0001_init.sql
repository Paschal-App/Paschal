-- 0001_init.sql — Paschal walking-skeleton schema.
--
-- This migration is the foundational schema for the MVP. Entities track the
-- spec's domain model in specs/05-domain-model.md, with deliberate omissions
-- for the skeleton:
--   * No `trustee_key` / `sealed_share` (Tier 2 not in this slice)
--   * No `signal` table (signal aggregator lands next)
--   * No `transparency_anchor` (Merkle anchoring lands at v1)
--
-- Once we move beyond the skeleton, migrations are append-only — never edit.

CREATE EXTENSION IF NOT EXISTS pgcrypto;

-- ---------------------------------------------------------------------------
-- Principals & auth
-- ---------------------------------------------------------------------------

CREATE TABLE principal (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    display_name  TEXT,
    primary_email TEXT NOT NULL UNIQUE,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE auth_identity (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    principal_id     UUID NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
    provider         TEXT NOT NULL CHECK (provider IN (
        'APPLE', 'GOOGLE', 'MICROSOFT', 'EMAIL_MAGIC_LINK', 'WEBAUTHN_ONLY'
    )),
    provider_subject TEXT NOT NULL,
    email            TEXT NOT NULL,
    added_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at       TIMESTAMPTZ,
    UNIQUE (provider, provider_subject)
);

-- A pending or used magic link.
CREATE TABLE magic_link (
    token_hash   BYTEA PRIMARY KEY,
    principal_id UUID NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at   TIMESTAMPTZ NOT NULL,
    consumed_at  TIMESTAMPTZ
);

-- Bearer session tokens.
CREATE TABLE session (
    token_hash    BYTEA PRIMARY KEY,
    principal_id  UUID NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at    TIMESTAMPTZ NOT NULL,
    revoked_at    TIMESTAMPTZ
);

CREATE INDEX session_principal_id_idx ON session(principal_id);

-- ---------------------------------------------------------------------------
-- Subscriptions (spec 12)
-- ---------------------------------------------------------------------------

CREATE TABLE subscription (
    id                       UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    principal_id             UUID NOT NULL UNIQUE REFERENCES principal(id) ON DELETE CASCADE,
    plan_id                  TEXT NOT NULL CHECK (plan_id IN (
        'estate_monthly_v1', 'estate_annual_v1'
    )),
    state                    TEXT NOT NULL CHECK (state IN (
        'TRIALING', 'ACTIVE', 'PAST_DUE', 'CANCELED', 'EXPIRED', 'DELETED'
    )),
    stripe_customer_id       TEXT,
    stripe_subscription_id   TEXT,
    started_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    trial_end_at             TIMESTAMPTZ,
    current_period_end       TIMESTAMPTZ,
    canceled_at              TIMESTAMPTZ,
    retention_until          TIMESTAMPTZ,
    updated_at               TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX subscription_state_idx ON subscription(state);

-- ---------------------------------------------------------------------------
-- Vaults & Letters
-- ---------------------------------------------------------------------------

CREATE TABLE vault (
    id                    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    principal_id          UUID NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
    name                  TEXT NOT NULL,
    tier                  TEXT NOT NULL CHECK (tier IN ('HONEST_OPERATOR', 'ZERO_KNOWLEDGE')),
    state                 TEXT NOT NULL CHECK (state IN (
        'ACTIVE', 'SUSPICIOUS', 'ALERT', 'COOLING_OFF', 'RELEASING', 'RELEASED', 'ARCHIVED'
    )) DEFAULT 'ACTIVE',
    cooling_off_seconds   INTEGER NOT NULL DEFAULT 15,
    last_attestation_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    cooling_off_started_at TIMESTAMPTZ,
    released_at           TIMESTAMPTZ,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX vault_principal_idx ON vault(principal_id);
CREATE INDEX vault_state_idx ON vault(state);

CREATE TABLE letter (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    vault_id        UUID NOT NULL REFERENCES vault(id) ON DELETE CASCADE,
    kind            TEXT NOT NULL CHECK (kind IN (
        'MESSAGE', 'CREDENTIAL_BUNDLE', 'FILE_ARCHIVE', 'ACTION'
    )) DEFAULT 'MESSAGE',
    title           TEXT NOT NULL,
    recipient_email TEXT NOT NULL,
    -- For the skeleton: ciphertext stored inline. Production uses object store.
    ciphertext      BYTEA NOT NULL,
    nonce           BYTEA NOT NULL,
    -- For Tier-2 (not yet): scheduled_release_at, sealed_shares JSON, etc.
    sealed_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX letter_vault_idx ON letter(vault_id);

-- ---------------------------------------------------------------------------
-- Heartbeats
-- ---------------------------------------------------------------------------

CREATE TABLE heartbeat (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    principal_id  UUID NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
    received_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    via           TEXT NOT NULL CHECK (via IN ('CLI', 'WEB', 'EMAIL', 'SMS', 'PUSH'))
);

CREATE INDEX heartbeat_principal_received_idx ON heartbeat(principal_id, received_at DESC);

-- ---------------------------------------------------------------------------
-- Releases
-- ---------------------------------------------------------------------------

CREATE TABLE release_event (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    vault_id        UUID NOT NULL REFERENCES vault(id) ON DELETE CASCADE,
    reason          TEXT NOT NULL CHECK (reason IN (
        'SIGNAL_TRIGGER', 'GUARDIAN_ATTESTATION', 'DRILL',
        'MANUAL_PRINCIPAL_RELEASE', 'SCHEDULED'
    )),
    triggered_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    released_at     TIMESTAMPTZ,
    cancelled_at    TIMESTAMPTZ
);

-- A claim token per recipient per release. The Heir page resolves these.
CREATE TABLE release_claim (
    token_hash      BYTEA PRIMARY KEY,
    release_event_id UUID NOT NULL REFERENCES release_event(id) ON DELETE CASCADE,
    letter_id       UUID NOT NULL REFERENCES letter(id) ON DELETE CASCADE,
    recipient_email TEXT NOT NULL,
    issued_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at      TIMESTAMPTZ NOT NULL,
    claimed_at      TIMESTAMPTZ
);

CREATE INDEX release_claim_release_idx ON release_claim(release_event_id);

-- ---------------------------------------------------------------------------
-- Transparency log (skeleton — anchoring lands at v1)
-- ---------------------------------------------------------------------------

CREATE TABLE transparency_entry (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    kind          TEXT NOT NULL,
    payload_hash  BYTEA NOT NULL,
    payload_json  JSONB NOT NULL,
    ts            TIMESTAMPTZ NOT NULL DEFAULT now(),
    principal_id  UUID REFERENCES principal(id) ON DELETE SET NULL,
    vault_id      UUID REFERENCES vault(id) ON DELETE SET NULL
);

CREATE INDEX transparency_entry_ts_idx ON transparency_entry(ts DESC);
CREATE INDEX transparency_entry_principal_idx ON transparency_entry(principal_id);

-- ---------------------------------------------------------------------------
-- updated_at trigger
-- ---------------------------------------------------------------------------

CREATE OR REPLACE FUNCTION touch_updated_at() RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER principal_touch BEFORE UPDATE ON principal
    FOR EACH ROW EXECUTE FUNCTION touch_updated_at();
CREATE TRIGGER subscription_touch BEFORE UPDATE ON subscription
    FOR EACH ROW EXECUTE FUNCTION touch_updated_at();
CREATE TRIGGER vault_touch BEFORE UPDATE ON vault
    FOR EACH ROW EXECUTE FUNCTION touch_updated_at();
