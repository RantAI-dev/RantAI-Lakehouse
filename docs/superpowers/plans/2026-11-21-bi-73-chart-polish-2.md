# `BI-73`: chart and alert polish from the `BI-56` acceptance run

Planner and reviewer: Claude Opus. Developer: Claude Sonnet. Base: `main`
(`969eda399`). Branch: `fix/bi-73-chart-polish-2`. Owner approved findings a–f
on 2026-11-21 ("oke setuju dengan saran mu") after the `BI-56` acceptance run
(`~/mtharreq/reports/2026-11-21-acceptance-bi-56.html`, gallery board
`qa BI-56 semua chart` on the QA stack, one tile per chart kind).

`#651` (`BI-72`) also edits `chart-option.ts` (value axes, goal labels) and is
not merged yet. Keep this branch's hunks away from those lines where you can;
whichever merges second resolves the overlap.

## Findings and anchors (main, `969eda399`)

a. **Category labels go missing on a narrow tile.** A bar chart on a 328 px
   tile shows 4 of 6 category labels, a box plot 3 of 6, so a bar cannot be
   named without hovering. `src/features/dashboards/chart-option.ts:162`
   (`interval: 0, hideOverlap: true, rotate: data.length > 6 ? 28 : 0`) and
   ~414–416 (same pattern): `hideOverlap` drops labels that collide, and with
   6 or fewer categories nothing rotates. Box plot: find its category axis
   (`chart-option.ts` or `type-b` options).
b. **Rotated category labels are clipped at the left edge** (stacked bar,
   combo): the grid does not leave room for the first rotated label.
   `chart-option.ts` grid / `containLabel`, `combined-option.ts` ~216.
c. **A line over unordered categories is smoothed**, so it dips between
   points that have no order (combo and line on regions). `chart-option.ts:275`
   (`smooth: true` on the combo's line), ~443, ~500, ~526 (`smooth: isLine`).
   `combined-option.ts:245` already uses `smooth: false`.
d. **Gantt: the "Today" label sits on the date axis labels.**
   `src/features/dashboards/gantt-option.ts:204`
   (`label: { formatter: "Today", … }` of the today mark line).
e. **The alert e-mail ignores the console's number format and opens with an
   emoji.** A rule fired on 8,330,172 (workspace number locale `en-US`) is
   written "8.330.172" in the body, and the subject is "⚠️ Alert: …".
   `rust/crates/lakehouse-alerts/src/lib.rs:3344` (subject), the body
   builder near it, and `:2742` (`"⚠️ Pipeline behind target"`); find where the
   value is formatted.
f. **The density map has no tooltip.** `src/features/dashboards/geo-option.ts:162`
   (`tooltip: { show: false }` on the geoheat option).

## Decisions (planner, 2026-11-21)

1. **(a) Every category label is readable or reachable.** When labels would
   collide, rotate them (about 30°) before hiding any, at any category count;
   when even rotated labels collide (many categories on a narrow tile), keep
   `hideOverlap` and truncate long labels with an ellipsis at a fixed width,
   the full name in the tooltip. Same rule on the box plot's category axis.
   Horizontal bars are not rotated (their labels are on the value side).
   Test: 6 categories on a narrow width produce a rotate, and every label is
   kept.
2. **(b) Rotated labels are not clipped.** The grid leaves room for the
   rotated labels (`containLabel: true` or an explicit left/bottom margin
   computed from the rotation), on bar, stacked, area, line, combo and
   combined. Test: the option for a rotated axis sets the containing grid.
3. **(c) Smooth only an ordered axis.** A line or area is smoothed only when
   its category axis is ordered: dates, months, years or other numbers (reuse
   the ordered-key check that `combined-option.ts` `categoryKeys` already
   uses after `BI-67`, or the date check in `chart-option.ts`; grep first,
   rule 4). Unordered text categories draw straight segments. Tests: regions
   → `smooth: false`; years → `smooth: true`.
4. **(d) "Today" never covers the axis.** Put the label at the top of the mark
   line (`position: "insideEndTop"` or `end`), inside the plot, not over the
   axis labels. Test: the mark line label position.
5. **(e) E-mails speak the console's numbers, without emoji.** The alert
   e-mail body formats values with the workspace number locale from
   Settings → Reporting (`numberLocale`, `en-US` or `id-ID`), the same rule
   the console uses (find how reports' e-mails or `lakehouse-store`
   reporting settings expose it; grep before writing a formatter, rule 4).
   Subjects drop the emoji: "Alert: <rule name>" and "Pipeline behind
   target". Rust tests: `en-US` → "8,330,172", `id-ID` → "8.330.172", the
   subject has no emoji. Update any test that asserted the old text.
6. **(f) The density map has a tooltip** like the region and point maps: the
   place (or coordinates when there is no name) and the value, formatted as
   the chart's other values. Test: the geoheat option has `tooltip.show`.

Out of scope: the Sankey label on a phone, the pivot's in-tile scroll on a
phone, maps filling half their tile, the viewer's settings menu (owner did not
pick them), and anything `BI-72` already changes.

Assistant parity: no tool or schema change (rendering and e-mail text only).

## Checks (this VM)

**No `cargo build/check/clippy/test`** (OOM): only `rustfmt --edition 2024
--check <changed .rs>`; CI compiles and tests. Console: wrap bun in `flock
/tmp/claude-1000/lh-bun.lock systemd-run --user --scope -q -p MemoryMax=3G
-p MemorySwapMax=0 nice -n 15 …`; `bun x tsc --noEmit -p .`, `bun x eslint
<changed>`, `bun test <touched tests>`. No prettier, no full test/lint. One
commit per decision group (console a–d+f, e-mail e) with CHANGELOG lines
(Unreleased, Fixed). Do not push.

## Handoff

Developer: Claude Sonnet. Commits: `b41671cf0` (console, a-d and f), `e056b0629` (e-mail, e). Not pushed.

Commands run (this VM, under the bun wrapper):

- `bun test` on category-labels, chart-axis, gantt-option, chart-option, combined-option, echart-stable, geo-chart, style-option, combined-tile: 102 pass, 0 fail.
- `bun x tsc --noEmit -p .`: no output (clean). `bun x eslint` on the changed `.ts`/`.tsx` files: no output (clean).
- `rustfmt --edition 2024 --check` on `lakehouse-alerts/src/lib.rs`, `routes/chart_alerts.rs`, `routes/alerts.rs`: clean.

What differs from the plan's wording:

- (a, b) `containLabel: true` was already set on every grid, so the clipping is not a missing `containLabel`. The fix is a new `features/dashboards/category-labels.ts`: builders draw the category axis unrotated (`categoryAxisLabel`), and the chart wrapper (`echart.tsx`) calls `fitCategoryLabels(option, tileWidth)` on apply and on resize. Colliding labels rotate 30 degrees, long ones are cut at 96 px with an ellipsis, and the grid's `left` gets extra room for the rotated first label. The old fixed rule (`rotate` only above 6 categories) is gone; width is estimated at 6.4 px per character, so it is a heuristic. The box plot, heatmap, combo, combined and every bar/line/area/stacked axis use it; horizontal bars are untouched. The histogram (`type-b-option.ts`) keeps its own rule (not in the plan's list).
- (c) The ordered-key check the plan mentions (`categoryKeys`) only sorts ISO dates and has no reusable predicate, so a new `isOrderedAxis` is in `lib/chart-axis.ts` (dates, months, years, quarters, weekdays, hours, numbers).
- (d) The label is `position: "end"` (top end of the vertical line), and the Gantt grid top grows by 8 px when the line is drawn so the label has room.
- (f) A heat layer has no shapes to hover, so besides `tooltip.show: true` an invisible scatter point per row (same data, with the row's label) carries the tooltip.
- (e) The plan's anchors (`:3344`, `:2742`) were `:3198` and `:2606` on this base. Numbers come from `lakehouse_bi::export_format::format_text` (the export formatter, `number` style, 0 decimals); the locale comes from `lakehouse_store::reporting_settings` via `routes::settings::format_settings(..).number_locale`, read once per run by the new `chart_alerts::number_locale` (falls back to `en-US` and logs when unreadable). The old `fmt_id_id` is removed. The digest's tile numbers use the same locale. `Reading::against_threshold`, `run_rules`, `run_one`, `run_alert`, `run_digest`, `digest_text` gained a locale; `run_rules` takes `(silence, state)` as a tuple and `run_one`/`run_alert`/`run_digest` take `(rule, locale)` to stay within `too-many-arguments-threshold = 8`. The recovery subject also lost its emoji ("Back to normal: <name>"), which the plan did not name; the digest subject keeps its emoji.

Not verified: all Rust compilation, clippy and Rust tests (CI only; new Rust tests: `fmt_number_*`, `a_reading_writes_its_numbers_in_the_workspace_locale`); no existing Rust assertion on the old emoji subjects or `fmt_id_id` text was found by grep, but a compile was not run. The browser look (label rotation thresholds, left room, ellipsis, Today label placement, the density tooltip actually firing on hover, the resize refit) was not seen. No Rust test checks the API reading the locale from Settings.
