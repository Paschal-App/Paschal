-- 0009_stripe_billing.sql — Real Stripe integration columns.
--
-- The MVP shipped with a stub BillingAdapter. This migration adds the
-- columns and indexes the production adapter actually needs to reconcile
-- state with Stripe.
--
-- `subscription.stripe_subscription_id` already exists from 0001_init; we
-- add UNIQUE here so the webhook handler can ON CONFLICT against it.
-- `subscription.stripe_customer_id` is moved up to the principal level
-- because Stripe customers are 1:1 with users across renewals.

ALTER TABLE principal
    ADD COLUMN IF NOT EXISTS stripe_customer_id TEXT;

DO $$ BEGIN
    CREATE UNIQUE INDEX principal_stripe_customer_id_idx ON principal(stripe_customer_id);
EXCEPTION WHEN duplicate_table THEN NULL;
END $$;

DO $$ BEGIN
    ALTER TABLE subscription
        ADD CONSTRAINT subscription_stripe_subscription_id_key UNIQUE (stripe_subscription_id);
EXCEPTION
    WHEN duplicate_table THEN NULL;
    WHEN duplicate_object THEN NULL;
    WHEN invalid_table_definition THEN NULL;
END $$;

-- The de-dup table for incoming Stripe webhooks. Insert the event ID before
-- processing; PK conflict means we've seen it.
CREATE TABLE IF NOT EXISTS stripe_webhook_event (
    event_id    TEXT PRIMARY KEY,
    event_type  TEXT NOT NULL,
    received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    processed_at TIMESTAMPTZ,
    -- The raw event body, for replay & debugging. Trimmed for old rows by
    -- the retention scheduler at 90 days.
    payload     JSONB NOT NULL
);

CREATE INDEX IF NOT EXISTS stripe_webhook_event_type_idx
    ON stripe_webhook_event(event_type);
CREATE INDEX IF NOT EXISTS stripe_webhook_event_received_at_idx
    ON stripe_webhook_event(received_at);
