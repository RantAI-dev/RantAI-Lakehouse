-- File upload, T3 of docs/superpowers/plans/2026-10-02-upload-file.md
-- (ADR 0014, decisions 5 and 6): the columns `file_upload` needs before a
-- route can be written over it. `0054_upload.sql` stays as it is: the plan
-- records it as applied on the development database, and `sqlx` refuses to
-- boot when the checksum of an applied migration changes
-- (CODE-STANDARD.md section 3.5).
--
-- MIGRATION NUMBERS: `0054` and this file are provisional. Backlog item
-- `DATA-1` plans to use `0054` on `main`, so both files are renumbered when
-- this branch meets `main`. They are not renumbered now: `0054` is already
-- recorded in the development database under its present name and checksum.
--
-- WHY `tenant_id`: an upload belongs to the uploader's active tenant, and
-- every route that names an upload answers 404 outside it (ADR 0014,
-- decision 6), the rule connectors follow on the column
-- `0042_tenant_provisioning.sql` gave them. This column has that shape:
-- nullable, a foreign key to `tenant(id)`, `ON DELETE SET NULL`. NULL
-- means "not assigned to a tenant" and is invisible to every
-- tenant-scoped read, so a row whose tenant is gone shows to nobody, never
-- to everybody (fail closed). Its object stays in the bucket under
-- `uploads/`, and no tenant-scoped route can reach it any more; only a
-- storage sweep can.
--
-- WHY the free-text `tenant` column goes: the sketch recorded the
-- deployment's constant `TENANT_ID` there, not the caller's tenant, and
-- compared it as text. Two tenant columns would leave one of them to be
-- trusted by mistake. There is nothing to backfill from it: no code path
-- inserted into `file_upload` before this migration (the routes and the
-- store module were never declared, so they were never compiled), so a
-- database that applied `0054` holds no rows. A row that did exist would
-- keep a NULL `tenant_id` and be invisible, rather than be guessed into a
-- tenant.
--
-- WHY `load_mode`: a load into a table an earlier upload created either
-- replaces its rows (the default) or adds to them (ADR 0014, decision 5).
-- What the last load was told to do is recorded with the upload, the way
-- `parse_options` records what it was told to read. NULL until a load is
-- requested. Plain TEXT with a CHECK, for the reason `status` is: the set
-- is cheaper to widen than a Postgres enum.
--
-- WHY `row_count`: what the load reported, read back from
-- `lake.bronze_meta.ingest_run` by the API (the job writes no Postgres,
-- ADR 0014 decision 5). NULL means NOT MEASURED and is never 0: a load
-- whose sink reported no total, a failed load, and a file not yet loaded
-- all say so instead of showing a number nobody counted.
--
-- INDEXES: the listing is newest-first within a tenant, so
-- `upload_tenant_created_idx` (on the dropped text column) is replaced by
-- one on `(tenant_id, created_at DESC)`. `bronze_table` is looked up by two
-- checks the ingest route makes before it launches a load ("did an upload
-- of this tenant create that table", "is another upload loading into it"),
-- so it gets an index of its own.
--
-- The one destructive statement is the DROP COLUMN; see above for why there
-- is nothing in it worth keeping.

DROP INDEX IF EXISTS upload_tenant_created_idx;

ALTER TABLE file_upload
    DROP COLUMN tenant,
    ADD COLUMN tenant_id  UUID REFERENCES tenant(id) ON DELETE SET NULL,
    ADD COLUMN load_mode  TEXT
        CHECK (load_mode IN ('replace', 'append')),
    ADD COLUMN row_count  BIGINT;

CREATE INDEX IF NOT EXISTS upload_tenant_id_created_idx
    ON file_upload (tenant_id, created_at DESC);

CREATE INDEX IF NOT EXISTS upload_bronze_table_idx
    ON file_upload (bronze_table);
