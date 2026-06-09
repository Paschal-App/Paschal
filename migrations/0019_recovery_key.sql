-- 0019_recovery_key.sql — Principal-level Recovery Key (PRK) for Private Vaults.
--
-- The PRK is a per-principal 256-bit key generated in the browser at first
-- passkey registration. It wraps every Private Vault DEK's *recovery* path, so
-- a single recovery code (or any of the principal's passkeys) can restore
-- access to all Private Vaults. The Operator stores the PRK only in wrapped
-- form and can never reconstruct it.
--
--   prk_code_*  = PRK wrapped under PBKDF2(recovery_code)   — lost-passkey path
--   prk_prf_*   = PRK wrapped under the passkey PRF-derived key — logged-in path
--
-- See specs/14-crypto-tier2-design.md and specs/07-cryptography-and-key-management.md.

CREATE TABLE IF NOT EXISTS principal_recovery_key (
    principal_id    UUID PRIMARY KEY REFERENCES principal(id) ON DELETE CASCADE,
    code_salt       BYTEA NOT NULL,
    prk_code_ct     BYTEA NOT NULL,
    prk_code_nonce  BYTEA NOT NULL,
    prk_prf_ct      BYTEA NOT NULL,
    prk_prf_nonce   BYTEA NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Per-Private-Vault DEK wrapped under the PRK (the vault's recovery envelope).
CREATE TABLE IF NOT EXISTS vault_prk_envelope (
    vault_id        UUID PRIMARY KEY REFERENCES vault(id) ON DELETE CASCADE,
    principal_id    UUID NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
    ciphertext      BYTEA NOT NULL,
    nonce           BYTEA NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS vault_prk_envelope_principal_idx
    ON vault_prk_envelope(principal_id);
