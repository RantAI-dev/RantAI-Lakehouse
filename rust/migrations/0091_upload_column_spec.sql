-- DATA-18 (slice 3, task T3.2): the columns of an upload as a person confirmed
-- them, what to do with values that do not fit, and what the load reported.
-- Plan `docs/superpowers/plans/2026-10-16-data-phase-3-uploads.md`, AD3 and AD6;
-- feature page `docs/core/features/upload-column-types.md`.
--
-- # Why on the upload row
--
-- `parse_options` already records what the LAST load was told to use, so that
-- "Try again" sends the same thing back (`retryInput`). The column list is part
-- of that: a retry that forgot the types would load everything as text, and the
-- Review step and the run view have to say what the load did. They are
-- overwritten by the next claim (`mark_ingesting*` clears them), like
-- `parse_options`, `bronze_table` and `error`.
--
-- - `column_spec`: the list the load was told, `[{source, name, type, dateFormat?,
--   timezone?}]`. NULL for a load with no list (an API caller; every column text,
--   decision 11) and for an upload never loaded.
-- - `on_bad_value`: `null` (load a value that does not fit as empty) or `stop`
--   (fail the load, writing nothing). Set exactly when `column_spec` is.
-- - `bad_values`: the report the load wrote when any value did not fit,
--   `{count, byColumn: [{column, count}], rows: [{row, column, value}]}`, the
--   first 100 values each cut to 200 characters (AD6). The job writes it as an
--   object beside the upload; the API reads it when it settles the row and keeps
--   it here, so a reader of the upload never touches storage. The plan names the
--   first two columns; this third is an addition, to keep the report where the
--   row is.
--
-- Existing rows: all three are NULL, which is what "loaded before types" means.

ALTER TABLE file_upload
    ADD COLUMN IF NOT EXISTS column_spec  JSONB,
    ADD COLUMN IF NOT EXISTS on_bad_value TEXT
        CHECK (on_bad_value IS NULL OR on_bad_value IN ('null', 'stop')),
    ADD COLUMN IF NOT EXISTS bad_values   JSONB;
