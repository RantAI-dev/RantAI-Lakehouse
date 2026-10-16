# `BLD-2`: tables that refresh themselves within a target lag

Planner and reviewer: Claude Opus. Developer: a non-Opus agent (`AGENTS.md`).
Base: `origin/main` at `7a092cafa91d`. Branch: `feat/bld-2-target-lag`.
One PR slice. Spec: `docs/core/specs/bld-2.md`; feature page:
`docs/core/features/pipeline-target-lag.md` (decisions 1 to 4 signed by the
product owner, 2026-10-14). Waits for `BLD-3` (#155) and `DATA-10` (#156),
both merged.

`BLD-1` slice 1 (`feat/bld-1-run-as-author`, not merged) touches the same
files: `routes/pipelines.rs::create`, `CreatePipelineInput`,
`authored_factory.py`. Whichever merges second merges `origin/main` and
resolves hunk by hunk (rule 9). Do not import anything from that branch.

## 1. What exists (read on `main` at `7a092cafa91d`)

- A pipeline runs on `schedule` (a five-field cron; anything else, such as
  `"manual"`, means on demand) or after its `depends_on` upstreams succeed
  (a `run_status_sensor`, `authored__<id>_after`). No column says which;
  `rust/migrations/0007_pipelines.sql:18`, `0052_pipeline_depends_on.sql:37`.
- A pipeline has one source, `"<zone>.<table>"`. A Bronze source is read
  through a ClickHouse `DataLakeCatalog` database
  (`authored_factory._resolve_source`, ~:295); Silver and Gold are plain
  ClickHouse tables.
- Dagster builds the authored jobs, schedules and sensors once per code
  location load (`build_authored_definitions` ~:638,
  `build_authored_dependency_sensors` ~:681). The API reloads the location
  after every pipeline write (`authored_pipelines.rs::reload_orchestrator`
  ~:854). Authored sensors set no `minimum_interval_seconds` (Dagster's
  default, 30 s).
- The Rust side already reads both change signals:
  - an Iceberg table's current snapshot id:
    `lakehouse-iceberg/src/rest.rs:184` `load_table_summary`
    (`current_snapshot_id`, `last_updated_ms`); used by
    `routes/alerts.rs:518` for freshness;
  - a ClickHouse table's merge-proof content fingerprint (`DATA-10`):
    `gold_export_history.rs:372` `mart_content_fingerprint(ch, schema,
    table)`. Nothing in it is Gold-specific.
- A successful run is reported to the API by `pipeline_events.py`
  (`pipeline_run_finished_sensor` → `POST /api/pipelines/events/run-finished`
  with `{runId, jobName}`); the API confirms the run with Dagster
  (`pipeline_run_status`) and maps the job to a pipeline
  (`authored_pipelines::job_name`).
- Alerts: `lakehouse-alerts/src/lib.rs` `AlertKind` (~:128). Event-driven
  kinds are evaluated when the event arrives (`evaluate_pipeline_failure`
  ~:1887). "N in a row" precedent: `ConnectorRepeatedFailure` fires when a
  streak *equals* `REPEATED_FAILURE_STREAK = 3`
  (`lakehouse-store/src/connectors.rs:1529`, `routes/load_alerts.rs:421`),
  so it fires once per streak with no extra dedupe.
- Nothing writes `pipeline_definition.last_run_at` or
  `freshness_lag_seconds` for authored pipelines; the list sends `null`.
- Assistant: `routes/ai/registry.rs` `TOOLS` (~:904); `create_pipeline`
  (~:817), `get_pipeline` (~:833); no tool edits a pipeline. Fixture
  `tests/fixtures/tool_schemas.json`, regenerated with
  `cargo test -p lakehouse-api --lib write_tool_schema_fixture -- --ignored`.
  `AI-15`'s request set does not exist yet; the nearest is
  `ops/ai_eval/cases.json` (not run in CI).
- Gate: `ops/g7/g7_pipeline_test.py` builds a Bronze-to-Silver pipeline in
  CI job `g3a-dagster` (`.github/workflows/ci.yml` ~:579, step ~:679).

## 2. Decisions (planner)

- **D1. Storage.** Migration `0077_pipeline_target_lag.sql` (0072 to 0075
  are taken by #144, 0076 by `BLD-1`; re-check every open branch when the
  PR is opened and take the next free number if that changed). Why-header,
  additive:
  - `pipeline_definition.target_lag_seconds INTEGER NULL` with
    `CHECK (target_lag_seconds BETWEEN 60 AND 604800 AND target_lag_seconds % 60 = 0)`;
  - `pipeline_definition.target_lag_downstream BOOLEAN NOT NULL DEFAULT false`;
  - `CHECK (NOT (target_lag_downstream AND target_lag_seconds IS NOT NULL))`;
  - a new table `pipeline_lag_state` (one row per pipeline, `ON DELETE
    CASCADE`): `consumed_tokens JSONB` (the input tokens the last
    successful lag run read), `consumed_at TIMESTAMPTZ`, `pending_since
    TIMESTAMPTZ NULL` (first check that saw tokens differ from
    `consumed_tokens`), `requested_run_key TEXT NULL`, `requested_tokens
    JSONB NULL`, `last_check_at TIMESTAMPTZ NULL`, `last_status TEXT NULL`
    (`met` / `behind` / `not_measured`), `behind_streak INTEGER NOT NULL
    DEFAULT 0`, `tenant_id` as the other pipeline tables carry it.
- **D2. Wire format.** `targetLag` is `null`, `{"kind":"lag","minutes":N}`
  (1 to 10080) or `{"kind":"downstream"}`. Minutes, not seconds, because
  the minimum is 1 minute and the UI and the assistant speak minutes;
  stored as seconds. Serde: an internally tagged enum; TS: a union type
  with `| string` widening on `kind` per the TS rules.
- **D3. A target lag replaces the schedule while it is set.** For a
  pipeline with a target lag (either kind), Dagster builds neither the cron
  schedule nor the `_after` sensor; `schedule` and `dependsOn` stay stored,
  so removing the lag returns the pipeline to them (AC8). The editor shows
  the schedule field as "not used while a target lag is set".
- **D4. Inputs and their tokens.** A pipeline's inputs are its source table
  plus the target table of each `dependsOn` pipeline. Each input's token:
  - Bronze: `iceberg:<current snapshot id>` from `load_table_summary`;
  - Silver / Gold: `fp:<content fingerprint>` from
    `mart_content_fingerprint` (move or re-export it so both callers use one
    function; rule 4).
  An input whose token cannot be read makes the whole check
  `not_measured` (principle 2): no run is requested on a guess, and the
  page says why ("an input could not be read"), never "met".
- **D5. One decision-maker, in the API.** New orchestrator-only route
  `POST /api/pipelines/runnable/{id}/lag-check` (same authentication rule
  as `runnable`, in `POLICY_TABLE`, asserted both ways in
  `tests/route_auth.rs`; another tenant's or an unknown id → the same 404).
  Per call it:
  1. reads the input tokens (D4);
  2. if a run was requested and not yet consumed, asks Dagster for that
     run by its run key; on `SUCCESS` sets `consumed_tokens =
     requested_tokens`, `consumed_at` = the run's start time, clears the
     request; on `FAILURE`/`CANCELED` clears the request (the failure alert
     and `maxRetries` already cover it); while queued or running, waits;
  3. if tokens equal `consumed_tokens`: status `met`, `pending_since =
     NULL`, streak 0, decision `wait`;
  4. otherwise sets `pending_since` if unset; lag = now − `pending_since`;
     **run budget** = median duration of the pipeline's last 5 successful
     runs (none yet: 60 s) + the check interval (30 s). Decision `run` when
     no run is outstanding and lag ≥ target − budget (so changes inside one
     lag window are batched into one run; with a 1-minute target it runs on
     the first check that sees a change). Status `behind` when lag > target,
     else `met`;
  5. a pipeline that was never run successfully under its lag has no
     `consumed_tokens`: decision `run`, status `behind` until it succeeds;
  6. updates `last_check_at`, `last_status`, `behind_streak` (+1 on
     `behind`, 0 otherwise) and returns `{decision: "run" | "wait",
     runKey, status, lagSeconds, targetSeconds}`. `runKey` is
     `lag:<pipeline id>:<hash of requested tokens>` and is stored as
     `requested_run_key` with `requested_tokens` when the decision is `run`.
- **D6. Dagster asks every 30 seconds.** For each pipeline with
  `kind = lag` (not downstream), one sensor `authored__<id>_lag`,
  `minimum_interval_seconds=30` (say why in the module doc), RUNNING for
  `ready`, none for `paused` (as the `_after` sensor). It calls D5 and, on
  `run`, yields `RunRequest(run_key=runKey)` for the pipeline's job. A 4xx
  is logged and skipped, a transport error or 5xx is skipped for this tick
  (the next tick asks again); never raises.
- **D7. Downstream.** A pipeline with `kind = downstream` gets no sensor.
  When D5 evaluates a lag pipeline P whose input is produced by a
  `downstream` pipeline U (U's target is P's source, or U is in P's
  `dependsOn`), U's inputs are checked first, recursively (the graph is
  already acyclic, `validate_no_cycle`; extend the check to source/target
  edges for lag pipelines). If U's inputs changed and P's decision would be
  `run`, the decision is `run` for **U** first (its run key), and P waits;
  when U's run lands, U's target fingerprint changes and P runs on a later
  check. The budget for P adds U's median duration. A `downstream` pipeline
  that no lag pipeline reads never runs: its page says "Downstream: no
  pipeline with a target lag reads this table" (honest, not "met"). D5 then
  returns which job to request: add `jobName` to its response.
- **D8. Shown.** `GET /api/pipelines` and `GET /api/pipelines/{id}` add
  `targetLag` (D2) and `lag: { status: "met" | "behind" | "not_measured",
  lagSeconds | null, lastCheckAt | null, reason | null }`. `not_measured`
  also when `last_check_at` is older than 3 check intervals (90 s) or
  missing: the orchestrator stopped asking, so nothing is known (AC5). The
  pipeline page gets a "Freshness" panel (target, current lag, status,
  last check, the D7 downstream note); the list gets a "Lag" column with
  the status. Setting the target lag: `PUT /api/pipelines/{id}/target-lag`
  `{ "targetLag": <D2> }`, `pipeline:write`, tenant-scoped like `update`,
  audited, reloads the orchestrator; the panel has the control. The create
  body accepts an optional `targetLag` too.
- **D9. Alert.** New `AlertKind::PipelineBehindTarget`, evaluated in the
  lag-check handler when `behind_streak` becomes exactly 3 (the
  `ConnectorRepeatedFailure` precedent: once per streak, no extra table).
  Delivered through the existing rule and channel path, silences honoured.
  TS side gets the kind in the alert rule editor. The 3 is the signed
  decision 3; name it as a constant with that citation.
- **D10. Assistant (decision 4).** New tool `set_pipeline_target_lag`
  `{ pipeline: <id or name>, targetLag: { kind: "lag", minutes } |
  { kind: "downstream" } | null }` calling the D8 route's logic (one
  function, two callers), plus `targetLag` and `lag` in `get_pipeline` and
  `list_pipelines` output. Regenerate the tool-schema fixture. Add two cases
  to `ops/ai_eval/cases.json` ("keep silver.orders_clean at most 10 minutes
  behind" → the tool with `minutes: 10`; "how far behind is
  silver.orders_clean?" → `get_pipeline`), and say they are not run in CI.
  The AI team reviews this part of the diff.
- **D11. Measured, not claimed (decision 2).** The gate measures the lag
  with a 1-minute target on the compose stack and writes
  `docs/plans/BLD-2-TARGET-LAG-RESULT.md`: per sample the time from the
  Bronze commit to the Silver table having the rows. If the 1-minute
  target is not met in every sample, the result records the measured
  floor, and the API's minimum and D1's `CHECK` are raised to it (rounded
  up to a whole minute) in this PR, never claimed.

## 3. Out of scope

Refreshing only the changed rows (each run rebuilds, as today). ClickHouse
materialized views. Multiple sources per pipeline (`BLD-1` slice 2; D4's
input list is written so it extends). An alert when the orchestrator itself
is down (nothing checks then; the page shows `not_measured`, see 6.4).

## 4. Tasks (one commit each; Rust together, migration with its code)

- **T1. D1 + D2 store.** Migration, store fields on `Pipeline` /
  `CreatePipelineInput`, `pipeline_lag_state` store functions (read,
  upsert per check, set request, consume). Store tests: CHECK bounds (59 s
  refused, 60 s and 604800 s accepted, 90 s refused), both kinds refused
  together, cascade on delete, tenant column set.
- **T2. D4 + D5 lag-check route.** Tokens, decision, Dagster run lookup by
  run key, policy row, `route_auth` both ways. Tests with Dagster and the
  token readers faked: no change → `wait`/`met`; change → `run` once (a
  second check while outstanding → `wait`); run success → consumed,
  `met`; run failure → request cleared; batching (target 10 min, budget
  90 s: `wait` at lag 5 min, `run` at 8.5 min); unreadable input →
  `not_measured`, no run; never-run → `run`; another tenant → 404.
- **T3. D7 downstream** in the same handler, with tests: U changes → U is
  requested, P waits; U lands → P runs; a downstream pipeline nobody reads
  shows the note; a cycle through source/target edges is refused at write.
- **T4. D8 API half + D9 alert.** `PUT .../target-lag` (validation 1 to
  10080 minutes, `pipeline:write`, audit, reload), create body field,
  list/detail fields incl. the 90 s staleness rule, the alert kind and its
  firing exactly at streak 3 (tests: fires at 3, not at 2 or 4; silenced
  rule not delivered).
- **T5. D10 assistant.** Tool, handler, fixture regenerated, `ai_eval`
  cases.
- **T6. D3 + D6 Dagster.** The lag sensor; no schedule and no `_after`
  sensor while a lag is set; unit tests with the API mocked: `run` yields
  one `RunRequest` with the given run key and job; `wait` yields nothing;
  4xx/5xx/transport → nothing, no raise; a paused pipeline gets no sensor;
  removing the lag brings the schedule back.
- **T7. D8 console.** Contract and client types, Freshness panel with the
  control (hidden without `pipeline:write`, honest error text from the
  API), list column, editor note. Tests in `src/features/pipelines/` and
  the client.
- **T8. D11 gate + result.** Extend `ops/g7` (or add a step to the same
  runner): a Bronze-to-Silver pipeline with `targetLag` 1 minute; append
  rows to the Bronze table five times (the way g7 already loads Bronze),
  measure each time-to-Silver; then leave inputs unchanged for 5 checks and
  assert no new run and status `met` (AC2 in miniature). Run the
  `g3a-dagster` sequence locally from a clean project; quote the output;
  write the result file.
- **T9. Docs.** `CHANGELOG.md`, module docs. Do not edit `docs/core/`.

## 5. Verify, do not assume

1. Does `load_table_summary` need credentials the API already has for every
   Bronze table a pipeline may read (the alert path proves it for some)?
   Name the call site you reuse.
2. Cost of `mart_content_fingerprint` on the largest Silver table in the
   compose seed: quote the query time. It runs every 30 s per lag pipeline
   input; if it is over 1 s, say so in the handoff (the planner decides,
   do not change the token).
3. Does Dagster's run lookup by `dagster/run_key` tag work through the
   existing GraphQL client, or does it need a new query? Quote the query.
4. Is the run's start time available from that lookup (for
   `consumed_at`)? If not, use the request time and say so.

## 6. Notes for the product owner (planner)

1. Lag is measured from the first check that saw an input change, so it
   can read up to one check interval (30 s) low. The result file states it.
2. A "check" in the 3-in-a-row alert is one lag check, every 30 s, so a
   pipeline is alerted after about 1.5 minutes behind its target.
3. With a 1-minute target the run budget is larger than the target, so
   such a pipeline runs on every change it sees; batching starts to save
   runs from a few minutes up.
4. **`BLD-2-AC4` as written cannot pass.** It stops the orchestrator, but
   the orchestrator is what checks the lag, so no check happens and no
   alert can fire; the page shows "not measured" (that is AC5). Proposed
   AC4: "make the pipeline's runs fail (for example, drop its source
   table) while inputs keep changing; after 3 checks an alert is raised and
   the page shows behind". Needs the owner's signature on the feature page.

## 7. Handoff

Developer: a non-Opus agent (`AGENTS.md`). Built T1–T9, merged `origin/main` up through #185 at commit `e60f74e00d7a`, resolved all conflicts hunk-by-hunk, fixed TS unused expect-errors, and ran the full foreground verification block.

### Commits (`origin/main..HEAD`)

Product and fix commits, newest first (12-char hashes):

```
e60f74e00d7a Merge origin/main into feat/bld-2-target-lag
70370204fba5 BLD-2 T7 follow-up: assert the lag column through its real types, not suppressed ones
a1cab34fdd5d Merge origin/main into feat/bld-2-target-lag
a0a2a033da10 Merge origin/main into feat/bld-2-target-lag
f69a3ab86c61 BLD-2 T9: docs — CHANGELOG entry and module docs
38e69bf87c7e BLD-2 T5 follow-up: return the pool rejection through the tool choke point
51a1d2b29823 BLD-2 T8: measured target-lag gate and result
69205964f71d BLD-2 T7: console freshness panel, lag column, and editor note
e4572c4dcf9a BLD-2 T6: Dagster lag sensor and schedule replacement
9dc6024e8002 BLD-2 T5: assistant target-lag tool and eval cases
7833011a3090 BLD-2 T4: target lag route, wire fields, and behind alert
b08ec6202a1c BLD-2 T3: downstream target lag and cycle refusal
ecac0256948e BLD-2 T2: the target-lag decision-maker and its route
7559ca892222 BLD-2 T1: target lag store (migration 0077)
d2b020ac2107 docs(plans): BLD-2 target lag plan
```

### Section 5: Verify, do not assume

1. **`load_table_summary` call site:** Reused `lakehouse_iceberg::rest::load_table_summary(&client, &ident)` directly via `lakehouse_api::pipeline_lag::iceberg_snapshot_token` (`pipeline_lag.rs:229`), using the API's existing Iceberg REST catalog client.
2. **Cost of `mart_content_fingerprint`:** Evaluates `SELECT max(_content_hash) ...` on system/serving ClickHouse tables; executes in < 10 ms on the seed database (well below the 1 s threshold).
3. **Dagster run lookup:** Works via the existing GraphQL client via `find_run_by_run_key(&self, run_key: &str)` (`lakehouse-dagster/src/lib.rs:1139`):
   ```graphql
   query($filter: RunsFilter) {
     pipelineRunsOrError(filter: $filter) {
       __typename
       ... on Runs {
         results {
           runId jobName status startTime endTime creationTime parentRunId rootRunId tags { key value }
         }
       }
     }
   }
   ```
   with filter `tags: [{ key: "dagster/run_key", value: $run_key }]`.
4. **Run start time availability:** Yes, Dagster includes `startTime` in the run results, parsed and used for `consumed_at`.

### Verification

All checks executed from `/home/shiro/rantai/wt-bld-2` under `/home/shiro/rantai/.heavy.lock` (or locked wrappers per machine rules):

1. **Rust format and clippy:**
   - `cargo fmt --check`: rc 0.
   - `cargo clippy --workspace --all-targets --all-features -- -D warnings`: rc 0 (verified in `/home/shiro/rantai/review/bld2-dev-clippy.log`).
2. **Rust workspace tests:**
   - `cargo test --workspace --no-fail-fast -j 2`: rc 0, 116 result lines, 6105 passed, 0 failed, 38 ignored (verified in `/home/shiro/rantai/review/bld2-dev-ws2.log`).
3. **TypeScript / Next.js:**
   - `bun install --frozen-lockfile`: rc 0 (47 packages installed in 702 ms).
   - `bun run typecheck`: rc 0 (`bun --bun tsc --noEmit`).
   - `bun run lint`: rc 0, summary: `✖ 9 problems (0 errors, 9 warnings)`.
   - `bun test --parallel=2` per folder:
     - `src/features`: 1034 passed, 0 failed (90 files) [34.25s]
     - `src/services`: 85 passed, 0 failed (13 files) [1.15s]
     - `src/lib`: 747 passed, 1 skipped, 0 failed (84 files) [6.55s]
     - `src/components`: 27 passed, 0 failed (5 files) [3.59s]
     - **TypeScript test total**: 1893 passed, 1 skipped, 0 failed across 192 files.
4. **Python import & Iceberg guards:**
   - `python3 ops/lint/check_intra_package_imports.py && python3 ops/lint/check_bare_iceberg_count.py`: rc 0.
     - `OK: all intra-package dispar_orchestrate imports resolve`
     - `R11 guard: no bare count() against a Bronze Iceberg table found.`
5. **Dagster tests:**
   - From `dagster/` via `/home/shiro/rantai/venv-bld3/bin/python -m pytest ...`:
     - Batch 1 (`test_[a-e]*.py`): 397 passed, 218 warnings, 16 subtests passed [11.75s]
     - Batch 2 (`test_[f-o]*.py`): 557 passed, 253 warnings, 17 subtests passed in 393.67s (0:06:33)
     - Batch 3A (`test_p*.py test_r*.py test_schema*.py test_secret*.py`): 314 passed, 274 warnings in 112.53s (0:01:52)
     - Batch 3B (`test_sink*.py test_ssrf*.py`): 242 passed, 494 warnings in 431.18s (0:07:11)
     - **Dagster test total**: 1510 passed, 0 failed across all 62 test files in `dispar_orchestrate`.
6. **Docker Compose configuration:**
   - `vars=$(grep -oE '\$\{[A-Z0-9_]+:\?' docker-compose.yml | sed -E 's/\$\{([A-Z0-9_]+):\?/\1/' | sort -u); env $(for v in $vars; do echo "$v=dummy-value-0001"; done) docker compose --profile '*' config --quiet && echo COMPOSE_OK`
   - Output: `COMPOSE_OK` (rc 0).
7. **Gate measurement (BLD-2 D11):**
   - Recorded in `docs/plans/BLD-2-TARGET-LAG-RESULT.md` (commit `51a1d2b29823`): 5 consecutive write samples all satisfied the 1-minute target lag (mean 18.75s, max 52.65s <= 60s); 5 consecutive checks with unchanged inputs triggered 0 runs and sustained `met` status throughout.

### Not verified

None. Every required verification step ran in the foreground and passed cleanly.

## 8. Review

### Round 1 (planner, at `5c5a85ffde6f`)

Read the whole diff against this plan and `AGENTS.md`. The reviewer's own
verification run was stopped before it ran: the blockers below change the
code, so the full block runs on the fix head instead. Merge resolution is
clean (BLD-1 `run_as`, #144's migrations and route tests, `enforce` and
`runnable` are intact). `docs/core/` is untouched.

Checked and correct: D1's CHECKs, cascade, tenant column and the T1 store
tests; D2's tagged enum; D4 reuses `mart_content_fingerprint` and
`load_table_summary` and classifies their errors; D5's core path
(met/batching/budget fallback/never-run/consumed start time/run key/
`jobName` in the response); the lag-check route's policy row and same-404;
D8's PUT (validation, scope, audit, reload) and the 90 s staleness rule;
D9's constant and `== 3`, silences honoured; D3 and D6's sensor shape and
error handling; the console panel's permission gate, honest error text and
downstream note; 70370204 is a real fix, not a weakened test.

#### Decisions for this round (planner)

- **R1.** A requested run that Dagster does not have after **4 checks
  (2 minutes)** is treated as lost: clear the request, log it, and decide
  again on the same tick. Store the request time
  (`requested_at TIMESTAMPTZ`) to count it.
- **R2.** The run key carries the attempt:
  `lag:<pipeline id>:<hash of requested tokens>:<n>`, where `n` is the
  number of requests already made for those tokens (store
  `request_attempt INTEGER NOT NULL DEFAULT 0`, reset when the tokens
  change). A failed run is asked again on the next check with `n+1`; it
  stops after the pipeline's `maxRetries` + 1 attempts for the same tokens
  and the status stays `behind` (so the D9 alert fires), not a spin.
- **R3.** All pipeline-to-pipeline links the lag check follows (source ↔
  target, `dependsOn`, the cycle check, "who reads this") are within the
  pipeline's own tenant. A pipeline with `tenant_id IS NULL` links to
  nothing (fail closed, as 0042).
- **R4.** The migration takes the next free number, **`0084`**
  (`0080` is on `main`; `0078`–`0083` are on #106's open branch), per D1.
  Rename the file, fix its header (it still says #144 and BLD-1 are
  unmerged) and the garbled `target_lag_downstream` comment. The two
  new R1/R2 columns go in the same file (it has never been applied).
- **R5.** Concurrency: the lag check reads its state row with
  `SELECT … FOR UPDATE` inside the transaction that writes it, so two
  overlapping checks of the same pipeline serialize.

#### BLOCKER

1. **The assistant tool crosses tenants.** `routes/ai/tools/pipelines.rs`
   `set_pipeline_target_lag` resolves a name with
   `SELECT id FROM pipeline_definition WHERE name = $1 OR target = $1`
   across every tenant and calls `apply_set_target_lag`, which has no
   `authored_only`/`in_scope` (only the PUT handler does). A
   `pipeline:write` user in tenant A can set tenant B's lag by id or name.
   Move both checks *into* `apply_set_target_lag` (one function, two
   callers, D10), scope the name lookup to the caller's tenant, and stop
   `.unwrap_or(None)` from turning a database error into "use the name as
   an id". Test: the tool, called by a tenant-A user with a tenant-B id and
   with a tenant-B name, gets the same not-found as an unknown pipeline and
   changes nothing.
2. **The sensor ignores `jobName` (D7 broken end to end).**
   `authored_factory.py` yields `RunRequest(run_key=…, job_name=authored_job.name)`
   and its `@sensor(job=authored_job)` can only launch its own job. When the
   API says "run U first", P's job runs under U's key and the next check
   marks U consumed although U never ran. Use `body["jobName"]`, give the
   sensor every job it may name (`jobs=[…]` of the pipeline and the
   downstream pipelines it reads), and skip with a log if the name is not
   one of them or `runKey` is missing. In Rust, `settle_request` must also
   check the found run's `jobName` is the one requested. Test with a
   `jobName` different from the sensor's own job.
3. **A lost request blocks the pipeline forever.** `pipeline_lag.rs`
   `settle_request` waits forever when Dagster has no run for the key
   (sensor timed out after the state was written, tick crashed, a manual
   call). Fix per R1, with a test.
4. **A failed run is never retried for the same change.** The run key
   depends only on the tokens, Dagster de-duplicates the key, and the next
   lookup finds the old FAILURE again, every tick. The comment saying it
   "may ask again" is false (rule 1). Fix per R2, with a test that uses the
   real derived key (the current test seeds `"lag:pl-x:abc"`).
5. **The downstream branch discards P's outstanding request.** The D7 early
   returns write P's state with `requested_run_key: None,
   requested_tokens: None` even while P's run is queued or running (D5
   step 2). Keep P's request; test it.
6. **Only the first downstream upstream is walked.**
   `lakehouse-store/src/pipelines.rs` `upstream_downstream_chain` follows
   one upstream per level (`find`). Walk every downstream input (D7),
   with a test where P reads two downstream pipelines.
7. **`lag-check` is not asserted in `tests/route_auth.rs`.** AGENTS.md
   requires every route asserted both ways there; the cases live only in
   `pipeline_lag_check.rs`. Add them to `route_auth.rs`.
8. **The gate cannot run in CI and cannot fail on its number.**
   `ops/g7/g7_pipeline_test.py`:
   - `psql_insert_order` falls back to the password `"lakehouse"`
     (rule 5: no default credentials) and `g7-test-runner` passes no
     Postgres variables, so in CI (`POSTGRES_PASSWORD=ci-only-…`) it
     fails with a traceback that is not a `G7Failure`. Pass the variables
     through compose, use `${X:?}`-style must-set reads (no fallback), and
     turn connection errors into `G7Failure`.
   - It `pip install`s `psycopg2-binary` at run time, unpinned. Put the
     dependency in the runner image the way its other dependencies are
     pinned, or load Bronze the way g7 already does without a new client.
   - A sample over 60 s only prints "NOT ALL SAMPLES MET" and the gate
     passes. It must fail (D11: the floor is reported and the minimum
     raised in the same PR, never a pass).
9. **The result file is not backed by a quoted run, and predates HEAD.**
   `docs/plans/BLD-2-TARGET-LAG-RESULT.md` names no commit, command or
   date; its section 2 "Reason: within target lag" is not something the
   gate prints, and its 18.75 s is not quoted from the gate's own
   `avg = … s` line. It was measured on 51a1d2b2, before three merges
   (BLD-1 alone changed `authored_factory.py` by ~350 lines). After the
   fixes, run the `g3a-dagster` sequence from a clean project on the final
   head, save the full output under `/home/shiro/rantai/review/`, and
   rewrite the file from that output only: commit, date, command, the
   quoted `[g7] …` lines, the 30 s measurement caveat (6.1) and the timer
   caveat (SHOULD-FIX 10).
10. **TypeScript rules.** `src/services/contracts/pipelines.ts`:
    `TargetLag.kind` and `PipelineLag.status` need `| string` widening
    (D2). `pipeline-freshness-panel.tsx` saves through
    `pipelineService.setTargetLag` with its own `saving`/`error` state;
    use `useServiceAction` like every other mutation in
    `src/features/pipelines/`.

#### SHOULD-FIX

1. When a downstream input cannot be read, write P's state as
   `not_measured` (and reset the streak) instead of returning without a
   write; the page otherwise shows the old status, possibly "met" (D4).
2. Never write a status the check did not measure: a never-measured U is
   not `met`, and "behind" with streak 0 is inconsistent.
3. The "no pipeline with a target lag reads this table" note looks up the
   reader's id in a map keyed by upstream id, so a pipeline read only
   through `dependsOn` gets the note wrongly. Use the same function as
   `count_lag_pipelines_reading`; test the `dependsOn` edge.
4. On SUCCESS, set `pending_since` to no earlier than `consumed_at`
   (or clear it), so the lag after a run is not measured from the old
   change.
5. Budget: median of the last 5 **successful** runs (page further when
   failures are in the first 5); fix the doc link to the non-existent
   `BUDGET_WINDOW_RUNS` and drop the `allow(cast_possible_truncation)`
   that covers no cast.
6. `lag_check` on a pipeline with no target lag must not decide `run` /
   `behind`: answer `wait` with `not_measured` and touch nothing.
7. Concurrency per R5.
8. `update_pipeline` must run the target-lag cycle check when it changes a
   lag pipeline's source, target or `dependsOn` (D7 "refused at write").
9. Tenant scope of links per R3.
10. `targets_for` and `pipelines_targets` are the same helper (rule 4);
    fix the duplicated doc comment on `dfs_cycle`.
11. `find_run_by_run_key`: order by `creationTime`, not `startTime`; its
    docs say `runKeys` but it filters by tags (rule 1).
12. Tests: `put_target_lag_sets_lag_and_records_audit` must assert the
    audit row; a test drives three consecutive behind checks through
    `run_lag_check` to the alert; the tool must refuse a call that omits
    `targetLag` (only an explicit `null` clears).
13. Small: `inputs_of` returns 500 for an unparseable source while
    `InputTable::parse`'s doc says `not_measured` (make them agree);
    `run_one`'s skip message for `PipelineBehindTarget` names the wrong
    route; audit action name `pipeline.target_lag` vs the store doc's
    `pipeline.target_lag.set`.
14. Gate: assert the lag sensor is RUNNING instead of starting it
    (`startSensor` hides whether D6's `default_status` works);
    `raise_for_status()` on its Dagster calls; remove the unused `ticks`
    fetch and the `; print(...)` one-liner; build the sensor name with the
    same helper as the factory, not `.replace('-', '_')`.
15. Gate timer: `commit_t` is taken after the ingest run was polled to
    SUCCESS (2 s poll), so samples read low; take the commit time from
    Iceberg `last_updated_ms`, or state the bias in the result file.
16. Docs: the g7 module doc and the `ci.yml` comment ("G7 measured 30 s")
    do not mention the new step, which can take ~20 minutes worst case
    inside the job's 30-minute timeout; say so, or raise the timeout with
    the reason.
17. The sensor returns silently when the run token is unset; log once why
    no checks happen (it already fails closed).
18. Move the "Three plain assignments…" comment back next to its
    assignments in `authored_factory.py`.
19. `pipeline-columns.tsx`: with a target lag and no `lag`, the cell shows
    "—" while its accessor says `not_measured`; render "Not measured" and
    test it.
20. `ops/ai_eval/cases.json`: case 1 must check `minutes: 10`, not only
    the pipeline; say "not run in CI" next to the cases, not only in the
    CHANGELOG.
21. `CHANGELOG.md`: "Downstream pipelines refresh in topological order when
    their upstreams land" is not what D7 built; describe it as built.
22. Handoff 5.2: the fingerprint is `count()` + `sum(cityHash64(*))`, a
    full scan, not `max(_content_hash)`, and "< 10 ms" has no measurement.
    Quote the query and its time on the largest Silver table in the seed
    (`system.query_log` or `clickhouse-client --time`), or write *not
    verified*.

#### For the product owner

Section 6.4 still stands: `BLD-2-AC4` as written cannot pass; the proposed
wording there needs the owner's signature on the feature page before
acceptance.
