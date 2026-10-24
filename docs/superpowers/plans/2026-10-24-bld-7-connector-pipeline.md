# BLD-7, BLD-8, BLD-9: a pipeline made from a connector runs

| | |
| --- | --- |
| Planner and reviewer | Claude Opus (`AGENTS.md`). Developer: a non-Opus agent |
| Date | 2026-10-24 |
| Backlog | `BLD-7` (P1), `BLD-8` (P1), `BLD-9` (P2), module Build. Decision `DEC-15` |
| Source | QA report of 2026-10-23: a pipeline created from the connector page's *Create pipeline* is saved as ready and every run fails in about 5 s |
| Base | `origin/main` at `563bb706`; anchors read there |
| Branch | `fix/bld-7-connector-pipeline`, worktree `/home/shiro/rantai/wt-bld7` |

`A` = `rust/crates/lakehouse-api/src`, `D` = `dagster/dispar_orchestrate`,
`P` = `src/features/pipelines`.

## What is wrong today

- `src/features/connectors/connector-detail-page.tsx:330` links to
  `/pipelines/create?connectorId=<id>`; `P/pipeline-create-page.tsx:129-130`
  fills `connectorId` from the URL.
- `A/routes/pipelines.rs:3165-3171` (`create`) only checks the connector is
  in the caller's tenant.
- `D/authored_factory.py:701-703` refuses every run whose definition has a
  `connectorId` (`CONNECTOR_SOURCE_UNSUPPORTED_REASON`, line 270), before
  it even looks at `sourceZone`/`sourceTable`.
- But `sourceZone` and `sourceTable` are required on every pipeline
  (`CreatePipelineBody`), so a pipeline with a connector already names the
  table it reads. In the QA case it was `bronze.northwind_orders`, the
  table that connector's ingest wrote, and `BLD-3` (#155) already reads
  Bronze. Only the guard stops it.

## Decisions (planner, product owner agreed 2026-10-24)

1. **`DEC-15`:** no direct read from a source into Silver. A pipeline's
   connector means *where its data came from*, never a second read path.
   No adapter, no secret resolution, no bulk load is built here.
2. **The rule, one place in the API.** A pipeline with a connector must
   read from `bronze`, and `sourceTable` must be one of that connector's
   ingest targets (the `target` of an entry in the connector's
   `source_objects`, `lakehouse-store` `connectors.rs`, see the
   `text(o, "target")` reader near line 1484; reuse it, do not write a
   second parser, rule 4). Otherwise 400 with one fixed sentence that says
   what to pick, e.g. "A pipeline with a connector reads one of that
   connector's Bronze tables: choose `bronze` and one of: <targets>.
   To read another table, choose No connector." Applied on create (T1)
   and, after T4, on update. A connector with no ingest targets yet gets
   "This connector has not loaded any table yet; run its ingest first."
3. **The run.** Remove the blanket guard. A run with a connector reads its
   `sourceZone.sourceTable` exactly as any Bronze pipeline does (`BLD-3`
   path, masking and the author's policy unchanged). Keep a narrow guard
   for rows stored before this change that break the rule: connector set
   and `sourceZone != "bronze"` fails with a new fixed reason that says to
   edit the pipeline (rule 2 of `AGENTS.md`: honest failure, no silent
   read). Rewrite the constant and its comment; `WS4 item E3` wording goes.
4. **The trigger.** A pipeline created from a connector page runs after
   that table's load: the form adds `ingest_job` to `dependsOn` by default
   (switch shown and on, user may turn it off). `BLD-4`'s per-table
   sensor already filters `ingest_job` by the pipeline's own Bronze table
   (`D/authored_factory.py` `_latest_record_for_ingest_table`), and the
   API already refuses `ingest_job` for a non-Bronze source
   (`A/routes/authored_pipelines.rs` around line 490-512). Reuse both;
   do not add a new sensor.
5. **The form (connector page entry).** With `?connectorId=`: source zone
   is fixed to `bronze` (shown, not editable), *Source table* is a picker
   of that connector's ingest targets (from the connector the page already
   loads via `connectorService`, not a new endpoint unless none returns
   `source_objects`; if a new read is needed, stop and report), and the
   "run after load" switch from decision 4 is on. With no connector, the
   form is as today. The picker's empty state says to run the ingest
   first and links to the connector's Ingest tab.
6. **`BLD-8`: the run's real state and reason.** The pipeline page must
   not show "Running" for a run that has ended. Find where the page gets
   the state (`P/pipeline-detail-page.tsx` around line 511, and the API
   route behind it) and show the latest run's real status. For a failed
   run, show the failure reason the API already records or can read from
   Dagster, as a fixed classified text: our own `Failure` descriptions
   (`config rejected: …`, `refused: …`) are ours and may be shown; any
   other upstream text is not shown (principle 4), the page says "The run
   failed; see the run log" with a link to the run instead. If the API
   holds no failure text today and adding it needs a migration, stop and
   report before writing one.
7. **`BLD-9`: change or clear the connector.** `PUT /api/pipelines/{id}`
   accepts `connectorId` (absent = unchanged, `null` or empty = clear),
   with decision 2's check. The Edit form shows the connector picker
   instead of the read-only box (`P/pipeline-create-page.tsx:344`), with
   "No connector" as an option.
8. **Assistant.** `A/routes/ai/tools/pipelines.rs:1205` sends
   `connector_id: None`; leave it (the assistant does not set a connector
   today). Say so in the handoff.

## Tasks (one commit each, in this order)

- **T1 API rule on create.** Decision 2 in `create`; a helper shared with
  T4. Tests in `A/routes/pipelines.rs` next to the existing
  `connectorId` create tests (around lines 5541 and 5580): accepted with
  `bronze` + a target of that connector; 400 with the fixed sentence for a
  non-Bronze zone, for a table not among the targets, and for a connector
  with no targets; the tenant check still runs first (unchanged
  behaviour, unchanged test). Each new test fails without the change.
- **T2 The run.** Decision 3 in `D/authored_factory.py`. Unit tests in the
  existing authored factory tests: a definition with a connector and a
  Bronze source reaches the source resolution and SQL build (assert on the
  built SQL or the resolved source, no network); a connector with a
  non-Bronze source fails with the new reason; the old reason string is
  gone (`grep -rn "WS4 item E3" dagster` empty). Test names are full
  sentences.
- **T3 The form.** Decisions 4 and 5 in `P/pipeline-create-page.tsx` (and
  the connector page link only if it needs a change). Console tests: with
  `?connectorId=` the zone is `bronze` and fixed, the table picker lists
  only that connector's targets, the run-after-load switch is on and its
  `dependsOn` reaches the create call; with no connector nothing changes.
- **T4 `BLD-9`.** Decision 7, API and form, with tests (set, change,
  clear, refused by the T1 rule).
- **T5 `BLD-8`.** Decision 6, with tests for an ended run never showing
  "Running" and for the reason shown or withheld.
- **T6 Docs.** No feature page covers authored pipelines in general;
  `docs/core/features/uploads-feed-pipelines.md` is the closest (an upload
  feeding a pipeline, `DATA-19`). Add a short section there, "A
  connector's tables feed pipelines", saying what a connector on a
  pipeline means, that it runs after that table's load, and that direct
  source reads are not supported (`DEC-15`). Do not edit `docs/core/BACKLOG.md` status/PR; the
  planner does.

## Verification before handoff

Scoped per commit (`AGENTS.md`, "Keep build time down"). Before the
handoff, the full block for Rust, TypeScript and Python:
`cargo fmt --check`, workspace clippy, `cargo test --workspace`,
`bun run typecheck`, `bun run lint`, `bun test src`, the two
`ops/lint` scripts, `(cd dagster && python -m pytest dispar_orchestrate -q)`,
`git status`.

## Out of scope

Direct source reads (`DEC-15`), column drop or hash at ingest (`SRC-33`),
the FBIC switch (`BLD-10`), any new sensor, the assistant's pipeline tool.

## Handoff

### 2026-10-24 — T1 to T6 (`fix/bld-7-connector-pipeline`)

Commits, in plan order:

- `BLD-7 T1` — `create` applies decision 2 through one helper
  (`routes::authored_pipelines::validate_connector_source`, shared with T4)
  plus `lakehouse_store::connectors::ingest_targets` (one reader for
  `source_objects[].target`, the old closure refactored onto it).
- `BLD-7 T2` — the blanket connector guard in `authored_factory.py` is gone;
  the narrow guard for a connector on a non-Bronze source has a new fixed
  reason that says to edit the pipeline. `grep -rn "WS4 item E3" dagster` is
  empty.
- `BLD-7 T3` — the `?connectorId=` form: zone fixed to bronze, table picker
  from that connector's ingest targets (`connectorService.getIngestSpec`, the
  route the connector page already reads; `listConnectors` carries no
  `sourceObjects`), "run after this connector loads the table" switch ON and
  its `ingest_job` in `dependsOn`. Rules in
  `src/features/pipelines/pipeline-connector-trigger.ts`.
- `BLD-9 T4` — `PUT` takes `connectorId` (absent / `null` / empty / value),
  held to decision 2 against the pipeline's own tenant; store writes the
  tri-state with a `CASE` on a boolean; restore leaves the connector alone.
  The edit form gets the picker with "No connector".
- `BLD-8 T5` — two fixes. The run-list poll effect depended on the whole
  `useService` object, which is new on every render; the page re-renders
  every second while a run is live, so the timer never fired and an ended
  run stayed "Running". It now depends on `runsStatus`. The failure reason
  is classified: our own `config rejected: …` / `refused: …` descriptions
  are shown, anything else says "The run failed; see the run log". No
  migration.
- `BLD-7 T6` — `docs/core/features/uploads-feed-pipelines.md`, section "A
  connector's tables feed pipelines".

Commands actually run (this worktree, `CARGO_TARGET_DIR` shared,
`cargo-locked … -j 2`):

| Command | Result |
| --- | --- |
| `cargo check -p lakehouse-store -p lakehouse-api` (T1) | `EXIT=0`, 2m10s |
| `cargo check -p lakehouse-store -p lakehouse-api --all-targets` (T4) | `EXIT=0`, 3m37s |
| `cargo test -p lakehouse-api connector` | `280 passed; 0 failed` (twice: lib + bin), `API_EXIT=0` |
| `cargo test -p lakehouse-store --test pipelines` | `60 passed; 0 failed`, `STORE_EXIT=0` |
| `cargo fmt --check` | clean |
| `bun run typecheck` | `EXIT=0` |
| `bun run lint` | `0 errors, 7 warnings` (all pre-existing, untouched files) |
| `bun test src/features/pipelines` | `103 pass, 0 fail` (run three times: 102/1, then 103/0 twice — the one failure was the BLD-8 poll test under a 1 s default while the machine was building; its waits are now 8–10 s) |
| `python3 ops/lint/check_intra_package_imports.py` | `OK` |
| `python3 ops/lint/check_bare_iceberg_count.py` | `R11 guard: … none found` |
| `(cd dagster && python -m pytest dispar_orchestrate/test_authored_factory.py -q)` | `107 passed` |
| `(cd dagster && python -m pytest dispar_orchestrate -q)` (full) | `2146 passed, 33 subtests passed in 1037.02s`, `PYTEST_EXIT=0` |
| `docker compose --profile '*' config --quiet` | `EXIT=0` with throwaway values for `POSTGRES_PASSWORD`, `RUSTFS_ACCESS_KEY`, `RUSTFS_SECRET_KEY`, `LAKEKEEPER_ENCRYPTION_KEY` **and `CONNECTOR_SECRETS_KEY`** (a fifth must-set variable the AGENTS.md block does not list) |

Not verified, with the reason:

- **Full workspace `cargo clippy --workspace --all-targets --all-features --
  -D warnings` and `cargo test --workspace`**: started, killed at 33 min when
  `df -h /` reached **4.1 GB free (100% used)**; `cargo-target` was 181 GB.
  I killed only my own processes (the wrapper and its cargo), then ran
  `/home/shiro/bin/cargo-target-prune.sh 15 25`, which took free space to
  25 GB. The full block needs ~88 GB of artifacts on this box, so it cannot
  run here until more is free; the reviewer's run is the next attempt.
  Nothing in the change is unbuilt: the two crates touched were checked with
  `--all-targets`, and their tests ran (numbers above).
- **Full `bun test src`**: queued under
  `/home/shiro/rantai/.heavy.lock` three times; the lock was held by another
  worktree's review runs (`review-pr.sh /home/shiro/rantai/wt-rel9`, a
  `cargo test -j` at a time), and `df -h /` fell to 4–16 GB free while they
  built. Cancelled (`BUNTEST_EXIT=143`) rather than keep competing for the
  lock. The scoped `bun test src/features/pipelines` run above covers every
  test file this branch touches, and `bun run typecheck`/`bun run lint` ran
  over the whole tree; the reviewer's own run is the full one.
- **`docker compose up`**: not run; no compose file changed.

Notes for the reviewer:

- Decision 8: the assistant still sends `connector_id: None`
  (`routes/ai/tools/pipelines.rs`), untouched.
- Two interpretations, both at the code with a comment: (a) a pipeline with a
  connector shows the connector-load switch instead of the DATA-19 upload
  switch, because the connector is what loads that table; a stored
  `file_ingest_job` is dropped on save there; (b) clearing a connector does
  not rewrite the rest of `dependsOn` (`ingest_job` stays, and is legal on a
  Bronze source).
- The BLD-8 regression test was checked against the old dependency: it fails
  (10 s timeout) with `[anyLive, reloadRuns, runsState]` and passes with
  `[anyLive, reloadRuns, runsStatus]`.

## Review

(planner appends here)

### Review round 1 (planner, 2026-10-24)

Diff read against the plan at `8dbf7751`. Decisions 1-8 are followed. The
six reported interpretations are accepted: reading
`GET /api/connectors/{id}/ingest-spec` is the existing read the connector
page uses (decision 5 asked for no new endpoint, and none was added); the
create test's `sourceTable: "orders"` keeps its tenant assertion; the
`BLD-8` cause was the client poll effect, and the fix is at that effect;
the two form behaviours are stated at the code; the compose note is a doc
gap for `AGENTS.md`, not this PR. No `BLOCKER`.

- **SHOULD-FIX 1 (fixed by the planner).** `update` checked decision 2
  only for a newly submitted `connectorId`. A body that left the connector
  out kept it, but could move the source to a table that connector never
  loads; `restore_version` had the same gap (it keeps the connector and
  replays an older source). Both now check the connector the row ends
  with. Test:
  `update_refuses_moving_a_kept_connectors_pipeline_to_a_table_it_does_not_load`.
- **SHOULD-FIX 2 (fixed by the planner).** `classifiedFailureReason`
  matched a bare `refused:`, so a driver's "Connection refused: ..." was
  shown as the reason, against decision 6. The pattern is now anchored to
  the factory's own `authored pipeline '<id>' config rejected:` /
  `refused:` prefix; the upstream-text test gained that line.
- **Nit (fixed).** The refusal sentence put backticks around `bronze`; the
  console shows it as plain text, so they are gone.

The developer's workspace clippy and test, and the full `bun test src`,
did not finish on this machine (disk, lock); the planner's full run below
is the one that counts.
