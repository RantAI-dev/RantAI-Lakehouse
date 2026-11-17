-- DATA-17 (slice 1, task T1.1): a chunked, resumable upload. The browser asks
-- for an upload session, sends the file in 8 MiB pieces, and completes it
-- (plan `2026-10-16-data-phase-3-uploads.md`, AD1).
--
-- # Why the state is in Postgres
--
-- The API writes each piece as one part of an S3 multipart upload. The
-- multipart id and each part's ETag must outlive one request and one API
-- process: any replica has to take the next piece, and a restart must lose
-- nothing, so neither can live in memory. The bytes themselves stay in the
-- warehouse bucket, as before (`0057_upload.sql`): this table holds only the
-- bookkeeping, a few dozen bytes per piece.
--
-- # What changes on `file_upload`
--
-- - `status` also allows `receiving`: the row exists, its pieces are still
--   arriving, and nothing may preview or load it. The API leaves it for
--   `uploaded` when the S3 complete call has succeeded and the file passed
--   its checks (`routes::uploads`).
-- - `declared_size`: the size the client announced when it opened the
--   session. Completion refuses a file whose pieces do not sum to it.
-- - `multipart_id`: S3's id for the upload in flight. SERVER-ONLY: it is
--   never serialised, because with the storage key it addresses the
--   multipart upload directly. NULL once the upload is complete or aborted.
-- - `parts_digest`: SHA-256 over the pieces' SHA-256s in order, written at
--   completion. The whole file is never held in memory, so there is no
--   single hash of its bytes; this digest is what the duplicate check
--   compares between chunked uploads.
--
-- The existing rows are untouched: every one keeps its `status`, and the
-- three new columns are NULL (they were not chunked).
--
-- # The CHECK is replaced, by its default name
--
-- `0057` declared the CHECK inline, so Postgres named it
-- `file_upload_status_check`. It is dropped and re-added with the new
-- value in one statement, so there is no moment without a constraint.
--
-- # `file_upload_part`
--
-- One row per piece received, keyed by (upload, part number). A piece sent
-- again REPLACES the earlier row (the store upserts), as S3 keeps the last
-- copy of a part number. `part_number` is S3's own 1..=10000 range. The rows
-- go with their upload (`ON DELETE CASCADE`): a deleted or expired session
-- leaves no piece records behind.
--
-- # The sweep's index
--
-- The hourly sweep looks for `receiving` rows older than the session TTL
-- (`UPLOAD_SESSION_TTL_HOURS`, decision 8 of the feature page). A partial
-- index on those rows keeps it cheap: they are few and short-lived.

ALTER TABLE file_upload
    DROP CONSTRAINT file_upload_status_check,
    ADD CONSTRAINT file_upload_status_check
        CHECK (status IN ('receiving', 'uploaded', 'ingesting', 'ingested', 'failed'));

ALTER TABLE file_upload
    ADD COLUMN IF NOT EXISTS declared_size BIGINT
        CHECK (declared_size IS NULL OR declared_size > 0),
    ADD COLUMN IF NOT EXISTS multipart_id  TEXT,
    ADD COLUMN IF NOT EXISTS parts_digest  BYTEA;

CREATE TABLE IF NOT EXISTS file_upload_part (
    upload_id   TEXT        NOT NULL REFERENCES file_upload (id) ON DELETE CASCADE,
    part_number INTEGER     NOT NULL CHECK (part_number BETWEEN 1 AND 10000),
    size_bytes  BIGINT      NOT NULL CHECK (size_bytes > 0),
    -- S3's ETag for the part, as the store returned it (quotes included):
    -- the completion call needs it back, in order.
    etag        TEXT        NOT NULL,
    -- Hex SHA-256 of the piece's bytes, computed by the API as it received them.
    sha256      TEXT        NOT NULL,
    received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (upload_id, part_number)
);

CREATE INDEX IF NOT EXISTS upload_receiving_created_idx
    ON file_upload (created_at)
    WHERE status = 'receiving';
