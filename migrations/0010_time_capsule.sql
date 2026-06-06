-- 0010_time_capsule.sql — Letter release modes (time-capsule support).
--
-- The MVP treated every Letter the same: it could fire on signals, on a
-- scheduled date, or on the earlier of the two. That's the right default,
-- but it doesn't cover an important class of Letter:
--
--     "Read at my daughter's wedding."
--     "Open on your 30th birthday."
--     "Give to the kids when the youngest graduates."
--
-- These are time-capsules. The principal does NOT want them released when
-- they die — the recipient doesn't need a wedding letter at a funeral. The
-- principal wants them released *on the date*, regardless of their own
-- state, as a way to be present at a moment they couldn't capture in person.
--
-- Three modes:
--
--   SIGNAL_OR_SCHEDULED  — default. The earlier of signal-release and
--                          scheduled_release_at fires. Backward-compatible
--                          with everything authored before this migration.
--
--   SCHEDULED_ONLY       — time-capsule. The Letter is excluded from any
--                          signal-triggered Vault release. It fires only
--                          when scheduled_release_at arrives, regardless
--                          of Vault state.
--
--   SIGNAL_ONLY          — signal-only. The Letter ignores any scheduled
--                          date entirely; it fires only when the Vault
--                          itself enters RELEASING via the signal
--                          aggregator. Useful for "if anything happens,
--                          send this — but don't fire it on a date."

ALTER TABLE letter
    ADD COLUMN IF NOT EXISTS release_mode TEXT NOT NULL
        DEFAULT 'SIGNAL_OR_SCHEDULED'
        CHECK (release_mode IN (
            'SIGNAL_OR_SCHEDULED',
            'SCHEDULED_ONLY',
            'SIGNAL_ONLY'
        ));

-- The scheduler queries "scheduled letters due"; a partial index keeps that
-- cheap as the letter table grows. SIGNAL_ONLY rows can't have a useful
-- scheduled_release_at, but we don't forbid one in the schema (the API
-- layer drops it).
CREATE INDEX IF NOT EXISTS letter_scheduled_due_idx
    ON letter(scheduled_release_at)
    WHERE scheduled_release_at IS NOT NULL
      AND release_mode IN ('SIGNAL_OR_SCHEDULED', 'SCHEDULED_ONLY');

-- And for the signal-release filter the scheduler does on every release
-- event: "give me letters that should fire on a signal release."
CREATE INDEX IF NOT EXISTS letter_vault_signal_release_idx
    ON letter(vault_id)
    WHERE release_mode IN ('SIGNAL_OR_SCHEDULED', 'SIGNAL_ONLY');
