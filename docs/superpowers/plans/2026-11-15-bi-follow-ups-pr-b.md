# BI follow-ups, PR B: handoff

The plan is `docs/superpowers/plans/2026-11-13-bi-follow-ups.md` on PR #527's branch (`fix/bi-export-safety`).
This file holds PR B's handoff only, because the plan file is not on `main`.

## Handoff, PR B (developer, 2026-11-15)

Branch `fix/bi-reports-render`, base `d72bf4ccd` (`origin/main`, 2026-11-15). Not pushed; no PR opened.
All commits as `mtharreqq <72479829+mtharreqq@users.noreply.github.com>`.

| Commit | Item |
| --- | --- |
| `af26494ff` | `BI-35`: Send now rate limit, a run past its budget recorded, dialog on `useServiceAction`, Delete confirms |
| `28b56accc` | `BI-36`: the print view reads an uploaded map's boundary through the render token |
| (none) | `BI-42`: **stopped, plan mismatch, see below** |

### BI-35

- **Self-approval: no change needed.** `SEC-57` (#342, 2026-11-02) already makes the requester unable to
  decide their own approval (`routes::agents::refuse_own_request`, on both decide routes). The report tools
  create their approvals through the same `request_approval`, so the rule covers them. The plan's anchor
  (written from the 2026-10-18 review) is stale. A test now pins it for the report tools
  (`ai_report_tools.rs`).
- **D5 values.** Send now: one press per subscription per **60 s**, configured by
  `REPORT_SEND_NOW_INTERVAL_SECS` (`Config::report_send_now_interval_secs`, `0` turns it off, an unparsable
  value falls back to 60; documented in `docs/CONFIGURATION.md`). The count is in process memory
  (resets on restart, per replica); a press inside the interval is a 429 with a fixed sentence and a
  `Retry-After` (whole seconds, at least 1). The claim is taken before the report is made.
- **Cut run.** `send_now` stops the run at **300 s** (`SEND_NOW_BUDGET`, a compile-time assert keeps it
  under the route's 310 s `EXPORT_REQUEST_TIMEOUT`) and calls `report_runner::record_cut_run`: a failed
  manual delivery with a fixed reason (`RUN_TOO_LONG`) and an audit row `report.send_now`, outcome
  `failed`. The scheduled runner has no such bound; only the route budget was in the item.
- **Console.** Row actions (send, pause, delete) and the form save use `useServiceAction`. Delete opens
  `ConfirmActionDialog` naming the subscription (its schedule sentence and the dashboard).
  The recipient search in the form still calls `reportService.people` from an effect (it is a debounced
  read with its own abort, not an action); left as it is.
- **Files.** `config.rs`, `routes/reports.rs`, `routes/report_runner.rs`, `docs/CONFIGURATION.md`,
  `subscriptions-dialog.tsx` and its test, tests in `report_routes.rs`, `report_runner.rs`, `ai_report_tools.rs`.

### BI-36

- New `POST /api/render/maps/{id}/geojson` (body `{ "token" }`), `Policy::Public` (public count 15 to 16, the
  policy test updated, the no-credentials walk in `route_auth.rs` names it). The render token's admission
  moved into one function, `render::admit`, shared with `/api/render/data`. The map is served only when a
  chart on the token's board draws on it (`embed::map_for_board`, now `pub(crate)` with a `Viewer`
  argument; embed and public link pass `Viewer::anonymous()` as before, the print view the person the
  token names). A chart token (PNG) opens its board's maps, not only its chart's map: a map is a boundary
  file any signed-in person can read.
- `render-view.tsx` passes a `loadMap` that posts the token to that route.
- `docs/core/PRODUCT.md`: the two "may miss an uploaded map" statements removed.

### BI-42: stopped

The plan says the render token carries the board's content sites "so image and web cards draw in a PDF or a
PNG". That is not enough on this code. `docker-compose.yml` starts Gotenberg with
`--chromium-allow-list=^<CONSOLE_INTERNAL_URL>(/|$)` (feature decision 3 in
`docs/core/features/export-pdf-excel.md`): Chromium in that container may open **only** the console, a
startup flag. So the page's `Content-Security-Policy` is not what blocks an image card in a PDF; the
allow-list would still refuse the fetch after the policy allowed it. The content sites are a runtime
setting (Admin > Settings), the flag is fixed at container start.

What was built and measured, then set aside (not committed; patch at
`/tmp/claude-1000/-home-hv-mtharreq/216f3aeb-6ca2-4979-892a-b9e8e9cbae0f/scratchpad/bi42-wip.patch`, 842 lines):
`POST /api/render/frame` (token to the board's own content sites, read as the token's person, empty for any
refusal), `proxy.ts` asking it for `/render/*` paths (`src/lib/render-frame.ts`), image and web cards drawn
in the print view (eager loading, waited for like a chart), tests for each. TS tests pass (51 across four
files); Rust not compiled.

Decision needed from the planner and owner:
1. Keep feature decision 3 (placeholder "Shown in the console only") and close `BI-42` as won't-do, or
2. widen the Gotenberg allow-list: how, given the sites are runtime settings? (an env list added to the
   flag, which needs a restart, or a console-side proxy for the card's resources); then the patch above
   completes the policy half.

### Commands actually run (no Rust compiles, by rule)

- `systemd-run --user --scope -p MemoryMax=3G -p MemorySwapMax=0 nice -n 15 bun install --frozen-lockfile`: 678 packages.
- `bun test src/features/dashboards/subscriptions-dialog.test.tsx` (wrapped as the plan says): 16 pass, 0 fail.
- `bun test src/features/dashboards/render-view.test.tsx`: 12 pass, 0 fail (BI-36 commit tree).
- `bun run typecheck` (wrapped): clean, run on the BI-35 tree and on the BI-36 tree plus the BI-42 work.
- `npx eslint` on the changed dialog, render-view and test files: no output.
- `cargo fmt --check` in `rust/`: clean after each commit.
- `bash scripts/ci/check_test_modules.sh`: passed (all new Rust tests are in existing test modules).

### Not verified

- **Rust: nothing was compiled, no clippy, no test run** (`cargo build/check/clippy/test` are forbidden on this VM).
  Every Rust change and test is unverified until CI: `config.rs` (new field and test), `routes/reports.rs`
  (`SendNowClaims`, `send_now`, the unit test), `report_runner.rs` (`record_cut_run`), `routes/render.rs`
  (`admit`, `map_geojson`), `routes/embed.rs`, `policy.rs`, `routes/mod.rs`, and the new tests
  (`report_routes.rs`, `report_runner.rs`, `ai_report_tools.rs`, `maps_routes.rs`, `route_auth.rs`).
- Likely CI trouble spots: clippy pedantic on `SendNowClaims` and the `admit` allow; the `ai_report_tools`
  test for self-approval (the requester's session, the 403 shape, `approval_item.status`); the `maps_routes`
  render test (the mock engine answers the chart read for a non-anonymous viewer); the audit row's `detail`
  column in the `record_cut_run` test.
- The cut-run path itself (300 s) is not exercised end to end: `record_cut_run` is tested directly, the
  timeout wrapper is not.
- The rate limit is per process: with two API replicas a press can pass once on each.
- Nothing was run in a browser (no dev server by rule): the Delete confirmation inside the subscriptions
  dialog (a dialog over a dialog) has only the jsdom-style test.
- The full `bun run test` and `bun run lint` were not run (rule); CI runs them.
