-- 0021_heir_envelope_modes.sql — Heir-delivery modes for the recipient password.
--
-- The recipient's key K (which AES-256-GCM-seals the body) can now reach them
-- three ways, chosen per Letter:
--
--   manual   — K = PBKDF2(passphrase). Principal shares the passphrase
--              out-of-band. `salt` set. (the original 0020 behaviour)
--   split    — K = recipient_share XOR operator_share. The recipient's share is
--              emailed at creation (operator does NOT store it); the operator's
--              share is `release_secret`, handed out only when the Vault
--              unseals. The operator never holds both halves → zero-knowledge.
--   operator — K is `release_secret`, held by the operator and handed out when
--              the Vault unseals. Convenient (nothing for the recipient to
--              keep) but the operator can read these Letters (honest-operator).

ALTER TABLE zk_letter_heir_envelope
    ADD COLUMN IF NOT EXISTS mode TEXT NOT NULL DEFAULT 'manual',
    ADD COLUMN IF NOT EXISTS release_secret BYTEA;

-- `salt` is only used by the manual (PBKDF2) mode.
ALTER TABLE zk_letter_heir_envelope ALTER COLUMN salt DROP NOT NULL;
