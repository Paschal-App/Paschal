-- 0003_attachments.sql — file attachments for Letters.
--
-- An Attachment is a sealed blob stored outside the DB (BlobStore). The DB
-- carries the metadata needed to fetch + decrypt it. The same content
-- transformation pipeline is applied to every upload before sealing — see
-- crates/paschal-transform.

CREATE TABLE attachment (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    letter_id           UUID NOT NULL REFERENCES letter(id) ON DELETE CASCADE,

    -- What the user uploaded
    original_filename   TEXT NOT NULL,
    original_mime       TEXT NOT NULL,
    original_size       BIGINT NOT NULL,

    -- What we actually stored after the Transformer pipeline ran
    transformed_mime    TEXT NOT NULL,
    transformed_size    BIGINT NOT NULL,
    sha256              BYTEA NOT NULL,
    transformer_notes   JSONB NOT NULL DEFAULT '{}'::jsonb,

    -- BlobStore location of the sealed ciphertext + AES-GCM nonce.
    -- `storage_key` is opaque to the DB; the BlobStore knows how to resolve it.
    storage_key         TEXT NOT NULL,
    nonce               BYTEA NOT NULL,

    sealed_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX attachment_letter_idx ON attachment(letter_id);

-- A release_attachment_claim allows a Recipient to fetch the sealed bytes via
-- a one-time URL. Mirrors release_claim but for the binary payload.
CREATE TABLE release_attachment_claim (
    token_hash        BYTEA PRIMARY KEY,
    release_event_id  UUID NOT NULL REFERENCES release_event(id) ON DELETE CASCADE,
    attachment_id     UUID NOT NULL REFERENCES attachment(id) ON DELETE CASCADE,
    recipient_email   TEXT NOT NULL,
    issued_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at        TIMESTAMPTZ NOT NULL,
    claimed_at        TIMESTAMPTZ
);

CREATE INDEX release_attachment_claim_release_idx
    ON release_attachment_claim(release_event_id);
