-- WS9 Phase A: extend the `connector.adapter` / `connector.ingest_mode`
-- allowlists for the four Tier 2 adapters (`mongodb`, `kafka`, `sftp`,
-- `oracle`), create `bronze_meta.ingest_offset` for Kafka micro-batch
-- offset tracking, and flip the four Tier 2 `connector_type` rows from
-- `supported = false` to `supported = true` with their now-real adapter
-- value.
--
-- ── Why DROP+ADD the CHECK constraints, not ALTER them ───────────────
--
-- Postgres has no `ALTER CHECK ... ADD VALUE` (that is a MySQL idiom); the
-- only way to extend a CHECK's allowed set is to DROP the constraint and
-- ADD it back with the wider set. Both `connector_adapter_check` (set on
-- `connector.adapter` by `0033_connector_ingest_spec.sql`) and
-- `connector_ingest_mode_check` (set there alongside) are dropped and
-- re-added here. No existing row is invalidated by the widening
-- (`sql`/`cdc`/`files`/`rest`/`sheets` are still in the new set; `batch`/
-- `cdc` likewise), so the DROP+ADD does not need an UPDATE between the
-- two statements.
--
-- ── Why UPDATE the four `connector_type` rows with a guarded predicate,
--    not a blanket UPDATE ─────────────────────────────────────────────
--
-- The migration targets `adapter IS NULL AND supported = false`
-- specifically, the same shape `0020_extend_role_grants.sql` /
-- `0028_connector_health_unknown_until_tested.sql` use: a deployment that
-- has since hand-edited a row (an operator who set
-- `supported = false, adapter = NULL` to keep a roadmap entry genuinely
-- unsuppoted, despite this migration landing) keeps its row verbatim,
-- exactly as a blanket UPDATE would not. A row already flipped to
-- `supported = true` (by an out-of-band operator) is also left alone.
--
-- ── Why a separate Postgres `bronze_meta.ingest_offset` table at all ──
--
-- The Dagster Tier 2 Kafka adapter (`dagster/dispar_orchestrate/adapters/
-- kafka.py` and `bronze_catalog.py::record_ingest_offset`) currently
-- writes its committed offset to ClickHouse's
-- `lake.bronze_meta.ingest_offset` (ReplacingMergeTree, see
-- `_INGEST_OFFSET_SCHEMA` in `bronze_catalog.py`). That is the active
-- surface this build actually reads from at restart time
-- (`last_committed_offset`). This Postgres table is the same logical
-- shape, mirrored here so that the same offset state is visible to any
-- future Rust-side / API-side reader without a round-trip through
-- ClickHouse, and so that a future migration of the offset writer off
-- ClickHouse has the schema already present on the Postgres side to
-- write into. The PRIMARY KEY `(connector_id, topic, partition_id)`
-- enforces the same "current position only, upsert per commit"
-- semantics `ReplacingMergeTree ORDER BY (...)` gives in ClickHouse:
-- one row per partition per topic, overwritten on every commit, never
-- a history log (history stays on `connector_probe_result`, not here).
--
-- Migration number is 0043, not the plan's literal 0041: WS8 has
-- claimed `0042_tenant_provisioning.sql` on the sibling branch, and per
-- the WS8/WS9 handover the branches reserve their slots against each
-- other to avoid a merge collision when the two stacks unify.
ALTER TABLE connector DROP CONSTRAINT connector_adapter_check;
ALTER TABLE connector
    ADD CONSTRAINT connector_adapter_check
    CHECK (adapter IS NULL OR adapter IN ('sql','cdc','files','rest','sheets','mongodb','kafka','sftp'));

ALTER TABLE connector DROP CONSTRAINT connector_ingest_mode_check;
ALTER TABLE connector
    ADD CONSTRAINT connector_ingest_mode_check
    CHECK (ingest_mode IS NULL OR ingest_mode IN ('batch','cdc','stream'));

-- `connector_type.adapter` (set by `0035_connector_type.sql`) has the same
-- closed-set CHECK as `connector.adapter`. Widen it to admit
-- `mongodb`/`kafka`/`sftp`; the existing set (`sql`/`cdc`/`files`/`rest`/
-- `sheets`) is a subset of the new set, so no row already in the table is
-- invalidated. Oracle reuses the existing `sql` value, so it does not
-- appear here.
ALTER TABLE connector_type DROP CONSTRAINT connector_type_adapter_check;
ALTER TABLE connector_type
    ADD CONSTRAINT connector_type_adapter_check
    CHECK (adapter IS NULL OR adapter IN ('sql','cdc','files','rest','sheets','mongodb','kafka','sftp'));

-- Flip the four Tier 2 connector types. Oracle reuses the existing
-- `sql` adapter (it parses as a `SqlDial` with `driver = "oracle"`,
-- `ingest_spec.rs`'s Tier-2 addition): one named adapter value per
-- adapter, not a parallel fourth SQL-driver-specific value. SFTP is
-- its own adapter (`sftp`), not a value of `FilesProtocol::Sftp`,
-- because `adapters/files.py` is s3fs-only and a `files`/`sftp`
-- round-trip would never have a working sink (`adapters/sftp.py` is
-- its own module, WS9 Phase D3). MongoDB and Kafka likewise map to
-- the closed set of adapter values each adapter module implements
-- today.
UPDATE connector_type
SET supported = true,
    adapter = 'kafka'
WHERE name = 'Kafka'
  AND adapter IS NULL
  AND supported = false;

UPDATE connector_type
SET supported = true,
    adapter = 'mongodb'
WHERE name = 'MongoDB'
  AND adapter IS NULL
  AND supported = false;

UPDATE connector_type
SET supported = true,
    adapter = 'sftp'
WHERE name = 'SFTP'
  AND adapter IS NULL
  AND supported = false;

-- Oracle is a `sql` adapter (Tier 2's `SqlDriver::Oracle` variant
-- dispatches into the same `sql` `dial`/`Dial::parse` shape
-- `PostgreSQL`/`MySQL`/`SQL Server` already use); the named `adapter`
-- stays `'sql'` so the wizard sees Oracle against the existing
-- Postgres/MySQL/MSSQL/Oracle driver dropdown, not a parallel fourth
-- adapter value.
UPDATE connector_type
SET supported = true,
    adapter = 'sql'
WHERE name = 'Oracle'
  AND adapter IS NULL
  AND supported = false;

CREATE SCHEMA IF NOT EXISTS bronze_meta;

CREATE TABLE bronze_meta.ingest_offset (
    connector_id     TEXT NOT NULL,
    topic            TEXT NOT NULL,
    partition_id     INT  NOT NULL,
    committed_offset BIGINT NOT NULL,
    committed_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (connector_id, topic, partition_id),
    CONSTRAINT bronze_meta_ingest_offset_connector_id_check
        CHECK (connector_id ~ '^[A-Za-z0-9._-]+$'),
    CONSTRAINT bronze_meta_ingest_offset_topic_check
        CHECK (topic ~ '^[A-Za-z0-9._-]+$'),
    CONSTRAINT bronze_meta_ingest_offset_partition_id_check
        CHECK (partition_id >= 0),
    CONSTRAINT bronze_meta_ingest_offset_committed_offset_check
        CHECK (committed_offset >= 0)
);