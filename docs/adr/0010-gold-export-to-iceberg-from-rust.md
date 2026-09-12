# ADR 0010 — Gold export to Iceberg happens in Rust, not ClickHouse

- **Status:** Accepted
- **Phase:** P1 (decision), implementation deferred
- **Date:** 2026-09-01
- **Supersedes:** the "Gold exported to Iceberg through the catalog" half of
  the Compute locked decision

## Context

The locked decisions state: *"Compute: ClickHouse only. … Gold exported to
Iceberg through the catalog when other engines or agents need it."*

Gate G1 measured this and it does not work on ClickHouse 26.3. See
[`docs/plans/G1-RESULT.md`](../plans/G1-RESULT.md) for the full reproduction.
In summary:

- `CREATE TABLE` inside a `DataLakeCatalog` database never contacts
  Lakekeeper — ClickHouse falls back to MergeTree and fails with `Code: 79`.
- `INSERT` into a partitioned catalog-registered table **segfaults the
  server** (signal 11 in `ChunkPartitioner::partitionChunk` via
  `IcebergStorageSink::consume`).
- `INSERT` into an unpartitioned catalog-registered table fails cleanly with
  `Code: 1000 … Can not extract empty value`.

The brief's §2 held that Iceberg INSERT has been production-ready since
26.2. That is not true for catalog-registered tables.

Only the Gold-export capability is affected. Bronze ingestion — writer →
Iceberg via Lakekeeper, ClickHouse reads — is unaffected and passed G1 (a).

## Decision

**Export Gold to Iceberg from Rust**, using the `lakehouse-iceberg` crate:
read the Gold mart from ClickHouse MergeTree with the existing
`lakehouse-clickhouse` client, then append through Lakekeeper with
`iceberg-rust`.

This is the same write path G1 (a) already proved works end to end with
vended credentials.

## Consequences

- **"ClickHouse only for compute" still holds.** No engine is added. Rust is
  the application backend, not a query engine; it is already the process
  that owns catalog operations per ADR 0003.
- **Strictly safer than the original plan.** The alternative drives writes
  through a code path that segfaults the server — a remote-triggerable
  crash. Routing around it is not a workaround, it is the correct call.
- Gold export is bounded by `iceberg-rust` 0.10.x's capabilities: **appends
  only**, no row-level update or delete. For an export of an aggregate mart
  this is sufficient — the natural pattern is write a new snapshot, not
  mutate rows in place.
- If ClickHouse fixes its Iceberg write path, this decision can be revisited,
  but there is no reason to prefer the engine-side write once a working
  Rust-side one exists.
- **The segfault should be reported upstream to ClickHouse.** It is
  reproducible from a clean stack with the versions recorded in
  `G1-RESULT.md`.

## Correction after re-measuring on 26.8 — decision re-based, not reverted

This ADR's original premise was that ClickHouse **crashed the server** on
`INSERT` into a partitioned catalog-registered Iceberg table. Re-measured on
`26.8.2.7`
([`docs/plans/CLICKHOUSE-26.8-REMEASUREMENT.md`](../plans/CLICKHOUSE-26.8-REMEASUREMENT.md)):

- **The segfault is fixed.** `INSERT` into a partitioned catalog table
  succeeds and the rows read back.
- `INSERT` into an unpartitioned catalog table also works (was `Code 1000`).

So the crash-based justification is gone, and it would be wrong to keep
citing it.

**The decision still stands on a narrower basis.** `CREATE TABLE` inside a
`DataLakeCatalog` database is **unchanged on 26.8** — it still falls back to
MergeTree and fails `Code 79`, and the catalog is never contacted (verified
via the catalog's own REST API: the namespace is not created). ClickHouse can
now *append to* a catalog-registered table it did not create, but still
cannot *create* one.

Gold export needs both. The Rust path does both, is proven end to end by
G1(a), and does not depend on an experimental setting. Restating the premise
honestly:

> Gold export happens in Rust because ClickHouse cannot create
> catalog-registered Iceberg tables — not because it crashes on insert, which
> was true on 26.3 and is not on 26.8.

**Consequence worth naming:** the "ClickHouse only for compute" constraint is
now *less* strained than when this ADR was written. If a future version fixes
catalog `CREATE TABLE`, the whole export could move engine-side and this ADR
should be revisited rather than assumed permanent.

## Not decided here (as of the original decision — now built; see below)

When Gold export is built. It is not on the P0–P6 critical path — no gate
depends on it — so it lands after P6 unless a customer requirement pulls it
in earlier.

## Update — built

Gold export is implemented, in `lakehouse-iceberg` + `lakehouse-api`:

- **What it does.** `lakehouse-api::gold_export::export_mart` reads a Gold
  mart from `ClickHouse` (`SELECT * FROM {schema}.{mart} FORMAT JSON`),
  maps each `ClickHouse` column type to an Iceberg primitive (strings,
  integers, floats, booleans, dates, timestamps — an unsupported type,
  e.g. `Array`/`Map`/`Decimal`, fails the export loudly rather than
  silently coercing it), builds an Arrow `RecordBatch`, and appends it as
  one new Iceberg snapshot via `lakehouse_iceberg::IcebergClient` — the
  same `iceberg-rust` + `iceberg-catalog-rest` + vended-credentials path
  G1(a) proved.
- **Namespace: `gold`, not `bronze`.** A new, separate, flat Iceberg
  namespace (`lakehouse_iceberg::gold::GOLD_NAMESPACE`), mirroring ADR
  0004's Bronze conventions rather than reusing them: every exported table
  carries a system column (`_exported_at`, mirroring Bronze's
  `_ingested_at`) and is partitioned `day(_exported_at)`, always
  format-version 2. Kept out of `bronze` because Gold is a derived,
  exported artifact, not raw ingested data — mixing the two into one
  namespace would make "everything under `bronze.*` is raw ingest" no
  longer true.
- **Append-only, on purpose, with a named consequence.** `iceberg-rust`
  0.10.x still has no `UPDATE`/`DELETE` (unchanged since this ADR's
  original decision). Each export run appends the mart's current rows as
  a new snapshot rather than replacing the table in place — the
  "write a new snapshot, not mutate rows" pattern this ADR called for.
  The named consequence: re-running the export against an unchanged mart
  grows the Iceberg table's cumulative row count (both appends stay
  visible). `_exported_at` lets a consumer filter to the latest run;
  collapsing history back down needs `expire_snapshots`/compaction against
  the `gold` namespace too, which this change does not add (P4's
  `maintenance.py` still runs against `bronze` only) — a documented
  follow-up, not a silent gap.
- **Trigger.** `dagster/dispar_orchestrate/gold_export.py`'s
  `gold_export_job`, scheduled daily at 04:00 (`gold_export_schedule`),
  calling `POST /api/gold/export/{mart}` over HTTP for every mart named in
  `GOLD_EXPORT_MARTS` — Dagster has no `iceberg-rust` binding of its own,
  so it is a caller of the Rust route, not a reimplementation. The route
  is also callable directly (`lakehouse-api::routes::gold`), and
  `GET /api/gold/export/{mart}` reads the table back through `iceberg-rust`
  independent of `ClickHouse`, for verification.
- **Authorization (ADR 0011).** A new principal, `gold-export`
  (`ops/oidc-mock`'s `PRINCIPALS`), granted `create`, `modify`, `select` on
  the `LAKEKEEPER_WAREHOUSE` warehouse by `lakekeeper-authz-init` — the
  same relation set as `rust-iceberg`, kept as its own principal (not a
  reuse of `rust-iceberg`) so it is independently auditable/revocable, per
  the same reasoning ADR 0011 gives for keeping `rust-iceberg`/`debezium`/
  `dlt` as three principals despite an identical relation set. Its
  pre-minted token is read from `/tokens/gold-export.jwt` by
  `lakehouse-api` at export-request time (see
  `Config::lakekeeper_gold_export_token_file`'s doc comment for why this
  is a file path, not a `secretRef` — the same "no static value exists
  ahead of compose bring-up" reasoning every other writer's token in this
  stack already follows).
- **Acceptance test.** `ops/gold_export/gold_export_test.py`
  (`gold-export-test-runner` in `docker-compose.yml`, `test` profile):
  seeds a real `serving.*` Gold mart in `ClickHouse`, triggers the export,
  confirms format-version 2 straight from Lakekeeper's own REST metadata
  (independent of what the Rust route claims), and reads the table back
  through `iceberg-rust` to confirm the row count matches what was seeded.
