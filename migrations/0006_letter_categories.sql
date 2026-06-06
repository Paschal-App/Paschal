-- 0006_letter_categories.sql — Expand Letter kind + add category metadata.
--
-- The MVP shipped with kind ∈ {MESSAGE, CREDENTIAL_BUNDLE, FILE_ARCHIVE, ACTION}.
-- The strategic plan adds three more letter shapes that the frontend wants
-- to recognise distinctly:
--
--   WILL_LOCATOR   — structured "where to find my will" Letter. The body
--                    is still encrypted prose, but the frontend knows to
--                    prefill it with a fielded template (executor, attorney,
--                    document location, witnesses).
--   VIDEO_MESSAGE  — a Letter whose primary payload is a recorded video
--                    attachment; the body acts as a transcript / framing
--                    note.
--   AUDIO_MESSAGE  — likewise for voice messages.
--
-- We also add a free-form `category` column for surfacing the chosen
-- template (e.g. "letter_to_children", "funeral_preferences",
-- "crypto_wallet_recovery"). The column is informational only — the
-- recipient never sees it; it just lets the dashboard group Letters and
-- the compose flow remember "this is the credit-card-cancel letter."

ALTER TABLE letter DROP CONSTRAINT IF EXISTS letter_kind_check;
ALTER TABLE letter ADD CONSTRAINT letter_kind_check
    CHECK (kind IN (
        'MESSAGE',
        'CREDENTIAL_BUNDLE',
        'FILE_ARCHIVE',
        'ACTION',
        'WILL_LOCATOR',
        'VIDEO_MESSAGE',
        'AUDIO_MESSAGE'
    ));

ALTER TABLE letter
    ADD COLUMN IF NOT EXISTS category TEXT;

-- An index helps "show me all my will-locator letters" queries.
CREATE INDEX IF NOT EXISTS letter_kind_idx ON letter(kind);
CREATE INDEX IF NOT EXISTS letter_category_idx ON letter(category)
    WHERE category IS NOT NULL;
