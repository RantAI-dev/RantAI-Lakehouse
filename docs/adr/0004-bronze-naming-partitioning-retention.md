# ADR 0004 — Bronze table naming, partitioning, and retention

- **Status:** Accepted
- **Phase:** P1
- **Date:** 2026-08-31

## Context

`lakehouse-iceberg` needs a concrete, documented convention for three
things every Bronze table shares: what it's called, how it's partitioned,
and what happens to old data/snapshots in it. The plan's P1 acceptance
criteria name one piece of this already ("default: ingestion day"
partitioning); this ADR fills in the rest and gives the "day" default a
rationale.

## Decision — naming

- **Namespace:** every Bronze table lives in one flat namespace, `bronze`
  (`bronze::BRONZE_NAMESPACE`), inside the tenant's Lakekeeper warehouse
  (ADR 0003). Not one namespace per source system or per connector — a
  single flat namespace keeps `` `bronze.<table>` `` addressable from
  `ClickHouse` with exactly the two-part backtick naming the plan's P1
  acceptance criteria already specify, and keeps the naming decision this
  ADR owns to one axis (the table name) instead of two.
- **Table name:** the connector/dataset slug, sanitized by
  `bronze::sanitize_table_name` — lowercased, non-`[a-z0-9_]` runs
  collapsed to a single `_`, leading/trailing `_` trimmed. This matches the
  shape `lakehouse_api::tenant::BRONZE_CURATED` slugs already use
  (`wisman-jakarta-per-bulan` → `wisman_jakarta_per_bulan`), so a
  connector-registry slug can be handed to table creation directly, with
  no separate naming pass a future caller could get out of sync with the
  registry.
- **No source-system prefix in the table name** (e.g. not
  `postgres_wisman_jakarta`). The namespace already establishes "this is
  Bronze"; encoding provenance in the name too is redundant with
  `console.pipeline_definition`/connector-registry metadata, which already
  records source system per table. If two different sources ever produce
  identically-slugged datasets, that is a genuine naming collision to
  resolve at the connector-registry level (reject the duplicate slug),
  not something this ADR should paper over by inventing a prefixing
  scheme nothing else in the codebase uses.

## Decision — partitioning

**Default: `day(_ingested_at)`** — a single-field identity-style partition
spec, transform `Day`, source column `_ingested_at`
(`bronze::INGESTED_AT_COLUMN`, field id `bronze::INGESTED_AT_FIELD_ID`,
always the first field in a Bronze schema).

- **Why a dedicated system column, not a source event-time column.**
  Not every connector's source data has a reliable, non-null event-time
  column, and even where one exists its name/type varies per source. A
  system column this crate itself stamps at write time is always present,
  always non-null, and always the same name/type — the partition scheme
  never depends on a source-data contract this crate does not control. A
  connector that DOES have a meaningful source event-time column can add
  it as a domain field and, later, an *additional* partition spec
  (`iceberg-rust` supports partition spec evolution) without touching this
  default.
  This is a deliberate choice already anticipated in the plan document
  itself, which is explicit that G3's small-file measurement is about a
  "synthetic small-file load equivalent to 14 days of CDC" — day-grain
  partitioning is the unit that measurement is already expressed in.
- **Why day, not hour or month.** Day is the grain
  `docs/plans/LAKEHOUSE-FOUNDATION-PLAN.md` names directly ("default:
  ingestion day") and the grain P4's acceptance criteria measure against
  ("14 days of CDC"). Hour-grain would multiply partition count roughly
  24× for the same data volume, working against R2 (small-file
  accumulation — already a **High** severity risk, upgraded further by the
  `CLICKHOUSE-MAINTENANCE-FINDINGS.md` correction that ClickHouS has no
  bin-pack rewrite and `OPTIMIZE ... MANIFEST` does not exist on 26.3).
  Month-grain would under-partition for any connector with real daily
  ingestion volume, defeating partition pruning on the query patterns this
  console already has (date-ranged dashboards). Day is the grain that
  balances both failure modes given what's already measured.
- **Field id 1 is reserved for `_ingested_at`.** `bronze::bronze_schema`
  rejects a caller-supplied domain field that reuses field id 1 — domain
  fields must start at id 2 (`FIRST_DOMAIN_FIELD_ID`). Reserving the low
  id keeps the system column's id stable across every Bronze table
  regardless of how many domain columns a given table has, which matters
  because Iceberg field ids, once assigned, are load-bearing for schema
  evolution (`iceberg-rust`'s schema/partition-spec builders key off
  field id, not name, internally).

## Decision — retention: explicitly deferred

**No retention policy ships in P1b.** Concretely, that means:

- No `expire_snapshots` scheduling, no time-based or count-based snapshot
  retention, no data-file TTL. Every snapshot this crate's `append`
  produces stays reachable indefinitely as of P1b.
- This is a documented gap, not a silent one. Reasons it is deferred
  rather than attempted now:
  1. **`expire_snapshots` is unverified on this ClickHouse version against
     a real catalog-registered Iceberg table** —
     `docs/plans/CLICKHOUSE-MAINTENANCE-FINDINGS.md` confirms the verb
     exists in `ALTER TABLE ... EXECUTE` grammar but explicitly defers
     confirming it *works* to a P4 acceptance item. Writing a retention
     policy against a mechanism not yet proven to work on this stack would
     be exactly the kind of "looks real, is not" gap
     `AI_PROJECT_INSIGHTS.md`'s existing stubs (`connectors::test_connection`,
     the Storage Cold/AI tiers) already warn against creating.
  2. **`iceberg-rust` 0.10.x's `ExpireSnapshotsAction` exists as a
     `Transaction` action** (confirmed by reading `transaction/mod.rs`),
     so a Rust-side retention path is technically available — but wiring
     retention scheduling (when to run it, against which tables, at what
     age threshold) is a P4 concern by the plan's own phase boundary
     ("P4 — Maintenance (G3)"), not P1's. Building it now would be scope
     creep into a phase whose acceptance criteria this task is not
     measuring against.
  3. Retention policy design benefits from the G3 measurement P4 already
     plans (file count / query-planning-time before and after maintenance
     verbs actually run) — deciding a retention window before that data
     exists would be a guess dressed as a decision.
- **What this means operationally today:** a Bronze table created via this
  crate accumulates snapshots without bound until P4 lands a policy. For
  the G1 test and any other P1b-scale usage, this is inconsequential
  (single-digit appends). It becomes a real operational concern only at
  CDC-scale ingestion volume (P5), by which point P4's maintenance chain
  is expected to exist.

## Consequences

- `bronze.rs` owns `BRONZE_NAMESPACE`, `INGESTED_AT_COLUMN`,
  `INGESTED_AT_FIELD_ID`, `FIRST_DOMAIN_FIELD_ID`, `sanitize_table_name`,
  `bronze_schema`, `bronze_namespace`, and `ingestion_day_partition_spec`.
- `catalog::IcebergClient::create_bronze_table` always applies this ADR's
  partition spec; there is no parameter to opt out of it in P1b. A future
  connector needing a different default would be a new decision (and
  likely a new ADR), not a silent per-call override.
- Retention remains explicitly open. The P1b report names this as a gap,
  not a false "done."

## Verification

`cargo test -p lakehouse-iceberg` — `bronze::tests` covers: sanitization
(dashes, mixed case, punctuation runs, all-punctuation and empty-input
rejection), the reserved field-id rejection, that a built schema includes
`_ingested_at` alongside domain fields, that the default partition spec is
exactly one `Day` transform on `_ingested_at`, and that the namespace
resolves to the flat `bronze` string. The G1 test additionally proves the
naming/partitioning convention holds against a real Lakekeeper-registered
table on both the Rust-write and `ClickHouse`-write paths.
