CREATE TABLE IF NOT EXISTS vault_contact (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    vault_id     UUID NOT NULL REFERENCES vault(id) ON DELETE CASCADE,
    display_name TEXT NOT NULL,
    email        TEXT NOT NULL,
    birthday     DATE,
    release_note TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (vault_id, email)
);
CREATE INDEX IF NOT EXISTS vault_contact_vault_idx ON vault_contact (vault_id);
