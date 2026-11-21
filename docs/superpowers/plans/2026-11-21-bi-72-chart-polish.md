# `BI-72`: chart polish from the acceptance run of 2026-11-21

Planner and reviewer: Claude Opus. Developer: Claude Sonnet. Base: `main`.
Branch: `fix/bi-72-chart-polish`. Owner approved on 2026-11-21 ("iya boleh
ini diperbaiki") after the acceptance run of `BI-16`/`BI-17`/`BI-18`
(`~/mtharreq/reports/2026-11-21-acceptance-bi-16-17-18-31.html`).

The run also reported "phone tile order differs from creation order". It
is not a fault: the phone stacks tiles in the desktop reading order (top
to bottom, then left to right, `src/lib/dashboard-tabs.ts` `readingOrder`
/ `phoneOrder`), checked on the run's board. Out of scope.

## Findings and anchors (main)

1. **A goal line's label shows its number twice** ("Goal 1,000 1,000")
   when the person's own label already holds it.
   `src/features/dashboards/chart-option.ts` ~730–746 (the line's name is
   the label as typed) and ~766–780 (`markOf`'s formatter appends the
   formatted value unconditionally).
2. **Two labels collide at the top of a value axis** ("15,7K" beside
   "15K"). `chart-option.ts` ~785–795 (`widen` / `topOf`) sets the axis max
   to the highest reference line's raw value, a non-round number, so
   ECharts draws a max label next to the last round tick. No value axis
   sets `axisLabel.hideOverlap` (only category axes do: ~162, ~315, ~415,
   `combined-option.ts` ~216, `style-option.ts` ~275), and the value axes
   use two formatters (`fmtCompact` ~156 vs `formatAxisValue` in
   `combined-option.ts` ~222).
3. **A web-page card whose site refuses framing is an empty white box**
   with only "Open in a new tab" (`src/features/dashboards/content-cards.tsx`
   ~40–63, `WebCard`; no load or error state; `ImageCard` ~22–36 has one).
   Browsers do not tell a page that a cross-origin frame was refused, so
   the console cannot see it; the server can, from the target's headers.

## Decisions (planner, 2026-11-21)

1. **The goal label carries its number once.** Append the formatted value
   only when the label does not already contain it: compare after
   stripping grouping separators and spaces, against both the formatted
   value (the chart's locale and format) and the plain number. A computed
   line (average, median, …) keeps "Average 1,234" as today. Tests: "Goal
   1,000" with value 1000 → "Goal 1,000"; "Goal" → "Goal 1,000"; "Q4
   target" → "Q4 target 1,000".
2. **A widened axis ends on a round tick, and value-axis labels never
   collide.** When a reference line widens the axis, set the max to the
   next "nice" step above it (1, 2, 2.5, 5 × 10ⁿ of the axis's interval),
   not the raw value, so the line sits inside the axis and no extra max
   label appears. Also set `axisLabel.hideOverlap: true` on every value
   axis (bar, line, area, combined, both sides). Tests: a 15,700 line on
   data up to 12,000 gives an axis max of 16,000 (or 20,000 by the nice
   rule), and every value axis option has `hideOverlap`.
3. **A web card says when its site refuses to be shown.**
   - **API:** `GET /api/dashboard/frame-check?url=` (needs
     `dashboard:read`; in `POLICY_TABLE` and asserted both ways in
     `tests/route_auth.rs`). It answers only for a URL on the content-sites
     allowlist (`routes/settings.rs` `content_sites`; otherwise 400 with one
     sentence). It dials through the same guarded path as the print view's
     image fetch (`routes/render.rs` `fetch_image` ~466 and the allowlisted
     resolver `connector_probe::resolve_checked`; read them and reuse, rule
     4): a `HEAD` (falling back to `GET` with the body discarded), no
     redirects followed, a 5 s timeout. It answers `{ "framable": true |
     false | null }`: `false` when `X-Frame-Options` is `DENY` or
     `SAMEORIGIN`, or CSP `frame-ancestors` is `'none'` or a list that does
     not include the console's own origin (`*` allows); `null` when the
     site could not be reached (no claim, principle 2). Remember answers
     per URL for 10 minutes in memory. Upstream errors never reach the
     response (SEC-11).
   - **Console:** `WebCard` asks once per URL (through `dashboardService`,
     `useService`). `false` → no iframe; the card shows one quiet line
     "This site doesn't allow being shown inside a dashboard." and the
     "Open in a new tab" link as a small outline button. `true` or `null`
     → the iframe as today. While asking, the card's normal placeholder.
   - Tests: Rust — `DENY`, `SAMEORIGIN`, `frame-ancestors 'none'`, a list
     with and without the console origin, no header → framable, a URL off
     the allowlist → 400, unreachable → `null`, auth both ways. Console —
     the refused state and the default state.

Out of scope: the phone order (see above), any other chart change.

## Checks (this VM)

**No `cargo build/check/clippy/test`** (OOM): only `rustfmt --edition 2024
--check <changed .rs>`; CI compiles and tests. Console: wrap bun in `flock
/tmp/claude-1000/lh-bun.lock systemd-run --user --scope -q -p MemoryMax=3G
-p MemorySwapMax=0 nice -n 15 …`; `bun x tsc --noEmit -p .`, `bun x eslint
<changed>`, `bun test <touched tests>`; no real fetch in tests. No
prettier, no full test/lint. CHANGELOG entry. Do not push.
