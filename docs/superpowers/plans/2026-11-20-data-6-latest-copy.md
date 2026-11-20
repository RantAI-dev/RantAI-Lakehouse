# `DATA-6`: a latest-copy-only table next to each published Gold table

Planner and reviewer: Claude Opus. Developer: a non-Opus agent (`AGENTS.md`).
Base: `origin/main` at `6b2242ff6`. Branch: `data-6-latest-copy` (this plan's
commit is its first commit; build on top of it). One PR slice.
Spec: `docs/core/specs/data-6.md`. Feature page:
`docs/core/features/gold-latest-copy.md`.

Every anchor below was read at `6b2242ff6`. If the code you find does not match
an anchor, or a step below cannot be done as written, **STOP and report the
mismatch in the Handoff section**. Do not improvise a different design.

## 1. The problem

Each successful publish of a Gold mart appends a full copy to the Iceberg table
`gold.<table>`, stamped with `_exported_at`. `DATA-2` keeps the newest 7 copies.
An outside tool that counts rows without filtering on the newest export time
counts every kept copy and reports wrong numbers. `DATA-6` adds a second Iceberg
table, `gold.<table>_latest`, that always holds only the newest copy.

## 2. Mismatch: what the code allows, and how this plan meets the target anyway

None of these lowers a target. Each one says what the code allows and how the
plan meets the spec despite it.

- **M1. The Rust Iceberg library cannot replace a table's contents.**
  `iceberg` 0.10.1 (pinned, `rust/crates/lakehouse-iceberg/Cargo.toml:15`) offers
  only `fast_append`, schema/property/sort-order/location/statistics updates and
  `expire_snapshots` on `Transaction`
  (`~/.cargo/registry/src/*/iceberg-0.10.1/src/transaction/mod.rs:135-172`). The
  trait a custom action would implement, `TransactionAction`, is `pub(crate)`
  (`.../transaction/action.rs:37`), and so is the builder of `TableCommit`
  (`.../catalog/mod.rs:350`). So the Rust export cannot write "delete everything,
  add this copy" as one commit. **How the target is met:** the latest table is
  written by the Dagster code location with PyIceberg 0.12.0 (already pinned,
  `dagster/pyproject.toml:20`, and already used by `DATA-2`'s trim). PyIceberg's
  `Transaction.overwrite` stages the delete and the append and sends them in
  **one** catalog commit (`pyiceberg/table/__init__.py`: `Transaction.overwrite`
  calls `self.delete(...)` then `_append_snapshot_producer`, and both are staged
  until `commit_transaction`, which makes one `_do_commit`). T0 measures this on
  the stack before any product code is written. **Consequence:** the latest
  table is replaced in its own commit, in a separate orchestrator run that starts
  right after the main publish succeeds, not in the same commit as `gold.<table>`
  (two tables, and the Rust side cannot write the second). The spec's
  "Consistency" target is about the latest table alone ("a reader sees the old
  copy or the new one, never a mix") and is met. The delay is a new customer
  limit, measured in T6, not assumed.
- **M2. A copy has no single identity today.** `_exported_at` is computed again
  for every batch (`rust/crates/lakehouse-api/src/gold_export.rs:1022`), with
  **second** precision (`unix_timestamp() * 1_000_000`). The batch size is 20,000
  rows (`rust/crates/lakehouse-api/src/config.rs:2106`). A mart over 20,000 rows
  therefore carries several `_exported_at` values per publish, and two publishes
  in the same second share one. The latest table needs to pick out "the rows of
  this one publish", so T1 makes `_exported_at` one value per publish, in
  microseconds. **Found defect in `DATA-2` (shipped):** `run_trim_for_mart`
  (`dagster/dispar_orchestrate/gold_export.py:638`) counts distinct
  `_exported_at` values as copies, so for a mart over 20,000 rows it keeps 7
  *batches*, not 7 copies. T1 fixes this as a side effect. The CHANGELOG entry
  (T7) says so. The reviewer reports it to the backlog owner.
- **M3. The main table's publish is not one commit either.** The export appends
  one snapshot per batch (`gold_export.rs:1034`). A reader of `gold.<table>`
  during the publish of a mart over 20,000 rows can see a partial newest copy.
  This is out of scope here (the latest table is the answer for such readers).
  The reviewer proposes it as a follow-up backlog item.
- **M4. A mart may itself be named `<other>_latest`.** The Iceberg table name is
  `sanitize_mart_name(mart)` (`rust/crates/lakehouse-iceberg/src/gold.rs:75`), so
  a mart `sales_latest` and the latest table of mart `sales` would be the same
  table. The plan marks every latest table with a table property and refuses
  both directions (D7), so the two never share a table.

## 3. Decisions

Signed by the product owner on 2026-11-20 (the spec's *(proposed)* targets,
accepted as the defaults; also recorded on the feature page):

- **D1. Name.** `gold.<table>_latest`, where `<table>` is the published table's
  own name (`sanitize_mart_name(mart)`). The suffix is the constant
  `LATEST_TABLE_SUFFIX = "_latest"`.
- **D2. Form.** A plain Iceberg table, not an Iceberg view. Format version 2.
- **D3. Consistency.** Replaced in one Iceberg commit on each successful publish.
  A reader sees the old copy or the new one, never a mix.
- **D4. Shown.** The publish card on the Gold table's page shows the table's name
  `gold.<table>_latest` with a copy button.
- **D5. Switch off.** Switching publishing off stops updates to both tables, and
  both tables stay. Nothing in this feature drops a table.

Planner decisions:

- **D6. Who writes it, and when.** After every successful publish that wrote
  rows (`rowsExported > 0`, not a skip), the API launches a new Dagster job,
  `gold_latest_copy_job`, for that one mart. The launch happens in
  `routes/gold.rs::export`, the one place every publish goes through: a manual
  "Publish now", a scheduled `gold_export_job` run, and a run after an authored
  pipeline. The job's single op, `refresh_gold_latest_copy`, reads that
  publish's rows from `gold.<table>` (`_exported_at = <that publish's value>`)
  and replaces `gold.<table>_latest` with them, through PyIceberg, as the
  existing `gold-export` principal (`load_gold_catalog`,
  `gold_export.py:615`). No new principal, no new grant, no compose change.
  Reading from the published table, not from ClickHouse, means the latest copy
  holds exactly the rows the publish wrote, with the masking applied at publish
  time (WS7 D2). Python never reads the mart.
- **D7. The marker.** Every latest table carries the table property
  `rantai.latest-copy-of = <table>` (constant `LATEST_COPY_OF_PROPERTY`). The
  Rust export refuses to append to a table that carries this property:
  `GoldExportError::TargetIsLatestCopy`, answered as a 422 with a fixed sentence
  and recorded as a failed publish. The Python op refuses to write a
  `gold.<table>_latest` that exists without the marker, or whose marker names
  another table (`name_taken`). Fail closed both ways.
- **D8. Layout.** Unpartitioned, with the same columns as `gold.<table>`,
  `_exported_at` included (it tells a reader which publish the copy is). It is
  created **with its first copy in one commit**
  (`catalog.create_table_transaction(...)` plus `tx.append(df)`), so it never
  exists empty. If its columns differ from `gold.<table>`'s (same names, types
  and required flags, in order), the op fails `schema_incompatible` and changes
  nothing. There is no automatic schema evolution: the Rust export never evolves
  `gold.<table>`'s schema either.
- **D9. Which copy, and never backwards.** The API passes the publish's
  `_exported_at` (microseconds) and `rowsExported` in the run config. The op:
  1. skips (`superseded`, `newer_copy_present`) when the latest table already
     holds a copy with `_exported_at >= ` the requested one, so runs that
     execute out of order never move the table backwards;
  2. fails `copy_incomplete` when the rows read for that `_exported_at` are not
     exactly `rowsExported`. This includes 0 rows, which happens when the
     trim already removed that copy.
- **D10. Switch off is checked when the run executes.** The op asks
  `GET /api/gold/publications` (`_enabled_publication_marts`,
  `gold_export.py:308`) and also accepts a mart listed in the `GOLD_EXPORT_MARTS`
  override (`cfg.marts`). This is the same union `list_gold_marts` uses
  (`gold_export.py:384`). A mart in neither list is `skipped`, reason
  `publishing_off`, and nothing is written. If that list cannot be read, the run
  fails with `publication_state_unavailable` and nothing is written (fail
  closed).
- **D11. Only the current snapshot is kept, and its files are freed safely.**
  After a successful replace, the op expires every snapshot of the latest table
  except the current one, then removes orphan files through ClickHouse with
  `GOLD_ORPHAN_MIN_AGE_SECONDS`. It reuses the trim's code, extracted into
  shared helpers (T3). While `MAINTENANCE_PAUSED=true`
  (`maintenance.py:249`), both steps are skipped and recorded. A failed cleanup
  never marks the copy failed: the copy *was* replaced. It is recorded in its
  own `cleanup` column. Without this, the latest table's storage would grow by
  one copy per publish.
- **D12. The record.** Each run writes one row to the ClickHouse table
  `console.gold_latest_copy_run`, created with `CREATE TABLE IF NOT EXISTS` by
  both writers (the op, and the API when a launch fails). This follows the
  pattern of `console.gold_trim_run` (`gold_export_history.rs:55-93`,
  `gold_export.py:555`). **No Postgres migration** is needed. The DDL text is
  in section 5 and must be identical in both languages.
  - `status` is one of `success`, `failed`, `skipped` or `superseded`.
  - `reason` comes from a closed set of fixed strings and is never exception
    text (principle 4). The full set is in section 6.
  - The exception detail goes to the run log only.
- **D13. A failed latest-copy update never changes the main publish result.**
  The publish's own response and its `console.gold_export_run` row stay exactly
  as they are today. If the launch fails, the API logs it, records a
  `failed` / `not_started` row, and answers the publish as a success with
  `latestCopy.status = "not_started"`. This follows the same stance as
  `grant_outside_readers_select` (`gold_export.rs:1055-1125`): the publish
  happened, and saying otherwise would be false.
- **D14. Permissions (AC3).**
  - Outside tools read `gold.<table>_latest` through the existing door
    (`/api/iceberg`, ADR 0016). Only a service identity may use it
    (`routes/iceberg_door.rs::refuse`). Only the `gold` namespace is served,
    and the table name passes `table_name_is_allowed`
    (`iceberg_door.rs:340`: lowercase, digits, underscore).
  - The catalog-side grant is namespace-scoped (`select` on `gold` for
    `outside-readers`, `gold_export.rs:1079`), so it covers the new table with
    no new grant.
  - In the console, the card is shown to anyone who may see the publication
    detail (`GET /api/gold/export/{mart}/publication`, `RequiresAuth`). The table
    name is not secret.
  - The switch stays `gold:export` only (`PUT .../publication`,
    `RequiresPermission("gold:export")`). Nothing new is gated.
  - The gate (T6) proves that a session that is not a service identity is
    refused by the door on the latest table.
- **D15. Not part of the dashboards assistant** (spec, "Asking the assistant").
  No tool change, no `tool_schemas.json` change.
- **D16. The job is serial.** `gold_latest_copy_job` joins `SERIAL_JOB_NAMES`
  (`dagster/dispar_orchestrate/ingest_factory.py:334`): one run at a time. Runs
  queue, and are never dropped (`dagster/dagster.yaml:64-85`). Two updates of the
  same table therefore never race. If something outside the orchestrator commits
  at the same moment, the commit conflicts and is recorded as `commit_conflict`.
  The run limit setting is `DAGSTER_RUN_LIMIT_GOLD_LATEST_COPY_JOB_SECONDS`,
  default 3600.

## 4. Anchors (verified at `6b2242ff6`)

Rust, `rust/crates/lakehouse-iceberg`:
- `src/gold.rs:43` `GOLD_NAMESPACE`; `:48` `EXPORTED_AT_COLUMN`; `:75`
  `sanitize_mart_name`; `:87-110` `gold_schema` (`_exported_at` is a required
  `Timestamp`, microseconds, no time zone).
- `src/catalog.rs:351` `load_gold_table`; `:397` `create_or_load_gold_table`;
  `:563` `GoldTable`; `:598`/`:608` snapshot accessors (add the new accessor
  next to them); `:624` `GoldTable::append`.

Rust, `rust/crates/lakehouse-api`:
- `src/gold_export.rs:90` `GoldExportError`; `:812` `GoldExportResult`; `:887`
  `export_mart`; `:972-979` create/load and the outside-readers grant; `:1012-1043`
  the batch loop (`:1022` the per-batch `now_micros`); `:1046-1053` the result.
- `src/routes/gold.rs:83-120` `From<GoldExportError> for ApiError`; `:453`
  `export`; `:512-520` the single-flight guard `_export_guard`; `:576-597`
  history record and the response body; `:1172-1226` `publication_body` (`:1212`
  `last_trim`, `:1224` `"trim"`).
- `src/gold_export_history.rs:52-93` the trim table's cached `ensure_*`;
  `:664-749` `GoldTrimSummary`, `millis_to_rfc3339`, `last_trim_sql` (binds the
  mart with `SqlLiteral::from`), `row_to_trim_summary`, `last_trim`.
- `src/state.rs:105` `AppState::dagster: Arc<DgClient>`.
- `src/routes/connector_discovery.rs:248-275` the pattern for launching a job
  with run config and handling a refused or unreachable launch.
- `src/lib.rs:34-36` the `pub mod gold_*` lines (add `pub mod gold_latest_copy;`
  in alphabetical order).
- `src/upstream_error.rs:116-130` `CATALOG`, `PUBLISH_BATCH` contexts.
- `tests/it.rs:115` `mod gold_publication;` (add `mod gold_latest_copy;` in
  order). `tests/gold_publication.rs:656-778` the trim seeding and detail tests to
  mirror. `tests/connector_discovery_launch.rs:22-31,83-93` a `wiremock` Dagster.
  `tests/ai_alert_tools.rs:134-150` a `wiremock` standing in for `ClickHouse`.
- `rust/crates/lakehouse-dagster/src/lib.rs:772` `LaunchOutcome`; `:1495`
  `launch_run_with_config`.

Python, `dagster/dispar_orchestrate`:
- `gold_export.py:134-135` the fixed-reason constants; `:199-265`
  `GoldExportConfig`; `:276` `_headers`; `:308` `_enabled_publication_marts`;
  `:384` `list_gold_marts`; `:531-552` `_sql_str`/`_sql_num`/`_sql_dt64`;
  `:555-612` the trim table DDL and writer; `:615` `load_gold_catalog`; `:638`
  `run_trim_for_mart` (`:848-879` snapshot expiry, `:881-918` orphan removal);
  `:1009-1021` `gold_export_job`; `:1116-1119` `gold_export_defs`.
- `maintenance.py:249` `maintenance_paused`; `:330` `ensure_catalog_database`;
  `:437` `run_remove_orphan_files`.
- `bronze_catalog.py:79` `ClickHouseTarget`; `:239` `_ch_exec`.
- `ingest_factory.py:323-360` `SERIAL_TAG`, `SERIAL_JOB_NAMES`, `job_tags`.
- `definitions.py:26-31` imports, `:111-128` `defs.jobs`.
- `op_metadata.py` `source_metadata`, `DEFAULT_RETRY_POLICY`.
- `test_gold_export.py:721-955` `GoldTrimOpTests` (fakes to mirror);
  `test_job_serial_tags.py:54,79` the serial-job table.

Console, `src/`:
- `src/services/contracts/gold.ts:17-24` `GoldExportResult`; `:117-123`
  `GoldTrimSummary`; `:135-149` `GoldPublication`.
- `src/services/clients/gold.ts:70,90` pass-through `getJson` (no mapping to
  change).
- `src/features/catalog/open-format-card.tsx:153` `OpenFormatCard`; `:207`
  the card description; `:297-313` the trim rows (place the new rows after them).
- `src/features/catalog/open-format-card.test.tsx:526-560` the trim tests to
  mirror.
- `src/components/copyable.tsx` `Copyable` (reuse it; do not write a second copy
  button, rule 4).

Gate and CI:
- `ops/gold_export/gold_export_test.py:438` `_outside_catalog`; `:657`
  `step_job_export_9_copies_and_trim_to_7`; `:796-821` `main`.
- `.github/workflows/ci.yml:3791-3885` the `gold-export` job (`timeout-minutes:
  31`, its CI-only `.env` with `GOLD_ORPHAN_MIN_AGE_SECONDS=0`).

Docs:
- `docs/adr/0010-gold-export-to-iceberg-from-rust.md` (append an update section).
- `docs/OPERATIONS.md:2825` "Outside tools read published tables".
- `CHANGELOG.md`.

## 5. Shared definitions (copy exactly)

Constants, in both languages (Python names them in a comment as the mirror of
the Rust constants):

| Rust (`lakehouse_iceberg::gold`) | Python (`gold_latest_copy.py`) | Value |
| --- | --- | --- |
| `LATEST_TABLE_SUFFIX` | `LATEST_TABLE_SUFFIX` | `"_latest"` |
| `LATEST_COPY_OF_PROPERTY` | `LATEST_COPY_OF_PROPERTY` | `"rantai.latest-copy-of"` |
| `lakehouse_api::gold_latest_copy::GOLD_LATEST_COPY_JOB` | job name | `"gold_latest_copy_job"` |

The record table. The DDL is identical in `gold_export_history.rs` /
`gold_latest_copy.rs` (Rust) and `gold_latest_copy.py` (Python):

```sql
CREATE TABLE IF NOT EXISTS console.gold_latest_copy_run (
    id String,
    mart String,
    latest_table String,
    status String,
    reason Nullable(String),
    copy_exported_at Nullable(DateTime64(6)),
    rows Nullable(UInt64),
    snapshot_id Nullable(Int64),
    cleanup Nullable(String),
    started_at DateTime64(3),
    finished_at DateTime64(3)
) ENGINE = MergeTree ORDER BY (mart, started_at)
```

`latest_table` holds the bare table name (`sales_latest`, no namespace).

Run config the API sends (op name `refresh_gold_latest_copy`):

```json
{"ops": {"refresh_gold_latest_copy": {"config": {
  "mart": "<mart as given to the route>",
  "table": "<GoldExportResult.table>",
  "latest_table": "<table>_latest",
  "copy_exported_at_micros": 1763632800123456,
  "rows": 123
}}}}
```

## 6. Failure and status table (closed sets)

| status | reason | When | Written |
| --- | --- | --- | --- |
| `failed` | `not_started` | The API could not launch the job (refused or unreachable) | by the API |
| `failed` | `invalid_request` | Run config missing a field, `rows < 1`, `copy_exported_at_micros < 1`, or `latest_table != table + "_latest"` | by the op |
| `failed` | `publication_state_unavailable` | `GET /api/gold/publications` failed | by the op |
| `skipped` | `publishing_off` | The mart is in neither the enabled list nor `GOLD_EXPORT_MARTS` | by the op |
| `failed` | `catalog_unavailable` | Loading the catalog or `gold.<table>` failed | by the op |
| `failed` | `name_taken` | `gold.<table>_latest` exists without `LATEST_COPY_OF_PROPERTY == table` | by the op |
| `failed` | `schema_incompatible` | The existing latest table's columns differ from `gold.<table>`'s | by the op |
| `superseded` | `newer_copy_present` | The latest table already holds `_exported_at >=` the requested one | by the op |
| `failed` | `copy_incomplete` | Rows read for that `_exported_at` != `rows` (0 included) | by the op |
| `failed` | `commit_conflict` | PyIceberg `CommitFailedException`, or `TableAlreadyExistsError` on create | by the op |
| `failed` | `write_failed` | Any other exception while writing or committing | by the op |
| `success` | null | Replaced (or created) | by the op |

`cleanup` (only on `success`, otherwise null): null when the cleanup succeeded,
`maintenance_paused`, `invalid GOLD_ORPHAN_MIN_AGE_SECONDS` (the existing
`INVALID_ORPHAN_MIN_AGE_REASON`), `expire_snapshots_failed`, or
`orphan_cleanup_failed`.

When a `failed` row is recorded, the op raises
`dagster.Failure(description=<reason>, allow_retries=<reason in
{"commit_conflict", "catalog_unavailable", "publication_state_unavailable"}>)`,
so the Dagster run shows as failed rather than green. `skipped`, `superseded`
and `success` return normally. If writing the record itself fails, that is
logged as a warning and never raised (the same as `run_trim_for_mart`).

## 7. Tasks (one commit each, in this order)

Scoped checks per commit (`AGENTS.md`, "Keep build time down"). Rust:
`cd rust && cargo fmt --check && cargo clippy -p <crate> --all-targets -- -D warnings && cargo test -p <crate> <filter>`.
Python: `cd dagster && python -m pytest dispar_orchestrate/<file> -q`.
Console: `bun run typecheck && bun run lint && bun test <folder>`.
Use the machine's `cargo-locked` wrapper for every cargo command where it
exists, and never `cargo clean`. Push after every commit (worker sessions can
die).

### T0. Spike on the compose stack, before any product code (no commit, unless a gate script)

From inside `dagster-code-location`, with `load_gold_catalog(GoldExportConfig.from_env())`,
against a published Gold table `gold.<t>` that has at least two copies:

1. `scan(row_filter=EqualTo("_exported_at", <int micros of the newest copy>)).to_arrow()`
   returns exactly that copy's rows. If an `int` literal is refused on a
   `Timestamp` column, try a naive UTC `datetime` and record which form works.
2. `catalog.create_table_transaction("gold.<t>_latest", schema=<gold.<t>'s schema>, properties={"rantai.latest-copy-of": "<t>", "format-version": "2"})`
   used as a context manager, with `tx.append(df)` inside, creates the table
   with its rows. Record the metadata log length (`len(table.metadata.metadata_log)`)
   and the snapshot count afterwards.
3. `with latest.transaction() as tx: tx.overwrite(df2)` on that table: record
   the metadata log length before and after, and the snapshots it added (their
   `summary.operation`). **Gate: the metadata log grows by exactly 1.** That is
   one catalog commit.
4. `SELECT count(), count(DISTINCT _exported_at) FROM icecat_gold_export.\`gold.<t>_latest\` WHERE 1`
   through ClickHouse returns the new copy only.
5. Expire every snapshot except the current one, then `run_remove_orphan_files(..., older_than_seconds=0)`
   on `gold.<t>_latest`: the old copy's Parquet file is gone from the object
   store.

Record every number in the Handoff. **If step 2 or 3 needs more than one
commit, or step 4 shows a mix, STOP and report.** Do not build T1–T7 on an
assumption.

### T1. Rust, `lakehouse-iceberg` + `lakehouse-api::gold_export`: one copy identity and the marker (D7, M2)

`rust/crates/lakehouse-iceberg/src/gold.rs`:
- Add `pub const LATEST_TABLE_SUFFIX: &str = "_latest";` and
  `pub const LATEST_COPY_OF_PROPERTY: &str = "rantai.latest-copy-of";` with
  doc comments naming `DATA-6` D1/D7.
- Add `pub fn latest_table_name(table: &str) -> String` (returns
  `format!("{table}{LATEST_TABLE_SUFFIX}")`; `table` is an already-sanitized
  name).
- Add `pub fn latest_copy_of(properties: &HashMap<String, String>) -> Option<&str>`.
  It returns the marker's value when present.

`rust/crates/lakehouse-iceberg/src/catalog.rs`: add
`GoldTable::latest_copy_of(&self) -> Option<&str>` next to
`current_snapshot_timestamp_ms` (`:608`), delegating to
`gold::latest_copy_of(self.table.metadata().properties())`.

`rust/crates/lakehouse-api/src/gold_export.rs`:
- Add `pub(crate) fn copy_exported_at_micros(now: time::OffsetDateTime) -> i64`,
  returning `i64::try_from(now.unix_timestamp_nanos() / 1_000)` (handle the
  conversion with `unwrap_or(i64::MAX)` or `?` on a new error, but no `unwrap`).
- In `export_mart`, compute it **once**, before the loop (`:1012`). Use that
  one value for every batch, replacing `:1022`. Rewrite the comment at
  `:1010-1021`: one value per publish, because a copy must be identifiable
  (`DATA-6` M2), and because it also corrects `DATA-2`'s trim count.
- Add `copy_exported_at_micros: Option<i64>` to `GoldExportResult` (`None` when
  `rows_exported == 0`).
- Add the variant `GoldExportError::TargetIsLatestCopy { mart: String }`. Its
  `Display` is a fixed sentence: "the Iceberg table for mart {mart:?} is the
  latest-copy table of another published table; rename the mart".
- Right after `create_or_load_gold_table` (`:973`), and before the grant and
  any append: if `table.latest_copy_of().is_some()`, return that variant.
- In `routes/gold.rs`'s `From` impl (`:83`), map it to
  `ApiError::Unprocessable(err.to_string())`. The text is fixed and carries no
  upstream words.

Tests (they fail on the old code: the names do not exist, or the old
second-precision value differs):
- `lakehouse-iceberg` `gold.rs` tests: `latest_table_name("sales") == "sales_latest"`;
  the two constants pinned to their exact strings;
  `latest_copy_of` returns `Some("sales")` for a map holding the property, and
  `None` for an empty map.
- `lakehouse-api` `gold_export.rs` tests:
  `copy_exported_at_micros(datetime!(2026-11-20 10:00:00.123456 UTC)) == 1_763_632_800_123_456`
  (sub-second precision is kept).
- `lakehouse-api` `routes/gold.rs` tests:
  `GoldExportError::TargetIsLatestCopy { mart: "x_latest".into() }` converts to
  `ApiError::Unprocessable` whose text names the mart and contains neither
  "iceberg catalog" nor "http".

Check: `cargo test -p lakehouse-iceberg gold` and
`cargo test -p lakehouse-api --lib gold_export` and `... --lib routes::gold`.

### T2. Rust, `lakehouse-api`: launch, record, read, show (D6, D12, D13)

New module `rust/crates/lakehouse-api/src/gold_latest_copy.rs` (a `//!` module
doc stating D6, D9, D12, D13 and M1 in prose). Register it in `lib.rs`. It
contains:
- `pub const GOLD_LATEST_COPY_JOB: &str = "gold_latest_copy_job";`
- `pub struct LatestCopyRequest<'a> { pub mart: &'a str, pub table: &'a str, pub copy_exported_at_micros: i64, pub rows: u64 }`
- `pub fn run_config(req: &LatestCopyRequest<'_>) -> serde_json::Value`, the
  exact shape of section 5 (it computes `latest_table` with
  `gold::latest_table_name`).
- `pub async fn ensure_gold_latest_copy_run_table(ch: &ChClient) -> Result<(), ChError>`.
  Use the DDL of section 5, behind a `tokio::sync::OnceCell` exactly like
  `TRIM_TABLE_ENSURED` (`gold_export_history.rs:52-65`).
- `pub async fn launch(dagster: &DgClient, ch: &ChClient, req: &LatestCopyRequest<'_>) -> &'static str`,
  which returns `"queued"` or `"not_started"`. It calls
  `dagster.launch_run_with_config(GOLD_LATEST_COPY_JOB, &run_config(req))`.
  `Ok(outcome)` with `outcome.failure.is_none()` gives `"queued"`. A refusal or
  an `Err` gives `"not_started"`: log it with `tracing::warn!` (mart and job
  only, never the upstream text), then insert one `failed` / `not_started` row
  (`copy_exported_at` and `rows` filled; strings bound with `SqlLiteral::from`;
  `fromUnixTimestamp64Micro(<i64>)` for the time). A failed insert is logged
  with `tracing::error!` and not propagated.
- `#[derive(Serialize)] #[serde(rename_all = "camelCase")] pub struct LatestCopyHolds { pub copy_exported_at: Option<String>, pub rows: Option<u64>, pub replaced_at: Option<String> }`,
  read from the newest `success` row.
- `#[derive(Serialize)] #[serde(rename_all = "camelCase")] pub struct LatestCopyRun { pub status: String, pub reason: Option<String>, pub cleanup: Option<String>, pub finished_at: Option<String> }`,
  read from the newest row of any status.
- `pub async fn latest_copy_state(ch: &ChClient, mart: &str) -> Result<(Option<LatestCopyHolds>, Option<LatestCopyRun>), ChError>`.
  It calls `ensure_*`, then runs two `ORDER BY started_at DESC LIMIT 1 FORMAT JSON`
  selects with `WHERE mart = {SqlLiteral}` (and `AND status = 'success'` for the
  first). Times come back as `toString(toUnixTimestamp64Micro(copy_exported_at))`
  / `toString(toUnixTimestamp64Milli(finished_at))`. Read them with
  `nullable_u64_col`/`nullable_i64_col` (`routes/support.rs`), as
  `row_to_trim_summary` does.
- `pub fn micros_to_rfc3339(us: i64) -> Option<String>`, which keeps the
  microseconds (`OffsetDateTime::from_unix_timestamp_nanos(i128::from(us) * 1000)`).
  Do not reuse `millis_to_rfc3339`: it drops the sub-second part, and two
  copies can fall in one second.

`routes/gold.rs::export` (`:453`):
- Rename `_export_guard` to `export_guard`. After `let result = export_result?;`
  (`:587`), call `drop(export_guard);` so the lock is not held during the
  launch.
- If `result.rows_exported > 0` and `result.copy_exported_at_micros` is
  `Some(us)`, call `gold_latest_copy::launch(&state.dagster, &state.clickhouse, &req)`.
- Add to the response body: `"copyExportedAt": <micros_to_rfc3339 or null>` and
  `"latestCopy": {"table": "gold.<latest_table>", "status": <"queued"|"not_started">}`,
  or `"latestCopy": null` when nothing was written. The skip answer
  (`skipped_body`) is unchanged.

`routes/gold.rs::publication_body` (`:1172`): after `last_trim`, call
`latest_copy_state`. Classify its error with
`upstream_error::ch_error(&upstream_error::DATABASE, &e)`, like its neighbours
(principle 4). Add:

```json
"latestCopy": {
  "table": "gold.<sanitize_mart_name(mart)>_latest",
  "holds": null | {"copyExportedAt", "rows", "replacedAt"},
  "lastRun": null | {"status", "reason", "cleanup", "finishedAt"}
}
```

`sanitize_mart_name` failing here is a 400 `invalid mart`, as in `consumers`
(`:1046-1047`).

Tests:
- `gold_latest_copy.rs` unit tests: `run_config` holds exactly the five keys
  under `ops.refresh_gold_latest_copy.config`, with `latest_table == "sales_latest"`
  for table `sales`; `micros_to_rfc3339(1_763_632_800_123_456)` ends in
  `.123456Z`; both selects contain `console.gold_latest_copy_run`,
  `ORDER BY started_at DESC LIMIT 1`, and the mart as a quoted literal (a mart
  holding `'` is escaped).
- New integration module `tests/gold_latest_copy.rs` (add `mod gold_latest_copy;`
  to `tests/it.rs`, and `use crate::common;` if you need the harness). Start a
  `wiremock` Dagster (`connector_discovery_launch.rs:83`) and a `wiremock` that
  answers every `ClickHouse` POST with 200 and an empty body (`ai_alert_tools.rs:149`):
  1. `a_launched_update_is_queued_and_records_nothing`: `LaunchRunSuccess` →
     `"queued"`. The ClickHouse mock received no `INSERT INTO console.gold_latest_copy_run`.
     The Dagster request body contains `gold_latest_copy_job` and `copy_exported_at_micros`.
  2. `a_refused_launch_is_recorded_as_not_started`: a `PipelineNotFoundError`
     answer → `"not_started"`. The ClickHouse mock received exactly one INSERT
     holding `'not_started'` and `'failed'`.
  3. `an_unreachable_orchestrator_is_recorded_as_not_started`: Dagster URL
     `http://127.0.0.1:1` → `"not_started"` and one INSERT.
  If `ChClient` cannot reach a `wiremock` listener because of the allowlisted
  resolver (`SEC-10`, `lakehouse-api/Cargo.toml:247`), use the shared ClickHouse
  test container from `gold_publication.rs:51-88` instead, and count the rows.
  Say which one you used in the Handoff.
- `tests/gold_publication.rs`, next to the trim tests (`:686-778`), with a
  `seed_latest_copy_run` helper:
  1. `publication_detail_names_the_latest_table_before_any_update`:
     `latestCopy.table == "gold.<mart>_latest"`, `holds == null`, `lastRun == null`.
  2. `publication_detail_reports_what_the_latest_table_holds`: a seeded
     `success` row gives `holds.rows`, a `holds.copyExportedAt` with
     microseconds, and `lastRun.status == "success"`.
  3. `a_failed_update_after_a_success_keeps_holds_and_reports_the_reason`:
     `success` then `failed`/`copy_incomplete` gives `holds` from the success
     and `lastRun.reason == "copy_incomplete"`.

Check: `cargo test -p lakehouse-api --lib gold_latest_copy`,
`cargo test -p lakehouse-api --test it gold_latest_copy::`,
`cargo test -p lakehouse-api --test it gold_publication::`.
Then, once, `cargo clippy -p lakehouse-api --all-targets -- -D warnings`.

### T3. Python: share the trim's cleanup steps (refactor, behaviour unchanged)

In `gold_export.py`, extract from `run_trim_for_mart`:
- `expire_all_but_current_snapshot(table) -> int` (the body of `:848-879`'s
  try block, returning the count expired);
- `remove_gold_orphan_files(cfg, table_name: str, older_than_seconds: int) -> int`
  (`:881-918`: `ensure_catalog_database(cfg, db_name="icecat_gold_export")`,
  then `run_remove_orphan_files(..., dry_run=False, catalog_db="icecat_gold_export", older_than_seconds=...)`,
  returning `deleted_data_files_count`).

`run_trim_for_mart` calls both. Its fixed reasons and records stay exactly as
they are. Check: every existing `GoldTrimOpTests` passes unchanged
(`python -m pytest dispar_orchestrate/test_gold_export.py -q`). Add one test per
helper, with fakes: `expire_all_but_current_snapshot` expires every id except
the current one and returns that count; `remove_gold_orphan_files` passes
`older_than_seconds` through.

### T4. Python: the op, the job, the record (D6, D8–D12, D16)

New module `dagster/dispar_orchestrate/gold_latest_copy.py`. Narrative module
docstring (M1, D6–D12). `from __future__ import annotations`. Contents:
- `LATEST_TABLE_SUFFIX`, `LATEST_COPY_OF_PROPERTY`, with a comment naming
  `lakehouse_iceberg::gold`'s constants as the source.
- `ensure_gold_latest_copy_run_table(target)` with the section 5 DDL, and
  `record_gold_latest_copy_run(target, *, mart, latest_table, status, reason=None, copy_exported_at_micros=None, rows=None, snapshot_id=None, cleanup=None, started_at, finished_at)`,
  modelled on `record_gold_trim_run` (`gold_export.py:580`). Add a helper
  `_sql_dt64_micros(us: int | None) -> str` that gives `fromUnixTimestamp64Micro(<int>)`
  or `NULL`. Reuse `_sql_str`/`_sql_num`/`_sql_dt64` from `gold_export`.
- `@dataclass(frozen=True) class LatestCopyRequest` with
  `from_op_config(config: dict) -> LatestCopyRequest`. It raises
  `InvalidLatestCopyRequest` on any rule in section 6's `invalid_request` row.
- `_columns(schema) -> list[tuple[str, str, bool]]` (name, `str(field_type)`,
  `required`) for D8's comparison.
- `refresh_latest_copy(cfg: GoldExportConfig, req: LatestCopyRequest, log=None) -> dict[str, Any]`,
  which runs the steps in this order. Every exit records exactly one row:
  1. Publishing on? (D10) `req.mart in _enabled_publication_marts(cfg) or req.mart in cfg.marts`.
     A `requests` exception → `failed`/`publication_state_unavailable`.
     Neither → `skipped`/`publishing_off`.
  2. `catalog = load_gold_catalog(cfg)`; `main = catalog.load_table(f"gold.{req.table}")`
     → `catalog_unavailable` on any exception.
  3. `latest = catalog.load_table(f"gold.{req.latest_table}")`, or `None` on
     `pyiceberg.exceptions.NoSuchTableError`. Any other exception →
     `catalog_unavailable`.
  4. If `latest` exists: `latest.properties.get(LATEST_COPY_OF_PROPERTY) != req.table`
     → `name_taken`. `_columns(latest.schema()) != _columns(main.schema())` →
     `schema_incompatible`. Read the held copy with
     `latest.scan(selected_fields=("_exported_at",), limit=1).to_arrow()`. If
     it is not empty and its value (as micros) is `>= req.copy_exported_at_micros`
     → `superseded`/`newer_copy_present`.
  5. `df = main.scan(row_filter=EqualTo("_exported_at", <the literal form T0 proved>)).to_arrow()`.
     `df.num_rows != req.rows` → `copy_incomplete`.
  6. Write, in ONE commit:
     - no latest table: `with catalog.create_table_transaction(f"gold.{req.latest_table}", schema=main.schema(), properties={LATEST_COPY_OF_PROPERTY: req.table, "format-version": "2"}) as tx: tx.append(df)`;
     - latest table exists: `with latest.transaction() as tx: tx.overwrite(df)`.
     `CommitFailedException` or `TableAlreadyExistsError` → `commit_conflict`;
     any other exception → `write_failed`. Never call `latest.delete(...)` and
     `latest.append(...)` separately: that would be two commits and would break
     D3.
  7. Reload the latest table. `snapshot_id = current_snapshot().snapshot_id`.
  8. Cleanup (D11): `maintenance_paused()` → `cleanup="maintenance_paused"`.
     Otherwise parse `GOLD_ORPHAN_MIN_AGE_SECONDS` with
     `parse_gold_orphan_min_age_seconds` (a bad value gives
     `INVALID_ORPHAN_MIN_AGE_REASON`), then call
     `expire_all_but_current_snapshot`, then `remove_gold_orphan_files`. Each
     failure sets `cleanup` to its reason and stops the cleanup there. The
     status stays `success`.
  9. Record `success`, with `copy_exported_at_micros`, `rows`, `snapshot_id`
     and `cleanup`.
  Detail of an exception goes to `log.error(...)` only. The row holds the
  fixed reason only.
- `@op(config_schema={"mart": str, "table": str, "latest_table": str, "copy_exported_at_micros": int, "rows": int}, retry_policy=DEFAULT_RETRY_POLICY, tags=source_metadata("dispar_orchestrate/gold_latest_copy.py::refresh_gold_latest_copy", reads=["Iceberg gold.{table} (one copy, by _exported_at)", "GET /api/gold/publications"], writes=["Iceberg gold.{table}_latest (replaced in one commit)", "ClickHouse console.gold_latest_copy_run"]))`
  `def refresh_gold_latest_copy(context) -> dict`. It builds the request: an
  invalid one gives `failed`/`invalid_request`, recorded with what is known,
  then a non-retryable `Failure`. It calls `refresh_latest_copy`, and raises
  `Failure` for a `failed` result as section 6 says.
- `@job(tags=job_tags("gold_latest_copy_job", "DAGSTER_RUN_LIMIT_GOLD_LATEST_COPY_JOB_SECONDS", 3600)) def gold_latest_copy_job(): refresh_gold_latest_copy()`
  with a docstring. No schedule and no sensor: only the API launches it (say
  so in a `definitions.py` comment, like `discover_job`'s at `:106-110`).
- `ingest_factory.py`: add `"gold_latest_copy_job"` to `SERIAL_JOB_NAMES`.
  `test_job_serial_tags.py:79`: add `(gold_latest_copy, "gold_latest_copy_job")`
  to its table.
- `definitions.py`: import and add `gold_latest_copy_job` to `defs.jobs`.

Tests, `dagster/dispar_orchestrate/test_gold_latest_copy.py`. No network: fake
catalog, table, transaction and ClickHouse target, as in `GoldTrimOpTests`. Each
test name is a full sentence:
1. `test_an_existing_latest_table_is_replaced_by_one_overwrite_in_one_transaction`:
   the fake records `transaction()` entered once, `overwrite` called once with
   the read frame, `commit` once, and no `delete`/`append` on the table itself.
   The row is `success`, with rows and snapshot id.
2. `test_a_first_update_creates_the_table_with_its_rows_in_one_commit`:
   `create_table_transaction` gets the marker property and the main schema;
   `append` is called once inside it.
3. `test_a_table_without_the_marker_is_not_touched_and_records_name_taken`.
4. `test_a_marker_naming_another_table_records_name_taken`.
5. `test_different_columns_record_schema_incompatible_and_write_nothing`.
6. `test_an_older_or_equal_request_is_superseded_and_writes_nothing`.
7. `test_a_row_count_that_differs_from_the_publish_records_copy_incomplete`
   (including 0 rows).
8. `test_a_commit_conflict_records_commit_conflict_and_the_op_fails_retryably`.
9. `test_any_other_write_error_records_write_failed_and_no_exception_text`: the
   recorded SQL does not contain the exception's message.
10. `test_publishing_switched_off_skips_and_writes_nothing`.
11. `test_a_mart_in_the_env_override_is_updated_even_when_not_switched_on`.
12. `test_an_unreadable_publication_list_fails_closed`.
13. `test_an_invalid_run_config_records_invalid_request_and_fails_without_retry`
    (missing key, `rows=0`, a `latest_table` that does not match).
14. `test_paused_maintenance_keeps_the_new_copy_and_records_the_pause_as_cleanup`.
15. `test_a_failed_snapshot_expiry_is_recorded_as_cleanup_and_the_status_stays_success`.
16. `test_the_constants_match_the_rust_names` (exact strings).
17. `test_the_job_is_serial_and_carries_its_run_limit`.

Check: `python -m pytest dispar_orchestrate/test_gold_latest_copy.py dispar_orchestrate/test_gold_export.py dispar_orchestrate/test_job_serial_tags.py -q`,
then the op-metadata walk test (`python -m pytest dispar_orchestrate -q -k "metadata or definitions"`),
and `python3 ops/lint/check_intra_package_imports.py`.

### T5. Console: name, copy button, what it holds, an honest failure (D4, AC3)

- `src/services/contracts/gold.ts`: add
  `GoldLatestCopyHolds = { copyExportedAt: string | null; rows: number | null; replacedAt: string | null }`,
  `GoldLatestCopyRun = { status: "success" | "failed" | "skipped" | "superseded" | string; reason: string | null; cleanup: string | null; finishedAt: string | null }`,
  `GoldLatestCopy = { table: string; holds: GoldLatestCopyHolds | null; lastRun: GoldLatestCopyRun | null }`.
  Add `latestCopy?: GoldLatestCopy | null` to `GoldPublication`. Add
  `copyExportedAt?: string | null` and
  `latestCopy?: { table: string; status: "queued" | "not_started" | string } | null`
  to `GoldExportResult`. Use `type`, and camelCase that mirrors serde.
- `open-format-card.tsx`, after the trim rows (`:313`), when `p.latestCopy` is
  set:
  - `dt` "Latest copy only" and `dd` `<Copyable value={p.latestCopy.table}><span className="font-mono text-xs">{p.latestCopy.table}</span></Copyable>`;
  - `dt` "Holds", and `dd` "The copy published <formatDateTime(copyExportedAt)> (<rows> rows)",
    or "Not written yet" when `holds` is null;
  - when `lastRun?.status === "failed"`: a `text-destructive` line
    `data-testid="latest-copy-failure"` reading
    `latestCopyFailureText(lastRun.reason)`;
  - when `lastRun?.status === "skipped"`: muted "Not updated: publishing is off.".
  `latestCopyFailureText` is a fixed map, one English sentence per section 6
  reason. For example `copy_incomplete` gives "The latest copy was not updated:
  the published copy was incomplete or already removed." An unknown code gives
  "The latest copy was not updated." followed by the code in parentheses. Never
  print "Up to date" from this block.
- Card description (`:207`): add one sentence: "A second table holds only the
  newest copy, for tools that do not filter on export time."

Tests in `open-format-card.test.tsx` (they fail on the old code):
1. The latest table's name is shown with a copy button
   (`getByText("gold.sales_latest")` and the `Copyable` button's accessible
   name).
2. "Not written yet" when `holds` is null.
3. A `failed`/`copy_incomplete` run shows its fixed sentence, and the `holds`
   line still shows the previous copy.
4. An unknown reason shows the generic sentence with the code.
5. A publication without `latestCopy` (an older API) renders without the block.

Check: `bun run typecheck && bun run lint && bun test src/features/catalog src/services`.

### T6. The gate proves it on the real stack (AC1, AC2, AC3, AC5)

Extend `ops/gold_export/gold_export_test.py`. Pass `tool_key` from `main()`
into the new steps. Add these steps after
`step_job_export_9_copies_and_trim_to_7()`:

1. `step_latest_copy_holds_only_the_newest_copy(tool_key)` (AC1): poll
   `GET .../publication` (at most 120 s) until `latestCopy.lastRun.status == "success"`
   and `latestCopy.holds.copyExportedAt` is the newest copy, compared in
   microseconds through ClickHouse:
   `SELECT toUnixTimestamp64Micro(max(_exported_at)) FROM icecat_gold_export.\`gold.{MART}\` WHERE 1`
   against the same query on `_latest`. Then assert:
   - `count()` on `gold.{MART}_latest` equals `SELECT count() FROM serving.{MART} WHERE 1`;
   - `count(DISTINCT _exported_at) == 1`;
   - the outside tool reads the same count through the door:
     `_outside_catalog(tool_key).load_table(f"gold.{MART}_latest").scan().to_arrow().num_rows`.
   Record the delay from the publish's answer to `success` (the M1 limit).
2. `step_a_reader_during_a_publish_never_sees_a_mix()` (AC2): start a thread
   that, every 0.2 s, runs
   `SELECT count(), count(DISTINCT _exported_at) FROM icecat_gold_export.\`gold.{MART}_latest\` WHERE 1`
   and keeps every sample. Insert one row into `serving.{MART}`, then
   `POST /api/gold/export/{MART}` (manual path) and assert the response has
   `latestCopy.status == "queued"`. Wait for `lastRun.status == "success"` with
   the new copy, then stop the thread. Assert:
   - every sample has `distinct == 1` and `count in {old_count, old_count + 1}`;
   - at least one sample of each count was seen. If none of the new count was
     seen, the step fails with "not measured".
3. `step_a_mart_cannot_publish_into_a_latest_table()` (AC3, D7): create
   `serving.{MART}_latest` in ClickHouse (one row), then
   `POST /api/gold/export/{MART}_latest`. Expect 422 with the fixed sentence,
   and `count()` on `gold.{MART}_latest` unchanged. Drop the ClickHouse table
   afterwards.
4. `step_the_door_refuses_a_session_on_the_latest_table()` (AC3, D14): the
   logged-in human session `API` calls
   `GET {API_URL}/api/iceberg/v1/<prefix>/namespaces/gold/tables/{MART}_latest`
   and gets the door's refusal (the same status
   `step_the_door_refuses_a_write_and_another_namespace` asserts for a
   non-service caller).
5. `step_switching_off_stops_both_tables()` (AC5): `PUT .../publication`
   `{"enabled": false}`, insert a row, trigger `gold_export_job` and wait for
   it. Assert `max(_exported_at)` of `gold.{MART}` and the count and
   `_exported_at` of `gold.{MART}_latest` are unchanged, and both tables still
   load.

Run the `gold-export` CI job's sequence locally from a clean compose project
(`ci.yml:3791-3885`, same `.env`). Quote the output, the wall time and the
measured delay. In `ci.yml`, raise `timeout-minutes` only by the measured
increase plus 5, with a comment citing `DATA-6`.

### T7. Docs

- `CHANGELOG.md`: a customer-readable entry. It covers the latest-copy table,
  its name, that it follows a publish by one orchestrator run (the measured
  delay), and the `DATA-2` trim correction (M2).
- `docs/adr/0010-gold-export-to-iceberg-from-rust.md`: append
  "## Update — the latest-copy table (`DATA-6`)". It explains M1 (why this
  table is written by PyIceberg in Dagster, not Rust), D3 (one commit), and D7
  (the marker).
- `docs/OPERATIONS.md:2825` section: an outside tool that reads without a
  filter reads `gold.<table>_latest`. Add a `pyiceberg` line next to the
  existing example.
- Module docs: `gold_latest_copy.rs`, `gold_latest_copy.py`, and
  `gold_export.rs`'s batch-loop comment.
- Do **not** edit `docs/core/` (the planner updates the feature page Limits and
  `BACKLOG.md` after merge).

## 8. How atomicity is achieved and tested

- **Achieved.** The latest table is only ever written in one catalog commit:
  `create_table_transaction` + `append` for the first copy (it never exists
  empty), and `Transaction.overwrite` for every copy after that. PyIceberg stages
  the delete and the append and commits them together. Iceberg catalogs swap a
  table's metadata pointer atomically, so a reader that loads the table gets
  either the old metadata or the new.
  - Concurrent writers are kept apart by the serial job (D16) and by the
    catalog's own check of the table's current snapshot (`CommitFailedException`
    → `commit_conflict`, nothing half-written).
  - Old data files are deleted only by orphan removal, which keeps files younger
    than `GOLD_ORPHAN_MIN_AGE_SECONDS` (3600 s by default; `DATA-2` review
    BLOCKER 1). So a reader that is still reading the previous copy keeps its
    files.
- **Tested.**
  1. T0 measures one commit (metadata log +1) on the real stack before anything
     is built.
  2. T4 tests 1–2 prove the op calls exactly one transaction, one overwrite,
     one commit, and never a separate delete/append.
  3. T6 step 2 reads the table continuously during a real publish and fails on
     any mixed or partial sample.

## 9. Out of scope

- Iceberg views (spec, "Not included").
- Making the main table's publish one commit (M3, proposed follow-up).
- Counting reads of `_latest` in the consumers route
  (`routes/gold.rs::consumers` counts `gold.<table>` only). Reads through the
  door are still recorded under the `_latest` name. Follow-up.
- Dropping or renaming either table. Schema evolution of the latest table (D8).
- Updating the latest table after a publish of zero rows: no copy was written,
  so the latest table keeps the previous one, as `gold.<table>` does.
- Hiding `gold_latest_copy_job` from the Pipelines list. It needs run config,
  so a manual trigger without config is refused by Dagster, as for
  `discover_job` today.
- Any assistant change (D15).

## 10. PR slicing

One PR from `data-6-latest-copy`, holding T1–T7 (T0 leaves no commit unless its
script becomes a gate step). The Rust work sits together (T1, T2). There is no
migration. Python follows (T3, T4), then the console (T5), then the gate (T6)
and the docs (T7). The planner opens a draft PR once T2 is pushed, so CI runs
the full Rust test partitions early.

## 11. Verification before handoff (on the final commit, in the foreground)

```bash
cd rust && cargo fmt --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace
bun run typecheck && bun run lint && bun run test
python3 ops/lint/check_intra_package_imports.py && python3 ops/lint/check_bare_iceberg_count.py
(cd dagster && python -m pytest dispar_orchestrate -q)
docker compose --profile '*' config --quiet   # needs the SEC-18 values: sh ops/init-env.sh once
# the gold-export CI sequence from a clean compose project (T6), quoted with its wall time
git status                                    # nothing generated staged
```

Quote each command with its counts. Anything not run is written down as *not
verified*, with the reason (rule 7). If the full `cargo test --workspace` does
not fit the machine, say so and rely on the draft PR's CI partitions. Quote the
CI run.

## 12. Handoff

One PR slice. Base `origin/main` at `6b2242ff6`; plan commit `5c84d3f73`.
Draft PR #612, head `b6b734a3ac32a40b5c9f3a0c2808e6a9ee02ae2d` (confirmed by
`gh pr view 612 --json headRefOid`). This handoff worker wrote no product code
and re-ran no heavy checks.

### Commits (plan, T0–T7)

| SHA | Task | Commit subject |
| --- | --- | --- |
| `5c84d3f73` | plan | `docs(data-6): plan the latest-copy table` |
| — | T0 | no commit — §7 says "no commit, unless a gate script"; no gate script or result file was committed (in the tree or in any branch's history) |
| `8d7b7c2b5` | T1 | one copy identity per publish and the latest-copy marker |
| `d339b5cb2` | T2 | launch the latest-copy job, record it, read it back |
| `3313f2d94` | T3 | share the trim's cleanup steps, behaviour unchanged |
| `799465b0f` | T4 | the latest-copy op, job and record |
| `652b7ce87` | T5 | the publication card names the latest-copy table |
| `a4221b818` | T6 | the gate proves the latest-copy table on the real stack |
| `b6b734a3a` | T7 | document the latest-copy table |

### T0 result (§7 T0, the stop gate)

T0 recorded **no numbers**. There is no `docs/plans/DATA-6-RESULT.md` in the
tree or in any branch's history, no spike/gate script was committed, and no
commit message carries the §7 T0 readings: no metadata-log length before or
after create or overwrite, no snapshot count, no row counts from the ClickHouse
read, and no object-store file-removal count. **Those numbers are *not
verified*.** The later gate's numbers are T6's, not T0's: the 209 s wall, the
6.4 s publish-to-success delay and the 17/18 sample counts must not be read
into T0.

Two qualitative T0 findings survive in the record:

- `docs/adr/0010-gold-export-to-iceberg-from-rust.md:221-225` (T7): "PyIceberg's
  `Transaction.overwrite` stages the delete and the append and sends both in
  ONE catalog commit — measured on this stack before any product code was
  written (`DATA-6` T0: the overwrite grew the metadata log by exactly 1, and a
  reader through `ClickHouse` saw the new copy only)."
- T4's commit message: the op reads with an "`int-micros` filter, T0-proved",
  so T0 established that an `int` literal works on the `Timestamp` column; the
  naive-`datetime` fallback in §7 T0 step 1 was not needed.

**Stop gate (§7 T0 steps 3–4): recorded as passed** — one catalog commit
(metadata log +1) and no mixed copy through ClickHouse. That is a qualitative
pass from the ADR/T4 records; the underlying readings were not written down,
so the numeric gate evidence is *not verified*.

### Checks each commit records (commit-message text only)

- `5c84d3f73` (plan): no checks recorded — docs only.
- T0: none recorded (above).
- `8d7b7c2b5` (T1): no command or count in the message. It records one fixture
  correction: the plan's `1_763_632_800_123_456` is 2025-11-20; the test
  asserts the true epoch micros for 2026-11-20 10:00:00.123456 UTC,
  `1_795_168_800_123_456`, so the property tested is unchanged.
- `d339b5cb2` (T2): no command or count in the message. It records the
  ClickHouse fallback in §7 T2: the launch tests use the shared real test
  container, not a wiremock, because the table-ensure `OnceCell` is
  process-wide in the one `it` binary — every module in `tests/it.rs` must
  share one ClickHouse or the DDL gets pinned to the wrong server.
- `3313f2d94` (T3): "every existing `GoldTrimOpTests` case passes unchanged";
  one new fake-based test per extracted helper. No exact command or count.
- `799465b0f` (T4): "23 tests with fakes cover the plan's seventeen cases";
  the invalid-config rule for a missing field is tested through
  `from_op_config`, since Dagster's own context builder refuses the config
  before the op runs. No exact command.
- `652b7ce87` (T5): no command or count in the message.
- `a4221b818` (T6): "all passing on the compose stack (gold-export CI
  sequence, 209s wall)" — AC1 (one copy, the mart's current row count, and the
  outside tool reads the same count through the door); AC2 (reader sampled
  every 0.2 s: counts 17 and 18, both observed, never a mix; publish answers
  `latestCopy.status == "queued"`; follow-up run 6.4 s after the publish's
  answer, the M1 limit); AC3 (the `<other>_latest` 422 and the human session
  refused at the door); AC5 (switch-off stops both tables and drops neither).
  The CI timeout rose 31 → 37 minutes: the new steps' measured wall is ~40 s
  (~1 minute), plus 5 minutes of slack.
- `b6b734a3a` (T7): no command or count in the message — docs only; the
  changelog quotes the 6.4 s delay as measured on the acceptance stack.

Each item above is what the commit message records. The exact commands and
counts are **not verified** here: this handoff worker did not re-run them (no
workspace cargo build, no compose stack, per the brief).

### CI

Draft PR #612 is `OPEN`/draft at head
`b6b734a3ac32a40b5c9f3a0c2808e6a9ee02ae2d`; `gh pr checks 612` shows all
checks pass at that head, including `Rust · fmt`, `Rust · clippy`,
`Rust · build`, `Rust · test 1/3`, `2/3` and `3/3 (stable)`,
`Rust · Gold export to Iceberg (ADR 0010)` (5m15s), `Dagster · unit tests`,
`Dagster · loader tests`, `Frontend · Lint · Typecheck · Test · Build`,
`Repo lints`, `cargo audit`, `cargo deny`, both gitleaks checks and
`ci-required`. CI was read, not re-run by this worker.

### Not verified

- D11's cleanup is not proved on the stack (`DATA-6 review R5`). The
  `gold-export` gate sets `GOLD_ORPHAN_MIN_AGE_SECONDS=0`, but it only
  counts Iceberg rows; it never asserts that the old copy's Parquet
  files are gone from the object store. Proving that needs an object-store
  file listing around a short-age update — more than a few lines of gate
  change — so the R5 fix takes the finding's "Not verified" option and
  records it here and in the feature page's Limits. The op's expiry and
  orphan-removal calls are covered by unit tests, but the end-to-end
  file removal is *not verified*.
- T0's numeric readings, and the numeric evidence for its stop gate — no
  result file or committed measurement exists (above).
- The §11 foreground verification block (`cargo fmt --check`, workspace
  clippy/test, `bun run typecheck && bun run lint && bun run test`, both Python
  lint scripts, `python -m pytest dispar_orchestrate -q`,
  `docker compose --profile '*' config --quiet` and a clean-project
  `docker compose up` for the gate): not run by this handoff worker — the
  brief forbids re-running heavy checks; CI on #612 is the substitute for the
  Rust, frontend, Python and gate jobs.
- Plan mismatches were recorded by the worker in the T1 and T2 commit messages
  (the fixture's epoch constant and the ClickHouse test-container fallback);
  both are reproduced above. No other stop-and-report mismatch was recorded.

### Fix round 1 (developer, 2026-11-20)

Findings R1 (BLOCKER) and R2–R6 (SHOULD-FIX) from §13 "Round 1" fixed, one
commit each in order, each pushed to `data-6-latest-copy`. No PR was opened
or merged by this worker.

| SHA | Finding | Commit subject |
| --- | --- | --- |
| `104c5bb20` | R1 | DATA-6 review R1: prove the one-commit claim in the gate, not T0 |
| `586dec35b` | R2 | DATA-6 review R2: a committed replace is never recorded as failed |
| `2bbf5b9c7` | R3 | DATA-6 review R3: record a row when an Iceberg scan throws |
| `e103481d4` | R4 | DATA-6 review R4: test one copy stamp across every batch of a publish |
| `06bbadb12` | R5 | DATA-6 review R5: record D11 cleanup as not verified |
| `d9fc7a2c1` | R6 | DATA-6 review R6: require the latest-copy contract fields |

Commands run (scoped, on the shared `CARGO_TARGET_DIR=/home/rismana/cargo-target-1`;
`bun install` run once at the start — 678 packages):

- R1: `python3 -m py_compile ops/gold_export/gold_export_test.py` — OK.
  `cd rust && cargo fmt --check && cargo clippy -p lakehouse-api --all-targets -- -D warnings && cargo test -p lakehouse-api --lib gold_latest_copy` —
  fmt clean, clippy clean, **5 passed / 0 failed / 3753 filtered out**.
- R2 and R3: `python3 -m py_compile` on the touched Python files — OK.
  The new tests were not executed (pytest missing); fail-on-old-code was
  checked by reading the pre-fix branches at `HEAD` — the `==` case returned
  `superseded`, the reload error returned `failed`/`catalog_unavailable`,
  and both scans sat outside any `try`, so no record row was written.
- R4: `cd rust && cargo fmt --check && cargo clippy -p lakehouse-api --all-targets -- -D warnings && cargo test -p lakehouse-api --lib one_publish_stamps_every_batch_with_the_same_micros` —
  fmt clean, clippy clean, **1 passed / 0 failed / 3758 filtered out**.
  `git show 8d7b7c2b5^:rust/crates/lakehouse-api/src/gold_export.rs` has
  neither helper and recomputes the stamp per batch at second precision.
- R5: documentation only; no language check applies.
- R6: `bun run typecheck` — pass. `bun test src/lib/gold-freshness.test.ts src/features/catalog/open-format-card.test.tsx` —
  **37 pass / 0 fail / 94 expect() calls**. Temporarily reverting the three
  fields to optional made typecheck fail with three `TS2322` (plus three
  `TS2769`) errors in the new type guard; it passes on this commit.
- Final, on the head commit: `bun run typecheck` — pass; `bun run lint` —
  **0 errors, 11 warnings**, none in touched files;
  `python3 ops/lint/check_intra_package_imports.py` — OK;
  `python3 ops/lint/check_bare_iceberg_count.py` — OK.

Not verified:

- The R1 gate change was not run (no compose stack here):
  `python3 -m py_compile` passed, and the gate itself runs in CI on PR #612.
- No Dagster unit test was executed in this environment (pytest is not
  installed), including the R2 and R3 tests; see the static checks above.
- D11's end-to-end file removal (R5), as recorded in "Not verified" above.
- The §11 full workspace verification block: this brief asked for scoped
  checks only, so no `cargo test --workspace`, no `bun run test`, no full
  `docker compose up`.

### Fix round 2 (developer, 2026-11-20)

Round 2 findings R7 (BLOCKER) and R8 (SHOULD-FIX) fixed in
`ops/gold_export/gold_export_test.py`, one commit each, pushed in order.
Both cite the finding at the fix site.

| SHA | Finding | Commit subject |
| --- | --- | --- |
| `73bbea22b` | R7 | DATA-6 review R7: attribute the metadata log growth to the replace |
| `e8ab7ddbf` | R8 | DATA-6 review R8: check the current snapshot's operation, not a sorted list |

R7 took the second option the finding prefers: keep the whole baseline
`metadata-log`, then find the entry that introduced the new current
snapshot (the first new entry whose `timestamp-ms` is at or after the
current snapshot's) and assert the entry before it is the baseline's last
entry. A delete-then-append split would put an extra commit between them
and fail, independently of D11's expiry commit. All metadata is read
through the gate's existing `_load_lakekeeper_metadata` helper; no new
metadata-file reader was added. R8 replaced the sorted-list comparison
with an assertion on the current snapshot's `summary.operation`
(`append` or `overwrite`) and dropped the now-unused snapshot-id baseline.

Checks: `python3 -m py_compile ops/gold_export/gold_export_test.py` — OK
after each commit. The gate itself is not runnable here (no compose
stack); it runs in CI on PR #612. No other language was touched, so no
other scoped check applies.

## 13. Review

(empty — the planner writes findings here, tagged `BLOCKER` or `SHOULD-FIX`)

### Round 1 — planner review, 2026-11-20, at `4e5599629`

Holds: the copy stamp is computed once per publish under the export lock;
out-of-order runs are `superseded` and never move the table back; a failed
write leaves the old table intact; the trim runs on `gold.<mart>` only and
keeps the newest copy; no new route; every `ClickHouse` error is classified;
`latest_table` is `table + "_latest"` from a server-set name; the T3 move is
verbatim.

**R1 BLOCKER: the one-commit claim is written as measured, and nothing
measured it.** `docs/adr/0010-gold-export-to-iceberg-from-rust.md:221-225`,
`dagster/dispar_orchestrate/gold_latest_copy.py:9-11` and `:363`,
`rust/crates/lakehouse-api/src/gold_latest_copy.rs:18-19` cite T0, which
recorded no numbers (§12). The gate's AC2
(`ops/gold_export/gold_export_test.py:943`) samples `count()` every 0.2 s,
which would miss a delete-then-append split into two commits. Fix: in AC2,
read the latest table's metadata-log length and the new snapshots'
`summary.operation` with `_load_lakekeeper_metadata(f"{MART}_latest")`
before and after the replace, and assert the log grew by exactly 1. Then the
ADR and both module docs cite the gate (T6, AC2) instead of T0, and the
int-micros claim cites AC1. Principles 2 and 5.

**R2 SHOULD-FIX: a replace that committed is recorded as failed.**
`gold_latest_copy.py:410-419`: a reload error after the commit records
`failed`/`catalog_unavailable` and raises a retryable `Failure`; the retry
then records `superseded`, and the card's "Holds" shows the previous copy
while the table holds the new one. After a committed write, record `success`
(with `snapshot_id` null and the cleanup skipped, said in the row). Also
`:359`: a rerun for the copy already held (`==`) is the idempotent success,
not `newer_copy_present`; only `>` is superseded. Tests for both.

**R3 SHOULD-FIX: two reads can throw without a record row.**
`gold_latest_copy.py:356` and `:367-369` (`scan(...).to_arrow()`) sit outside
any `try`, so an object-store error leaves no `gold_latest_copy_run` row (D12).
Wrap both in the fixed-reason pattern (`catalog_unavailable`); test with a
fake that raises.

**R4 SHOULD-FIX: the DATA-62 fix has no test that fails on the old code.**
Nothing asserts that a publish of more than one batch writes one
`_exported_at` (`gold_export.rs:878`, `:1065`). Add a test with a small batch
size (a test-only override of the batch constant, or a unit test on the
function that stamps the batches) asserting one distinct stamp across
batches.

**R5 SHOULD-FIX: the cleanup (D11) is unproven.** The gate runs with the
default 3600 s orphan age, so it removes nothing. Either assert in the gate,
with a short age, that the old copy's files are gone, or write it under Not
verified in the handoff and in the feature page's limits.

**R6 SHOULD-FIX: contract fields optional for an "older API".**
`src/services/contracts/gold.ts:30`, `:37`, `:204`: `copyExportedAt`,
`latestCopy` and the publication's `latestCopy` ship with the console; make
them required (`AGENTS.md`, TypeScript: no optional-ing fields).

### Round 2 — planner review, 2026-11-20, at `6cc84b81d`

R2–R6 are fixed as asked. R1's direction is right, but the new gate check
is wrong in two ways:

**R7 BLOCKER: the gate counts the cleanup's commit as part of the
replace.** `ops/gold_export/gold_export_test.py` (the R1 block) reads
`metadata-log` after the job has finished, and the job's D11 cleanup
(`expire_all_but_current_snapshot`, `gold_latest_copy.py`) is itself a
catalog commit whenever an older snapshot exists. On any run after the
first, the log grows by 2 and the gate fails a correct replace (or, if the
fix is to accept 2, it can no longer tell a split replace from a cleanup).
Fix: tie the count to the run's own record. Read the run's
`gold_latest_copy_run` row; expected growth is 1, plus 1 when its `cleanup`
column says snapshots were expired. Better still, find the metadata-log
entry whose metadata holds the new current snapshot and assert that the
entry before it is the baseline (`log_before`'s last entry) — that proves
the replace was one commit regardless of what the cleanup did after. Pick
one, say which in the comment.

**R8 SHOULD-FIX: the operations check compares a sorted list with an
unsorted one.** `sorted(...)` of `["delete", "append"]` is
`["append", "delete"]`, which is not in the accepted tuple, so a
delete-then-append replace inside one commit would be refused. Compare
sets, or sort the accepted lists too. Note that after the cleanup only the
current snapshot survives, so the new-snapshot list is usually just the
current one; assert on the current snapshot's `summary.operation`
(`overwrite` or `append`) instead of on every new id.
