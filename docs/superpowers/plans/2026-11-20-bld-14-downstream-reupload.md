# `BLD-14`: a re-upload runs the pipelines downstream too

Planner and reviewer: Claude Opus. Developer: **Hy4 flash** (OpenCode), one
agent for the whole PR. The planner does not build any of it and the
developer does not change a decision below; if the code at an anchor is not
what this plan says, stop and report the mismatch (`AGENTS.md`).

Base: `origin/main` at `c7cd01a76`. Branch: `bld-14-downstream-reupload`
(this plan is its first commit). Backlog: `BLD-14` (Task, under `DATA-19`),
decision `DEC-22` (product owner, 2026-11-12), QA case `LH-DATA-19-02`.
Feature page: `docs/core/features/uploads-feed-pipelines.md`. No spec of its
own (a Task); `docs/core/specs/data-19.md` has no number this changes.

**Blocked by:** nothing open. `BLD-14` waited for `BLD-1` part 1's order
between pipelines (`input_tables`, plan
`2026-11-12-bld-1-part-1-steps-and-sql.md` D11), which is on `main`.

## 1. Why

`DATA-19` decision 2 starts only the pipelines that read the re-uploaded
table: a pipeline whose first input is `bronze.<table>` and whose `dependsOn`
lists `file_ingest_job` (the "Run again after this table is re-uploaded"
switch). A Silver to Gold pipeline that reads that Silver pipeline's output
does not run, so a chart on Gold changes only after a manual run (QA
`LH-DATA-19-02`). `DEC-22`: *the pipelines downstream of the ones that read
the uploaded table run too.* The backlog row adds: each one starts after the
one before it succeeds, once per re-upload, and the chain stops at a failure
with the reason shown.

Read on `main` at `c7cd01a76`:

- The only re-upload trigger is the `authored__<id>_after` run-status sensor
  (`authored_factory.py:1043` `build_authored_dependency_sensor`). It fires on
  a SUCCESS of an upstream in `dependsOn`, requires ALL of them fresh, and
  filters `file_ingest_job` by the loaded table
  (`:1215`-`:1272`, `_latest_record_for_table` `:939`). It yields one
  `RunRequest(run_key="authored-deps:…")` (`:1280`) with no tags, so nothing
  downstream can tell that a run belongs to a re-upload.
- Nothing launches a pipeline because another pipeline's *output* changed,
  unless the author listed it in `dependsOn` by hand. The order between
  pipelines exists only as data: `pipeline_definition.input_tables` vs
  `target` (store `pipelines.rs:305` `input_tables_of`), mirrored in Python
  by `_pipeline_sources` (`authored_factory.py:1412`) and `_pipeline_target`
  (`:1401`), which the lag sensors already use to walk downstream pipelines
  (`build_authored_lag_sensors`, `:1449`).
- A pipeline that did not run has no run, so the run list cannot say why;
  the only "skip reason" surface is sensor ticks, which the page shows only
  for a pipeline with a cron schedule (`schedule-history.tsx`, rendered at
  `pipeline-detail-page.tsx:585` with `hasSchedule`) and which roll off after
  50 ticks (`authored_pipelines.rs:82` `MAX_TICKS`). That is not a place a
  reason can be "shown".

## 2. Decisions

Signed:

- **D1 (`DEC-22`, product owner 2026-11-12).** A re-upload runs the pipelines
  downstream of the pipelines that read the uploaded table, not only those.
- **D2 (`BLD-14` row, product owner 2026-11-12).** Each downstream pipeline
  starts after the pipeline(s) before it succeed; it runs once per
  re-upload; a failure stops the chain below it, and the reason is shown.

Made by the planner from the code (not product choices):

- **D3. What "downstream" means.** Pipeline `B` is directly after pipeline
  `A` when they are in the same tenant, `A != B`, and `A`'s target
  (`zone.table`) is one of `B`'s inputs. This is `BLD-1` part 1 D11's order:
  `_pipeline_target` and `_pipeline_sources` in Python, the same rule as the
  store's `input_tables_of`. No new field on `GET /api/pipelines/runnable`:
  it already carries `definition.inputs`, `definition.targetZone/Table`,
  `tenantId`, `status`, `targetLag` and `dependsOn` (`RunnablePipeline`,
  store `pipelines.rs:1790`), and rule 4 says reuse the helpers rather than
  derive the order a second time.
- **D4. Where the chain starts.** A chain starts at a SUCCESS run `R` of
  `file_ingest_job` that loaded `bronze.<T>`. Its first pipelines are the
  ones `DATA-19` already starts (first input `bronze.<T>`, `file_ingest_job`
  in `dependsOn`, `status = ready`, no target lag). With the switch off on a
  first pipeline, nothing below it runs either (`DATA-19-AC6` holds).
- **D5. The chain's identity is a run tag.** Every run the chain starts, and
  every first pipeline the `_after` sensor starts from a file load, carries
  two tags: `lakehouse/reupload-root = <R's run id>` and
  `lakehouse/reupload-table = <T>`. The chain's runs use
  `run_key = "reupload-chain:<R>:<pipeline id>"`; the orchestrator never
  launches the same `run_key` twice from one sensor, which is "once per
  re-upload" by construction.
- **D6. Who launches.** One plain `@sensor` per pipeline that the chain can
  reach, named `authored__<safe_id>_chain`, every 30 seconds, `default_status
  = RUNNING` (the same reason as `_after`: a chain sensor shipped STOPPED
  never fires). For each recent root `R` that can reach it, it launches the
  pipeline when every *parent in the chain for `T`* has a SUCCESS run tagged
  `R`. Parents are D3's direct predecessors that are themselves in `T`'s
  chain; a second input that the re-upload does not reach is not waited for.
- **D7. One launcher per re-upload.** When a pipeline has chain parents for
  table `T`, the chain sensor owns its run for that re-upload, and its
  `_after` sensor skips (with a `SkipReason` naming `BLD-14`) any tick whose
  fresh upstream run is either a `file_ingest_job` load of `T` or a run
  tagged `lakehouse/reupload-root` of one of those parents. Everything else
  `_after` does is unchanged: a manual or scheduled upstream run still
  triggers it as today.
- **D8. The reason is a stored record, shown on the pipeline's page.**
  New table `pipeline_chain_step` (migration `0153`). The chain sensor posts
  one record per (re-upload, pipeline) to an orchestrator-only route; the
  pipeline page reads the newest one and shows one sentence (section 4). The
  record holds codes and ids, never orchestrator or engine text (principle
  4); the failed pipeline's own reason stays on *its* failed run, as
  `BLD-8` shows it today.
- **D9. Outcomes.** `started` (with the run id), `not_run` with a reason
  code (`upstream_failed`, `upstream_cancelled`, `wait_expired`) and the
  pipeline that caused it, or `superseded`. Section 4 has the sentences.

Planner defaults (minimal; each one is a choice the product owner may change
before or after the build, and none blocks starting):

- **P1. Paused or target-lag pipelines are not in the chain.** *(planner
  default, owner may change)* A pipeline that is `paused`, or has a target lag
  (BLD-2 D3 already gives it its own trigger), is neither launched nor waited
  for, and a pipeline reachable only through it does not run for that
  re-upload. No record is written for those.
- **P2. A cycle is left out.** *(planner default, owner may change)* Pipelines
  that read each other's outputs in a loop are not chain nodes (a cycle would
  wait for itself); anything reachable only through the loop does not run.
  Saving such pipelines is not refused (out of scope).
- **P3. No depth limit.** *(planner default, owner may change)* The chain is
  bounded by the tenant's pipelines; each runs at most once per re-upload.
- **P4. Two re-uploads close together: the newer one wins.** *(planner
  default, owner may change)* If a newer re-upload of the same table has
  reached the window before an older chain launched a pipeline, that
  pipeline is not launched for the older one (`superseded`) and runs once,
  for the newer data. A pipeline never runs twice at the same time: while it
  has a queued or running run, the chain waits.
- **P5. Waiting ends after six hours.** *(planner default, owner may change)*
  If the parents have not all finished six hours after the re-upload's load
  ended (the authored run limit, `DEFAULT_RUN_LIMIT_SECONDS`,
  `authored_factory.py:129`), the pipeline is recorded `not_run /
  wait_expired`, naming the first parent without a SUCCESS.
- **P6. A retry does not resume the chain.** *(planner default, owner may
  change)* Retrying the failed pipeline by hand does not start the pipelines
  below it for that re-upload; a new re-upload or a manual run does.
- **P7. Connector loads are unchanged.** *(planner default, owner may change)*
  `DEC-22` is about a re-upload; a connector load (`ingest_job`) keeps
  starting only the pipelines that read its table (`BLD-4`).
- **P8. No per-pipeline switch below the first.** *(planner default, owner
  may change)* The first pipeline's "Run again after this table is
  re-uploaded" switch governs the whole chain below it.
- **P9. Records kept: the newest 20 per pipeline.** *(planner default, owner
  may change)* Older rows are deleted in the same statement that inserts.
- **P10. Only re-uploads after the sensor first ran.** *(planner default,
  owner may change)* A new or re-created chain sensor does not act on loads
  that ended before its first tick (its cursor floor), so creating a Gold
  pipeline does not replay last week's re-upload into it.

Assistant parity: `BLD-14` is not a BI feature and adds no assistant tool;
no `tool_schemas.json` change.

## 3. Security rules (conditions, not options)

- `POST /api/pipelines/runnable/{id}/chain-step` admits only a service
  identity or a Platform Admin, by the same handler check as `runnable`
  (`authored_pipelines.rs:1669`-`1680`); the `POLICY_TABLE` row is the
  `pipeline:read` floor like `lag-check` (`policy.rs:624`).
- `GET /api/pipelines/{id}/reupload-chain` is `pipeline:read` and runs
  `authored_pipelines::in_scope` first, exactly like `runs`
  (`pipelines.rs:402`-`421`); another tenant's id answers the same 404 as an
  unknown one.
- A cause pipeline's name is returned only when that pipeline is in the
  same tenant as the record; otherwise `causePipelineName: null`.
- Every value posted is validated and bound; no `format!` of a value. A
  store failure answers the classified `"database error"`. The record holds
  no text from the orchestrator or the engine.

## 4. Contracts

**Migration `0153_pipeline_chain_step.sql`.** `0151` is the highest on
`main`, `0152` is reserved by `data-4-silver-publish`; no open branch has
`0153` (checked over every `origin/*` branch on 2026-11-20). Recheck before
the PR opens; if `0153` is taken, use the next free number and say so in the
handoff. Why-header (BLD-14, DEC-22, D8, P9), then:

```sql
CREATE TABLE pipeline_chain_step (
    root_run_id       TEXT NOT NULL,         -- the file_ingest_job run of the re-upload
    pipeline_id       TEXT NOT NULL REFERENCES pipeline_definition(id) ON DELETE CASCADE,
    tenant_id         UUID,                  -- the pipeline's, copied at insert
    source_table      TEXT NOT NULL,         -- 'bronze.<table>' the re-upload loaded
    outcome           TEXT NOT NULL CHECK (outcome IN ('started', 'not_run', 'superseded')),
    reason            TEXT CHECK (reason IN ('upstream_failed', 'upstream_cancelled', 'wait_expired')),
    run_id            TEXT,                  -- outcome = 'started'
    cause_pipeline_id TEXT,                  -- outcome = 'not_run'
    recorded_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (root_run_id, pipeline_id),
    CHECK ((outcome = 'not_run') = (reason IS NOT NULL)),
    CHECK ((outcome = 'started') = (run_id IS NOT NULL))
);
CREATE INDEX pipeline_chain_step_newest ON pipeline_chain_step (pipeline_id, recorded_at DESC);
```

`cause_pipeline_id` has no foreign key on purpose (a deleted cause keeps the
record; the read answers its name as `null`). Say so in the header.

**`POST /api/pipelines/runnable/{id}/chain-step`** (orchestrator only). Body:

```json
{ "rootRunId": "…", "sourceTable": "bronze.orders", "outcome": "not_run",
  "reason": "upstream_failed", "runId": null, "causePipelineId": "pl-…" }
```

`rootRunId`: 1 to 64 chars of `[0-9a-zA-Z-]`. `sourceTable`: `bronze.` plus a
name `lakehouse_core::ident::Ident::new` accepts. The outcome/reason/runId
pairing of the CHECKs above, plus `causePipelineId` required for `not_run`
and absent otherwise, and in the same tenant as `{id}`. Any violation: 422
with a fixed sentence naming the field. Unknown `{id}`: 404. Answer: `200
{"recorded": true}` on insert, `{"recorded": false}` when the pair exists
(first write wins; a sensor retry changes nothing).

**`GET /api/pipelines/{id}/reupload-chain`.** Answer:

```json
{ "last": null }
{ "last": { "rootRunId": "…", "sourceTable": "bronze.orders", "outcome": "not_run",
            "reason": "upstream_failed", "runId": null,
            "causePipelineId": "pl-…", "causePipelineName": "Silver orders",
            "recordedAt": "2026-11-20T10:02:11Z" } }
```

TypeScript (`src/services/contracts/pipelines.ts`): `type ReuploadChainStep`
mirroring the above, `outcome: "started" | "not_run" | "superseded" | string`,
`reason: "upstream_failed" | "upstream_cancelled" | "wait_expired" | string |
null`; `getReuploadChain(id, signal)` on `PipelineService`.

**The sentences** (console; tests pin them; `<T>` is `sourceTable`, `<when>`
is `formatDateTime(recordedAt)`, `<Name>` links to `/pipelines/<causeId>`;
with no name: "a pipeline before it"):

- started: `Ran after the re-upload of <T> on <when>.` (run id as a link
  that opens the run, as the schedule history does)
- not_run / upstream_failed: `Not run after the re-upload of <T> on <when>:
  <Name> failed first.`
- not_run / upstream_cancelled: `Not run after the re-upload of <T> on
  <when>: <Name> was cancelled.`
- not_run / wait_expired: `Not run after the re-upload of <T> on <when>:
  <Name> did not finish within 6 hours.`
- superseded: `Not run for the re-upload of <T> on <when>: a later
  re-upload of the same table replaced it.`
- any other code: `After the re-upload of <T> on <when>: <outcome>.`
  (widened union, never a crash).

## 5. Anchors (`main` at `c7cd01a76`)

- `dagster/dispar_orchestrate/authored_factory.py`: `:129`
  `DEFAULT_RUN_LIMIT_SECONDS`; `:283` `_dagster_safe_name`; `:746`
  `build_authored_definitions`; `:789` `build_authored_dependency_sensors`;
  `:849` `FILE_INGEST_OP_CONFIG`; `:902` `PER_TABLE_UPSTREAM_SCAN_LIMIT`;
  `:919` `_run_config_value`; `:939` `_latest_record_for_table`; `:1043`
  `build_authored_dependency_sensor` (paused `:1105`, decorator `:1135`,
  upstream loop `:1215`, `RunRequest` `:1280`); `:1401` `_pipeline_target`;
  `:1412` `_pipeline_sources`; `:1439` `_pipeline_tenant`; `:1444`
  `_pipeline_depends_on`; `:1449` `build_authored_lag_sensors` (the
  same-tenant walk to copy the shape of); `:1517`-`:1527` module-level
  assembly.
- `dagster/dispar_orchestrate/definitions.py:156`-`159` the authored sensor
  lists.
- `dagster/dispar_orchestrate/pipeline_events.py:130` `_post_event` (bounded
  retry, `timeout=`, `raise_for_status()`): the posting pattern to reuse.
- `dagster/dispar_orchestrate/test_authored_factory.py:791`-`:960` the fake
  run, record, instance and context; `:976` `TestDependencySensor`; `:1508`
  `TestFileIngestUpstreamIsFilteredByTable`.
- `rust/crates/lakehouse-store/src/pipelines.rs:305` `input_tables_of`;
  `:1790` `RunnablePipeline`; `:1877` `record_pipeline_run_event` (insert-once
  shape).
- `rust/crates/lakehouse-api/src/routes/authored_pipelines.rs:115`
  `sensor_name`; `:1669` `runnable` (service-identity guard); `:2650`
  `schedule_ticks`.
- `rust/crates/lakehouse-api/src/routes/pipelines.rs:402` `runs` (scope
  check); `:3746` `authored_status`, `:3780`-`:3791` the `_after` sensor
  switch.
- `rust/crates/lakehouse-api/src/policy.rs:624` the `lag-check` row;
  `audit_coverage.rs:278` (`EX_LAG_CHECK`) and `:288` (`EX_MACHINE`);
  `routes/mod.rs:491` the `runs` route and `:619`-`:630` the `runnable`
  router.
- `rust/crates/lakehouse-api/tests/route_auth.rs:3665` the `lag-check`
  both-ways assertion to copy.
- `src/features/pipelines/pipeline-detail-page.tsx:167`
  `PipelineDetailPage`, `:517` "Run history" heading;
  `src/features/pipelines/schedule-history.tsx` (link-to-run pattern);
  `src/services/contracts/pipelines.ts:455`, `src/services/clients/pipelines.ts:167`.
- `ops/g9/upload_test.py:1376` `step_create_pipeline_over_uploaded_table`,
  `:1491` `step_reupload_reruns_the_pipeline`, `:1667`
  `step_feeds_a_pipeline`.

### Overlap with PR #619 (`bld-16-joins-aggregations`, draft, not merged)

Files both touch: `rust/crates/lakehouse-api/src/policy.rs`,
`rust/crates/lakehouse-api/src/routes/mod.rs`,
`rust/crates/lakehouse-api/tests/route_auth.rs`,
`rust/crates/lakehouse-api/src/audit_coverage.rs` (only if #619's later
commits add a row; it has none at `origin/bld-16-joins-aggregations` today, so
check at rebase), `src/services/contracts/pipelines.ts`,
`src/services/clients/pipelines.ts`. Each is an append in a different place;
add rows and methods at the end of their blocks. This plan deliberately does
**not** touch `lakehouse-store/src/pipelines.rs`, its test file, or
`pipeline-detail-page.test.tsx`, which #619 changes heavily: the store code
goes in a new module and the page test in a new file. #619 does not change
what `definition.inputs` holds (zone and table), so D3 is unaffected.
Whichever merges second rebases and resolves hunk by hunk (rule 9).

## 6. Tasks

One task per commit, in order. **Push after every commit.** Per commit, run
only the scoped checks named in the task, through the shared
`CARGO_TARGET_DIR` and the machine's `cargo-locked` wrapper where it has one;
never `cargo clean`. The reviewer opens a draft PR after the first code
commit is pushed, so CI runs the whole suite. Every fix site and commit body
cites `BLD-14` and the decision (`D7`, `P4`, …) it implements.

### Rust (T1 to T4, one sitting: the migration lands with its code)

**T1. Migration and store.** `rust/migrations/0153_pipeline_chain_step.sql`
(section 4). New module `lakehouse-store/src/pipeline_chain.rs`
(`pub mod pipeline_chain;` in `lib.rs`, module doc with the why): a
`ChainStep` input type, a `ChainStepView` output type (serde camelCase),
`record_chain_step(pool, &ChainStep) -> Result<bool, StoreError>` (one
transaction: `INSERT … ON CONFLICT (root_run_id, pipeline_id) DO NOTHING`,
the tenant copied from `pipeline_definition` in the same statement, then
P9's delete of all but the newest 20 for that pipeline), and
`latest_chain_step(pool, pipeline_id) -> Result<Option<ChainStepView>,
StoreError>` (newest by `recorded_at`, `LEFT JOIN pipeline_definition c ON
c.id = s.cause_pipeline_id AND c.tenant_id IS NOT DISTINCT FROM s.tenant_id`
for the name). `# Errors` on both.
Tests: new `lakehouse-store/tests/pipeline_chain.rs` with `mod
pipeline_chain;` in `tests/it.rs`:
`the_second_record_for_the_same_reupload_and_pipeline_is_not_written`,
`the_newest_record_is_the_one_read`,
`a_cause_in_another_tenant_has_no_name`,
`a_deleted_cause_keeps_the_record_with_no_name`,
`only_the_newest_twenty_records_of_a_pipeline_are_kept`,
`deleting_the_pipeline_deletes_its_records`,
`the_table_refuses_a_not_run_without_a_reason` (the CHECK).
*Check:* `cargo fmt --check && cargo clippy -p lakehouse-store --all-targets
-- -D warnings && cargo test -p lakehouse-store --test it pipeline_chain::`.

**T2. The record route.** `POST /api/pipelines/runnable/{id}/chain-step` in
`routes/authored_pipelines.rs` next to `runnable` (or a new
`routes/pipeline_chain.rs` if the handler would push the file's functions
over the 100-line lint), registered in the `runnable` router
(`routes/mod.rs:619`); `POLICY_TABLE` row `pipeline:read` with a comment
like `lag-check`'s; `audit_coverage.rs` row `Coverage::Excluded { reason:
EX_MACHINE }`; validation per section 4, returning
`ApiResult<ApiJson<Value>>`.
Tests: new `lakehouse-api/tests/pipeline_reupload_chain.rs` (`mod
pipeline_reupload_chain;` in `tests/it.rs`):
`a_user_with_pipeline_write_cannot_record_a_chain_step` (403),
`the_orchestrator_records_a_step_once` (true, then false),
`a_not_run_step_without_a_reason_is_refused` (422),
`a_started_step_without_a_run_id_is_refused` (422),
`a_cause_from_another_tenant_is_refused` (422),
`a_source_table_outside_bronze_is_refused` (422),
`an_unknown_pipeline_answers_404`. `route_auth.rs`: both-ways assertion
like `:3665`.
*Check:* `cargo fmt --check && cargo clippy -p lakehouse-api --all-targets
-- -D warnings && cargo test -p lakehouse-api --test it --
pipeline_reupload_chain:: route_auth:: audit_coverage:: sec11_guard::`.

**T3. The read route.** `GET /api/pipelines/{id}/reupload-chain` in
`routes/pipelines.rs` beside `runs`, `in_scope` first (section 3); policy
row `pipeline:read`; no audit row (a GET). Tests in the same file:
`the_page_reads_null_before_any_reupload`,
`the_page_reads_the_newest_step_with_the_cause_name`,
`another_tenants_pipeline_answers_the_same_404_as_an_unknown_one`;
`route_auth.rs` both ways.
*Check:* as T2.

**T4. Pausing stops the chain sensor too.** `authored_pipelines.rs`:
`chain_sensor_name(id) -> String` = `format!("{}_chain", job_name(id))` with
a doc naming the Python twin. `authored_status` (`pipelines.rs:3780`)
switches `chain_sensor_name(id)` exactly as it switches `sensor_name(id)`
(a missing sensor tolerated the same way). Test (route level, the Dagster
mock as the F2 walk at `pipelines.rs:10175` sets it up):
`pausing_a_pipeline_stops_its_chain_sensor_and_resuming_starts_it` — asserts
the mock received the stop and start for `authored__<id>_chain`. Fails on
`main` (nothing asks for that sensor).
*Check:* `cargo clippy -p lakehouse-api --all-targets -- -D warnings &&
cargo test -p lakehouse-api --lib -- chain_sensor pausing_a_pipeline`.

### Python (T5 to T7)

**T5. The chain graph.** In `authored_factory.py`, after
`_pipeline_depends_on`: constants `REUPLOAD_ROOT_TAG =
"lakehouse/reupload-root"`, `REUPLOAD_TABLE_TAG = "lakehouse/reupload-table"`,
`CHAIN_WAIT_LIMIT_SECONDS = DEFAULT_RUN_LIMIT_SECONDS` (P5), and a frozen
dataclass `ChainEntry(parents: tuple[str, ...], ancestors: tuple[str, ...])`
plus a pure function `reupload_chain_graph(pipelines: list[dict]) ->
dict[str, dict[str, ChainEntry]]` (pipeline id → bronze table name →
entry), using only the existing helpers:
1. nodes: `status == "ready"`, `targetLag` is None, tenant not None (P1);
2. edges `A → B` by D3, same tenant, `A != B`;
3. drop every node on a cycle (strongly connected component of more than
   one node) (P2);
4. per bronze table `T`: the first pipelines (D4: `definition.sourceZone ==
   "bronze"`, `sourceTable == T`, `file_ingest_job` in `dependsOn`, a node);
   reach = everything reachable from them over the remaining edges;
5. for each reached `B` with at least one parent inside reach: an entry with
   its parents in reach and all its ancestors in reach (sorted tuples).
Tests in new `test_reupload_chain.py` (pure, no Dagster instance), names as
sentences: `test_a_gold_pipeline_reading_a_silver_output_is_after_it`,
`test_a_second_input_the_reupload_does_not_reach_is_not_a_parent`,
`test_a_diamond_waits_for_both_branches`,
`test_another_tenants_pipeline_is_never_in_the_chain`,
`test_a_paused_pipeline_and_what_only_it_feeds_are_left_out`,
`test_a_target_lag_pipeline_is_left_out`,
`test_a_cycle_is_left_out`,
`test_with_the_switch_off_nothing_below_the_first_pipeline_is_in_the_chain`,
`test_a_first_pipeline_that_also_reads_another_first_pipeline_has_parents`.
*Check:* `(cd dagster && python -m pytest dispar_orchestrate/test_reupload_chain.py -q)`.

**T6. `_after` tags a re-upload and steps aside (D5, D7).**
`build_authored_dependency_sensors` computes `reupload_chain_graph` from the
pipelines it is given and passes each pipeline's entry map to
`build_authored_dependency_sensor(pipeline, job, chain=None)` (default keeps
every existing caller and test). Inside the sensor, while walking upstreams:
- the `file_ingest_job` record it picked is the root `R` for `T =
  bronze_table`; if `chain` has an entry for `T`, yield `SkipReason("BLD-14:
  the re-upload chain starts this pipeline after <parents>")` and return;
- an upstream `pl-` record whose run has `REUPLOAD_ROOT_TAG` and whose
  pipeline is a parent in `chain` for that run's `REUPLOAD_TABLE_TAG`: the
  same skip;
- otherwise, when a `file_ingest_job` record was used, the `RunRequest`
  gains `tags={REUPLOAD_ROOT_TAG: R, REUPLOAD_TABLE_TAG: T}`; the `run_key`
  is unchanged.
Tests in `test_authored_factory.py` (new class
`TestDependencySensorTagsAReupload`, reusing the fakes; extend
`_FakeDagsterRun` with `tags` if it has none):
`test_a_file_load_run_request_carries_the_reupload_tags` (fails on `main`:
no tags), `test_a_run_after_another_pipeline_carries_no_reupload_tags`,
`test_a_pipeline_the_chain_owns_skips_the_file_load`,
`test_a_tagged_run_of_a_chain_parent_is_skipped_with_bld_14_named`,
`test_an_untagged_manual_run_of_the_same_parent_still_triggers` (D7's
"unchanged").
*Check:* `(cd dagster && python -m pytest dispar_orchestrate/test_authored_factory.py dispar_orchestrate/test_reupload_chain.py -q)`.

**T7. The chain sensor.** New module `dagster/dispar_orchestrate/reupload_chain.py`
(narrative docstring: `DEC-22`, D5-D9, P1-P10, why a polling `@sensor` and
not a `run_status_sensor` — it must see failures and diamonds, and one
run-status event cannot; why the record is posted on the tick after the
launch — the run id exists only then). It imports from `authored_factory`
(never the reverse). Contents:
- `chain_sensor_name(pipeline_id)` = `authored__<safe_id>_chain`;
- `decide(root, entry, lookups, now) -> Decision`, pure, where `lookups`
  answers "the run of job X tagged R" (status, run id) and "does pipeline
  B have a queued or running run". Order, per root `R` of table `T`:
  1. `B` already has a run tagged `R` → `record started(run id)`, settle;
  2. a newer root of `T` exists in the window and `B` has no run tagged
     `R` → `record superseded`, settle (P4);
  3. an ancestor's run tagged `R` is FAILURE → `record not_run
     upstream_failed(ancestor)`, settle; CANCELED → `upstream_cancelled`;
  4. every parent has a SUCCESS run tagged `R`: if `B` has a queued or
     running run → wait (P4); else `RunRequest(run_key=
     f"reupload-chain:{R}:{B}", tags={root, table})`;
  5. `now - R.end_time > CHAIN_WAIT_LIMIT_SECONDS` → `record not_run
     wait_expired(first parent without SUCCESS)`, settle (P5);
  6. otherwise wait.
- `build_reupload_chain_sensor(pipeline, job, entries, cfg)`: `@sensor(name=
  chain_sensor_name(id), job=job, minimum_interval_seconds=30,
  default_status=DefaultSensorStatus.RUNNING)`. Roots: the newest
  `PER_TABLE_UPSTREAM_SCAN_LIMIT` SUCCESS records of `file_ingest_job`,
  kept when `_run_config_value(…, FILE_INGEST_OP_CONFIG)` is a table in
  `entries`, ended at or after the cursor's floor, and not in the cursor's
  settled list. Cursor: JSON `{"floor": <epoch>, "settled": [<root>, …]}`
  (at most 100, oldest dropped); no cursor → floor = now and nothing else
  that tick (P10). Records go to D8's route with `requests`, `timeout=`,
  `raise_for_status()` and `pipeline_events`' bounded retry (reuse
  `_retryable`/the pattern; do not copy a second retry loop if one can be
  imported). A root is added to `settled` only after its record was
  accepted (2xx); a failed post is logged and retried next tick, never
  raised. Yield the `RunRequest`s of the tick; if there are none, one
  `SkipReason` summarising what it waits on (no upstream text). With
  `PIPELINE_RUN_TOKEN` unset: build no sensors, log once (the factory's
  degraded posture).
- `build_reupload_chain_sensors(pipeline_jobs)`: one sensor per pipeline
  with a non-empty entry map; module-level
  `reupload_chain_sensors = build_reupload_chain_sensors(_authored_pipeline_jobs)`;
  `definitions.py` adds `*reupload_chain_sensors` after
  `*authored_dependency_sensors` with a comment.
Tests in `test_reupload_chain.py` (stub instance with tag filtering; no
network: patch the poster): one per `decide` branch —
`test_a_pipeline_starts_when_every_parent_succeeded_for_this_reupload`
(fails on `main`: module absent),
`test_a_pipeline_waits_while_one_branch_of_a_diamond_is_running`,
`test_a_failed_parent_stops_the_chain_and_records_the_cause`,
`test_a_failure_two_steps_up_is_named_as_the_cause`,
`test_a_cancelled_parent_records_upstream_cancelled`,
`test_the_run_key_is_the_same_for_the_same_reupload_so_it_runs_once`,
`test_a_newer_reupload_supersedes_an_older_one_that_has_not_launched`,
`test_a_pipeline_with_a_run_in_flight_is_not_launched_again`,
`test_waiting_past_six_hours_records_wait_expired`,
`test_the_first_tick_sets_the_floor_and_launches_nothing`,
`test_a_root_is_settled_only_after_its_record_was_accepted`,
`test_the_sensor_name_matches_the_api_chain_sensor_name` (pins
`authored__pl_x_chain`, the Rust twin of T4),
`test_no_token_builds_no_chain_sensors`.
*Check:* `(cd dagster && python -m pytest dispar_orchestrate -q) &&
python3 ops/lint/check_intra_package_imports.py`.

### TypeScript (T8)

**T8. The sentence on the pipeline page.** Contract and client per section 4
(append at the end of the type block and of the client object; `apiFetch`
only). New `src/features/pipelines/reupload-chain-note.tsx` (`"use client"`
first; `useService((s) => pipelineService.getReuploadChain(id, s), [id])`;
renders nothing while loading, on error, or for `last: null`; otherwise one
line in the muted text style already used under "Run history", with the
links of section 4). One import and one `<ReuploadChainNote pipelineId=… />`
line in `pipeline-detail-page.tsx`, directly above the Run history section
(`:517`), for a non-draft authored pipeline. This is one sentence on an
existing page in an existing style, not a new screen, so it stays with this
developer (owner's rule of 2026-11-04 covers revamps and new UI).
Tests: new `reupload-chain-note.test.tsx`: each sentence of section 4 byte
for byte (six cases, including the unknown code and the null cause name),
the cause link's href, nothing rendered for `last: null`; client test in
`src/services/clients/pipelines.test.ts` for the path.
*Check:* `bun run typecheck && bun run lint && bun run test -- reupload-chain-note pipelines.test`.

### Gate (T9)

**T9. `ops/g9/upload_test.py` step 10 grows a chain.** After
`step_reupload_reruns_the_pipeline` (`:1491`), inside `step_feeds_a_pipeline`
(`:1667`), add (and say so in the module docstring's step 10 paragraph):
1. `step_create_gold_after_silver`: a pipeline reading
   `silver.<table>_clean` into `gold.<table>_daily` (no `dependsOn`), made
   ready; wait until the code location lists `authored__<id>_chain` (as
   `_dagster_lists_job` waits for a job; up to 120 s).
2. `step_reupload_runs_the_chain`: re-upload with one row changed; expect
   one new Silver run and then **exactly one** new Gold run whose
   `queuedAt` is not before the Silver run's `endedAt`; the Gold table shows
   the change; `GET …/reupload-chain` for Gold answers `started` with that
   run id. Wait two more sensor intervals (75 s) and expect still one Gold
   run for that re-upload.
3. `step_failed_silver_stops_the_chain`: change the Silver pipeline's step so
   that the next load makes it fail (a SQL step with `throwIf(<column> =
   'boom')`, or a cast of a text value; whichever the step check accepts —
   say which in the handoff), re-upload a file with that value; expect the
   Silver run to fail, no new Gold run after 75 s, and Gold's
   `reupload-chain` to answer `not_run` / `upstream_failed` / cause = the
   Silver pipeline with its name. Restore the Silver step at the end and in
   `_clean_up_upload`'s path.
The gate needs the real stack (`docker compose --profile '*' up` with
`PIPELINE_RUN_TOKEN` and `DAGSTER_URL` set). Run it if the machine can;
otherwise write *not verified* and why; the reviewer runs it before merge.
*Check:* `python3 -m py_compile ops/g9/upload_test.py` and, with the stack,
`python3 ops/g9/upload_test.py`.

## 7. Tests that fail on `main`

T1 and T2 (no table, no route: compile failure, then 404), T3 (no route),
T4 (no `_chain` stop requested), T6 `test_a_file_load_run_request_carries_the_reupload_tags`
and the two skips, T7 (module absent), T8 (component absent), T9 step 2 (no
Gold run on `main`: the QA case itself).

## 8. PR slicing

One PR, `bld-14-downstream-reupload` → `main`: T1-T9 in order. The parts
are not useful apart (a chain with no stored reason breaks D2's "reason
shown"; a reason with no chain has nothing to record), and the TypeScript
is one component.

## 9. Verification for the handoff (the full block)

Once, in the foreground, on the final commit:

```bash
cd rust && cargo fmt --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace
bun run typecheck && bun run lint && bun run test
python3 ops/lint/check_intra_package_imports.py && python3 ops/lint/check_bare_iceberg_count.py
(cd dagster && python -m pytest dispar_orchestrate -q)
python3 ops/g9/upload_test.py   # with the stack up; else "not verified" and why
git status                      # nothing generated staged
```

Per the owner's rule of 2026-10-30, CI's cargo run on the draft PR stands in
for the local `cargo test --workspace`; quote the scoped local runs and CI's
run with counts. No compose change, so no `docker compose up` for compose;
the stack is needed only for T9.

## 10. Out of scope

- A connector load starting the pipelines below the ones it feeds (P7).
- Refusing a cycle of pipelines when one is saved (P2).
- Resuming a stopped chain from a retry (P6), or a button to run "everything
  below this pipeline".
- Showing the chain on the upload's page or in lineage; a per-pipeline
  switch below the first (P8).
- The run list's `trigger` naming the re-upload (it shows the sensor name,
  `pipelines.rs:1672`).
- `docs/core/features/uploads-feed-pipelines.md`: the planner rewrites
  decision 2, the customer limit ("once per upload") and adds acceptance
  rows (a Gold pipeline runs after Silver on a re-upload; a failed Silver
  stops Gold with the sentence shown) when the PR is reviewed; the developer
  does not edit docs other than this plan's Handoff.

## Handoff

Developer: `Hy4 flash` (OpenCode). Branch `bld-14-downstream-reupload` off
`a438555d0` (the plan commit on `c7cd01a76`). All anchors were re-read by
their quoted text before each task; every one matched. Migration `0153`
was still free (highest on the branch was `0151`). No mismatch with the plan.
Every commit was pushed after `git status` showed only the intended files;
no generated files were staged.

### Commits (one per task, in order)

1. `8a7ce635b` — T1, `feat(store): record a re-upload chain step once (BLD-14, D8, P9)`.
   Migration `0153`, `pipeline_chain.rs`, seven store tests. The module and
   table do not exist on the parent (`git grep` on `HEAD`), so all seven
   tests fail to compile there.
2. `f46639c6a` — T2, `feat(api): record a re-upload chain step (BLD-14, D8)`.
   Orchestrator-only POST route, policy floor, `EX_MACHINE` audit decision,
   validation, 7 HTTP tests and a both-ways route-auth assertion. On the
   parent the route is absent (confirmed with `git grep` on `HEAD`): the
   success case is a 404 instead of 200.
3. `922641749` — T3, `feat(api): read a pipeline's newest reupload chain step (BLD-14, D8)`.
   GET route with `in_scope`, 3 more page-read tests, both-ways auth. Absent
   on the parent, so the page tests get 404 instead of 200.
4. `a6405083a` — T4, `feat(api): pause and resume the reupload chain sensor (BLD-14, D6)`.
   `chain_sensor_name` and the stored-state toggle in `authored_status`;
   the Dagster-mock test finds no `authored__<id>_chain` lookup/start and
   only one `stopSensor` on the parent.
5. `639e4ec6f` — T5, `feat(orchestrator): derive the reupload chain graph (BLD-14, D3)`.
   Nine pure graph tests; `reupload_chain_graph` does not exist on the
   parent, so the test module cannot import it.
6. `26edd72d3` — T6, `feat(orchestrator): tag file loads and step aside for chain pipelines (BLD-14, D5, D7)`.
   Five new sensor tests cannot pass the `chain` argument to
   `build_authored_dependency_sensor` on the parent. While running them, a
   regression I introduced (the pre-existing "never had a SUCCESS run" skip
   moved into the new branch) broke 12 existing sensor tests; it was found
   and restored before the commit.
7. `ac0bc8dd2` — T7, `feat(orchestrator): poll and launch the reupload chain (BLD-14, D6-D9)`.
   New `reupload_chain.py`, definitions wiring, 13 sensor tests. The module
   does not exist on the parent.
8. `c3315ad0a` — T8, `feat(console): show why a pipeline did not run after a reupload (BLD-14, D8)`.
   Contract, client, component and 10 component/client tests. Neither the
   method nor the component exists on the parent, so the new tests cannot
   compile there.
9. `48924bd73` — T9, `test(g9): prove the reupload chain in step 10 (BLD-14, D1, D2, D9)`.
   Gate steps 1-3 and cleanup restore. The failure step uses a SQL step with
   `SELECT *, throwIf(qty = 'boom') AS bad FROM <input>`: that is the shape
   `check_pipeline_step_sql` accepts (one query, one named input), and the
   uploaded row carries `boom` in `qty`.

### Verification on the final commit (foreground, counts as run)

`CARGO_TARGET_DIR=/home/rismana/rantai/cargo-target-6`, shared and outside
the checkout; no `cargo clean`. No `cargo test --workspace` / workspace
clippy — CI runs that on the draft PR (developer brief). The system
`python` has no `pytest`; the checks below use
`/home/rismana/rantai/venv-agents-1/bin/python`, whose `dagster==1.13.20`
matches `dagster/pyproject.toml`.

- `cd rust && cargo fmt --check` — clean.
- `cargo clippy -p lakehouse-store --all-targets -- -D warnings` — clean.
- `cargo clippy -p lakehouse-api --all-targets -- -D warnings` — clean.
- `cargo test -p lakehouse-store --test it pipeline_chain::` — 7 passed, 0 failed, 771 filtered out.
- `cargo test -p lakehouse-api --test it pipeline_reupload_chain::` — 11 passed, 0 failed, 1,776 filtered out.
- `cargo test -p lakehouse-api --test it -- audit_coverage:: sec11_guard:: route_auth::` — 135 passed, 0 failed, 1,652 filtered out.
- Per-task counts during the build: route_auth 102 passed; the combined
  `pipeline_reupload_chain:: route_auth:: audit_coverage:: sec11_guard::`
  run at T2 was 141 passed.
- `bun run typecheck` — clean.
- `bun run lint` — 0 errors, 12 pre-existing `react-hooks/exhaustive-deps`
  warnings in unchanged files.
- `bun run test` — 4,643 passed, 1 skipped, 0 failed (4,644 tests, 429 files).
  Touched tests: `bun test -- reupload-chain-note pipelines.test` — 33 passed, 0 failed.
- `python3 ops/lint/check_intra_package_imports.py` — OK.
- `python3 ops/lint/check_bare_iceberg_count.py` — OK.
- `python -m pytest dispar_orchestrate -q` (venv) — 2,692 passed, 1 skipped,
  2 failed. The two failures are in untouched files
  (`test_adapters_oracle.py` TLS DN, `test_parquet_ingest.py` float repr);
  `git diff a438555d0..HEAD` confirms neither module is changed on this
  branch. The T7 scoped runs were 116 passed (`test_authored_factory.py`
  plus `test_reupload_chain.py`) and 22 passed (`test_reupload_chain.py`).
- `python3 -m py_compile ops/g9/upload_test.py` — OK.
- `git status` — clean; no generated files staged.

### Not verified here

- **The G9 gate itself** (`python3 ops/g9/upload_test.py`): no compose stack
  in this environment, so only `py_compile` was run. The reviewer runs it
  with `PIPELINE_RUN_TOKEN` and `DAGSTER_URL` set before merge (T9's check).
- `cargo test --workspace` / workspace clippy: CI's job on the PR, per the
  developer brief and the plan's note.
- The two unrelated Dagster failures above are reported, not papered over.

## Review

### Round 1 — planner review, 2026-11-20, at `23e9f9ede`

Holds: the graph is derived only through `_pipeline_target`/`_pipeline_sources`,
same tenant; cycles removed (Tarjan) before reach (P2); paused and
target-lag pipelines excluded (P1); fan-in waits for every parent; each
parent is looked up by its run tagged with this re-upload; fixed `run_key`
per re-upload and pipeline; P6 and P10 as planned; routes gated and asserted
both ways; migration `0153` with a why-header; Python rules kept. The two
Dagster failures in the handoff do not touch this change.

**R1 BLOCKER: a re-upload can leave the sensor's view and is lost without
a record.** `reupload_chain.py:254-269` scans only the newest 20 successful
`file_ingest_job` runs across every table and tenant, while a chain waits
up to 6 hours. Twenty loads of other tables later, Gold's tick no longer
sees the re-upload: Gold never runs, nothing is recorded, P5's expiry never
fires. Fix: find the open re-uploads by their own key, not a recency window
— e.g. `get_run_records` filtered by job, status SUCCESS, the uploaded
table's tag and `updated_after` = now − 6 h (P5), or a record the chain
writes when it first sees a re-upload, read back until it is finished. A
test with 21+ loads of other tables between the re-upload and Gold's turn.

**R2 BLOCKER: P4 assumes the newer re-upload starts its own first
pipeline, and the existing `_after` sensor does not guarantee it.**
`reupload_chain.py:131-132` with `authored_factory.py:1212-1217`: if R2
ends while R1's first pipeline run is still queued, `_after` skips R2 once
and never retries, so no run is tagged R2; Gold records R1 `superseded`,
then waits on R2 and records `wait_expired` with a false sentence while
Silver did load R2's data. Fix: the chain sensor owns the first pipeline's
launch too (same `run_key` per re-upload and pipeline), and `_after` steps
aside for a file load whose table starts a chain; or a later successful
first-pipeline run (started after R2's load ended) counts as R2's. Pick
one, say which in the module doc. Test: two re-uploads with R2 ending while
R1's first run is queued — Gold runs once, for R2, and no false sentence.

**R3 SHOULD-FIX: two runs of one pipeline in one tick.**
`reupload_chain.py:299-318`: `in_flight` reads the instance, which does not
yet hold requests yielded earlier in the same tick. Launch at most one run
per pipeline per tick; the other root waits for the next tick. Test.

**R4 SHOULD-FIX: an older re-upload can come back.** `reupload_chain.py:
302-304` compares only against unfinished re-uploads; compare against every
re-upload of the same table, finished ones included. Test.

**R5 SHOULD-FIX: placement and validation.** Move `GET /api/pipelines/{id}/
reupload-chain` out of `authored_pipelines_router` (whose doc says
orchestrator-only) next to `runs` in `routes/pipelines.rs`, as T3 said; and
validate `runId`'s shape and length on the POST route (section 3).

Not confirmed, check while fixing: whether Dagster keeps a sensor cursor
across stop and start, so a resumed pipeline acts on re-uploads from while
it was paused (P10's floor would not apply).
