-- 0012_co_steward_postmortem.sql — event-on-demand releases + post-mortem
-- Co-Steward powers.
--
-- Two capabilities a surviving family deputy needs once the principal is gone:
--
--   1. Control non-date-specific ("event") releases — a wedding, a graduation
--      with no fixed date. These are held: excluded from both signal and
--      scheduled release, fired by hand by a Co-Steward when the event happens.
--
--   2. Update where automated messages go — a recipient's email or address
--      changes over the years a Letter waits. After death the principal can no
--      longer fix it, so a Co-Steward can, with a log + notify + hold guardrail.
--
-- Both powers are gated (in the API layer) to vaults that have RELEASED — i.e.
-- the principal is verifiably gone. The Co-Steward stays strictly read-only
-- while the principal is alive.

-- 1. EVENT_ON_DEMAND release mode. Held until a deputy triggers it.
ALTER TABLE letter DROP CONSTRAINT IF EXISTS letter_release_mode_check;
ALTER TABLE letter ADD CONSTRAINT letter_release_mode_check
    CHECK (release_mode IN (
        'SIGNAL_OR_SCHEDULED', 'SCHEDULED_ONLY', 'SIGNAL_ONLY', 'EVENT_ON_DEMAND'
    ));

-- Records when an event Letter was triggered, so it cannot double-fire.
ALTER TABLE letter ADD COLUMN IF NOT EXISTS event_released_at TIMESTAMPTZ;

-- 2. Pending recipient-contact changes requested by a Co-Steward post-mortem.
-- The change is held until `effective_at`; a worker applies due changes. The
-- old address is notified and the request is transparency-logged on creation.
CREATE TABLE IF NOT EXISTS letter_recipient_change (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    letter_id      UUID NOT NULL REFERENCES letter(id) ON DELETE CASCADE,
    co_steward_id  UUID NOT NULL REFERENCES co_steward(id) ON DELETE CASCADE,
    old_email      TEXT NOT NULL,
    new_email      TEXT NOT NULL,
    requested_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    effective_at   TIMESTAMPTZ NOT NULL,
    applied_at     TIMESTAMPTZ,
    cancelled_at   TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS letter_recipient_change_due_idx
    ON letter_recipient_change(effective_at)
    WHERE applied_at IS NULL AND cancelled_at IS NULL;

CREATE INDEX IF NOT EXISTS letter_recipient_change_letter_idx
    ON letter_recipient_change(letter_id);
