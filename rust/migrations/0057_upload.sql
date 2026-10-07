-- Phase 1 of the file-upload feature: the registry of files a user has
-- uploaded, and what became of each one.
--
-- NAMING: `file_upload` in the default `public` schema, like every
-- other Postgres table in this repo. The "console" in `console.upload`
-- would have been a ClickHouse database name, not a Postgres schema —
-- the two stores are described with the same word in the architecture
-- docs and are not the same namespace.
--
-- WHY this table exists: before it, the only way raw data entered this
-- system was a database connection configured in `.env` by an operator
-- (see `dagster/dispar_orchestrate/dlt_pipeline.py`). A file someone
-- exports from SAP, Excel or a vendor portal had no way in at all. This
-- table is the console's record of "a file arrived": who sent it, where
-- its bytes live, what we believe its shape to be, and which Bronze table
-- it eventually became.
--
-- WHAT IS NOT HERE: the file's bytes. They go to object storage (the same
-- RustFS/S3 warehouse bucket Bronze already uses, under an `uploads/`
-- prefix), and `storage_key` points at them. A 25 MB SAP export stored as
-- a row here would be 25 MB this database reads on every `SELECT *` and
-- never uses — Postgres holds the console's state, not the customer's
-- data.
--
-- `storage_key` IS THE SERVER'S OWN NAME for the object, never the user's
-- filename: a name like `../../etc/passwd`, a 2 KB name, or two users
-- uploading `data.csv` must not be able to collide or escape the prefix.
-- The original name is kept in `original_filename` for display only.
--
-- `parse_options` is JSONB because the shape differs per format and is
-- still settling (delimiter and encoding for CSV/TSV; sheet name for a
-- future XLSX). It records what the INGEST WAS TOLD TO USE, not what
-- detection guessed — a preview's guess the user then corrected must not
-- be what a re-run replays.

CREATE TABLE IF NOT EXISTS file_upload (
    id                TEXT PRIMARY KEY,
    -- Display name, as the browser reported it. Never used to build a
    -- storage path (see the header comment).
    original_filename TEXT        NOT NULL,
    -- Object key inside the warehouse bucket, e.g.
    -- `uploads/<tenant>/<uuid>.csv`. Unique so two rows can never claim
    -- the same bytes.
    storage_key       TEXT        NOT NULL UNIQUE,
    content_type      TEXT        NOT NULL DEFAULT '',
    size_bytes        BIGINT      NOT NULL DEFAULT 0,
    -- SHA-256 of the bytes as stored. Lets the console tell a user "you
    -- already uploaded this file" instead of silently appending the same
    -- rows to Bronze a second time — Bronze is append-only, so a
    -- duplicate upload doubles every count downstream.
    sha256            TEXT        NOT NULL DEFAULT '',
    uploaded_by       TEXT        NOT NULL DEFAULT '',
    tenant            TEXT        NOT NULL DEFAULT '',
    -- `uploaded` -> `ingesting` -> `ingested` | `failed`. Deliberately a
    -- plain TEXT with a CHECK rather than an enum type: this set is still
    -- moving (a future `previewed` or `archived`), and altering a CHECK is
    -- a cheaper migration than altering a Postgres enum.
    status            TEXT        NOT NULL DEFAULT 'uploaded'
        CHECK (status IN ('uploaded', 'ingesting', 'ingested', 'failed')),
    -- What the ingest was told to use: {encoding, delimiter, header_row,
    -- skip_rows, ...}. NULL until an ingest is requested.
    parse_options     JSONB,
    -- The Bronze table this file became, once it has been ingested.
    bronze_table      TEXT,
    -- Dagster run id of the ingest, for linking to its logs.
    run_id            TEXT,
    error             TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- The console lists uploads newest-first, scoped to a tenant.
CREATE INDEX IF NOT EXISTS upload_tenant_created_idx
    ON file_upload (tenant, created_at DESC);

-- Duplicate detection looks a checksum up directly.
CREATE INDEX IF NOT EXISTS upload_sha256_idx
    ON file_upload (sha256)
    WHERE sha256 <> '';
