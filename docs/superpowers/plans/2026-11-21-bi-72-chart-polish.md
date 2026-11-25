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

## Handoff

Developer: Claude Sonnet. Branch `fix/bi-72-chart-polish`, not pushed.

### Commits

1. `7b712e061` BI-72 fix(dashboards): a goal label carries its number once
2. `7ab407042` BI-72 fix(dashboards): a widened axis ends on a round tick, value labels never collide
3. `7e985256f` BI-72 feat(dashboards): a web card says when its site refuses to be shown

Each commit carries its CHANGELOG line (Unreleased, Fixed).

### What was done, and where it differs from the plan's wording

- **Decision 1:** `carriesNumber` in `chart-option.ts` compares the label, with grouping separators and spaces stripped, against the formatted and the plain number. Computed lines keep their suffix (the check needs `p.name`, which a computed line always has too, so "Average 1,234" is unchanged only because its name never holds the number: not separately tested).
- **Decision 2:** `niceCeil` sets the widened max to the next 1, 2, 2.5 or 5 x 10^n step, sized for about five intervals from the axis minimum; a 15,700 line over data to 12,000 gives 20,000 (the plan allowed 16,000 or 20,000). A data max above the line is kept. `hideOverlap: true` on the value axes of `chart-option.ts` (the two builders, bar/line/area and hbar) and `combined-option.ts` (both sides). Value axes of other kinds (gauge, scatter, type-b, gantt) were not touched (out of the plan's list).
- **Decision 3:**
  - `rust/crates/lakehouse-api/src/routes/frame_check.rs` (new), route in `routes/mod.rs`, `POLICY_TABLE` entry `RequiresPermission("dashboard:read")`.
  - Rule 4: the print view's image fetch built a pinned client inline; it is now `connector_probe::pinned_https_client` (+ `PinRefusal`), used by `render.rs::fetch_image` and the new route. This edits `fetch_image` (same refusals and fixed sentences, no behaviour change intended); the reviewer should read that hunk.
  - `null` is also answered for a redirect, an error status, an unresolvable or refused host, and a `frame-ancestors` list when `CONSOLE_PUBLIC_URL` is unset (only `*` and `'none'` can be judged without it). `X-Frame-Options: SAMEORIGIN` is a refusal unless the site is the console's own origin.
  - Cache: process-wide `LazyLock<Mutex<HashMap>>` (10 min, 512 entries, cleared when full), not a field of `AppState`.
  - Console: `frameCheck` in the dashboards contract and client (any non-OK answer, including 401 on a public link or embed, is `null`), `WebCard` asks through `useService`; refused shows one line plus an outline `Button` (`render={<a/>}`, so it has the app's hover, press and focus states).
  - The new route needs a session, so on a public link or a signed embed the card behaves as before (frame shown). Not extended to those paths (outside the plan).

### Commands run (this VM)

- `rustfmt --edition 2024 --check` over `frame_check.rs`, `connector_probe.rs`, `render.rs`, `routes/mod.rs`, `policy.rs`, `tests/frame_check.rs`, `tests/route_auth.rs`: clean after one `rustfmt` run on the new files.
- `bun test src/features/dashboards/chart-option.test.ts src/features/dashboards/combined-option.test.ts`: 33 pass, 0 fail (with 3 new tests).
- `bun test src/features/dashboards/content-cards.test.tsx src/features/dashboards/type-b-tiles.test.tsx src/features/dashboards/render-view.test.tsx`: 41 pass, 0 fail (4 new tests in `content-cards.test.tsx`; `type-b-tiles.test.tsx`'s web card tests now spy on `dashboardService.frameCheck` and wait for the frame).
- `bun test src/services/clients/dashboards.test.ts`: 39 pass, 0 fail.
- `bun x tsc --noEmit -p .`: no errors. `bun x eslint` over every changed `.ts`/`.tsx`: no output.
- All bun runs under `flock` + `systemd-run` (MemoryMax 3G). `node_modules` was a symlink, removed before each commit; not committed.

### Not verified

- **No Rust was compiled or run** (`cargo build/check/clippy/test` forbidden on this VM). CI is the first compile of `frame_check.rs`, the `pinned_https_client` refactor, `tests/frame_check.rs`, the `route_auth` test and the `wiremock` unit tests. Likely places for a first-compile error: `reqwest::header` imports, the `match (Origin::parse(..), pinned_https_client(..).await)` in `frame_check`, `Err(not_allowed().into())`, and clippy pedantic on `frame_check.rs`.
- `tests/frame_check.rs` relies on `https://frame-check.invalid` never resolving, and on the settings save accepting `contentSites` with a `CH_URL` mock answering zone count 1 (copied from `reporting_settings_route.rs`).
- No `sec11_guard`, `bun run lint`, full `bun run test` or `bun run typecheck` run; no browser check of the card or the chart labels; the assistant-parity part (tool schema) is not affected (no assistant tool changed).
- `docs/core/features/*` pages not updated for the new route (not in the plan).
