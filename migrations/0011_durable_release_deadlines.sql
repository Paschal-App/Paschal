-- 0011_durable_release_deadlines.sql — make cooling-off deadlines durable.
--
-- Phase 0 of the web-native / scale work. Previously the cooling-off → release
-- transition was driven by an in-process tokio timer (scheduler::spawn_release_elapse):
-- the deadline lived only in the memory of the handling process. That timer is
-- lost on restart and double-fires across replicas — a silent failure of the
-- core promise (a Letter that never releases, or releases twice).
--
-- We make the deadline durable by recording it on the release_event. A
-- DB-polled scheduler (release_elapse_scheduler) claims due rows and performs
-- the release; the COOLING_OFF → RELEASING transition is an atomic conditional
-- UPDATE, so exactly one worker ever fires a given release regardless of how
-- many timers or replicas are running.

ALTER TABLE release_event
    ADD COLUMN IF NOT EXISTS cooling_off_ends_at TIMESTAMPTZ;

-- The poll query selects open releases whose deadline has elapsed. A partial
-- index keeps that scan cheap even as the release_event table grows.
CREATE INDEX IF NOT EXISTS release_event_due_idx
    ON release_event (cooling_off_ends_at)
    WHERE released_at IS NULL AND cancelled_at IS NULL;
