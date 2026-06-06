-- 0008_storage_addon.sql — Storage add-on (purchase extra GB).
--
-- The plan registry sets a base storage quota (100 MB / 1 GB / 10 GB / 100 GB).
-- This migration adds an *additional* per-subscription byte budget that
-- accumulates from one-off Stripe add-on purchases (currently $0.50/GB/month).
--
-- The effective quota at runtime is:
--   plan_features(sub.plan_id).storage_bytes + sub.extra_storage_bytes
--
-- The column is non-null so we never have to second-guess; a sub with no
-- add-on simply carries 0.

ALTER TABLE subscription
    ADD COLUMN IF NOT EXISTS extra_storage_bytes BIGINT NOT NULL DEFAULT 0
        CHECK (extra_storage_bytes >= 0);

-- A small audit table so we can answer "where did the extra 5 GB come from?"
CREATE TABLE IF NOT EXISTS storage_addon_grant (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    principal_id    UUID NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
    bytes_granted   BIGINT NOT NULL CHECK (bytes_granted > 0),
    -- Stripe invoice ID that funded this grant (NULL = manually granted).
    stripe_invoice_id  TEXT,
    granted_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at      TIMESTAMPTZ,
    reason          TEXT
);

CREATE INDEX IF NOT EXISTS storage_addon_principal_idx ON storage_addon_grant(principal_id);
