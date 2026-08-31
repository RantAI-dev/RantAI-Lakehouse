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

## Not decided here

When Gold export is built. It is not on the P0–P6 critical path — no gate
depends on it — so it lands after P6 unless a customer requirement pulls it
in earlier.
