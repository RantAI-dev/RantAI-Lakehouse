# `DATA-10`: a "has this Gold table changed" signal that background merges and full rewrites do not fool

Planner and reviewer: Claude Opus. Developer: a non-Opus agent (`AGENTS.md`).
Base: `origin/main` at `8f93fc19`. Branch: `feat/data-10-gold-change-fingerprint`.
One PR slice.

## 1. The problem

Publishing a Gold table to open format skips the copy when the table has not
changed (`?ifChanged=true`, `DATA-1`). "Changed" today is
`max(modification_time)` over the table's active parts in `system.parts`
(`routes/gold.rs::mart_last_changed_at`), compared with the last successful
export's start (`gold_export_history::last_success_started_at`).

That signal moves when the content did not change:

- a background merge writes a new part with a new `modification_time`;
- a pipeline built in the console rewrites its whole target on every run
  (`authored_factory.py::_write_clickhouse_table`: staging table, then
  `EXCHANGE TABLES`), so every run looks like a change even with identical
  rows.

So the feature page's acceptance row 8 ("run again with no change → no new
copy") cannot pass, and storage grows by one full copy per run
(`features/gold-publish-per-mart.md`, Limits).

## 2. Decisions (planner)

- **D1. The signal is a content fingerprint, computed in ClickHouse.**
  One query over the mart:
  `count()`, `sum(cityHash64(*))` (UInt64 sum, wraps; order-independent,
  and unlike an XOR it does not cancel duplicate rows), and a hash of the
  column list (name and type, in position order, from `system.columns`).
  The fingerprint is one string: `"<count>:<sum>:<schemaHash>"`. Read every
  number with `toString(...)`, as `mart_last_changed_at` does, so no UInt64
  goes through a JSON float. Identifiers bound as `SqlLiteral` / quoted the
  way the existing gold queries do; no new `format!` of a user value.
- **D2. Store the fingerprint on the export history row.** The history is a
  ClickHouse table owned by `gold_export_history.rs`
  (`ensure_gold_export_run_table`). Add column `content_fingerprint String
  DEFAULT ''` with `ALTER TABLE ... ADD COLUMN IF NOT EXISTS` inside the
  ensure function, so existing deployments pick it up. No Postgres migration.
  The fingerprint is computed **before** the export reads rows, and written
  on the history row of that export. If the mart changes during the export,
  the stored value is stale and the next check exports again: the safe
  direction.
- **D3. Skip only on a measured match.** `?ifChanged=true` skips when the
  current fingerprint equals the fingerprint on the newest `success` row.
  Anything unknown exports: no successful export yet, an empty stored
  fingerprint (rows written before this change), a failed fingerprint read.
  A failed read is a classified `ApiError`, never a silent skip (same rule as
  today's `unchanged_since_last_export_reason`). Keep the decision a pure
  function with unit tests for each outcome, as `if_changed_skip_reason` is
  today; replace it, do not keep two signals.
- **D4. The console's status uses the same signal.** `publication_body` adds
  `"changedSinceExport": true | false | null` (null = not measured) computed
  from D3's comparison. Keep `lastChangedAt` and `lastExportedAt` in the
  response (the page shows them). The console's "Up to date" / "Out of date" /
  "Not measured" / "Never published" line reads `changedSinceExport` instead
  of comparing the two times. Contract type updated with `| string`-style
  widening rules from `AGENTS.md`.
- **D5. Cost, written at the code.** The fingerprint is a full scan of the
  mart. Gold marts are capped at `GOLD_EXPORT_MAX_ROWS` (5,000,000 by
  default). Measure it once on the compose stack with the largest table you
  can load and put the number, the row count and the machine in the module
  doc. Do not invent a number; if you could not measure, write "not measured".
- **D6. No assistant change.** This is not a BI feature; the assistant's
  tools for Gold publishing keep their schema. If a tool returns
  `lastChangedAt`-based status text, list it in the handoff; do not change it.

## 3. Anchors (`main` at `8f93fc19`)

- `rust/crates/lakehouse-api/src/routes/gold.rs`: `ExportQuery`,
  `if_changed_skip_reason`, `unchanged_since_last_export_reason`, `export`
  (its "Skip" doc section), `mart_last_changed_at`, `publication_body`.
- `rust/crates/lakehouse-api/src/gold_export_history.rs`:
  `ensure_gold_export_run_table`, `NewGoldExportRun`, `insert_sql`,
  `last_success_started_at`, `last_success_sql`.
- `rust/crates/lakehouse-api/src/gold_export.rs`: `export_mart`.
- `dagster/dispar_orchestrate/gold_export.py`: module doc (lines ~45-60 and
  ~455), the `skipped_verbs=["unchanged"]` path. Update the docs that say
  "`max(modification_time)`"; behaviour of the Dagster side should not need
  to change (it already sends `ifChanged=true` and reads `skipped`). Verify.
- `src/features/gold/` (status line) and its contract under `src/`.
- `ops/gold_export/gold_export_test.py` and the `gold-export` CI job.
- `docs/core/features/gold-publish-per-mart.md` (Limits; the planner edits
  it, not you).

## 4. Tasks (one commit each, in order)

- **T1. Fingerprint read + history column** (`gold_export_history.rs`,
  `gold.rs`), with unit tests for the SQL text and the parsing (including
  a UInt64 sum above 2^53 kept exact). *Check:* `cargo fmt --check`,
  `cargo clippy -p lakehouse-api --all-targets -- -D warnings`,
  `cargo test -p lakehouse-api gold`.
- **T2. D2 + D3: record on export, skip on match.** Replace the old skip
  rule; tests for: match → skip; differ → export; no success row → export;
  empty stored fingerprint → export; read error → classified error.
- **T3. D4: the API field and the console.** Rust field + test; TS contract,
  status component, its test. *Check:* `bun run typecheck && bun run lint &&
  bun run test`.
- **T4. The gate proves it.** Extend `ops/gold_export/gold_export_test.py`:
  export once; run `OPTIMIZE TABLE <mart> FINAL` (forces a merge, no content
  change) and call with `ifChanged=true` → `skipped: true`; rewrite the mart
  with the same rows via staging + `EXCHANGE TABLES` → skipped; insert one
  row → exported. Run the `gold-export` CI job's sequence locally from a
  clean project and quote its output.
- **T5. Docs:** Dagster module doc, `CHANGELOG.md`. Measurement from D5.

## 5. Out of scope

Removing old copies (`DATA-2`), a latest-copy view (`DATA-6`), storing Gold in
open format first (`DATA-3`), Silver publishing (`DATA-4`).

## 6. Verify, do not assume

1. Does `cityHash64(*)` accept every column type a Gold mart has here
   (Nullable, Decimal, DateTime64, LowCardinality)? Test each on the stack.
2. Is `sum` over UInt64 wrapping or widening on ClickHouse 26.8? Quote the
   result type (`toTypeName`).
3. Does `OPTIMIZE ... FINAL` change `modification_time` (proving the old bug)
   and leave the fingerprint equal? Quote both values.
4. Every reader of `lastChangedAt` in `src/` and in the assistant's tools.

## 7. Handoff

(Developer appends here.)

## 8. Review

(Reviewer.)
