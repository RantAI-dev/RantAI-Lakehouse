# ADR 0019: Gold is stored in Iceberg first; ClickHouse holds a fast copy

- Status: **Proposed — awaiting the product owner's signature.** Nothing is built on it until it
  is accepted (`docs/core/specs/data-3.md`, "Architecture record").
- Date: 2026-11-20
- Backlog: `DATA-3` (Feature, Build, P3). **Blocked by `DATA-4`** (Silver publish, not merged)
  and by `DATA-6` (#612, not merged), whose shared PyIceberg helpers this design reuses.
- Supersedes **in part**: ADR 0010, for the Gold tables that choose the new path only (see
  "What this replaces in ADR 0010").
- Feature page: `docs/core/features/gold-in-iceberg-first.md`. Plan:
  `docs/superpowers/plans/2026-11-20-data-3-gold-iceberg-first.md`. Measurements:
  `docs/plans/DATA-3-RESULT.md` (written by the plan's T0 gate before any product code).

## Context

- **Today a Gold table lives in ClickHouse.** An authored pipeline whose target zone is
  `serving` computes its `SELECT` in ClickHouse and replaces `serving.<t>` through a staging
  table and `EXCHANGE TABLES` (`dagster/dispar_orchestrate/authored_factory.py:474`,
  `_write_clickhouse_table`). That MergeTree table is the only copy dashboards, Query Studio,
  alerts and the assistant read.
- **The open copy comes afterwards (ADR 0010, `DATA-1`, `DATA-2`, `DATA-6`).** When publishing
  is switched on, `POST /api/gold/export/{mart}` reads `serving.<t>` and appends it to the
  Iceberg table `gold.<t>` with `iceberg-rust`, stamped with `_exported_at`; the last 7 copies
  are kept; `DATA-6` adds `gold.<t>_latest`. The open copy is always behind, is masked for the
  publisher at copy time, and is a second system to keep in step.
- **The product owner's direction (2026-10-14 and 2026-11-20):** Iceberg becomes the source of
  truth for Gold; ClickHouse stays, as the fast copy dashboards read. Most competitors work this
  way (Snowflake managed Iceberg tables, StarRocks over Iceberg with materialized views,
  Databricks with the engine as a cache).
- **What the code allows, measured or read:**
  - `iceberg-rust` 0.10.1 can only append; it cannot replace a table's contents in one commit
    (`DATA-6` plan, mismatch M1: `TransactionAction` and `TableCommit`'s builder are
    `pub(crate)`).
  - PyIceberg 0.12.0 (pinned, `dagster/pyproject.toml:20`) stages a delete and an append in one
    `Transaction` and commits them as **one** catalog commit (`Transaction.overwrite`). `DATA-6`
    relies on this and measures it in its own T0.
  - ClickHouse 26.8 still cannot `CREATE TABLE` inside a `DataLakeCatalog` database
    (ADR 0010, correction after re-measuring on 26.8); it can append to a table it did not
    create, but has no replace.
  - ClickHouse reads catalog-registered Iceberg tables (every Bronze read does, through
    `icecat_*` databases) and keeps a MergeTree table fast for dashboards (BI-4's result cache
    watches `system.parts`, which a MergeTree copy has and an Iceberg table does not).

## Decision

1. **Per-table choice, made when the Gold table is created.** An authored pipeline whose target
   zone is the Gold zone (`Config::gold_source_schema`, default `serving`) has a storage
   setting: `clickhouse` (today's path, the default) or `iceberg` (this ADR). It is fixed at
   creation. Existing tables keep today's path. Switching an existing table is not part of this
   ADR's first build (see Consequences).
2. **Iceberg is the source of truth.** For an `iceberg` table, `gold.<t>` in the catalog
   tenant's warehouse (namespace `gold`, `lakehouse_iceberg::gold::GOLD_NAMESPACE`) holds the
   table's current rows and nothing else: no `_exported_at`, no kept copies. Its history is
   Iceberg snapshots. It carries the table property `rantai.gold-storage = iceberg-first`, so
   no other writer (the `DATA-1` publish, the `DATA-6` latest copy) mistakes it for a published
   table, and it never takes over a `gold.<t>` that exists without that property.
3. **What writes Iceberg: the Dagster code location, with PyIceberg, in one commit per run.**
   ClickHouse stays the only compute engine. The run asks the API for the compiled statement as
   today (`_compiled_sql`, rewritten as the pipeline's author), streams its result out of
   ClickHouse as Arrow, and replaces `gold.<t>` with a PyIceberg overwrite: one catalog commit,
   so an outside reader sees the old table or the new one, never a mix. The first run creates
   the table and writes its first rows in the same commit, so the table never exists empty. The
   writer is the existing `gold-export` catalog principal (`load_gold_catalog`,
   `gold_export.py:615`); no new principal or grant. A row limit applies before the commit (the
   same `GOLD_EXPORT_MAX_ROWS`, 5,000,000 by default): over it, nothing is committed.
4. **How the copy is kept current.** Right after the commit, the same run refreshes
   `serving.<t>` from Iceberg, **pinned to the snapshot it just committed**, through a staging
   table and `EXCHANGE TABLES` (the swap today's path already uses). The copy table keeps the
   column types of the pipeline's own `SELECT`, not the types Iceberg reads back as, so
   dashboards see the same table they see today. Every refresh records, in the ClickHouse table
   `console.gold_copy_run`, the snapshot id copied, the commit time, the time the swap finished
   and the lag between them. A reconcile sensor compares, per `iceberg` table, the catalog's
   current snapshot with the last copied one and starts a refresh-only run when they differ; it
   also rebuilds a copy that was lost.
5. **What readers use.**
   - Dashboards, Query Studio, alerts, reports and the assistant read `serving.<t>`, the copy,
     unchanged: same name, same masking and row filters on every read, same result cache.
   - Outside tools read `gold.<t>` through the API's door (ADR 0016), with no publish step.
   - The `DATA-1` publish does not apply: publishing an `iceberg` table is refused with a fixed
     sentence ("This table is stored in open format; there is nothing to publish."), and it is
     never listed for the nightly publish.
6. **Masking for outside readers: refused, not approximated** *(owner decision, recommended)*.
   The Iceberg table holds the unmasked truth (the copy needs it to mask per reader). A file
   reader cannot be masked. So the door refuses to serve an `iceberg` table that any ready
   masking or row-filter policy names, with a fixed sentence, checked on every table load and
   every credentials request. Dashboards keep masking on every read of the copy. An outside read
   of a masked table needs a masked published table, which is a follow-up, not this ADR.
7. **Failure behaviour (principles 2–4).**
   - *Iceberg commit fails, or the catalog or object store is down:* the run fails with a fixed
     reason and changes nothing. The copy still equals the last committed snapshot, so
     dashboards keep showing the last good data. The table page says the open table could not be
     reached and that freshness is not measured; it never says "current".
   - *Commit succeeded, copy refresh failed:* the run fails with reason `copy_refresh_failed`.
     Iceberg is ahead; dashboards show the previous copy, whole, never half-refreshed (the swap
     is atomic). The page shows "Copy behind since <commit time>". The reconcile sensor retries.
   - *ClickHouse down:* dashboards are down, as today. Iceberg is intact and outside reads
     continue. When ClickHouse returns, the sensor refreshes the copy.
   - *Copy older than the lag target:* shown as behind, with its age. Dashboards are not
     blocked: a stale copy is labelled, not hidden (the owner may choose otherwise; feature page
     decision 9).
   - Upstream error text never reaches a response or the record: fixed reasons only, the detail
     goes to the run log.
8. **Tenants.** The table lands in the catalog tenant's warehouse (ADR 0003, ADR 0018). With
   more than one tenant, only a pipeline of the catalog tenant may choose `iceberg`; others are
   refused with a fixed sentence. With one tenant nothing is checked.
9. **Snapshots kept.** After each successful run the `iceberg` table keeps its newest 7
   snapshots (`GOLD_KEEP_COPIES`, the `DATA-2` number) and frees older files with the orphan
   removal `DATA-2` and `DATA-6` already use, honouring `MAINTENANCE_PAUSED`.

## What this replaces in ADR 0010

ADR 0010's direction is **superseded in part, for Gold tables whose storage is `iceberg`**:

- "Read the Gold mart from ClickHouse MergeTree … then append through Lakekeeper with
  `iceberg-rust`" no longer describes these tables: they are written to Iceberg first, by
  PyIceberg, and ClickHouse is filled from Iceberg.
- "Each export run appends the mart's current rows as a new snapshot" and `_exported_at` do not
  apply: the table is replaced in one commit and holds one copy.
- The trigger (`gold_export_job`, `POST /api/gold/export/{mart}`) does not run for them.

Unchanged: ADR 0010 remains the record for tables on today's path and for Silver (`DATA-4`
keeps Silver on it, `specs/data-3.md` "Not included"). Its finding that ClickHouse cannot
create catalog tables still holds, and is one reason PyIceberg writes here. On acceptance the
planner changes ADR 0010's status line to "Accepted; superseded in part by ADR 0019 for Gold
tables stored in open format first".

## Consequences

- **Two writes per run instead of one.** Each run writes Iceberg, then the copy. The time
  between them is the copy lag, the number this feature promises; it is measured, not assumed.
- **A new customer limit.** A dashboard can be up to one copy refresh behind the open table.
- **Python writes Gold Iceberg tables.** Until now only Rust wrote `gold`; `DATA-2` and `DATA-6`
  already write it from Python for trimming and the latest copy. The type rules must be the
  same in both languages: one fixture holds the ClickHouse-to-Iceberg type mapping
  (`gold_export.rs:282`, `ch_type_to_primitive`) and both test suites read it.
- **Memory.** The run holds the result as Arrow before the commit. The T0 gate measures peak
  memory at 1,000,000 and 5,000,000 rows; if 5,000,000 does not fit, the plan's streamed
  variant (delete and batched appends in one transaction) is used, and if that does not fit
  either, the row limit for this path is lowered only with the owner's signature.
- **A copy can be rebuilt.** A lost or damaged ClickHouse copy is rebuilt from Iceberg by the
  reconcile sensor; today a lost Gold table must be recomputed from Silver.
- **Switching an existing table** is a follow-up: its published `gold.<t>` (with
  `_exported_at` and kept copies) already holds the name, and outside tools depend on its shape.
- **Not decided here:** Silver on this path; moving existing tables automatically; masked
  outside reads; incremental (merge) writes instead of a full replace each run.

## Alternatives considered

1. **Keep ClickHouse first, with export (today, ADR 0010).** Rejected as the direction: the open
   copy is always behind, holds many copies an outside reader must filter (`DATA-6` exists to
   patch that), and is a second system to keep in step. It stays for existing tables.
2. **ClickHouse reads Iceberg directly, with no copy.** Rejected for dashboards: every tile would
   read Parquet from object storage through the catalog, with no MergeTree indexes and no
   `system.parts` for BI-4's result cache, so remembered results could not tell when the data
   changed. The spec requires dashboards no slower than today. It remains the fallback the
   reconcile sensor could use while a copy is being rebuilt, but it is not built here.
3. **Trino over Iceberg for BI.** Rejected: it adds a second query engine for dashboards
   ("ClickHouse only for compute", ADR 0010), the BI layer, masking rewriter and result cache
   are written for ClickHouse SQL, and the stack's Trino serves other routes only.
4. **ClickHouse writes Iceberg (`INSERT INTO` a catalog table).** Rejected: it cannot create
   catalog tables (ADR 0010) and has no replace, so each run would append.
5. **Rust writes Iceberg (`iceberg-rust`).** Rejected for now: append only (`DATA-6` M1). Revisit
   when `iceberg-rust` offers an overwrite.

## Measurements needed, and where they are recorded

All in `docs/plans/DATA-3-RESULT.md`, raw output quoted, machine type, data size and versions
named, produced by the plan's T0 gate **before any product code**. A number not measured is
written "not measured", with the reason, and blocks the build.

| # | Measurement | Pass |
| --- | --- | --- |
| M1 | PyIceberg overwrite at 1,000,000 and 5,000,000 rows: snapshots added, operation, wall time, peak memory | One snapshot per overwrite; memory within the run's container |
| M2 | ClickHouse reads a named snapshot of a catalog table (pinned read) | Rows of that snapshot only, or the plan's fallback proven |
| M3 | Copy lag, commit to swap, 20 runs at the reference size | p95 within 60 s *(spec target, signed 2026-10-14)* |
| M4 | Type and value round trip for every type the mapping accepts | Copy column types equal the `SELECT`'s; content checksums equal |
| M5 | Dashboard p95, today's table vs the copy, same data, results not remembered | Copy within 10 % of the baseline *(spec target, signed 2026-10-14)* |
| M6 | Streaming ClickHouse Arrow output into PyIceberg | Peak memory recorded for both variants |
