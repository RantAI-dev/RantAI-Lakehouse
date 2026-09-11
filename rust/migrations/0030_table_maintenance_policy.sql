-- WS2 §4: per-table Iceberg maintenance policy, replacing the single
-- global sweep dagster/dispar_orchestrate/maintenance.py currently runs
-- against every bronze.* table. A table with no row here keeps today's
-- default behaviour (remove_orphan_files only).
--
-- The CHECK constraints are defense in depth behind the future
-- POST /api/lakehouse/tables/{ns}/{table}/maintenance route's own
-- validation, not a substitute for it:
--   * `snapshots_to_keep >= 1` — the current snapshot is always kept, so a
--     keep-count of 0 is impossible.
--   * `orphan_age_hours >= 1` — an orphan age of 0 would delete files that
--     in-flight writes still need.
--   * `schedule IN ('daily','weekly')` — the only two cadences the
--     maintenance job understands.
--   * `namespace`/`table_name` are restricted to `[a-z0-9_]+` because both
--     values ultimately get interpolated into an
--     `ALTER TABLE iceberg."{namespace}"."{table_name}" EXECUTE ...` Trino
--     statement (`format!` only for constant identifiers) elsewhere in this
--     workstream; a value that could not appear there should never be
--     accepted here either.
CREATE TABLE table_maintenance_policy (
    namespace            TEXT NOT NULL,
    table_name           TEXT NOT NULL,
    snapshots_to_keep    INT,
    orphan_age_hours     INT,
    compact_small_files  BOOLEAN NOT NULL DEFAULT FALSE,
    schedule             TEXT,
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (namespace, table_name),
    CONSTRAINT table_maintenance_policy_schedule_check
        CHECK (schedule IS NULL OR schedule IN ('daily', 'weekly')),
    CONSTRAINT table_maintenance_policy_snapshots_to_keep_check
        CHECK (snapshots_to_keep IS NULL OR snapshots_to_keep >= 1),
    CONSTRAINT table_maintenance_policy_orphan_age_hours_check
        CHECK (orphan_age_hours IS NULL OR orphan_age_hours >= 1),
    CONSTRAINT table_maintenance_policy_namespace_check
        CHECK (namespace ~ '^[a-z0-9_]+$'),
    CONSTRAINT table_maintenance_policy_table_name_check
        CHECK (table_name ~ '^[a-z0-9_]+$')
);

-- WS2 plan review W8: POST /api/lakehouse/tables/{ns}/{table}/maintenance
-- needs a real permission — the grand plan names `governance:write`, which
-- does not exist as a seeded grant yet. Following
-- 0020_extend_role_grants.sql's exact idiom: idempotent, regex-guarded so
-- re-applying this migration (or a hand-run copy of it) never appends the
-- token twice, and folded onto the role that already owns policy/residency/
-- audit administration (0002_seed_identity.sql) rather than inventing a new
-- role.
UPDATE role
SET permissions = permissions || ', governance:write'
WHERE name = 'Governance Admin'
  AND permissions !~ '(^|,)\s*governance:write\s*(,|$)';
