# `BI-63`, `BI-64`, `BI-65`: the acceptance findings that block `BI-5`, `BI-26` and `BI-19`

Planner and reviewer: Claude Opus. Developer: Claude Sonnet, one session per
PR. Base: `main`. Three PRs, one per item, each on its own branch:
`fix/bi-63-stop-in-history`, `fix/bi-64-embed-polish`,
`feat/bi-65-digest-schedule`. This file is committed on each branch.

The owner asked to close the 27 `BI` acceptance items first (2026-11-21);
these three findings from the acceptance run of 2026-11-16 are what keeps
`BI-5`, `BI-26` and `BI-19` from acceptance. Anchors were read at `main`
`7345cfba8`.

---

## PR 1, `BI-63`: a run stopped from the console is in the history (`BI-5-AC2`)

**Finding.** Stop ends the query in about a second and the console says
"Stopped", but History never lists the run. A stop sent through the API
while the run request stays open is recorded as stopped.

**Cause (read).** `run_statement_with`
(`rust/crates/lakehouse-api/src/routes/query.rs` ~2093–2225) writes the
history row for a failed or stopped run (`record_uncompleted_run`, ~2222,
~2405–2445) only after the engine call returns. The console's Stop
(`src/features/queries/use-query-studio.ts` ~1303–1307) fires
`queryService.cancel(runId)` and at once aborts the run request
(`runControllerRef.current.abort()`); axum drops the handler future with the
connection, so the write never runs. Trino runs get no cancel at all.
`query_cancel.rs` marks the run stopped in memory (`query_limits::mark_stopped`)
and kills it on ClickHouse; it has no SQL text, so it cannot write the row.

**Decisions.**
1. The engine call and its history write run in a detached task
   (`tokio::spawn`), and the handler awaits its `JoinHandle`. A dropped
   connection then no longer cancels the write. The task keeps everything it
   needs (owned clones); the statement timeout still bounds it.
2. When the run finishes after a stop, the row's status is **stopped** if the
   run was marked stopped (`query_limits`' stopped mark for that run id),
   whatever the engine answered. This covers Trino too: Trino has no KILL here,
   so its query runs to its end, but the row says stopped, not succeeded.
3. The console calls `cancel` for every engine (not only ClickHouse) and
   waits for the cancel's answer (up to 2 s) before it aborts the run
   request. The local "Stopped" still shows at once.
4. Tests: a Rust test that a run whose connection is dropped after a stop
   still records a stopped history row (a test double for the engine that
   waits until killed); a console test that Stop calls cancel before abort.

## PR 2, `BI-64`: embeds look right on the host page (`BI-26-AC2`, `AC5`)

**Findings.** (a) `background=transparent` leaves the frame's body opaque;
(b) the height posted to the host page leaves out the 8 px top and bottom
padding, so the frame scrolls by 16 px; (c) a raw table prints a year as
"2,020".

**Anchors.** `src/lib/embed-look.ts:35` (`transparent`),
`src/features/dashboards/embed-view.tsx:199-200` and `:228` (outer
`min-h-screen p-2` with `boardLook.className` and `!bg-transparent`), the
measured root at `:229`, `src/features/dashboards/use-embed-height.ts:53`,
`src/lib/cell-format.ts` ~98 and `src/lib/value-format.ts` ~111.

**Decisions.**
1. With `background=transparent`, nothing behind the tiles paints: not
   `html`, not `body`, not the outer wrapper, not the grid. Tiles keep their
   cards (by design, `embed-view.test.tsx:130-141`). Find in a real browser
   which element paints (computed `background-color` of `html`, `body`, the
   wrapper, the grid) and clear exactly those for the embed route when the
   parameter is set. Say in the handoff which element it was.
2. The posted height includes the wrapper's padding: move `p-2` inside the
   measured root (do not measure a `min-h-screen` element, per the hook's own
   note), so the posted height equals the content's height and the frame
   does not scroll.
3. A **year column** is printed without a thousands separator: an integer
   from 1000 to 9999 in a column whose name is a year (`year`, `tahun`, `yr`,
   or ends with `_year` / `_tahun`, any case) gets no grouping, in raw tables
   and in table cells everywhere (`cell-format.ts` and the default branch of
   `value-format.ts`). An explicit format with `thousands` set still wins.
   This applies to the console's own tables too, not only embeds.
4. Tests for each: the transparent classes on the embed root, the posted
   height including the padding, and `2020` in a `tahun` column printed as
   "2020" while `2020` in a `visitors` column is "2,020".

## PR 3, `BI-65`: dashboard digests run on a schedule (`BI-19-AC12`)

**Finding.** A digest saved with a weekly Monday 08:00 schedule is stored
without it (the field is dropped silently) and the Alerts page has no
schedule control. Read further: with no schedule at all, `run_digest`
(`rust/crates/lakehouse-alerts/src/lib.rs` ~3291–3330) has no time gate,
so a digest is sent on **every** 15-minute alerts run
(`dagster/dispar_orchestrate/alerts_run.py` `*/15`).

**Anchors.** `AlertRuleInput` (`lakehouse-alerts/src/lib.rs` ~440–500, no
`deny_unknown_fields`, no schedule), `AlertRule` (~314), the ClickHouse
`console.alert_rule` table (CREATE ~1160, `ALTER … ADD COLUMN IF NOT EXISTS`
~1178–1200, insert ~1402), `routes/alerts.rs` `parse_body` ~441,
Postgres `alert_rule_state` (`rust/migrations/0113_alert_rule_state.sql`),
the console contract `SaveAlertRuleInput` (`src/services/contracts/alerts.ts`
~150–172), the form `DigestConditionFields`
(`src/features/alerts/alert-rule-page.tsx` ~354–386), the AI tools
`create_alert_rule` / `update_alert_rule`
(`rust/crates/lakehouse-api/src/routes/ai/tools/alerts.rs`).

**Decisions.**
1. **One schedule shape for reports and digests.** A digest's schedule is
   the report subscription's schedule (`ScheduleInput` in `routes/reports.rs`
   ~115 → `lakehouse_store::report_schedule::Schedule`, with
   `next_run_after(time_zone, now)`) plus a `timeZone` (IANA, as reports).
   Stored on the rule as two new nullable `console.alert_rule` columns
   (`schedule` JSON text and `time_zone`), added with the existing
   `ADD COLUMN IF NOT EXISTS` pattern; `AlertRule` and the contract carry
   them; only `type = digest` may have them (refused on other kinds).
2. **A digest is sent when it is due.** Due when the schedule's next run
   after the digest's last send (or after its creation, for one never sent)
   is at or before now. The last send time is kept in the rule's state
   (Postgres `alert_rule_state`; add a column in a new migration
   `NNNN_alert_rule_state_digest_sent.sql` with a why-header if no fitting
   column exists). The 15-minute tick stays; a digest is sent within 15
   minutes of its time, and the form says "within 15 minutes" in its
   one-line help.
3. **An existing digest without a schedule** is due once a day at 08:00 in
   the deployment's report time zone (Settings → Reporting), not on every
   tick. A new digest must have a schedule (400 with one sentence if not).
4. **No field is dropped silently** (principle 3): `AlertRuleInput` gets
   `deny_unknown_fields`. Grep every producer of this input (console, AI
   tools, tests, fixtures) so none sends an extra key; a stale client gets a
   400 naming the field.
5. **Console.** `DigestConditionFields` gets the schedule control the
   Subscriptions dialog already has (`src/features/dashboards/subscriptions-dialog.tsx`
   ~48, ~323–367: Every day / week / month, weekday, time, the
   `TimeZoneSelect`): extract it into one shared component used by both, so
   the two cannot drift (rule 4). Default: weekly, Monday 08:00, the report
   time zone. The digest row on the Alerts page shows its schedule in one
   short line ("Mondays · 08:00 WIB").
6. **Assistant parity** (`AGENTS.md`): the `create_alert_rule` /
   `update_alert_rule` tools accept the digest schedule with the same shape;
   regenerate `tests/fixtures/tool_schemas.json`.
7. Tests: Rust — a digest with a weekly schedule is not sent before its time
   and is sent once after it, not again on the next tick; a digest with no
   schedule is sent once a day; an unknown field is a 400; a schedule on a
   non-digest rule is a 400. Console — the shared schedule control in both
   places, the digest row's schedule line.

---

## Checks (this VM, every PR)

**No `cargo build/check/clippy/test` on the VM** (out of memory): only
`rustfmt --edition 2024 --check <changed .rs>`; CI compiles and tests. Be
meticulous with every constructor, match and caller you touch. Console: wrap
bun in `flock /tmp/claude-1000/lh-bun.lock systemd-run --user --scope -q -p
MemoryMax=3G -p MemorySwapMax=0 nice -n 15 …`; `bun x tsc --noEmit -p .`,
`bun x eslint <changed>`, `bun test <touched test files>`; tests restore
globals and never let a real fetch through. No prettier, no full test/lint,
no dev server. CHANGELOG entry per PR. Do not push.

---

## Handoff, PR 2 (`BI-64`)

Branch `fix/bi-64-embed-polish`, not pushed.

**Commits** (decisions 1 and 2 share the first commit, because both change
`embed-view.tsx` hunks that sit side by side; tests for all four are in the
last one):
1. `BI-64 embed: clear html and body for background=transparent; height includes padding`
   (`embed-view.tsx`, `globals.css`): an effect puts `embed-transparent` on
   `<html>` while the option is on; `html.embed-transparent, html.embed-transparent body`
   are transparent. `p-2` moved from the `min-h-screen` wrapper to the
   measured root (`<div ref={measured} className="p-2">`).
2. `BI-64 tables: print a year column without a thousands separator`:
   `isYearColumn` / `isYearValue` in `value-format.ts`; `formatValue(..., column?)`
   and `cellContent(..., column?)`; `CellValue` takes `column`, passed by
   `raw-table.tsx` and the table tile in `tile-body.tsx`. Pivot cells are not
   a named column, so they are unchanged.
3. `BI-64 tests and changelog` (decision 4 and the CHANGELOG entry).

**Which element painted:** `body`, by `body { @apply bg-background }` in
`design-system/globals.css` (`@layer base`), reached through the root
`layout.tsx`; `html` has no background of its own, the canvas takes the body's.
The embed wrapper's own `!bg-transparent` was already there, which is why the
frame stayed opaque. **Not verified in a browser**: the auto-mode classifier
refused the login to the QA console, so this comes from reading the cascade
(`globals.css`, `design-system/globals.css`, `layout.tsx`, `app-frame.tsx`,
which skips all chrome for `/embed/*`, `.theme-light` tokens, which set
variables only). The reviewer should check the computed `background-color` of
`html` and `body` in an embed with `background=transparent`.

**Commands** (bun wrapped in flock + systemd-run, `node_modules` symlinked and
removed before every commit):
- `bun test value-format.test.ts cell-format.test.ts embed-view.test.tsx` (+ a
  `use-embed-height` pattern that matched no file): 58 pass, 0 fail; after the
  last test edit `value-format.test.ts` alone: 29 pass, 0 fail.
- `bun x tsc --noEmit -p .`: no output (clean).
- `bun x eslint` on the 8 changed source and test files: no output (clean).
- Not run: full test/lint, Rust, dev server, browser.

**Gap:** the posted height cannot be measured in happy-dom, so the test asserts
the structure (the measured root has `p-2`, the wrapper has none), not pixels.
There is no `raw-table` test file; the year rule is tested on the formatters
the table calls.
