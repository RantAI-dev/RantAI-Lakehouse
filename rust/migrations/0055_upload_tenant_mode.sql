-- File upload, T3 of docs/superpowers/plans/2026-10-02-upload-file.md
-- (ADR 0014, decisions 5 and 6): the columns `file_upload` needs before a
-- route can be written over it, and `upload_table_claim`, the record of who
-- owns a raw table name. `0054_upload.sql` stays as it is: the plan records
-- it as applied on the development database, and `sqlx` refuses to boot when
-- the checksum of an applied migration changes (CODE-STANDARD.md
-- section 3.5).
--
-- MIGRATION NUMBERS: `0054` and this file are provisional. Backlog item
-- `DATA-1` plans to use `0054` on `main`, so both files are renumbered when
-- this branch meets `main`. They are not renumbered now: `0054` is already
-- recorded in the development database under its present name and checksum.
--
-- EDITED TWICE AFTER T3, FROZEN FROM THE TRIAL DEPLOY ON: T5a of the plan
-- added a soft-delete timestamp column here, after the review of slice B
-- part 1 (finding B1); T6a took it out again and added `upload_table_claim`,
-- after the review of slice B part 2 (finding B4). Editing an applied
-- migration is what section 3.5 of docs/CODE-STANDARD.md forbids, and it was
-- allowed here only because this file had been applied on throwaway test
-- databases and nowhere that persists: the development database has never
-- run it, so that column was never applied anywhere that kept it and there is
-- nothing to drop. From the trial deploy on, this file is frozen like any
-- applied migration, and a further change to `file_upload` or
-- `upload_table_claim` is a new migration.
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
-- WHY `upload_table_claim` (T6a, finding B4): who owns a raw table name is a
-- record of its own, because no upload row can carry it. A row names only the
-- table of its LAST load (`bronze_table` is overwritten by the next one), it
-- goes when the upload is deleted, and nothing stops the rows of two tenants
-- from naming one table. T5a read ownership from those rows and two failures
-- followed: tenant B's load into `t` fails before it writes, tenant A then
-- creates `t`, and B's retry passes the check and replaces A's rows; and an
-- upload that loaded `x` and was then loaded into `y` no longer names `x`, so
-- nothing says an upload made `x`.
--
-- A claim is one row here, keyed by the table name. The primary key is the
-- point: one table name has one owner and the database decides which, even
-- when two tenants ask for the same new name in the same moment
-- (`lakehouse_store::uploads::claim_table` is one `INSERT ... ON CONFLICT`
-- statement). A claim is made when a load into the table is first requested,
-- before the upload is marked as loading and before the job is launched, and
-- it is NEVER RELEASED: not when the upload is deleted, not when the load
-- fails (the job may have written before it failed), not when the table is
-- left unused. The cost is that a name a tenant's upload once asked for stays
-- that tenant's, even if the load never wrote. A connector may not take a
-- claimed table either (plan task T8).
--
-- `tenant_id` has the shape of `file_upload.tenant_id`: nullable, a foreign
-- key to `tenant(id)`, `ON DELETE SET NULL`. A claim whose tenant is gone
-- belongs to nobody, and the store answers "not yours" to every tenant for it
-- (fail closed), so the name stays reserved for good. `upload_id` is the
-- upload that first asked for the name, kept for the record. It has no
-- foreign key: the claim must outlive the upload, and a deleted upload's row
-- is really deleted.
--
-- INDEXES: the listing is newest-first within a tenant, so
-- `upload_tenant_created_idx` (on the dropped text column) is replaced by
-- one on `(tenant_id, created_at DESC)`. `bronze_table` is looked up by the
-- check the ingest route makes before it starts a load ("is another upload
-- loading into this table"), so it gets an index of its own. The questions
-- about who owns a table name ("whose claim is it", "may a connector take
-- it") are answered by the primary key of `upload_table_claim`.
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

CREATE TABLE IF NOT EXISTS upload_table_claim (
    bronze_table TEXT        PRIMARY KEY,
    tenant_id    UUID        REFERENCES tenant(id) ON DELETE SET NULL,
    upload_id    TEXT        NOT NULL,
    claimed_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
