-- DATA-13 (docs/core/specs/data-13.md; plan
-- docs/superpowers/plans/2026-10-16-data-13-column-lineage.md, task 1; finding
-- F1): column-level lineage, which the platform never recorded
-- (`GET /api/governance/lineage` has answered `columnMappings: []` since it
-- was written).
--
-- Why a table: the table-level graph is rebuilt on every request from other
-- records, but which source column feeds which output column exists nowhere
-- else once a statement has run, so it has to be written down when the
-- statement is known. Three producers write it (`producer_kind`):
--   * `pipeline`   an authored pipeline run that succeeded; `producer_ref` is
--                  the pipeline id and `tenant_id` the pipeline's tenant;
--   * `build`      a Silver or Gold table built by SQL outside a pipeline,
--                  read from the ClickHouse query log; `producer_ref` is the
--                  target table, shared (`tenant_id` NULL) like the catalog;
--   * `sql_source` a saved dashboard SQL source; `producer_ref` is the source
--                  id and `to_table` is `sql_source:<id>`.
-- A producer's rows are replaced as a set on every write (the store deletes
-- by `producer_kind, producer_ref` and inserts in one transaction), so a
-- changed statement never leaves the old mapping behind.
--
-- Table keys are written the way the table graph names them: `bronze.x`,
-- `silver.x`, `serving.x`, `sql_source:<id>`.
--
-- Honesty (principle 2): a column the parser could not map is stored, not
-- dropped. `partial` is true and `reason` says why. `from_column` is NULL
-- when no source column could be resolved at all, and `from_table` is NULL
-- when not even a source table could be told (a mapping is then drawn from
-- nothing and ends the walk up). A walk never follows a NULL.
--
-- `column_lineage_pending` holds the SQL the API approved at the pipeline
-- enforce call until the run's success event arrives. It is in Postgres and
-- not in memory because the API can restart between the two. One row per
-- pipeline: the latest enforce wins.
--
-- `column_lineage_state` is a one-row cursor for the build pass: the newest
-- query-log time already mapped, so each 15-minute tick reads only newer
-- statements. No row means "never ran": the pass starts from the retention
-- window.
--
-- Retention is 365 days (the spec); the API deletes older rows on the same
-- tick. The CHECKs below are shape only; which producer may write what is the
-- store's and the handlers' job.
CREATE TABLE column_lineage (
    id            BIGSERIAL PRIMARY KEY,
    producer_kind TEXT NOT NULL,
    producer_ref  TEXT NOT NULL,
    tenant_id     UUID,
    from_table    TEXT,
    from_column   TEXT,
    to_table      TEXT NOT NULL,
    to_column     TEXT NOT NULL,
    transform     TEXT NOT NULL,
    partial       BOOLEAN NOT NULL DEFAULT FALSE,
    reason        TEXT,
    recorded_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT column_lineage_producer_kind_check
        CHECK (producer_kind IN ('pipeline', 'build', 'sql_source')),
    CONSTRAINT column_lineage_transform_check
        CHECK (transform IN ('copy', 'cast', 'expression', 'unknown')),
    -- A mapping that is not partial names both ends.
    CONSTRAINT column_lineage_complete_check
        CHECK (partial OR (from_table IS NOT NULL AND from_column IS NOT NULL)),
    -- A partial mapping always says why.
    CONSTRAINT column_lineage_reason_check
        CHECK (NOT partial OR (reason IS NOT NULL AND char_length(reason) <= 300))
);

CREATE INDEX column_lineage_to_idx ON column_lineage (to_table, to_column);
CREATE INDEX column_lineage_from_idx ON column_lineage (from_table, from_column);
CREATE INDEX column_lineage_producer_idx ON column_lineage (producer_kind, producer_ref);
CREATE INDEX column_lineage_recorded_idx ON column_lineage (recorded_at);

CREATE TABLE column_lineage_pending (
    pipeline_id TEXT PRIMARY KEY,
    sql         TEXT NOT NULL,
    enforced_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE column_lineage_state (
    singleton            BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    builds_processed_until TIMESTAMPTZ NOT NULL
);
