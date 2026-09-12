# ADR 0006 — Schema-evolution propagation: source -> Bronze Iceberg -> Silver

- **Status:** Accepted
- **Phase:** P4
- **Date:** 2026-09-01

## Context

R7 in the risk register: "Nested struct/array/map type changes are not
readable by ClickHouse — mitigation: enforce in the connector contract;
reject at registration, not at read time." That mitigation needs a
concrete policy for what "enforce" means at each of the three schema
boundaries a Bronze table crosses: the source system, Bronze Iceberg
(where dlt today, and Debezium in P5, write), and Silver (ClickHouse
MergeTree, materialized/viewed off Bronze). Each boundary has a genuinely
different capability, measured directly during this phase, not assumed:

**Bronze Iceberg reads additive schema changes with zero code action.**
Verified: adding a nullable column to a live, catalog-registered table via
`pyiceberg`'s `update_schema()` (the same schema-evolution API `dlt`'s
`iceberg` destination uses internally) is immediately visible to
ClickHouse's `DataLakeCatalog` — a `DESCRIBE` against the table picks up
the new column with no `ALTER TABLE`, no cache flush, nothing. A plain
`SELECT new_column, count() ... GROUP BY new_column` correctly returns
`NULL` for every row written before the column existed — exactly Iceberg's
schema-evolution contract (new columns are optional, absent in old data
files, backfilled as null on read). **One caveat, also measured:** an `IS
NULL` predicate on that same retroactively-added column against
pre-existing data files fails with `NOT_FOUND_COLUMN_IN_BLOCK` — a
ClickHouse Iceberg-reader rough edge in null-predicate pushdown against
files whose physical schema predates the column, not a general read
failure. Recorded here as a known gap; a plain `SELECT`/`GROUP BY` on the
column is unaffected.

**Silver (ClickHouse MergeTree) has no equivalent automatic mechanism.**
A MergeTree table's columns are fixed by its `CREATE TABLE` DDL; a new
Bronze column does not appear in a Silver view/materialization until
something re-issues that DDL (`ALTER TABLE ... ADD COLUMN` or
`CREATE OR REPLACE VIEW`, per today's `demo/clickhouse/05_silver.sql`
pattern of `silver.*` passthrough views over `lake.*`).

**`iceberg_engine_ignore_schema_evolution` exists but is inert** —
`system.settings` describes it as "Obsolete setting, does nothing" on
26.3. There is no setting to control or suppress Bronze-side schema
evolution behavior; it is unconditional and automatic, confirming the
measurement above rather than being a configurable policy knob.

## Decision — three-boundary policy

### 1. Source -> Bronze: additive-only, enforced at connector registration, not at read time

A connector (dlt today; Debezium in P5) may propagate to Bronze:

- **New column** (nullable) — always allowed. Iceberg/dlt/pyiceberg all
  support this natively; ClickHouse reads it with no action, per the
  measurement above.
- **Type widening** within the ClickHouse-readable scalar set (e.g.
  `Int32` -> `Int64`, `Float32` -> `Float64`) — allowed, matching Iceberg's
  own schema-evolution rules (Iceberg only permits promotions, never
  narrowing, at the format level — this ADR does not need to invent a
  stricter rule than the format already enforces).
- **Column rename** — allowed; Iceberg resolves by field id, not name, so
  a rename is a metadata-only change that does not touch existing data
  files (consistent with ADR 0004's "Iceberg field ids ... are
  load-bearing for schema evolution").
- **Nested struct/array/map type changes** (R7) — **rejected at connector
  registration**, not discovered later at read time. Concretely: the
  connector-registry contract (owned by ADR 0007, due P5) must validate a
  source schema against the ClickHouse-readable type set *before* a
  connector is allowed to start writing, not after Bronze already has
  data ClickHouse cannot read. This phase does not implement that
  validator (ADR 0007's connector-registry work is P5-scoped), but fixes
  the boundary where it belongs: registration-time, source-side, per R7's
  original mitigation text.
- **Column type narrowing or removal** — rejected outright at the
  connector level. Iceberg does not support narrowing at the format level
  (matching the point above), and column removal changes historical query
  semantics silently; neither is safe to auto-propagate.

### 2. Bronze -> Silver: explicit, not automatic

Because MergeTree has no equivalent of Iceberg's automatic schema
evolution, a Bronze schema change does **not** propagate to Silver until a
human or a pipeline step explicitly re-issues the Silver DDL. This is a
deliberate choice, not a gap:

- Silver is a **curated, conformed** layer (ADR-adjacent language already
  used in `05_silver.sql`'s own header comment: "Lapisan Silver — model
  bersih & terkonform") — it is not supposed to silently inherit every
  upstream shape change. A new Bronze column should not appear in a
  business-facing Silver view until someone decides it belongs there,
  possibly renamed/retyped/derived, which is exactly what
  `silver.dim_customer`-style curated views already do today (selecting
  and deriving specific columns, not `SELECT *`).
- **Passthrough Silver views** (`CREATE OR REPLACE VIEW silver.x AS SELECT
  * FROM lake.x`, per `05_silver.sql`'s existing pattern) DO pick up a new
  Bronze column automatically on next `SELECT *` evaluation, since a
  ClickHouse view is not materialized — this is a side effect of the
  existing `SELECT *` pattern, not a new mechanism this ADR adds, and it
  only applies to the subset of Silver tables that are pure passthroughs.
- **Curated Silver views** (`dim_*`/`fct_*`/`enr_*`) require an explicit
  edit to name the new column, by design — this is the same manual step
  adding any new business-facing field to a conformed model always
  requires, in any warehouse.

### 3. What P4 actually ships against this policy

- `dagster/dispar_orchestrate/dlt_pipeline.py`'s `iceberg_adapter`/dlt
  schema inference already produces additive-only Bronze schemas (dlt's
  own default behavior: new source columns become new nullable Iceberg
  columns; dlt does not narrow or drop columns on schema drift). No code
  change was needed to conform to boundary (1)'s additive rule — it is
  already what the existing P3 ingestion path does.
- No new Rust or Dagster validation code ships in P4 enforcing R7's
  nested-type rejection — that enforcement point is the connector
  registry, which is P5-scoped (ADR 0007). This ADR fixes WHERE that
  validation belongs (registration-time) so ADR 0007 has a settled answer
  to build against, rather than re-deciding it then.
- Silver's manual-propagation behavior needs no new code either — it is
  the natural consequence of MergeTree DDL being explicit, which is
  already true today.

## Consequences

- A future connector-registry validator (ADR 0007) must run schema
  compatibility checks against the ClickHouse-readable type set BEFORE
  accepting a connector definition, per boundary (1) above — this is now
  a settled requirement, not an open question, for whoever implements
  ADR 0007.
- Operators should expect: Bronze silently gains new nullable columns as
  sources evolve (no alert, no gate) — this is intentional per Iceberg's
  own model — but Silver does NOT gain them without an explicit change.
  This asymmetry is the policy, not an oversight, and should be
  communicated to anyone building Silver models against this stack.
- The `IS NULL`-predicate-on-retroactive-column ClickHouse rough edge
  (measured above) is a known, narrow gap: avoid filtering newly-added
  Bronze columns with `IS NULL`/`IS NOT NULL` until upstream fixes it;
  `GROUP BY`/plain `SELECT` on the same column work correctly today.

## Verification

- `pyiceberg`'s `table.update_schema().add_column(...)` against a live
  Lakekeeper-registered table, followed immediately by `DESCRIBE` and
  `SELECT ... GROUP BY` through ClickHouse's `DataLakeCatalog` — no
  `ALTER TABLE`, no restart, no cache action, executed against the
  `p4check` compose stack, see `docs/plans/G3-RESULT.md`'s stack for the
  environment this was run in.
- `SELECT name, default, description FROM system.settings WHERE name =
  'iceberg_engine_ignore_schema_evolution'` confirms there is no
  configurable policy knob overriding this behavior on 26.3.
