# G1 result — measured, reproduced independently

Gate G1 has two halves. **(a) passes. (b) fails on a ClickHouse defect.**

All findings below were reproduced from scratch against a fresh
`docker compose` stack (project `g1verify`, volumes destroyed afterward),
independently of the implementing agent's run.

Versions: ClickHouse `26.3.26.3` (build id `6EFC606D87BD3BB970628AD59E4DE56B7118904B`,
git hash `4545d1a94b01a8c0ad70326c028ea2998617b960`), Lakekeeper `0.13.3`,
RustFS `1.0.0-rc.4`, `iceberg` / `iceberg-catalog-rest` `0.10.1`,
`object_store` `0.14.1`.

## What works

| Path | Result |
| --- | --- |
| Rust → Lakekeeper: create Bronze table, format-version 2 | Works |
| Rust → Lakekeeper: append data files (vended credentials) | Works |
| ClickHouse: `CREATE DATABASE … ENGINE = DataLakeCatalog` over Lakekeeper | Works |
| ClickHouse: `SHOW TABLES` through the catalog | Works |
| ClickHouse: `DESCRIBE` through the catalog | Works — returns `_ingested_at DateTime64(6)`, `id Int64`, `label String` |
| ClickHouse: `SELECT` through the catalog | Works |

**The primary Bronze path is sound.** Rust (and later Debezium/dlt) writes
Iceberg through Lakekeeper; ClickHouse reads it. That is G1 (a), and it is
the path the whole medallion design depends on.

## What fails: ClickHouse writing Iceberg through the catalog

### 1. `CREATE TABLE` silently falls back to MergeTree

```sql
SET allow_database_iceberg=1;
CREATE TABLE icecat.`bronze.probe` (id Int64, label String);
-- Code: 79. DB::Exception: MergeTree storages require data path. (INCORRECT_FILE_NAME)
```

Lakekeeper is never contacted. ClickHouse does not route `CREATE TABLE`
inside a `DataLakeCatalog` database to the catalog; it applies the default
engine and fails on the missing data path. Explicit `ENGINE = Iceberg`
still demands a literal S3 URL — i.e. path-based only, which the locked
decisions forbid.

### 2. `INSERT` into a **partitioned** catalog table segfaults the server

```
Received signal 11 (Segmentation fault)
Address: 0x3f5b5b60. Access: read. Address not mapped to object.
  3. DB::ColumnString::get(unsigned long, DB::Field&) const
  4. DB::ChunkPartitioner::partitionChunk(DB::Chunk const&)
  5. DB::IcebergStorageSink::consume(DB::Chunk&)
query: INSERT INTO icecat.`bronze.g1_rust_write` (id, label) FORMAT Values
```

The server process dies; the container restarts (`RestartCount` 0 → 1) and
the client sees `ATTEMPT_TO_READ_AFTER_EOF`. The target table is
`day(_ingested_at)`-partitioned per ADR 0004, and the INSERT supplied only
the non-partition columns.

### 3. `INSERT` into an **unpartitioned** catalog table fails cleanly

Creating an unpartitioned format-version-2 table directly via the Iceberg
REST API and inserting into it does **not** crash — `RestartCount` stays
put — but still fails:

```
Code: 1000. DB::Exception: Exception: Can not extract empty value. (POCO_EXCEPTION)
```

So the write path fails in both shapes; partitioning determines whether the
failure is a crash or an exception. **The segfault is a genuine upstream
ClickHouse bug and is worth reporting to them** — it is a remote-triggerable
null-ish deref in the Iceberg sink.

## Impact on locked decisions

The affected decision is: *"Gold may be exported to Iceberg through
Lakekeeper when external engines/agents need it."* On 26.3 that export
cannot be done by ClickHouse.

The brief's §2 also states "Iceberg writes production-ready since 26.2
(INSERT)". Measured against 26.3, that is not the case for
catalog-registered tables.

**Nothing else in the design is blocked.** Bronze ingestion (Debezium/dlt →
Iceberg → ClickHouse reads) does not depend on ClickHouse writing Iceberg.

### Recommended resolution

Export Gold to Iceberg **from Rust** via `lakehouse-iceberg`, which already
appends successfully. Read Gold from ClickHouse MergeTree with the existing
`lakehouse-clickhouse` client, append through Lakekeeper with
`iceberg-rust`. This does not violate "ClickHouse only for compute" — it
adds no engine, and reuses a path proven in G1 (a). It is also strictly
safer than driving writes through an engine that segfaults on them.

Deferring Gold export entirely until ClickHouse fixes the write path is the
alternative; it costs the "other engines/agents can read Gold" capability.

## R1 is NOT retired

Lakekeeper ran with `"authz-backend":"allow-all"` (confirmed via
`/management/v1/info`). R1 is specifically "ClickHouse catalog-registered
writes fail against Lakekeeper's authz enforcement on metadata updates" —
untestable here, because those writes never reach Lakekeeper at all.
Enabling authz means standing up OpenFGA or the OPA bridge and defining an
authorization model: a distinct task, not a config flag.

## Outstanding defects in our own work

1. **The G1 test is not CI-runnable.** It executes on the host, but
   Lakekeeper vends `http://rustfs:9000/` — a Docker-internal name the host
   cannot resolve. It was made to pass by hand-editing `/etc/hosts`. The
   working rule is that integration tests must run under `docker compose` in
   CI, so the test must move inside the compose network.
2. **`cargo deny check licenses` fails.** `tiny-keccak v2.0.2` is `CC0-1.0`,
   reached via `arrow-array → ahash → const-random → const-random-macro`.
   Not fixable without upstream changes. CC0-1.0 is public-domain-equivalent
   and widely treated as permissive; adding it to the allowlist is a
   deliberate policy decision, not a workaround, and is left for the owner.
