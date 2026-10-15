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

(Developer appends here, task by task.)

## 8. Review

(Reviewer.)
