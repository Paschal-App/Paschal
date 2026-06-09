-- 0020_zk_letter_heir.sql — Heir-passphrase release path for Private Letters.
--
-- A Private Letter's body is sealed in the browser under the vault DEK (the
-- author/recovery path). To let the *recipient* open it after release without
-- the Principal's passkey, the Principal may also seal the body under a
-- passphrase they share out-of-band (e.g. written with their will) — the
-- "Heir-passphrase path" of specs/14-crypto-tier2-design.md §4.
--
-- This is a second, independent AES-256-GCM encryption of the same body, keyed
-- by PBKDF2(recipient passphrase, salt). Per-letter, so one recipient's
-- passphrase never opens another recipient's Letter. Optional — set only when
-- the Principal supplies a passphrase at compose time.

CREATE TABLE IF NOT EXISTS zk_letter_heir_envelope (
    letter_id   UUID PRIMARY KEY REFERENCES letter(id) ON DELETE CASCADE,
    ciphertext  BYTEA NOT NULL,
    nonce       BYTEA NOT NULL,
    salt        BYTEA NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
