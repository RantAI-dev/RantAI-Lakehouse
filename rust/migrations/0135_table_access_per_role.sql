-- GOV-7 (D3, `docs/core/specs/gov-7.md`, feature page
-- `docs/core/features/table-access-per-role.md`, plan
-- `docs/superpowers/plans/2026-11-09-gov-7-table-access-per-role.md`):
-- read access per schema and table, enforced everywhere data is read.
--
-- WHY:
-- `query:read` reads every table. Masks and row filters per role exist
-- (`policy_engine`), but nothing can hide a table from a role, so a company
-- cannot give Finance only the `finance` schema. This migration adds the two
-- rows `lakehouse-api::table_access` reads on every governed read path.
--
-- 1. `role.read_scope` is `'all'` by default, so an upgrade changes nobody's
--    access (D1): every existing role keeps reading every table until an
--    administrator narrows it. The CHECK constraint is named, so a future
--    migration can drop it by name rather than by guessing Postgres's
--    generated one.
-- 2. `role_read_grant` holds the union a narrowed role may read. `table_name
--    = '*'` means the whole schema. Names need not exist yet: a grant may
--    name a table a pipeline creates later, so there is deliberately no FK to
--    any catalog of tables (there is none — the catalog is assembled from
--    `ClickHouse` and Iceberg at read time).
-- 3. `created_by` is the administrator who set the grant (audit context),
--    `ON DELETE SET NULL` so removing the person keeps the grant: the grant is
--    a property of the ROLE, and losing it because an admin left would widen
--    access silently. `role_id` cascades, so deleting a role takes its grants
--    with it.
--
-- The primary key doubles as the uniqueness rule (a schema/table pair is
-- named once per role) and as the index every read uses.

ALTER TABLE role ADD COLUMN read_scope TEXT NOT NULL DEFAULT 'all';

ALTER TABLE role
    ADD CONSTRAINT role_read_scope_check CHECK (read_scope IN ('all', 'listed'));

CREATE TABLE role_read_grant (
    role_id     UUID NOT NULL REFERENCES role(id) ON DELETE CASCADE,
    schema_name TEXT NOT NULL,
    table_name  TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_by  UUID NULL REFERENCES app_user(id) ON DELETE SET NULL,
    PRIMARY KEY (role_id, schema_name, table_name)
);
