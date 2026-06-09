-- Covert, user-armed duress signal.
--
-- The Principal arms a private webhook and connects it to a panic action they
-- can perform under coercion without tipping off a captor — e.g. a specific
-- gift-card purchase wired through their bank automation. A POST to the webhook
-- sets triggered_at, which (a) freezes the dead-man's switch so no Vault can
-- release while the duress is active, and (b) silently alerts a trusted contact.
--
-- Security is the opaque webhook_id; the trigger endpoint takes no auth header
-- and always returns 200 so an observer can't probe it. Disarming deletes the
-- row. Expand-only: additive table, safe under blue/green.
CREATE TABLE IF NOT EXISTS duress_signal (
    principal_id  UUID PRIMARY KEY REFERENCES principal(id) ON DELETE CASCADE,
    webhook_id    UUID NOT NULL UNIQUE,
    alert_email   TEXT,
    armed_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    triggered_at  TIMESTAMPTZ,
    last_alert_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_duress_webhook ON duress_signal (webhook_id);
