-- Add panic_mode to duress_signal.
--
-- 'freeze' (default, existing behaviour): while duress is active, every
--   Vault's release is blocked — the dead-man's switch is paused.
-- 'release': triggering duress immediately starts cooling-off on all
--   active Vaults — for situations where the principal wants their wishes
--   to be carried out without delay (not a coercion scenario).
--
-- Expand-only: additive column with DEFAULT, safe under blue/green.
ALTER TABLE duress_signal
    ADD COLUMN IF NOT EXISTS panic_mode TEXT NOT NULL DEFAULT 'freeze';
