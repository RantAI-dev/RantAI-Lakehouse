-- DATA-55 (QA `LH-DATA-15-06`, `TC-1731`, decision `DEC-21`): a table made by
-- a file upload can be dropped from its page and restored there for 7 days.
-- This table is the record of a drop: what the console needs to answer "is
-- this table dropped right now", "which of my tenant's tables are dropped and
-- until when", "who dropped it and how many rows it held", and "may a new load
-- take this name yet".
--
-- WHY A TABLE OF ITS OWN, and not the catalog's deleted list. The catalog
-- (`Lakekeeper`) knows that a tabular was deleted, but its answer carries no
-- tenant, no person and no row count, and it lists every dropped table of the
-- warehouse, a connector's included. None of those three things can be
-- recovered from the catalog, and a list filtered client-side could not be
-- written to fail closed per tenant.
--
-- WHY ONE ROW PER TABLE NAME, and not a row per drop: this table answers "what
-- is the current state of this name", like `file_upload` does for a file. The
-- history of who dropped and restored what, and when, is the audit trail's job
-- (`audit_event`, actions `catalog.drop` and `catalog.restore`); it is
-- append-only and never rewritten. A restore removes the row
-- (`forget_table_drop`), because a table that is there is not dropped.
--
-- WHY `row_count` may be NULL: the count is a scan of the whole table that can
-- fail or time out, and NULL says "not counted" honestly. It is never 0 for an
-- unknown count, because 0 is a real answer for an empty table (`DATA-55` D7).
--
-- WHY `tenant_id` has the shape of `upload_table_claim.tenant_id`: nullable, a
-- foreign key to `tenant(id)`, `ON DELETE SET NULL`. A row whose tenant is gone
-- shows in no tenant's list (fail closed) and still closes the name, so the
-- table stays restorable and no new load silently takes the name.
--
-- A row COUNTS while `expires_at > now()`: that is the same 7-day window the
-- catalog's soft delete profile keeps (`DELETE_RETENTION_SECONDS`,
-- `lakekeeper-auth/src/openfga.rs`), written by the database clock so the API
-- and the catalog agree on when the window ends. A drop whose window has
-- passed no longer counts, and the next drop of the same name replaces the
-- row (`record_table_drop`).

CREATE TABLE upload_table_drop (
    bronze_table TEXT        PRIMARY KEY,
    tenant_id    UUID        REFERENCES tenant(id) ON DELETE SET NULL,
    asset_id     TEXT        NOT NULL,
    dropped_by   TEXT        NOT NULL,
    row_count    BIGINT,
    dropped_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at   TIMESTAMPTZ NOT NULL
);

-- The list route reads one tenant's unexpired drops, newest first; the index
-- keeps that read off a sequential scan.
CREATE INDEX upload_table_drop_tenant_idx
    ON upload_table_drop (tenant_id, dropped_at DESC);
