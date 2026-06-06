-- 0007_co_stewards.sql — Co-Steward (read-only family deputy).
--
-- A Co-Steward is a person whom the Principal nominates to be able to SEE
-- the state of their Vaults — last heartbeat, signal strength, scheduled
-- releases, plan tier, retention window — without ever being able to read
-- Letter bodies or modify anything.
--
-- This is the half-step toward family-plan sharing without exposing
-- ciphertext. A Co-Steward is gated to plans that include the feature
-- (Estate+ and above; Estate has 1 slot, Estate+ has 3, Legacy has 5;
-- enforcement is in the plan registry, not the schema).
--
-- The Co-Steward authenticates via a passphrase set during the confirmation
-- flow and a session token (lives in the `session` table, with the
-- co_steward_id field below distinguishing the session role).
--
-- Co-Stewards never carry an HPKE key, never receive Letter ciphertext, and
-- never appear as a Recipient. They are a *read-only mirror* of the
-- principal's dashboard.

CREATE TABLE co_steward (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    principal_id            UUID NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
    display_name            TEXT,
    email                   TEXT NOT NULL,
    -- argon2id-derived; salt stored separately on the same row.
    passphrase_hash         TEXT,
    passphrase_salt         BYTEA,
    -- The one-time confirmation token (hashed). Cleared once consumed.
    confirmation_token_hash BYTEA,
    confirmed_at            TIMESTAMPTZ,
    revoked_at              TIMESTAMPTZ,
    -- For audit: when did this Co-Steward last view the dashboard?
    last_viewed_at          TIMESTAMPTZ,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (principal_id, email)
);

CREATE INDEX co_steward_principal_idx ON co_steward(principal_id);
CREATE INDEX co_steward_confirmation_token_idx
    ON co_steward(confirmation_token_hash)
    WHERE confirmation_token_hash IS NOT NULL;

-- Extend `session` to optionally carry a co_steward_id. A session belongs
-- to either a principal (auth_role='PRINCIPAL') or a co_steward
-- (auth_role='CO_STEWARD'). Default 'PRINCIPAL' keeps existing rows valid.
ALTER TABLE session
    ADD COLUMN IF NOT EXISTS co_steward_id UUID
        REFERENCES co_steward(id) ON DELETE CASCADE,
    ADD COLUMN IF NOT EXISTS auth_role TEXT NOT NULL DEFAULT 'PRINCIPAL'
        CHECK (auth_role IN ('PRINCIPAL', 'CO_STEWARD'));

CREATE INDEX IF NOT EXISTS session_co_steward_idx
    ON session(co_steward_id) WHERE co_steward_id IS NOT NULL;
