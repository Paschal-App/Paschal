-- 0004_account_deletion.sql — Account-deletion lifecycle.
--
-- Per specs/10-legal-civil-liberties-and-abuse.md, account deletion is
-- distinct from Subscription cancellation. A deletion request enters a
-- 30-day cool-off; if not cancelled, the retention scheduler purges
-- ciphertext + KMS-wrapped material at T+30, then transitions the
-- Subscription to DELETED at T+60.
--
-- We track the deletion state on principal itself rather than create a
-- new table — there is at most one in-flight deletion per principal.

ALTER TABLE principal
    ADD COLUMN deletion_requested_at TIMESTAMPTZ,
    ADD COLUMN deletion_scheduled_for TIMESTAMPTZ;

CREATE INDEX principal_deletion_due_idx
    ON principal(deletion_scheduled_for)
    WHERE deletion_scheduled_for IS NOT NULL;
