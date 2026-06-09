-- WebAuthn passkey credentials (one principal may have many)
CREATE TABLE IF NOT EXISTS principal_passkey (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    principal_id    UUID NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
    credential_id   BYTEA NOT NULL UNIQUE,
    passkey_json    TEXT NOT NULL,
    backed_up       BOOLEAN NOT NULL DEFAULT FALSE,
    transports      TEXT[] NOT NULL DEFAULT '{}',
    name            TEXT NOT NULL DEFAULT 'Passkey',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at    TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS passkey_principal_idx ON principal_passkey(principal_id);

-- Short-lived WebAuthn ceremony state
CREATE TABLE IF NOT EXISTS webauthn_challenge (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    principal_id    UUID REFERENCES principal(id) ON DELETE CASCADE,
    state_json      TEXT NOT NULL,
    ceremony        TEXT NOT NULL CHECK (ceremony IN ('REGISTRATION', 'AUTHENTICATION')),
    expires_at      TIMESTAMPTZ NOT NULL DEFAULT (now() + INTERVAL '5 minutes')
);
CREATE INDEX IF NOT EXISTS webauthn_challenge_expires_idx ON webauthn_challenge(expires_at);

-- ZK vault DEK wrapped under each passkey's PRF-derived key
CREATE TABLE IF NOT EXISTS zk_key_envelope (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    vault_id        UUID NOT NULL REFERENCES vault(id) ON DELETE CASCADE,
    passkey_id      UUID NOT NULL REFERENCES principal_passkey(id) ON DELETE CASCADE,
    ciphertext      BYTEA NOT NULL,
    nonce           BYTEA NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (vault_id, passkey_id)
);
CREATE INDEX IF NOT EXISTS zk_envelope_vault_idx ON zk_key_envelope(vault_id);

-- ZK vault DEK wrapped under PBKDF2(recovery_phrase)
CREATE TABLE IF NOT EXISTS zk_recovery_envelope (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    vault_id        UUID NOT NULL REFERENCES vault(id) ON DELETE CASCADE,
    principal_id    UUID NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
    code_salt       BYTEA NOT NULL,
    ciphertext      BYTEA NOT NULL,
    nonce           BYTEA NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (vault_id, principal_id)
);

-- Per-principal recovery code verifier
CREATE TABLE IF NOT EXISTS principal_recovery (
    principal_id    UUID PRIMARY KEY REFERENCES principal(id) ON DELETE CASCADE,
    code_salt       BYTEA NOT NULL,
    code_verifier   BYTEA NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
