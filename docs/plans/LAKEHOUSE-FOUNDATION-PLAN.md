# Lakehouse Foundation Plan

Takes RantAI Lakehouse from a console over Postgres + ClickHouse + Dagster to
a working lakehouse: object storage, an Iceberg REST catalog, Bronze
ingestion, ClickHouse serving, and in-engine maintenance.

Locked decisions live in the task brief and are not restated here. Open
questions are ADRs in [`docs/adr/`](../adr/). Phase gates are in §3.

## 1. Current state

Today the product is a **ClickHouse warehouse with medallion-shaped database
names**, not a lakehouse. `bronze`, `silver`, and `gold` are ClickHouse
databases (`demo/clickhouse/02_bronze.sql`, `05_silver.sql`); data lives in
MergeTree, coupled to the only engine that can read it. There is no object
storage, no open table format, and no catalog over open tables. This is why
`routes/storage.rs` hardcodes the Cold and AI tiers to zero — there is
nowhere cold to put anything.

### Rust crates

| Crate | State | Note |
| --- | --- | --- |
| `lakehouse-core` | Real | `ApiError`, `Ident`/`SqlLiteral` injection boundary |
| `lakehouse-clickhouse` | Real | HTTP client, single `lib.rs` |
| `lakehouse-store` | Real | 9 Postgres domains; `connect_lazy` is non-fatal |
| `lakehouse-auth` | Real | 4 authenticators, `Principal` seam |
| `lakehouse-bi` | Real | Chart specs + ClickHouse board store |
| `lakehouse-dagster` | Real | GraphQL: launch, terminate, re-execute, schedules |
| `lakehouse-llm` / `-embed` / `-notify` / `-alerts` | Real | External-system clients |
| `lakehouse-api` | Real | 12.9k LOC, 19 route modules — largest crate, watch growth |
| `lakehouse-test-support` | Real | Postgres integration harness |

**No crate does object I/O, Iceberg, or catalog work.** `rust/Cargo.toml`
declares exactly one database driver: `sqlx` with the `postgres` feature. No
`object_store`, no `iceberg`, no S3 client of any kind.

### Frontend service domains

Ten of twelve are real (`src/services/index.ts`). Still mocked: `streaming`
(no streaming engine exists anywhere) and `knowledge.search` (no vector
store). Both are deliberate and documented.

### Stubbed — looks real, is not

- **`connectors`** is a CRUD registry. `connectors::test_connection`
  (`lakehouse-store/src/connectors.rs:444`) opens no socket: it bumps
  `last_test_at` and returns stored health with hardcoded latency
  (84ms / 2400ms). `secretRef` is never resolved to a credential.
- **`0014_seed_connectors.sql`** seeds 28 connectors — Kafka, MQTT, MongoDB,
  Oracle, SAP, SFTP, Iceberg, S3 — **none of which can dial**. Treat this as
  demo fixture, not a requirements list; it shrinks to match reality in P6.
- **Storage Cold/AI tiers** always report zero.

### Doc accuracy

`docs/ARCHITECTURE.md` and `docs/OPERATIONS.md` are accurate and high
quality. Stale and corrected as they are touched: root
`FEATURE_COVERAGE.md` ("All product data paths are mock adapters" — false
for 10 domains), `AI_PROJECT_INSIGHTS.md` ("frontend preview only"),
README's MSRV note (says 1.85; `Cargo.toml` says 1.88).

## 2. Compose service inventory

| Service | Status | Phase |
| --- | --- | --- |
| `postgres` (16) | Exists | — |
| `clickhouse` (24.8) | Exists | Needs ≥26.2 for Iceberg writes — bump in P1 |
| `lakehouse-api` | Exists | Built from `Dockerfile.api`; converge in P0 |
| `rustfs` | New | P1 — default S3 implementation |
| `lakekeeper` | New | P1 — Iceberg REST catalog |
| `lakekeeper-migrate` | New | P1 — one-shot schema init |
| `seaweedfs` | New | P2 — matrix profile only |
| `dagster` (webserver + daemon + code location) | New | P3 |
| `debezium-server` | New | P5 |
| `trino` | Conditional | P4 only if G3 fails |

Every new service gets a healthcheck, an env var in `config.rs` and the
README table, an `OPERATIONS.md` entry, and a failure-mode note.

## 3. Phases, tasks, acceptance

**P0 — Unblock.** `main.rs:17` declares `mod tenant;` but
`src/tenant.rs` is uncommitted, so a clean clone does not build. The module
is used by 5 route modules (`ops`, `dashboard`, `governance`, `catalog`,
`pipelines`), so it is committed, not removed. Converge `rust/Dockerfile`
and `rust/Dockerfile.api` per `OPERATIONS.md`, point compose at the survivor.
*Accept:* clean clone + `docker compose up --build` green; CI green except
the known gitleaks history job.

**P1 — Floor (G1).** RustFS + Lakekeeper in compose. New crate
`lakehouse-iceberg`: `object_store` client, Lakekeeper REST client, Bronze
table create + append via `iceberg-rust`. ClickHouse `DataLakeCatalog`
database over Lakekeeper (`allow_database_iceberg = 1`; backtick
`` `db.table` `` naming). *Accept:* integration test under compose, using
**vended credentials** against RustFS — (a) Rust appends, ClickHouse SELECTs
the rows; (b) ClickHouse `CREATE TABLE` + `INSERT` through the catalog
(`allow_insert_into_iceberg = 1`), read back via `iceberg-rust`.
**Stop condition:** if (b) fails specifically against Lakekeeper, halt and
report with logs and versions. This is the one finding that changes the
design.

**P2 — Storage boundary (G2).** Same suite against SeaweedFS by env/config
change only — no code diff. Write `docs/STORAGE-COMPATIBILITY.md` (v4
signing, ListObjectsV2, multipart, range GET, conditional writes,
STS-or-remote-signing, lifecycle). *Accept:* matrix green on both stores.

**P3 — Batch ingest (G3a).** Dagster code location; dlt `sql_database`
reads a real Postgres table into Bronze through Lakekeeper. *Accept:*
end-to-end test; table visible in console catalog; lineage recorded.

**P4 — Maintenance (G3).** Dagster job, per Bronze table, in order:
`expire_snapshots` → `remove_orphan_files` → `OPTIMIZE` →
`OPTIMIZE … MANIFEST`, each behind its experimental setting. `dry_run`
metrics surfaced in console. *Accept (G3):* synthetic small-file load
equivalent to 14 days of CDC; measure file count and query planning time per
partition before/after. **ClickHouse has no bin-pack rewrite of small data
files** — if planning degrades beyond 2×, add Trino-as-cron for `optimize`
on Bronze only, and record the ADR. Otherwise Trino stays out.

**P5 — CDC (G4).** Debezium Server + debezium-server-iceberg from Postgres
logical replication into Bronze; upsert mode; snapshot then stream; schema
evolution constrained to the ClickHouse-readable set. Connector registry
generates the config. Slot lag and WAL retention exposed as metrics/alerts.
*Accept:* insert/update/delete on source visible in ClickHouse within the
agreed latency; replication slot cleanup verified on connector delete.

**P6 — Console.** Storage/Catalog/Ingestion/Maintenance surfaces in the
existing UI domains; replace mocks the new layer makes real; shrink the
fictional connector seed; update `FEATURE_COVERAGE.md`, `ARCHITECTURE.md`,
`OPERATIONS.md`, README.

## 4. ADRs

| ADR | Subject | Due |
| --- | --- | --- |
| 0001 | Dockerfile convergence and the `tenant` module | P0 |
| 0002 | `secretRef` resolution: env → file → provider trait | P1 |
| 0003 | Tenant → Lakekeeper project/warehouse mapping | P1 |
| 0004 | Bronze naming, partitioning (default: ingestion day), retention | P1 |
| 0005 | Dagster code-location ownership and compose packaging | P3 |
| 0006 | Schema-evolution propagation: source → Bronze → Silver | P4 |
| 0007 | Connector registry → Debezium/dlt config generation | P5 |
| 0008 | Initial snapshot/backfill for large tables | P5 |
| 0009 | Small-file compaction (outcome of G3) | P4 |
| 0010 | Gold export to Iceberg happens in Rust, not ClickHouse | **Done** (P1) |

ADR 0002 is load-bearing: Lakekeeper storage secrets, Debezium source
credentials, and dlt all block on it.

## 5. Risk register

| # | Risk | Severity | Mitigation |
| --- | --- | --- | --- |
| R1 | ClickHouse catalog-registered writes fail against Lakekeeper's authz on metadata updates | **Critical** | **Measured in P1: moot as framed.** ClickHouse cannot write through the catalog at all (see [G1-RESULT.md](G1-RESULT.md)), so the writes never reach Lakekeeper's authz. Gold export moves to Rust ([ADR 0010](../adr/0010-gold-export-to-iceberg-from-rust.md)). Lakekeeper ran `allow-all`; standing up OpenFGA is **deferred to P5**, when CDC and dlt are writing through the catalog under load and authz on metadata updates actually bites |
| R2 | No bin-pack rewrite in ClickHouse; Bronze accumulates small files from CDC. **Raised in P1a:** `OPTIMIZE … MANIFEST` is also a syntax error on 26.3, so two assumed mitigations are unavailable — see [CLICKHOUSE-MAINTENANCE-FINDINGS.md](CLICKHOUSE-MAINTENANCE-FINDINGS.md) | **High** | G3 measures it; Trino-as-cron is the pre-authorized escape hatch, and is now more likely to be needed |
| R3 | RustFS is young; storage is the durability layer | **High** | S3 API is the boundary; G2 proves the swap; customer S3 is first-class |
| R4 | `debezium-server-iceberg` is community-maintained (Memiiso), not Debezium-official | Medium | Pin version; isolate behind the connector-registry config seam so it is replaceable |
| R5 | Debezium replication slot pins WAL and fills a customer's primary disk | **High** | Slot lag + WAL retention as first-class alerts (P5); verified slot cleanup on delete |
| R6 | `iceberg-rust` 0.10.x is pre-1.0; API churn expected | Medium | Pin exact minor; confine to `lakehouse-iceberg` so upgrades touch one crate |
| R7 | Nested struct/array/map type changes are not readable by ClickHouse | Medium | Enforce in the connector contract; reject at registration, not at read time |
| R8 | New services inherit "Postgres-down is quiet" — failures surface only as request-time 503s | Medium | Healthcheck per service; revisit `GET /health/ready` (proposed in `OPERATIONS.md`) |
| R9 | Six new components for a stack with zero object storage today | Medium | Strict gate order; no phase starts before the prior gate passes |
| R11 | **Added in P5. Silent wrong answers, not errors — the most dangerous class in this build.** On ClickHouse 26.3, a bare `count()` / `count(*)` / `count(<col>)` against an Iceberg table with merge-on-read equality deletes (i.e. any CDC-fed Bronze table) takes a metadata-only fast path and **overcounts**, returning every physical row Debezium ever wrote including superseded ones — measured 6 where the correct answer is 4. Adding *any* `WHERE` or `GROUP BY` forces the row-scan path and is correct. Audited P5: no existing product code is affected, because the console reads the `bronze_meta.*` registry (MergeTree), not the Iceberg tables. **P6 is exactly when this bites** — surfacing Bronze in the console is the first time product code would count an Iceberg table | **High** | Never emit an unqualified `count()` against a Bronze Iceberg table. Use `count() … WHERE <always-true>` or `GROUP BY`. See [P5-RESULT.md](P5-RESULT.md); enforced today in `ops/g4/g4_test.py` and `bronze_catalog.py`. A lint or a shared helper in `lakehouse-clickhouse` would be better than convention |
| R10 | **Added in P3.** The `bronze_meta.*` registry schema is now defined in two places — `demo/clickhouse/04_registry.sql` and `dagster/dispar_orchestrate/bronze_catalog.py`'s `CREATE TABLE IF NOT EXISTS`. Verified byte-identical today (same columns, types, `ReplacingMergeTree`, `ORDER BY`), but nothing enforces that. If one drifts, `IF NOT EXISTS` silently keeps the stale table and the console reads wrong data | Medium | The Dagster-side DDL exists because a bare compose stack never applies `demo/clickhouse/*.sql`. Fix properly by giving the registry schema one owner — either ship it as a migration the compose stack applies, or have the Dagster path assert the schema instead of creating it |

R1 and R2 are the two that can force a redesign. Both are measured before
anything is built on top of them.
