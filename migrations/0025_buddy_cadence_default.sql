-- 0025: default buddy prompt cadence drops from 90 to 7 days.
-- Column-default only — existing buddies keep their chosen cadence.
-- Expand-only; safe to re-run.

ALTER TABLE buddy ALTER COLUMN prompt_cadence_days SET DEFAULT 7;
