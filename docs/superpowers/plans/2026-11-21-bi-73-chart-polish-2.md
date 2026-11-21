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

(developer appends here)
