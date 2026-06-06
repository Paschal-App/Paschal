-- 0017_vault_storage_region.sql — per-vault attachment storage region.
--
-- Estate+ and Legacy plans may place a Vault's sealed attachment blobs in a
-- chosen region (Australia / EU / US) and move them between regions. Only the
-- attachment *blobs* are region-placed; letter ciphertext and all metadata
-- stay in the Beacon's primary-region database. This is a storage-location
-- choice, not a hard data-residency guarantee — see specs/05 and specs/12.
--
-- The region is recorded on the vault as the AWS region code. New blobs for a
-- vault go to its current region; each attachment's storage_key independently
-- encodes the region its bytes live in (the BlobStore resolves it), so a move
-- can be crash-safe while objects exist transiently in two regions.
--
-- Expand step only (add column, backfill default). No reads switch in this
-- release; nothing is dropped. Forward-only and re-runnable.

ALTER TABLE vault
    ADD COLUMN IF NOT EXISTS storage_region TEXT NOT NULL DEFAULT 'ap-southeast-2';

-- Guard the allowed set at the database boundary. Drop-then-add so re-runs and
-- future region additions (a new 00NN migration) replace cleanly.
ALTER TABLE vault DROP CONSTRAINT IF EXISTS vault_storage_region_check;
ALTER TABLE vault ADD CONSTRAINT vault_storage_region_check
    CHECK (storage_region IN ('ap-southeast-2', 'eu-central-1', 'us-east-1'));

CREATE INDEX IF NOT EXISTS vault_storage_region_idx ON vault(storage_region);
