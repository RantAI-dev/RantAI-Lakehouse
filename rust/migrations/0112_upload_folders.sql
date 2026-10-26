-- DATA-35: folders for uploaded files.
-- Plan `docs/superpowers/plans/2026-10-26-data-35-upload-folders.md`, AD1 and
-- AD2; feature page `docs/core/features/upload-folders.md`, decisions 1 to 8.
--
-- # What a folder is (decision 1)
--
-- A grouping on the Uploaded files tab and nothing else. `file_upload.folder_id`
-- is read by the tab and by nothing a pipeline, chart, saved query, lineage graph
-- or search reads: a file's table keeps its name whatever folder the file is in.
--
-- # Why a table of its own, in Postgres, with a tenant (decision 5, SEC-16)
--
-- The dashboard folders live in ClickHouse without a tenant column and cannot
-- hold a tenant's rule. `upload_folder.tenant_id` is NOT NULL and cascades with
-- the tenant: a folder with no tenant would show to nobody, and there is nothing
-- to keep when its tenant is gone. Every store query binds the tenant.
--
-- # Names and depth (decisions 3 and 6)
--
-- Names are 1 to 100 characters after trim (CHECK) and unique among the folders
-- that share a parent, case-insensitively (`lower(name)`); the root level is one
-- parent too, which is why the key uses `COALESCE(parent_id, '')` (a plain
-- unique index treats every NULL as different). The depth limit of 3 and the
-- "no cycle" rule need a walk up or down the tree, so they are checked in the
-- store inside one transaction under a per-tenant advisory lock
-- (`lakehouse_store::upload_folders`), never in this file and never by the
-- console's 100-row list.
--
-- # Deletion (decision 8)
--
-- A folder holding a file or a folder cannot be deleted. `file_upload.folder_id`
-- is `ON DELETE RESTRICT` so that a delete racing with a move into the folder
-- fails in the database even if the store's own check was passed a moment
-- earlier. `parent_id` uses the default `NO ACTION` and not RESTRICT: both refuse
-- to delete a parent that still has children, but NO ACTION is checked at the
-- end of the statement, so deleting a whole tenant (`ON DELETE CASCADE` on
-- `tenant_id`) can remove a parent and its children in one statement; RESTRICT
-- would fail on whichever row the cascade reached first. A tenant that still has
-- foldered files cannot be deleted either way, which is the safe side: the
-- files' own rows are kept by `ON DELETE SET NULL` on their `tenant_id`, and
-- their folder must not disappear under them.
--
-- # The table's folder for re-uploads (decision 2)
--
-- `upload_table_claim.folder_id` records the folder of the latest load into the
-- table, so that a re-upload into an existing table preselects it. The claim
-- outlives uploads (`0058`), so deleting a folder only forgets it here
-- (`ON DELETE SET NULL`); a claim can never block a folder's deletion.
--
-- Existing rows are untouched: `folder_id` is NULL, "no folder".

CREATE TABLE IF NOT EXISTS upload_folder (
    id         TEXT        PRIMARY KEY,
    tenant_id  UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    parent_id  TEXT        REFERENCES upload_folder (id),
    name       TEXT        NOT NULL
        CONSTRAINT upload_folder_name_length
        CHECK (char_length(btrim(name)) BETWEEN 1 AND 100),
    created_by TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS upload_folder_sibling_name_idx
    ON upload_folder (tenant_id, COALESCE(parent_id, ''), lower(name));

CREATE INDEX IF NOT EXISTS upload_folder_parent_idx
    ON upload_folder (parent_id)
    WHERE parent_id IS NOT NULL;

ALTER TABLE file_upload
    ADD COLUMN IF NOT EXISTS folder_id TEXT
        REFERENCES upload_folder (id) ON DELETE RESTRICT;

CREATE INDEX IF NOT EXISTS file_upload_tenant_folder_idx
    ON file_upload (tenant_id, folder_id);

ALTER TABLE upload_table_claim
    ADD COLUMN IF NOT EXISTS folder_id TEXT
        REFERENCES upload_folder (id) ON DELETE SET NULL;
