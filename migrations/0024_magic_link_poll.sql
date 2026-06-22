-- Magic-link polling: lets the "Check your email" waiting tab auto-complete
-- when the emailed link is clicked in another tab or device.
--
-- poll_id           — opaque UUID returned to the waiting tab at sign-in time
-- poll_principal_id — set when the magic link is consumed (link was clicked)
-- poll_redeemed_at  — set when the waiting tab redeems the poll (single-use)
--
-- Expand-only (data safety rule 1); IF NOT EXISTS so a manual-then-embedded
-- apply path doesn't fail.

ALTER TABLE magic_link
    ADD COLUMN IF NOT EXISTS poll_id UUID NOT NULL DEFAULT gen_random_uuid(),
    ADD COLUMN IF NOT EXISTS poll_principal_id UUID REFERENCES principal(id) ON DELETE CASCADE,
    ADD COLUMN IF NOT EXISTS poll_redeemed_at TIMESTAMPTZ;

CREATE UNIQUE INDEX IF NOT EXISTS magic_link_poll_id_idx ON magic_link(poll_id);
