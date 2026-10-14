# `DATA-2`: a published Gold table keeps its last 7 copies, the rest are removed

Planner and reviewer: Claude Opus. Developer: a non-Opus agent (`AGENTS.md`).
Base: `origin/main` at `caf5784b` (after `DATA-10`, PR #156). Branch:
`feat/data-2-gold-keep-copies`. One PR slice.
Spec: `docs/core/specs/data-2.md` (product owner signed 7 copies, operator
setting, at least 1, on 2026-10-14).

## 1. The problem

Each publish of a changed Gold mart appends a full copy to the Iceberg table
`gold.<mart>`, stamped `_exported_at`, partitioned `day(_exported_at)`
(`lakehouse-iceberg/src/gold.rs`, `EXPORTED_AT_COLUMN`,
`export_day_partition_spec`). Nothing removes old copies
(`features/gold-publish-per-mart.md`, Limits). The Rust Iceberg library here
(`iceberg` 0.10.1) can append and expire snapshots but cannot delete rows or
overwrite. PyIceberg 0.12.0 is already a pinned dependency of the Dagster
code location (`dagster/pyproject.toml`) and has `Table.delete` and
`Table.maintenance` (checked by the planner in the code location's venv).

## 2. Decisions (planner)

- **D1. Where and when.** The trim is a new op in the Dagster gold export
  job (`gold_export.py`), run for a mart right after that mart's export
  succeeded. A skipped ("unchanged") or failed export does not trim. A manual
  "Publish now" from the console calls the API, not this job, so it does not
  trim; the next job run does. Write that limit on the feature page's
  Limits (the planner does, after merge).
- **D2. Credential.** The trim writes to the `gold` namespace as the existing
  `gold-export` Lakekeeper principal (ADR 0011): its pre-minted token
  `gold-export.jwt` on the shared token volume, mounted READ-ONLY into
  `dagster-code-location`, path in `LAKEKEEPER_GOLD_EXPORT_TOKEN_FILE`, as the
  API's mount does. No new principal, no new grant, no admin token. If that
  principal cannot delete rows or expire snapshots, STOP and report (T1).
- **D3. What is removed.** Keep the newest `N` distinct `_exported_at`
  values. Delete every row with `_exported_at` older than the `N`-th newest,
  in one `Table.delete` with a row filter on `_exported_at`. Then expire the
  snapshots that only the removed copies needed, so their data files are
  deleted; keep the snapshot of the current state. Never delete anything when
  the table has `N` or fewer copies.
- **D4. The setting.** `GOLD_KEEP_COPIES` on `dagster-code-location`,
  compose `${GOLD_KEEP_COPIES:-7}` (a count, not a credential, so a dev
  default is allowed). Parsed as an integer >= 1. Anything else fails the
  trim closed with the fixed reason `"invalid GOLD_KEEP_COPIES"`; nothing is
  deleted.
- **D5. The record.** Each trim writes one row to a new ClickHouse table
  `console.gold_trim_run` (created with `CREATE TABLE IF NOT EXISTS` by the
  writer, like `console.gold_export_run`): mart, started/finished,
  `status` (`success` / `failed` / `nothing_to_trim`), `copies_kept`,
  `oldest_kept_exported_at`, `copies_removed`, `snapshots_expired`, and a
  `reason` from a closed set of fixed strings, never exception text
  (principle 4). The detail of a failure goes to the run log.
- **D6. Shown.** The API's publication detail (`routes/gold.rs`,
  `publication_body`) adds `"trim": { "status", "copiesKept",
  "oldestKeptExportedAt", "finishedAt", "reason" } | null` from the newest
  `gold_trim_run` row; `null` means never trimmed. The console's publish card
  shows "Copies kept: N (oldest <time>)" and a failed trim's reason; never
  "trimmed" for a failed or missing row.
- **D7. No assistant change** (not a BI feature).

## 3. Anchors (`main` at `caf5784b`)

- `dagster/dispar_orchestrate/gold_export.py`: `export_gold_mart` op and the
  job graph; `GoldExportConfig.from_env`.
- `dagster/dispar_orchestrate/dlt_pipeline.py`: how a `RestCatalog` is built
  against Lakekeeper (base URI, warehouse, token).
- `dagster/dispar_orchestrate/bronze_catalog.py`: `ClickHouseTarget`,
  `_ch_exec` (writing a `console.*` row).
- `rust/crates/lakehouse-iceberg/src/gold.rs`: `_exported_at`, partition spec.
- `rust/crates/lakehouse-api/src/gold_export_history.rs`: the
  `console.gold_export_run` pattern to mirror for reading `gold_trim_run`.
- `rust/crates/lakehouse-api/src/routes/gold.rs`: `publication_body`.
- `src/features/gold/` and `src/features/catalog/open-format-card.tsx`.
- `docker-compose.yml`: `dagster-code-location` (env, volumes), the token
  volume mount the API uses.
- `ops/gold_export/gold_export_test.py` and the `gold-export` CI job.

## 4. Tasks (one commit each, in order)

- **T1. Spike, measured on the compose stack, before any product code.**
  With the `gold-export` token, from inside `dagster-code-location`, against
  a Gold table with 9 copies: `Table.delete` on `_exported_at` < the 7th
  newest, then expire snapshots, then list the table's data files in the
  object store. Record in the handoff: the PyIceberg calls used, the row
  count before and after, the snapshot count before and after, and whether
  the removed copies' data files are gone. Commit only the script under
  `ops/gold_export/` if it becomes T5's gate step; otherwise nothing. **If
  any step is refused or the files remain, STOP and report.**
- **T2. D2 + D4: compose and config.** Token mount and env on
  `dagster-code-location`; `GOLD_KEEP_COPIES` parsing with unit tests (`7`,
  `1`, `0`, `-1`, `x`, empty).
- **T3. D1 + D3 + D5: the trim op**, with unit tests (no network; PyIceberg
  and ClickHouse mocked): 9 copies keep 7; 7 copies delete nothing and
  record `nothing_to_trim`; a delete failure records `failed` with a fixed
  reason and leaves the export's own result untouched; a skipped export does
  not trim.
- **T4. D6: API field and console**, with Rust tests for the field (null,
  success, failed) and console tests (`src/features/gold/`,
  `src/features/catalog/` folders only).
- **T5. The gate proves it.** Extend `ops/gold_export/gold_export_test.py`:
  publish 9 changed copies through the job path, then assert 7 copies remain
  (the newest 7), the object store holds no data files of the removed ones,
  and the publication detail shows `copiesKept: 7`. Run the `gold-export` CI
  job's sequence locally from a clean project; quote the output and wall
  time. Raise that job's `timeout-minutes` only by measured time plus 5.
- **T6. Docs.** `CHANGELOG.md`, module docs. Do NOT edit `docs/core/`.

## 5. Out of scope

`DATA-6` (the `_latest` table), keeping copies by age, trimming from the
console's manual publish (D1), Silver (`DATA-4`).

## 6. Verify, do not assume

1. Does `Table.delete` on a `day(_exported_at)` table drop whole files for
   whole-day partitions and rewrite only a partially kept day? Quote what
   PyIceberg did (new snapshot's summary).
2. Which PyIceberg 0.12 call expires snapshots, and does it delete data
   files or only metadata? If only metadata, what deletes the files?
3. Can the Rust export append to the table while a trim runs? The API's
   per-mart lock (`gold_lock`) does not cover Dagster; say what happens on a
   commit conflict and make the trim retry or fail cleanly, never corrupt.
4. Does an outside reader that filters on `max(_exported_at)` still see the
   same rows after a trim?

## 7. Handoff

(Developer appends here, task by task.)

## 8. Review

(Reviewer.)
