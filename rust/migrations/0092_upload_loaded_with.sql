-- DATA-17 (slice 5, task T5.1): several files loaded into one table.
-- Plan `docs/superpowers/plans/2026-10-16-data-phase-3-uploads.md`, AD4; feature
-- page `docs/core/features/upload-big-files-and-more-formats.md`, decision 4.
--
-- # No batch object
--
-- A load of several files is ONE ingest call, ONE Dagster run and ONE table. The
-- call names a primary upload and up to nine more; the run's outcome is recorded
-- once, under `upload:<primary id>` (`ingest_run`), and the API settles every file
-- of the load from that one row. Each file stays its own `file_upload` row, so it
-- is still listed, deleted and duplicate-checked as before.
--
-- # What changes on `file_upload`
--
-- - `loaded_with`: the primary upload of the load a file belongs to. NULL on a
--   file that loaded (or is loading) alone, and on the primary itself; set on
--   every OTHER file of a load, from the claim until a later claim of the file
--   (or of its primary) replaces it. The claim, the settling and the lineage
--   label all read it, so it is the whole of the group.
--
-- # Why TEXT and not UUID
--
-- The plan names the column `UUID`; `file_upload.id` is `TEXT` (`up-<uuid>`,
-- `0057_upload.sql`), so a UUID column could not hold the primary's id. The type
-- follows the key it points at.
--
-- # Why a foreign key, and ON DELETE SET NULL
--
-- Deleting the primary of a load that finished must not strand the others with an
-- id of nothing, and must not be refused (the delete rules are unchanged: only a
-- file that is `ingesting` cannot be deleted). The others keep their status and
-- lose the pointer, which is what "the primary is gone" means.
--
-- # The index
--
-- Settling a load updates every row with `loaded_with = <primary>`, and the
-- lineage graph counts them per claim. The index keeps both cheap; few rows have
-- a value, so it is partial.
--
-- Existing rows are untouched: `loaded_with` is NULL, "loaded alone".

ALTER TABLE file_upload
    ADD COLUMN IF NOT EXISTS loaded_with TEXT
        REFERENCES file_upload (id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS file_upload_loaded_with_idx
    ON file_upload (loaded_with)
    WHERE loaded_with IS NOT NULL;
