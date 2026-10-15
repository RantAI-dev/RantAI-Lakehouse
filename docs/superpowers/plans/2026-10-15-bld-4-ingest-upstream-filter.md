# `BLD-4`: a pipeline after `ingest_job` runs only after its own table loads

Planner and reviewer: Claude Opus. Developer: a non-Opus agent (`AGENTS.md`).
Base: `origin/main` at `a84f4e121409`. Branch: `feat/bld-4-ingest-upstream-filter`.
One PR slice. Backlog: `BLD-4` (Task, P3; was blocked by `DATA-19`, merged as
#165). No spec: a Task that fixes a wrong trigger.

## 1. The problem (read on `main` at `a84f4e121409`)

- A pipeline that names `ingest_job` in `dependsOn` gets a
  `run_status_sensor` (`authored_factory.build_authored_dependency_sensor`)
  that, for every upstream except `file_ingest_job`, takes the newest SUCCESS
  run of the job (`get_run_records(..., limit=1)`) as the freshness anchor.
- `ingest_job` is ONE static job for every connector
  (`ingest_factory.py`, `@job(name="ingest_job")`), launched with
  `{"ops": {"run_ingest": {"config": {"connector_id": "<id>"}}}}` plus an
  optional `object`/`action` for a one-table run (`routes/connectors.rs`
  ~:3356). So any connector's successful load fires every pipeline that
  names `ingest_job`, whatever table it reads. Wasted runs at best; at
  worst a pipeline rebuilt from a Bronze table that did not change, many
  times a day.
- `DATA-19` fixed the same shape for `file_ingest_job` by reading the
  loaded table from each run's config (`FILE_INGEST_OP_CONFIG`,
  `_latest_record_for_table`, `FILE_INGEST_SCAN_LIMIT = 20`). That does not
  carry over: an `ingest_job` run's config names the connector, not the
  tables, and one run loads every selected object of the connector.
- What does name the table: each loaded object logs
  `AssetMaterialization(asset_key=f"bronze.{obj['target']}")`
  (`ingest_factory.py` ~:1263), inside the run that loaded it.

## 2. Decisions (planner)

- **D1. Per table, from materializations.** For an upstream `ingest_job`,
  the freshness anchor is the newest **SUCCESS** run of `ingest_job` that
  materialized the pipeline's Bronze source table (the asset key the line
  above writes for `obj['target'] == sourceTable`). A run that loaded other
  tables only, or a run that materialized the table and then FAILED, is not
  a run of this chain. Bounded like `DATA-19`: look at the newest
  materializations of that one asset key (or the newest `ingest_job` SUCCESS
  runs, see 5.1), at most `FILE_INGEST_SCAN_LIMIT` per tick, reuse that
  constant or rename it to a shared one with a comment (rule 4: one window,
  not two).
- **D2. A non-Bronze source cannot name `ingest_job`.** Same rule and the same
  helper as `file_ingest_job`: generalise
  `authored_pipelines::validate_file_ingest_upstream` (rename it, keep one
  function, one refusal text per job or one shared text that names the job)
  so create, update and restore refuse `ingest_job` with a non-`bronze`
  source. A stored row that predates the rule gets a `SkipReason` and
  matches nothing (fail closed), exactly as `DATA-19` did.
- **D3. Nothing else changes.** Other upstreams (authored `pl-` ids, other
  Dagster jobs) keep the current rule byte for byte. The `run_key`
  derivation, the queued-downstream skip and the `RunRecord` time reads stay.
- **D4. Skip reasons name the table**, as `DATA-19`'s do: "upstream
  'ingest_job' has no SUCCESS run that loaded '<table>' in its last N
  loads; a load of another table never triggers this pipeline".

## 3. Tasks (one commit each)

- **T1. D1 + D3 + D4 in `authored_factory.py`**, module doc and the sensor's
  docstring updated (the docstring says `file_ingest_job` is "the one
  upstream the freshness walk filters"; that becomes two). Unit tests with a
  stub instance whose records have the SAME shape the real storage returns
  (`RunRecord` with `start_time`/`end_time`, a `DagsterRun` without them;
  `DATA-19`'s finding): a load of another table does not trigger; a load of
  the pipeline's table does; a run that materialized the table but FAILED
  does not; nothing within the window gives the D4 reason; an authored and a
  `file_ingest_job` upstream behave as before.
- **T2. D2 in the API.** The generalised validator, its tests (both jobs, a
  bronze source accepted, each other zone refused), wired into create,
  update and restore exactly where the current one is.
- **T3. The gate proves it.** A real stack, because a stub cannot (that is
  how the `start_time` bug hid): extend the `g3a-dagster` job's gate
  (`ops/g7`, which already builds a Bronze-to-Silver pipeline after a G3a
  ingest) with a pipeline whose `dependsOn` is `["ingest_job"]` on Bronze
  table A; run an ingest that loads only table B and assert the pipeline does
  not run within two sensor intervals; run an ingest that loads A and assert
  it runs. If the G3a fixture has only one table, use a one-table run
  (`object`) of another object or add the smallest second table to the
  fixture, and say which. Run the job's sequence locally from a clean
  project under the heavy lock; quote the output.
- **T4. Docs.** `CHANGELOG.md` (a pipeline after a connector load now runs
  only when its own table loaded), module docs. Do not edit `docs/core/`.

## 4. Out of scope

Target-lag pipelines (`BLD-2` reads input tokens itself). Changing what
`ingest_job` logs. Connector-level filtering by `connector_id` (D1 is
per table, which is stricter and needs no mapping).

## 5. Verify, do not assume

1. Which instance call finds "the newest SUCCESS run that materialized asset
   X" in the pinned `dagster==1.13.20`, bounded? (`fetch_materializations`
   / `get_event_records` with an `AssetKey` filter, then the run's record;
   or the newest `ingest_job` SUCCESS run records, then each run's
   materialization events.) Quote the signature you used and why the other
   was rejected.
2. What `AssetKey` does `AssetMaterialization(asset_key="bronze.orders")`
   produce: one component `"bronze.orders"`, or two? Prove it with a test
   against the real class, not a stub, and build the filter from that.
3. Does a materialization logged by a mapped step survive into the event log
   when a later step of the same run fails? (D1 depends on checking the
   RUN's status, not only the event.)

## 6. Handoff

(Developer appends here, task by task.)

### Section 5 answers (verified against the pinned `dagster==1.13.20` before T1)

**5.1 Two bounded instance calls, intersecting on `run_id`.** The newest
SUCCESS run of `ingest_job` that materialized the pipeline's Bronze table
is found by `instance.fetch_materializations(asset_key, limit)` followed by
`instance.get_run_records(...)`:

- `DagsterInstance.fetch_materializations(records_filter: Union[AssetKey,
  AssetRecordsFilter], limit: int, cursor: str | None = None,
  ascending: bool = False) -> EventRecordsResult` -- verified by printing
  `inspect.signature(DagsterInstance.fetch_materializations)` in the pinned
  venv. `ascending=False` is the default, so `EventRecordsResult.records`
  is NEWEST FIRST; each element is an `EventLogRecord` carrying
  `storage_id`, `event_log_entry`, `.run_id`, `.timestamp`, `.asset_key`,
  `.asset_materialization`. Proven by a scripted `execute_in_process`
  session against `instance_for_test()`: three runs of a job that logs
  `AssetMaterialization`, the newest being a FAILED one, returned
  `[(<failed run>, t3), (<success run>, t1)]` for `limit=20`.
- The call filters by asset key ONLY -- `AssetRecordsFilter` has no
  `job_name` and no status field -- so the RUN's status comes from the run
  side: `instance.get_run_records(filters=RunsFilter(job_name="ingest_job",
  statuses=[DagsterRunStatus.SUCCESS]), limit=<window>)`. The anchor is the
  newerst materialization record whose `run_id` is one of those SUCCESS
  run records. In that same script the anchor resolved to the SUCCESS run
  that loaded `bronze.orders`, NOT to the newer run that loaded
  `bronze.invoices` and NOT to the newer run that FAILED.
- REJECTED, `instance.get_event_records(EventRecordsFilter(event_type=...,
  asset_key=...))`: `EventRecordsFilter` (`dagster._core.event_api`,
  dumped in full) has no job/status field either, so it would still need
  the run-side join, AND the method is deprecated in 1.13.20 -- calling it
  prints `UserWarning: Use fetch_materializations instead of
  get_event_records to fetch materialization events`.
- REJECTED, `instance.get_run_record_by_id(run_id)` per materialization:
  that turns ONE bounded window into up to `FILE_INGEST_SCAN_LIMIT`
  instance calls per tick, for a piece of information the one SUCCESS-run
  query already carries.

**5.2 `AssetMaterialization(asset_key="bronze.orders")` is a TWO-component
key.** On the real classes: `AssetMaterialization(asset_key="bronze.orders")
.asset_key` is `AssetKey(['bronze', 'orders'])`; `to_string()` is
`["bronze", "orders"]`, `to_user_string()` is `bronze/orders`. So the
filter is built as `AssetKey(["bronze", <sourceTable>])`; the one-component
`AssetKey(["bronze.orders"])` matches nothing, and `AssetKey.from_user_string
("bronze.orders")` is the inverse (slash-joined) form and is NOT used. Pinned
by `TestIngestMaterializationKeyTest` in
`dagster/dispar_orchestrate/test_authored_factory.py` (T1), which asserts
against the real `AssetKey`/`AssetMaterialization` classes and then proves
the two shapes against a real event log.

**5.3 Yes, a materialization from a mapped step survives a later step's
failure.** The event log is append-only per step: a job whose FIRST mapped
step logs `AssetMaterialization(asset_key="bronze.orders")` and whose LATER
step raises still has that materialization in the log
(`instance.fetch_materializations(AssetKey(["bronze","orders"]), limit=5)`
returns the record), while the run's status is `DagsterRunStatus.FAILURE`
and `get_run_records(statuses=[SUCCESS])` is empty. This is exactly why D1
reads the RUN's status and not only the event: the anchor for that chain is
`None`. Proven on the real classes with the job built in the real
`.map(...)` shape `ingest_factory.ingest_job` uses (T1's
`TestIngestMaterializationKeyTest`).

## 7. Review

(Reviewer.)
