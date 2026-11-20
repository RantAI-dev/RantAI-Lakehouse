# SRC-46: the connector page keeps up with a run that ends while it is open

Backlog: `SRC-46` (Task, Data, P2). Base: `origin/main` at `6b2242ff6`.
Branch: `fix/src-46-console-freshness`. One PR.

## Why

QA `LH-SRC-6-43` (staging retest 2026-11-19, build `9c702ba0`): with the
Kafka broker stopped, run `88d5d168` ended Failed after 4 attempts. On
screen, the open run log listed attempts 1 to 3 only, and the badge stayed
Healthy.

The planner read the staging database on 2026-11-20. The record is right:

| Time (UTC) | Record |
|---|---|
| 08:27:33 | Attempt 4 `timeout` in `connector_run_attempt`; `run_ended` in `connector_run_log` |
| 08:27:34 | Dagster: `FAILURE` |
| 08:28:40 | Failure sensor counted the run (`pipeline_run_event` `failure`, `failure_alerted`), so the connector was `degraded` |
| 08:34:13 | Recovery run succeeded, so the connector is `healthy` again |

The page is the gap. There are two causes.

1. **The run log is read once.** `RunLog`
   (`src/features/connectors/connector-ingest-panel.tsx:1770-1771`) calls
   `useService` with deps `[connectorId, runId]`. The run list around it is
   polled every 3 s while a run is going (`useIngestRunState`, `:1422-1455`).
   So the row's status changes to Failed, but a log opened while the run
   was going never shows the later attempts or the `run_ended` line.
2. **The connector detail is read once.** That detail carries `health`,
   `lastRunSuccessAt`, `lastRunFailureAt` and `failureStreak`
   (`src/features/connectors/connector-detail-page.tsx:227`). The health
   changes only when the orchestrator's failure or success sensor reports
   the run to the API (`routes/load_alerts.rs:581`, `record_run_once`).
   That is about a minute after the run ends: 66 s for this run. Nothing on
   the page reads the detail again after a run ends.

## Decisions (made, not to be changed by the developer)

- **D1. An open log follows its run.** While the run's record changes
  (its `status`, `attempts` or `endedAt`), an open `RunLog` reads its log
  again. The run row passes those changes down. There is no separate
  timer: the run list's own 3 s poll is the clock, so the log costs nothing
  when no run is going.
  - The reload keeps the lines already on screen: no skeleton flash, so a
    reload for the same `runId` uses `keepDataOnReload`. `useService`
    keeps data only for a reload with the same deps
    (`src/hooks/use-service.ts`), so call `reload()` from an effect keyed
    on the change; do not add the changing values to the deps.
  - A log that is not open fetches nothing (today's behaviour; keep it).
- **D2. The page reads the connector again after a run ends, until the
  run is counted.**
  - `ConnectorIngestPanel` gets an optional `onRunEnded?: (run:
    ConnectorRun) => void`. It is called once, when a run the panel was
    watching is no longer going: the same edge as `wasWatching`,
    `:1450-1453`. It passes the run as the list's refresh returns it.
  - The detail page passes a handler. The handler reloads the detail
    (`state.reload`, which already uses `keepDataOnReload`). It then reads
    the detail again every **10 s** until the run is counted, for at most
    **3 minutes**. The run is counted when the detail's `lastRunFailureAt`
    (for a failed run) or `lastRunSuccessAt` (for a succeeded run) differs
    from its value at the moment the run ended.
  - The 3 minute cap covers a run that is never counted: a paused run
    (`SRC-11` D13), or a sensor that missed it (`load_alerts.rs` module
    doc, "Retries, stated once"). After the cap the page stops asking and
    says nothing; the next page load reads the truth.
  - A status other than `failed` or `succeeded` (for example `paused`)
    reloads once and does not poll.
  - The timer is cleared when the page unmounts, and when a new run ends
    first, in which case the newer run's wait replaces the older one.
  - The create page (`connector-create-page.tsx:390`) passes no handler;
    nothing changes there.
- **D3. No new words on the page.** The health badge simply changes when
  the count lands. A "health updates about a minute after a run" line was
  considered and left out. The backlog item does not ask for it, and the
  run row already says the status follows the orchestrator (`SRC-41`).
- **D4. Out of scope:**
  - the Kafka `timeout` versus `unreachable` class (noted in the backlog row);
  - the connectors list page's health column;
  - any API, Rust or Dagster change.

  The record is right, so nothing server-side changes.

## Anchors (verified at `6b2242ff6`)

- `src/features/connectors/connector-ingest-panel.tsx`
  - `:1294` `POLL_MS = 3_000`
  - `:1422-1467` `useIngestRunState` (`wasWatching` at `:1450-1453`)
  - `:1495` `IngestRuns`, with the `onResumed?: () => void` prop at `:1509`
  - `:1670` `RunCard` (renders `<RunLog>` at `:1748`)
  - `:1770-1771` `RunLog`
  - the panel's own props, with `onResumed` at `:368` and `:432`
- `src/features/connectors/connector-detail-page.tsx:227` (detail
  `useService`) and `:457-465` (`<ConnectorIngestPanel … onResumed={state.reload} />`)
- `src/hooks/use-service.ts`: `keepDataOnReload` holds only for a reload
  with the same deps
- `src/services/contracts/connectors.ts`: `ConnectorRun` (`:1012`);
  `lastRunSuccessAt` and `lastRunFailureAt` on the detail (`:18-20`)

## Tasks

One commit each, in this order. TypeScript only: per commit, run
`bun run typecheck` and the touched test files (`bun run test <file>`).
No cargo, no Python.

1. **An open run log follows its run (D1).**
   - `RunCard` passes `RunLog` a value that changes with the run's record
     (`status`, `attempts`, `endedAt`). `RunLog` reloads with
     `keepDataOnReload` when that value changes after the first read.
   - Acceptance: a test in `connector-ingest-panel.test.tsx` does the
     following.
     - A run is `running` with 3 attempts; the user opens the log, which
       shows 3 attempt lines.
     - The run list's next poll returns the run `failed` with 4 attempts.
       The log is read again and shows the 4th attempt and the "Run
       ended" line.
     - Asserted: the log fetch is called twice.
   - Second test: a run that does not change does not re-read its log
     across polls.
2. **The panel reports a run that ended (D2, panel side).**
   - Add `onRunEnded` to `ConnectorIngestPanel` and thread it to
     `useIngestRunState`. Call it once on the watching→not-watching edge,
     with the run as the refreshed list returns it. The run is the one
     that was active; take it from the refresh result by `runId`, not from
     the stale list.
   - Acceptance: a panel test sees a run go from `running` to `failed`
     and `onRunEnded` called exactly once with that run's `runId` and
     status `failed`. A panel with no handler does not throw.
3. **The detail page waits for the count (D2, page side).**
   - The handler in `connector-detail-page.tsx` follows D2: an immediate
     reload, then a 10 s poll until the outcome's timestamp changes or
     3 minutes pass; cleared on unmount and replaced by a newer run's
     wait.
   - Acceptance, in `connector-detail-page.test.tsx` with fake timers:
     - (a) A failed run: the first reload still says `healthy`, with
       `lastRunFailureAt` unchanged. A later reload says `degraded` with a
       new `lastRunFailureAt`. The badge reads "Degraded", and no further
       detail fetch happens after that.
     - (b) A count never arrives: fetches stop after 3 minutes.
     - (c) A `paused` run: exactly one reload.
     - (d) Unmount during the wait: no fetch after unmount.
4. **Handoff.** Append the "Handoff" entry below (AGENTS.md step 3).

## Verification before handoff

```bash
bun run typecheck && bun run lint && bun run test
git status   # only the two feature files, their tests and this plan
```

Do not commit `.env.staging` or the `AGENTS.md` block in the working tree
(they are local and intentionally uncommitted).

## Handoff

**Slice 1 (the only slice), branch `fix/src-46-console-freshness`.**

Commits:

- `f1293db4c` SRC-46 fix(connectors): an open run log follows its run (task 1)
- `0a2702b40` SRC-46 feat(connectors): the ingest panel reports a run that ended (task 2)
- `f81a6a87c` SRC-46 fix(connectors): the connector page waits for a run to be counted (task 3)

Commands run, in the foreground, with counts:

- `bun test src/features/connectors/connector-ingest-panel.test.tsx` after task 1: 97 pass, 0 fail.
- Same file after task 2: 99 pass, 0 fail (two new tests for `onRunEnded`).
- `bun test src/features/connectors/connector-detail-page.test.tsx` after task 3: 46 pass, 0 fail (four new tests: (a) to (d)).
- Note: `bun run test <file>` runs `bun test src <file>`, which is the whole `src` tree, so the touched files were run with `bun test <file>` directly.

Not verified locally: `bun run typecheck && bun run lint && bun run test` (the full block), and any build. Not verified locally, runs in CI (owner's instruction 2026-11-20). `bun run typecheck` was started once and passed the 120 s limit, then was stopped on that instruction.

Notes for the reviewer:

- Task 2: `useIngestRunState` keeps the id of the last active run in a ref; on the watching to not-watching edge it asks for one more read and calls `onRunEnded` with the run found by `runId` in the list that read returns. The effect that resolves it is declared before the edge effect, so the list that ended the watch is not taken as the "last read".
- Task 3: Bun's fake timers have no async advance, so the tests move time in 1 s steps, each in its own `act`.

**Fix round 1.** `BLOCKER 1` (CI Lint, `react-hooks/refs`): `detailRef` (detail page) and `lastActiveRunId` (panel) are now synced in effects, not during render. Fixing it showed a second rule, `react-hooks/set-state-in-effect`, on the detail page's "counted" effect; the counted state is now derived (`countWaiting`) instead of set from an effect, with the same behaviour. `./node_modules/.bin/eslint` on the two files: exit 0, no errors. `bun test` on the panel file: 99 pass, 0 fail; on the detail page file: 46 pass, 0 fail. Full typecheck and suite: not verified locally, runs in CI.

## Review

### Round 1 (planner, 2026-11-20, head `98c978be1`)

The diff matches D1 to D3. Both deviations in the handoff are accepted: the
ref that keeps the ended run's id, and 1 s fake-timer steps.

- **BLOCKER 1.** CI `Lint` fails with 2 errors, `react-hooks/refs`
  ("Cannot access refs during render"):
  - `connector-detail-page.tsx:241`: `detailRef.current = state.data` is
    written in the render body;
  - `connector-ingest-panel.tsx:1465`: `if (activeRun)
    lastActiveRunId.current = activeRun.runId` is written in the render
    body.

  Fix without writing or reading a ref during render. For example, take
  the detail value in the `onRunEnded` callback through a state updater or
  an effect-synced ref, and record the active run id in an effect. Keep
  the behaviour and the tests the same. Run CI run 44667901397, job
  `Lint`.

