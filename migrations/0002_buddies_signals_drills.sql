-- 0002_buddies_signals_drills.sql — MVP completion features.
--
-- Adds:
--   * buddy table (lighter-weight Guardian, see specs/06)
--   * signal_subscription + signal tables (signal aggregator state)
--   * apple_shortcut_subscription (HMAC-verified Apple ping receiver)
--   * letter.is_drill / drill_ciphertext / drill_nonce / scheduled_release_at
--   * release_event.is_drill
--   * subscription updated-at touch trigger refinements (not needed; existing trigger covers)

-- ---------------------------------------------------------------------------
-- Buddies (specs/06)
-- ---------------------------------------------------------------------------

CREATE TABLE buddy (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    principal_id            UUID NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
    display_name            TEXT,
    email                   TEXT NOT NULL,
    phone                   TEXT,
    prompt_cadence_days     INTEGER NOT NULL DEFAULT 90,
    last_prompt_at          TIMESTAMPTZ,
    last_response_at        TIMESTAMPTZ,
    last_response           TEXT CHECK (last_response IN ('WELL', 'WORRIED', 'UNABLE_TO_REACH')),
    confirmation_token_hash BYTEA,
    confirmed_at            TIMESTAMPTZ,
    revoked_at              TIMESTAMPTZ,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (principal_id, email)
);

CREATE INDEX buddy_principal_idx ON buddy(principal_id);

-- ---------------------------------------------------------------------------
-- Signal subscriptions + observations (specs/06)
-- ---------------------------------------------------------------------------

CREATE TABLE signal_subscription (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    vault_id    UUID NOT NULL REFERENCES vault(id) ON DELETE CASCADE,
    source      TEXT NOT NULL,
    weight      REAL NOT NULL DEFAULT 0.10,
    enabled     BOOLEAN NOT NULL DEFAULT TRUE,
    config      JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (vault_id, source)
);

CREATE INDEX signal_subscription_vault_idx ON signal_subscription(vault_id);

CREATE TABLE signal (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    vault_id      UUID NOT NULL REFERENCES vault(id) ON DELETE CASCADE,
    source        TEXT NOT NULL,
    observed_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    contribution  REAL NOT NULL DEFAULT 0.0,
    evidence      JSONB NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX signal_vault_observed_idx ON signal(vault_id, observed_at DESC);
CREATE INDEX signal_source_idx ON signal(source);

-- ---------------------------------------------------------------------------
-- Apple Shortcut bindings (specs/06)
-- ---------------------------------------------------------------------------

CREATE TABLE apple_shortcut_subscription (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    principal_id    UUID NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
    installation_id TEXT NOT NULL UNIQUE,
    secret_hmac_key BYTEA NOT NULL,
    last_ping_at    TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX apple_shortcut_principal_idx ON apple_shortcut_subscription(principal_id);

-- ---------------------------------------------------------------------------
-- Drill mode + scheduled releases (specs/12 + specs/06)
-- ---------------------------------------------------------------------------

-- Drill payloads live alongside production payloads. When the orchestrator
-- runs a Drill, it uses the drill_* fields; in a real release it uses the
-- main ciphertext/nonce.
ALTER TABLE letter
    ADD COLUMN drill_ciphertext      BYTEA,
    ADD COLUMN drill_nonce           BYTEA,
    ADD COLUMN scheduled_release_at  TIMESTAMPTZ;

CREATE INDEX letter_scheduled_idx
    ON letter(scheduled_release_at)
    WHERE scheduled_release_at IS NOT NULL;

ALTER TABLE release_event
    ADD COLUMN is_drill BOOLEAN NOT NULL DEFAULT FALSE;
