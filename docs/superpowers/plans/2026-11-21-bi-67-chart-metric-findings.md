# `BI-67`: three small chart and metric findings

Planner and reviewer: Claude Opus. Developer: Claude Sonnet. Base: `main`
at the branch point. Branch: `fix/bi-67-chart-metric-findings`. Owner asked
for it on 2026-11-21. Findings from the owner acceptance run of 2026-11-16
(backlog row `BI-67`).

## Findings and anchors (main)

1. **Combined chart (BI-13): a layer-only category lands last.**
   `src/features/dashboards/combined-option.ts:119-126` `categoryKeys`: base
   keys first in base order, then each layer's unseen keys appended; it
   re-sorts only when every key is an ISO date. Bars by year 2020–2025 with
   a layer that also has 2019 draw 2019 after 2025, as a lone point.
2. **An alert whose metric's source is gone reports "skipped".**
   `rust/crates/lakehouse-alerts/src/lib.rs`: `metric_statement` (~:1884-1920)
   fails with "the metric's source is not available."; `run_alert`
   (~:3251-3288) maps it to a reason; `settle`'s `Err` arm (~:3173-3190) calls
   `state.record_failed` **and then returns `skipped(rule, reason)`**, so the
   run result (`AlertRunResult.skipped`, `src/services/contracts/alerts.ts:195`)
   says skipped. Chart rules share the path
   (`rust/crates/lakehouse-api/src/routes/chart_alerts.rs` ~:239).
3. **A SQL source a metric uses can be deleted.**
   `rust/crates/lakehouse-api/src/routes/dashboard_sources.rs` ~:385-413
   refuses only when a chart uses it (`sources::charts_using_source`,
   `rust/crates/lakehouse-bi/src/sources.rs:182-190`), then soft-deletes. A
   metric's `MetricDef { source_kind: SourceKind::SqlSource, source_id }`
   (`rust/crates/lakehouse-bi/src/metrics.rs:101-104`) is not checked. The
   metric delete guard (`routes/metrics.rs:586-613`, `metric_usage`
   :268-280) is the pattern: 409 naming what uses it.

## Decisions (planner, 2026-11-21)

1. **Ordered axes stay ordered.** In `categoryKeys`, after merging, sort the
   keys when every key is an ISO date (as today) **or every key is a
   number** (years, numeric buckets; compare numerically). Otherwise keep
   today's order (base order, then layer-only keys). Layer-only points stay
   on the axis (they are data); they just sit where they belong. Tests: the
   2019 case sorts first; mixed text keys keep base order; dates still sort.
2. **A rule that could not be evaluated is "failed", not "skipped".** Add
   `failed: Option<String>` to the run result (Rust `RunResult`, serialized
   as `failed`, and `AlertRunResult.failed?: string` in the contract).
   `settle`'s `Err` arm (a read or statement failure, which already calls
   `record_failed`) returns a result with `failed` set and `skipped` unset.
   Every deliberate skip (no state, owner disabled, not due, duplicate send,
   …) stays `skipped`. Where the console shows a run's outcome (find every
   reader of `skipped` in `src/`), a `failed` one reads "Failed: <reason>"
   in the destructive tone, a skip as today. Rust tests: a metric rule whose
   SQL source is gone reports `failed` with the source sentence; an existing
   skip test still reports `skipped`.
3. **Deleting a SQL source a metric uses is refused**, like a chart: before
   `delete_source`, list the metrics whose `source_kind` is `SqlSource` and
   `source_id` is this source (a `metrics_using_source` beside
   `charts_using_source`, or a shared helper; grep first, rule 4) and answer
   409 with one sentence naming them, the same shape as the chart refusal
   (charts and metrics together when both use it). Rust test: a source used
   only by a metric is refused with the metric's name; an unused one is
   deleted. The console's delete action already shows a 409's sentence
   (check; if not, show it).

Out of scope: marts (a mart is not deletable from the console), restoring a
deleted source, anything else in `BI-67`'s neighbours.

## Checks (this VM)

**No `cargo build/check/clippy/test` on the VM** (out of memory); only
`cargo fmt --check` / `rustfmt --edition 2024 --check` on changed files; CI
compiles and tests. Be careful with every call site and test that builds a
`RunResult` (grep `skipped(` and `RunResult {`). Console: wrap bun in
`flock /tmp/claude-1000/lh-bun.lock systemd-run --user --scope -q -p
MemoryMax=3G -p MemorySwapMax=0 nice -n 15 …`; `bun x tsc --noEmit -p .`,
`bun x eslint <changed>`, `bun test` on `combined-option.test.ts` and the
alert UI tests you touch. CHANGELOG entry.

## Handoff

Developer: Claude Sonnet. Branch `fix/bi-67-chart-metric-findings`, not pushed.

### Commits

1. `904ab2d73` decision 1: `categoryKeys` sorts numerically when every key is a strict number (`/^-?\d+(\.\d+)?$/`); dates still sort as before; mixed keys keep base order.
2. `5efb6dd8d` decision 2: `failed` on the run result, console shows "Failed: <reason>".
3. `b1d1b6e75` decision 3: a SQL source a metric uses is refused with 409.

Each commit also has a CHANGELOG entry under `[Unreleased]` / `### Fixed`.

### Commands run (VM, scoped)

- `bun test src/features/dashboards/combined-option.test.ts`: 12 pass, 0 fail (2 new).
- `bun x eslint` on the changed dashboards files: clean.
- `bun x tsc --noEmit -p .`: clean (run once, on the decision 2 tree).
- `bun x eslint` on the alert files: 0 errors, 1 warning (`react-hooks/exhaustive-deps` on `rules` in `alerts-page.tsx`, present before this change).
- `bun test src/features/alerts/run-result.test.ts src/features/alerts/alerts-page-test-run.test.tsx`: 6 pass, 0 fail (3 new in `run-result.test.ts`).
- `bun test src/features/alerts/alerts-page.test.tsx`: 36 pass, 0 fail.
- `rustfmt --edition 2024 --check` on every changed `.rs` file: clean.

### Every place changed for the new `failed` field

- `rust/crates/lakehouse-alerts/src/lib.rs`: field `RunResult::failed` (`skip_serializing_if = "Option::is_none"`, doc says never set with `skipped`); `failed: None` added to all five `RunResult { .. }` literals (`settle` x2, `run_digest`, `run_freshness` x2) and to `skipped()`; new `fn failed(rule, reason)`; `settle`'s read-failure `Err` arm returns `failed(..)` (still calls `record_failed` first). `metric_value` doc says "failed". The other `settle` `Err` arms (state unreadable, claim failed, claimed elsewhere), `stop`, the gate `Stop`, the digest restriction, freshness, and the event-driven kinds stay `skipped`.
- Rust tests updated from `skipped` to `failed` where the path goes through `settle`'s read-failure arm: gate-refused (`REFUSED`), metric gone, metric store down, the demo-metric-gone one, "the value could not be read." New test `a_metric_whose_sql_source_is_gone_reports_failed_with_the_source_sentence`; the metric-gone test was renamed `..._fail_with_a_fixed_reason_...`. Existing skip tests (Stop, restricted digest, freshness, chart rule via `run_one`, state unreadable, claimed elsewhere) are unchanged and still assert `skipped`.
- `rust/crates/lakehouse-api`: no non-test code builds or reads a `RunResult` field that needed changing (`alerts.rs` only assigns `result.skipped` for `PipelineLate`; `chart_alerts.rs` goes through `settle`). Tests changed to read `failed`: `routes/alerts.rs` (tenant-scope refusal, `JOB_NEEDS_CATALOG_TENANT`), `routes/ai/tools/alerts.rs` (chart rule that cannot be read; SEC-20 refusal), `tests/ai_alert_tools.rs` (chart_number sentence, and the exact key set now `failed, fired, id, name, type`).
- `src/services/contracts/alerts.ts`: `AlertRunResult.failed?: string`.
- `src/features/alerts/run-result.ts` (new, `formatRunResult` moved out of `alerts-page.tsx`, "Failed: <reason>"), `run-result.test.ts` (new), `alerts-page.tsx` (icon and text in the destructive tone for `failed`; skip keeps amber).
- No fixtures changed: `tests/fixtures/tool_schemas.json` does not describe the run result, and grep found no JSON fixture with a run result's `skipped`.

### Decision 3 details

- `lakehouse_bi::sources::metrics_using_source(&[MetricDef], id) -> Vec<&str>` beside `charts_using_source` (only `SourceKind::SqlSource` with that `source_id`), with a unit test.
- `routes/dashboard_sources.rs::delete` reads the metric list (a read failure refuses, fail closed) and builds the 409 with `in_use_message(charts, hidden_charts, metrics)`. Charts only: same wording as before except the sentence now always ends with a full stop (before, only the hidden-dashboard one did). Metrics are named for everyone (they are not per-person).
- Route tests in `routes/organise_paths.rs` (beside the hidden-chart one): metric-only use is refused with exactly "SQL source is still used by 1 metric(s): orphaned." and no write; a source with only a metric on another source or a mart metric of the same id is deleted (an `INSERT ... is_deleted` is sent).
- Console check: no console screen deletes a SQL source (the only caller of `DELETE /api/dashboard/sources` is `deleteSqlSource` in `src/services/clients/dashboards.ts`, which has no UI consumer), and that client already throws a `ServiceError` carrying the server's 409 sentence. Nothing to change there.

### Not verified

- **All Rust is uncompiled** (no `cargo build/check/clippy/test` on this VM, by instruction). Only `rustfmt --check` ran. Risks I could not rule out: clippy pedantic on the new code; the wiremock-based tests (the `bi_metric` row shape, mock ordering in the new alerts test, and the `organise_paths` engine fallthrough returning no rows for the other statements); `outcome.is_ok()` in the new route test.
- The two new alerts-crate behaviours depend on `lakehouse_bi::metrics::list_metrics` and `get_source` going through the mocked ClickHouse as I read them; CI is the first run.
- No full `bun run test` / `bun run lint`; no browser check of the "Failed:" row.
