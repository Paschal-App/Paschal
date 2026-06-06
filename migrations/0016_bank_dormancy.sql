CREATE TABLE IF NOT EXISTS bank_dormancy_subscription (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    principal_id     UUID NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
    webhook_id       UUID NOT NULL UNIQUE DEFAULT gen_random_uuid(),
    secret_hmac_key  BYTEA NOT NULL,
    last_webhook_at  TIMESTAMPTZ,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX IF NOT EXISTS bank_dormancy_principal_idx
    ON bank_dormancy_subscription (principal_id);
