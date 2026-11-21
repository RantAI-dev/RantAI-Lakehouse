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
