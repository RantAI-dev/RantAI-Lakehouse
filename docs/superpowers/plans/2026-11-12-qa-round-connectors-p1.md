# QA round, Connectors and alerts (P1): `SRC-38`, `SRC-39`, `SRC-42`

Fixes for three findings of the QA round of 2026-11-02 to 11-12, read against `main` at
`512f6e37c` on 2026-11-12 (every anchor below was verified there; the backlog rows were read at
`657c2502`, and their anchors still hold):

- `SRC-38` (QA `LH-SRC-6-53`, Major): a load from object storage whose endpoint is down ends as
  "the failure is not classified", is not retried, and the connector stays Healthy.
- `SRC-39` (QA `TC-1702`, `TC-1706`, `LH-SRC-11-11`, `LH-SRC-8-02`): a connector run that never
  finishes answers 409 "a run is already running" for days, and a run that never starts sits
  queued for hours.
- `SRC-42` (QA `LH-SRC-7-04`, Major; decision `DEC-16`, product owner, 2026-11-12): the Alerts
  list shows every rule's webhook address and e-mail address to everyone with `alert:read`.
  `SRC-42` is not in `docs/core/BACKLOG.md` on this checkout yet; the planner adds the row in
  the same pull request.

Base: `main` at `512f6e37c`. Branch: `fix/qa-round-connectors-p1`. One pull request for the
slice, opened as a draft by the planner once the first commits are pushed (the integration
tests and the compose change are proven by CI, not on the build machine). Push after every
commit.

Planner and reviewer: Claude Opus. Developer: another agent (not Opus). The developer writes
the code and tests of section 2, in order, one task per commit, and changes documents only
where a task says so. If an anchor or a statement below does not match the code, stop and
report it in the handoff instead of improvising.

No route is added or changed in its policy, so `POLICY_TABLE` and
`rust/crates/lakehouse-api/tests/route_auth.rs` stay as they are.

## 1. Decisions

### All three

- **D1. One task per commit, in the order of section 2** (Python, then Rust together, then
  compose, then the console, then documents). Each commit body cites the QA case IDs and the
  backlog ID. Each fix site carries a short comment naming the case (rule 13), for example
  `# QA LH-SRC-6-53 (SRC-38): ...` or `// QA TC-1702 (SRC-39): ...`.
- **D2. No new helper where one exists** (rule 4). Named in the tasks: `_root` and
  `_class_names` (`dagster/dispar_orchestrate/ingest_errors.py:188`, `:196`),
  `RetrySettings.from_env` as the pattern for a setting (`:561-579`),
  `parse_positive_u32_or_default` (`rust/crates/lakehouse-api/src/config.rs:1469`),
  `DgClient::terminate_run` (`rust/crates/lakehouse-dagster/src/lib.rs:1696`),
  `connector_run::end_run` (`rust/crates/lakehouse-store/src/connector_run.rs:572`),
  `chart_alerts::may_manage` (`rust/crates/lakehouse-api/src/routes/chart_alerts.rs:308`),
  `alerts::listed_rules` (`rust/crates/lakehouse-api/src/routes/alerts.rs:103`).

### `SRC-38`: object storage that is down

- **D3. What is raised (read in the libraries the Dagster image installs: `s3fs` 2026.7.0,
  `aiobotocore` 3.9.2, `botocore` 1.43.106, `dlt` 1.30.0).** The files adapter calls
  `fs.info` and `fs.cat_file` (`adapters/files.py:73-90`). `aiobotocore` turns every `aiohttp`
  failure into a `botocore.exceptions` class (`aiobotocore/httpsession.py:340-372`):
  a refused or unresolvable endpoint becomes `EndpointConnectionError`, a dropped connection
  `ConnectionClosedError`, a timeout `ConnectTimeoutError` or `ReadTimeoutError`. `s3fs` passes
  these on unchanged (`s3fs/core.py:161-215`, `translate_boto_error` returns an error with no
  `response` as it is). `dlt` wraps the adapter's generator failure twice:
  `PipelineStepFailed` (has `.exception`) from `ResourceExtractionError` from the `botocore`
  error. `botocore.exceptions.ConnectionError` is `botocore`'s own class, not Python's
  `ConnectionError`, so no line of `_classify_one` (`ingest_errors.py:417-508`) recognises it
  and `classify` answers `other`.
- **D4. The rule, by class name, with nothing imported.** One block in `_classify_driver`
  for `root == "botocore"`, in the style of the `kafka` and `pymongo` blocks (`:398-411`):
  `ConnectTimeoutError` or `ReadTimeoutError` is `timeout`; `EndpointConnectionError` or
  `ConnectionClosedError` is `unreachable`. The timeout names are tested first, because
  `ConnectTimeoutError` is also a `botocore` `ConnectionError`. No message text is read (the
  module's rule, `:28-37`). Left as `other` on purpose: `SSLError` (a certificate problem does
  not pass by itself), `ProxyConnectionError` (this adapter sets no proxy), the bare
  `HTTPClientError`, and `ClientError` (the storage answered; `s3fs` turns those into Python
  errors). `aiohttp` and `s3fs` need no rule of their own: every `aiohttp` failure arrives
  inside a `botocore` one, and `fsspec`'s `FSTimeoutError` is already a Python `TimeoutError`.
- **D5. The walk's bound is a second, real defect, and it is fixed too.** `_chain`
  (`:165-185`) loops `range(_MAX_CAUSE_DEPTH)` over queue pops, and every exception pushes
  four entries (`orig`, `exception`, `__cause__`, `__context__`), most of them `None` or a
  repeat. So a `raise ... from` chain is read three exceptions deep and an implicit-context
  chain two deep, not eight. In the files case the `botocore` error is the third exception and
  is reached on the last pop, so the missing rule (D4) is the cause of the QA finding; one
  more wrapper, or a rule that sits on the fourth link, would be cut off. Fix: the bound
  counts exceptions yielded, and becomes 16. What bounds the walk after the change: `seen`
  (each exception object once, so a cycle ends) and the cap of 16 exceptions; each exception
  adds at most four queue entries, so the loop ends after at most 64 pops.
- **D6. Retries need no change.** `unreachable` and `timeout` are in `RETRIED_CLASSES`
  (`:95`), so `_decide` (`ingest_factory.py:1183-1187`) gives the waits of
  `INGEST_RETRY_DELAYS_SECONDS` (`ingest_errors.py:532-536`: 60, 300, 900). The API's words
  for both classes exist (`connector_run.rs:220-221`).
- **D7. Health needs no code change; the rule exists and is tested.** A failed run already
  changes the connector's health: `RECORD_RUN_SQL`
  (`rust/crates/lakehouse-store/src/connectors.rs:1767-1787`) sets `degraded` for one or two
  failed runs in a row and `unhealthy` from three (`SRC-7` decision 6, signed 2026-10-09). It
  runs in `connector_run_event` (`rust/crates/lakehouse-api/src/routes/load_alerts.rs:418`)
  when the orchestrator's failure sensor reports the run, and is proven by
  `rust/crates/lakehouse-store/tests/connectors.rs:1815` and the tests in `load_alerts.rs`
  (`:901-918`). That report needs `PIPELINE_RUN_TOKEN` on the API and the code location
  (optional in compose, `docker-compose.yml:410`, `:2496`; `SRC-7` decision 9), while the run
  record QA read ("Failed for another reason") arrives on the other token
  (`INGEST_SERVICE_TOKEN`). So a deployment with run reports off, or one report lost (the
  sensor posts once per run, `pipeline_events.py` module text), shows a failed run and a
  Healthy connector. *Not verified*: whether staging had the token set on the day of the test.
  The retest instruction is in section 3. Nothing is built for it here.

### `SRC-39`: a run that never finishes

- **D8. The orchestrator ends a stuck run itself: its run monitoring is switched on.** Read in
  the pinned `dagster==1.13.20`: `run_monitoring.enabled` defaults to `False`
  (`dagster/_core/instance/methods/settings_methods.py`, `_initialize_run_monitoring`), and
  `dagster/dagster.yaml` sets only `storage`, so today nothing ends a run whose worker died or
  that was never launched. Two places hold the lock and both read the orchestrator's run
  list: the API's run route (`rust/crates/lakehouse-api/src/routes/connectors.rs:3868-3884`,
  `:4073-4075`) and the schedule sensor (`_busy_connector_ids`,
  `dagster/dispar_orchestrate/ingest_factory.py:1611-1622`, statuses `:1575-1580`). Only a run
  that really ends in the orchestrator frees both, and only the orchestrator's own daemon acts
  without a person pressing a button. With monitoring on
  (`dagster/_daemon/monitoring/run_monitoring.py`): a run still `NOT_STARTED` or `STARTING`
  after `start_timeout_seconds` is failed; a `STARTED` run older than its `dagster/max_runtime`
  tag is cancelled and failed. A failed run is then reported by the existing failure sensor,
  and the existing path counts it (health, streak, `connector_failure` alert) and closes its
  record. This is the answer to "is run monitoring the right place for the queued-run
  timeout": yes.
- **D9. The limits *(proposed, see Open questions)*.**
  - A run may run for **6 hours** (`INGEST_RUN_MAX_RUNTIME_SECONDS`, default `21600`). Reasons:
    the console already treats a run with no end after six hours as stopped
    (`STALE_RUN_MS`, `src/features/connectors/connector-ingest-panel.tsx:1275`, and re-enables
    "Run now", which the API then refuses: that is the QA case); the retry policy alone holds
    a failing table's run open for about 21 minutes (`SRC-11` decision 1); the longest measured
    loads are minutes (`docs/plans/DATA-17-RESULT.md`: 2 GiB in under two minutes;
    `docs/plans/SRC-11-RESULT.md`: the whole gate in 183 seconds). The limit is a tag on
    `ingest_job` only, not `run_monitoring.max_runtime_seconds`, which would cap every job of
    the instance.
  - A run must start within **10 minutes** (`start_timeout_seconds: 600` in `dagster.yaml`;
    the orchestrator's default is 180). It is a fixed number in the file because the
    instance schema takes a plain `int` there, not an environment value
    (`dagster/_core/instance/config.py:543-554`). The API reads the same number as
    `INGEST_RUN_START_TIMEOUT_SECONDS`, default `600`; whoever changes one changes both
    (written beside both).
- **D10. The API's run route stops holding the lock for an overdue run** (the decided
  behaviour: ask the orchestrator to stop the run, record it as failed with a fixed reason,
  let the next run start). A queued or starting run older than the start limit, and a running
  run older than the run limit, each plus a fixed five minutes, no longer answers 409. The
  five minutes let D8 act first (the daemon polls every 60 seconds), so the route only acts
  when the orchestrator's monitoring did not. The route then asks the orchestrator to cancel
  the run with `terminateRun(..., terminatePolicy: MARK_AS_CANCELED_IMMEDIATELY)`: the client
  used here has `terminate_run` with `SAFE_TERMINATE` only (`lakehouse-dagster/src/lib.rs:1696`),
  which the orchestrator refuses for a run that is not started or whose worker is gone
  (`dagster_graphql/implementation/execution/__init__.py:133-163`); the forced policy still
  tries to stop the worker first, then marks the run cancelled. A cancel that fails is logged
  and the launch goes on: the alternative is the connector locked for good. A run with no
  usable time is never overdue (fail closed: the 409 stays).
- **D11. The fixed reason.** A new end reason `timed_out`. The run's last log line reads
  `run ended: failed, the run did not finish in 6 h and was stopped` (the number is the
  setting). It is written in two places and nowhere else: when the failure sensor reports a
  failed run that ran for the limit or longer and whose record is still `running` (D8's path),
  and by the run route for a running run it cancels (D10's path). The run's own end report
  still wins when it arrived first (`end_run` records the first end only).
- **D12. What counts.** A run the orchestrator's monitoring fails is a failed run and counts
  (health, streak, alert), through the existing report. A run the run route cancels ends as
  cancelled in the orchestrator and is not counted, as `SRC-11` decision 3 says of a cancelled
  run; its record still says failed with `timed_out`.
- **D13. A run that never started gets no row in the history.** The record begins with the
  run's own start report (`SRC-11` decision 7), and nothing is invented for a run that sent
  none (`settle_run`, `connector_run.rs:616-643`). A run the orchestrator's monitoring fails is
  counted as a failed run and alerted by D8's path. A run still waiting in the queue is not
  failed by that monitoring (`review round 1, R1`): it is cancelled by the run route only after
  it has waited longer than a run may run, is not counted, and gets no row either (D12's rule
  for a run this route cancels). See Open questions.
- **D14. Relation to `SRC-18` and `SRC-32`.** `SRC-18` (open: a paused connector still loads
  when a run is launched from the orchestrator; the schema-observation routes' permission) is
  not touched: the pause check (`connectors.rs:3835-3839`) still answers before the new age
  check, and nothing here launches a run for a paused connector. `SRC-32` (done, #252: a late
  report does not move the streak) is not touched: a run failed by monitoring is reported with
  its real end time through the same `record_run_once`; the new end reason changes the run
  record only, never the count.

### `SRC-42`: who sees where a rule sends (`DEC-16`)

- **D15. One place.** `listed_rules` (`routes/alerts.rs:103-131`) is already the one function
  that turns rules into the list's JSON, for `GET /api/alerts` and for the assistant's
  `list_alert_rules` (`routes/ai/tools/alerts.rs:108-121`). The rule page has no read of its
  own: it reads the list (`src/features/alerts/alert-rule-page.tsx:40-44`, `:402-405`). The
  redaction goes there.
- **D16. Who sees the target.** A caller for whom `chart_alerts::may_manage(principal,
  Some(&rule))` is true: `alert:write`, or the owner of a chart alert for their own rule (the
  address is their own, `BI-19` part C). Everyone else, a missing principal included, does
  not (fail closed).
- **D17. What the others get.** The rule without the `target` key, and `"targetHidden": true`.
  `channel` stays, so the list can still say Webhook or E-mail. No count of recipients is
  returned: a rule holds exactly one target (one address is parsed as one mailbox,
  `lakehouse-notify/src/lib.rs:191`), so the console shows "Webhook" or "E-mail", not
  "E-mail (2)" (principle 2: no invented count).
- **D18. What is not changed, read at `512f6e37c`.** `POST` and `PUT /api/alerts` answer the
  saved rule to a caller who just passed `authorise_save` and typed the target
  (`alerts.rs:685-717`): unchanged. `GET /api/alerts/status` holds one boolean
  (`alerts.rs:198-221`). `GET /api/overview/alerts` (the incidents list) answers `AlertItem`,
  which has no target field (`lakehouse-store/src/overview.rs:37-90`); its texts are built
  from names and ids (`load_alerts.rs:57-61`, `alerts.rs:1203-1258`) and its delivery reason
  is one of the fixed sentences. `POST /api/alerts/test` answers `RunResult`, which has no
  target. Two pinning tests keep the first two that way (T9).
- **D19. Text on screen (English, as the console).** In the rules list the Delivery cell
  reads **"Webhook"** or **"E-mail"** when the target is hidden. For such a rule the switch is
  shown but disabled, and the test, edit and delete actions are not shown (the API refuses
  each of them for this caller already). The rule page for such a rule shows
  **"You cannot change this rule"** / **"Seeing where a rule sends, or changing it, needs the
  alert:write permission."** A run that timed out reads **"The run did not finish within the
  time limit and was stopped."**
- **D20. Assistant.** The alert tools are not a dashboard feature, but the list tool is the
  same helper, so the assistant gets the same redaction with no change of its own. No tool
  schema or description changes, so `tests/fixtures/tool_schemas.json` is not regenerated.

## 2. Tasks

### T1. The cause chain is read sixteen exceptions deep, not eight queue pops (`LH-SRC-6-53`; `SRC-38`)

- Anchors: `dagster/dispar_orchestrate/ingest_errors.py:159-162` (the comment and
  `_MAX_CAUSE_DEPTH = 8`), `:165-185` (`_chain`).
- Change: `_MAX_CAUSE_DEPTH = 16`. In `_chain`, replace `for _ in range(_MAX_CAUSE_DEPTH):`
  with a loop that runs while `pending` is not empty and fewer than `_MAX_CAUSE_DEPTH`
  exceptions were yielded (count one per `yield`). Keep the order of the four entries pushed,
  the `seen` set and the `None` skip exactly as they are. Rewrite the comment at `:159-161`
  to say what D5 says: the bound counts exceptions, `seen` ends a cycle, and why popping was
  the wrong thing to count.
- Check, in `dagster/dispar_orchestrate/test_ingest_errors.py` (chains built by hand by
  setting `__cause__` or `__context__` on exception objects; no network):
  - `test_a_failure_five_wrappers_deep_is_still_found`: four `RuntimeError`s linked by
    `__cause__`, the innermost a `ConnectionRefusedError`; `classify` is
    `Classified(UNREACHABLE, retried=True)`. Today it is `other`.
  - `test_a_failure_under_implicit_context_is_found_three_links_deep`: two `RuntimeError`s
    and a `ConnectionRefusedError` linked by `__context__` only; `unreachable`. Today `other`.
  - `test_the_walk_stops_after_sixteen_exceptions`: a `__cause__` chain of `RuntimeError`s with
    a `ConnectionRefusedError` as the 16th exception is `unreachable`; as the 17th it is
    `other`.
  - `test_a_cycle_in_the_cause_chain_ends_the_walk` (`:172`) stays unchanged and green.

### T2. An object-storage endpoint that is down is "unreachable" and retried (`LH-SRC-6-53`; `SRC-38`)

- Anchors: `ingest_errors.py:39-42` (the list of drivers in the module text), `:364-414`
  (`_classify_driver`; the new block goes after the `pyiceberg` line, `:412-413`).
- Change: the block of D4, using `names = _class_names(exc)` already computed at `:397`:
  for `root == "botocore"`, `names & {"ConnectTimeoutError", "ReadTimeoutError"}` returns
  `TIMEOUT`, then `names & {"EndpointConnectionError", "ConnectionClosedError"}` returns
  `UNREACHABLE`. Add `botocore` to the list at `:39-42` and one sentence there: it is what an
  object-storage read raises through `s3fs`, and it is still not imported. Do not import
  `botocore`, `s3fs` or `aiohttp` in this module.
- Check, in `test_ingest_errors.py`. `botocore` is installed in the Dagster environment (a
  dependency of the pinned `s3fs`); import it inside the test functions, as
  `test_a_real_postgres_refused_sign_in_is_recognised_without_a_sqlstate` does with
  `psycopg2`. Build the errors as the library does, with a made-up address under `.invalid`:
  `EndpointConnectionError(endpoint_url="http://storage.invalid:9000/b")`.
  - `test_an_object_storage_endpoint_that_is_down_is_unreachable_and_retried`: the chain of D3
    (two stand-in wrapper classes made with `_driver_error("dlt", ...)`, the outer one with an
    `exception` attribute and `__cause__`, the inner with `__cause__`, then the real
    `EndpointConnectionError`); `classify` is `Classified(UNREACHABLE, retried=True)`. Today
    `other`.
  - `test_each_object_storage_connection_error_gets_its_class`: `ConnectionClosedError` is
    `unreachable`; `ConnectTimeoutError` and `ReadTimeoutError` are `timeout`; all three
    retried.
  - `test_a_certificate_failure_at_the_object_store_is_not_called_unreachable`: `SSLError` is
    `Classified(OTHER, retried=False)`.
  - `test_the_classifier_imports_no_object_storage_library`: none of `botocore`, `s3fs`,
    `aiohttp` is a name in `vars(ingest_errors)`.
  - In `dagster/dispar_orchestrate/test_ingest_retries.py` (it already clears
    `RETRY_DELAYS_ENV`, `:22`):
    `test_an_unreachable_object_store_is_retried_after_one_five_and_fifteen_minutes`:
    `ingest_factory._decide(error, n).wait_seconds` is 60, 300, 900 for `n` 0, 1, 2 and `None`
    for 3.

### T3. Every connector run carries its time limit (`TC-1702`; `SRC-39`)

- Anchors: `dagster/dispar_orchestrate/ingest_factory.py:217-236` (`IngestFactoryConfig`, the
  place for the new setting), `:1550` (`@job(name="ingest_job")`).
- Change: beside `IngestFactoryConfig`, a frozen dataclass `RunLimitSettings` with one field
  `max_runtime_seconds: int = 21_600` and `from_env()`, written like `RetrySettings.from_env`
  (`ingest_errors.py:561-579`): reads `INGEST_RUN_MAX_RUNTIME_SECONDS`; unset or blank keeps
  the default; a value that is not a whole number of 1 or more keeps the default and logs a
  warning that names the variable and never echoes the value. A function
  `_ingest_job_tags()` returns `{"dagster/max_runtime": str(RunLimitSettings.from_env().max_runtime_seconds)}`
  and the decorator becomes `@job(name="ingest_job", tags=_ingest_job_tags())`. Do not pass
  `run_tags`: with `tags` alone both launch paths copy the tag onto the run (read in 1.13.20:
  the GraphQL launch merges the job's `tags`, `dagster_graphql/.../run_lifecycle.py:79`; the
  sensor merges `run_tags`, which is `tags` when `run_tags` is not set,
  `dagster/_daemon/sensor.py:1398-1400`, `job_definition.py:355-358`). The comment at the
  site says the tag is only enforced when run monitoring is on (T10) and names D9's reasons.
- Check, new file `dagster/dispar_orchestrate/test_ingest_run_limit.py` (narrative module
  docstring, `from __future__ import annotations`, `monkeypatch` for the variable):
  - `test_an_ingest_run_is_limited_to_six_hours_by_default`
  - `test_the_run_limit_comes_from_the_setting` (`"900"` gives the tag `"900"`)
  - `test_a_run_limit_that_cannot_be_read_falls_back_to_six_hours` (parametrised: `"abc"`,
    `"0"`, `"-5"`, `"1.5"`, `"  "`), and the warning does not contain the bad value
  - `test_the_ingest_job_and_its_runs_carry_the_limit_tag`: `"dagster/max_runtime"` is a key
    of both `ingest_factory.ingest_job.tags` and `ingest_factory.ingest_job.run_tags`.

### T4. The orchestrator client reports when a run was created and can cancel one for good (`TC-1702`; `SRC-39`)

- Anchors: `rust/crates/lakehouse-dagster/src/lib.rs:105-119` (`DgConfiguredRun`),
  `:1187-1201` (`list_runs_for_job_with_config`), `:1696-1728` (`terminate_run`), `:80-103`
  (`ReexecutionStrategy`, the pattern for a closed enum with a `graphql()` word), tests at
  `:3944` and `:4150-4199`.
- Change:
  1. `DgConfiguredRun` gets `pub creation_time: Option<f64>` with `#[serde(default)]` and a
     doc line (Unix seconds the run was created; the only time a run that never started has).
     The query at `:1194-1198` asks for `creationTime` as well.
  2. A private enum `TerminatePolicy { Safe, MarkCanceledNow }` with
     `const fn graphql(self) -> &'static str` (`SAFE_TERMINATE`,
     `MARK_AS_CANCELED_IMMEDIATELY`). Move the body of `terminate_run` into a private
     `terminate_with(&self, run_id, policy)`; `terminate_run` calls it with `Safe` and keeps
     its signature, its doc and its behaviour. New
     `pub async fn cancel_run_now(&self, run_id: &str) -> Result<LaunchOutcome, DgError>`
     calls it with `MarkCanceledNow`; its doc says when it is right (a run past its limit, D10)
     and has `# Errors`. The policy word is a constant of this crate, never caller text.
  3. `DgConfiguredRun { .. }` is built by hand in
     `rust/crates/lakehouse-api/src/routes/connectors.rs:4103-4111` (`dg_run`): add the field
     there so the workspace compiles (T8 changes that helper further).
- Check, in the test module of `lakehouse-dagster/src/lib.rs` (wiremock, no network):
  - `list_runs_for_job_with_config_returns_when_each_run_was_created`: a result with
    `"creationTime": 4.0` gives `Some(4.0)`; a result without the field gives `None`.
  - `cancel_run_now_asks_for_the_run_to_be_marked_cancelled_at_once`: the request body
    contains `MARK_AS_CANCELED_IMMEDIATELY` and the answer `TerminateRunSuccess` gives the run
    id.
  - `terminate_run_still_asks_for_a_safe_termination`: the request body contains
    `SAFE_TERMINATE`. The three existing `terminate_run_*` tests stay green unchanged.

### T5. A run record can end as "timed out" (`TC-1702`; `SRC-39`)

- Anchors: `rust/migrations/0072_connector_run_record.sql:54-56` (the CHECK),
  `rust/migrations/0097_connector_run_attempt_saas_classes.sql` (the pattern: why-header, then
  DROP and ADD of the named constraint), `rust/crates/lakehouse-store/src/connector_run.rs:113-146`
  (`EndReason`), `:269-275` (`wait_phrase`), `:563-615` (`end_run`), `:616-643`
  (`settle_run`), tests in `rust/crates/lakehouse-store/tests/connector_run.rs` (`:422`,
  `the_sensor_settles_only_a_run_still_running`).
- Change:
  1. New migration, the next free number (`0137` at `512f6e37c`; check `rust/migrations/`
     again before committing and renumber if another branch took it):
     `0137_connector_run_timed_out.sql`. Why-header: `SRC-39`, QA `TC-1702`; a run that
     outlived its limit ends with its own word instead of "a table failed"; Postgres cannot
     extend a CHECK in place; every value already allowed stays allowed. Then DROP and ADD
     `connector_run_end_reason_check` with
     `('completed', 'table_failed', 'connector_paused', 'other', 'timed_out')`, keeping the
     `end_reason IS NULL OR` part.
  2. `EndReason::TimedOut`: `as_str` `"timed_out"`, `phrase`
     `"the run did not finish in time and was stopped"`.
  3. Move the body of `end_run` after its `Running` check into a private
     `end_run_with_line(pool, connector_id, run_id, status, reason, at, line: &str)`.
     `end_run` calls it with the line it formats today, unchanged.
  4. New `pub async fn end_timed_out_run(pool, connector_id, run_id, limit_seconds: i64, at) -> Result<bool, StoreError>`:
     calls `end_run_with_line` with `RunStatus::Failed`, `EndReason::TimedOut` and the line
     `run ended: failed, the run did not finish in {limit} and was stopped`, where `{limit}` is
     whole hours as `6 h` when the seconds divide by 3600, whole minutes as `90 min` when they
     divide by 60, else `45 s` (a small private function beside `wait_phrase`). A run with no
     record, or of another connector, answers `Ok(false)` (nothing is invented, as
     `settle_run`); a run that already ended keeps its end and answers `Ok(false)`. Doc with
     `# Errors`.
- Check, in `rust/crates/lakehouse-store/tests/connector_run.rs` (not verified locally, CI):
  - `a_run_past_its_limit_ends_as_failed_with_the_limit_in_its_last_log_line`: after
    `start_run`, `end_timed_out_run(.., 21_600, ..)` is `true`; `list_runs` shows `failed`
    and `timed_out`; the last line of `get_run_log` is
    `run ended: failed, the run did not finish in 6 h and was stopped`.
  - `a_run_that_already_ended_or_has_no_record_is_left_alone_by_the_time_limit`: both answer
    `false`, and the ended run keeps `succeeded` and `completed`.

### T6. The two limits are settings of the API (`TC-1702`; `SRC-39`)

- Anchors: `rust/crates/lakehouse-api/src/config.rs:1125-1132` (the last field and its doc),
  `:1418-1421` (the hand-written `Debug`), `:2023-2028` (`from_map`), `:2393-2411` (the test
  to copy), `:1469` (`parse_positive_u32_or_default`).
- Change: two `pub` fields of type `u32`, each with a doc naming `SRC-39`, the variable and
  the default: `ingest_run_max_runtime_seconds` from `INGEST_RUN_MAX_RUNTIME_SECONDS`, default
  `21_600`; `ingest_run_start_timeout_seconds` from `INGEST_RUN_START_TIMEOUT_SECONDS`,
  default `600`. Both read with `parse_positive_u32_or_default`, both added to `Debug`. The
  doc of the second says it must equal `start_timeout_seconds` in `dagster/dagster.yaml`.
- Check, in the test module of `config.rs`:
  `the_run_limits_default_to_six_hours_and_ten_minutes_and_ignore_garbage` (defaults; `"0"`,
  `"-3"`, `"many"`, `""` keep them; `"900"` and `"120"` are taken).

### T7. A failed run that ran past the limit closes its record as "timed out" (`TC-1702`; `SRC-39`)

- Anchors: `rust/crates/lakehouse-api/src/routes/load_alerts.rs:439-453` (`confirm_status`,
  then `connector_run::settle_run`), `:168-177` (`run_end`), test harness `:803-863`
  (`knows_run_at` answers `startTime: 1000.0` and the end time given), `:1762`
  (`a_paused_run_is_not_counted_and_an_unclosed_run_is_settled`, the test to copy).
- Change: a private pure function
  `ran_at_least(start: Option<f64>, end: Option<f64>, limit_seconds: u32) -> bool` (`false`
  when either time is missing). In `connector_run_event`, before `settle_run`: when
  `outcome == Outcome::Failure` and
  `ran_at_least(info.start_time, info.end_time, state.config.ingest_run_max_runtime_seconds)`,
  call `connector_run::end_timed_out_run(pool, &connector.id, &req.run_id, i64::from(limit), run_end(info.end_time))`.
  `settle_run` stays where it is and then finds the run ended (or does its usual work for
  every other run). Nothing else in the function changes: the count, the alerts and their
  order are as today (D12, D14).
- Check, in the test module of `load_alerts.rs` (`#[sqlx::test]`; not verified locally, CI):
  - `a_failed_run_that_ran_past_the_limit_closes_its_record_as_timed_out`: `start_run`, then
    `knows_run_at("run-long", Some(CONNECTOR), "FAILURE", 1000.0 + 21_601.0)` and
    `h.failed("run-long")`; the record is `failed` with `end_reason` `timed_out`, and
    `streak(&pool)` is `("degraded", 1)`.
  - `a_failed_run_inside_the_limit_still_ends_as_a_failed_table`: the same with the default
    end time; `end_reason` is `table_failed`.
  - A plain `#[test]` `a_run_with_a_missing_time_is_never_called_timed_out` for `ran_at_least`.

### T8. An overdue run no longer locks the connector (`TC-1702`, `TC-1706`, `LH-SRC-11-11`; `SRC-39`)

- Anchors: `rust/crates/lakehouse-api/src/routes/connectors.rs:3862-3884` (the guard),
  `:4057-4095` (`ConnectorIngestRun`, `is_active`, `connector_runs`), `:4103-4140` (their unit
  tests), `rust/crates/lakehouse-api/tests/ingest_run.rs:56-61` (`ingest_run_for`, fixed
  `startTime`), `:486` and `:954` (the 409 tests).
- Change:
  1. A private enum `Overdue { NeverStarted, RanTooLong }`; a private struct `RunAgeLimits`
     with `start_seconds: f64` and `running_seconds: f64`, built from the two settings of T6,
     each plus the constant `OVERDUE_GRACE_SECONDS = 300.0` (its doc gives D10's reason).
  2. A private pure function
     `overdue(run: &DgConfiguredRun, now_unix: f64, limits: &RunAgeLimits) -> Option<Overdue>`,
     on the orchestrator's own status word: `QUEUED`, `NOT_STARTED` or `STARTING` with a
     `creation_time` more than `start_seconds` ago is `NeverStarted`; `STARTED` or `MANAGED`
     with a `start_time` more than `running_seconds` ago is `RanTooLong`; a missing time, and
     every other status, is `None`.
  3. `ConnectorIngestRun` gets `overdue: Option<Overdue>`; `connector_runs` takes `now_unix`
     and the limits and fills it. `is_active` is unchanged.
  4. In `launch_ingest`: collect this connector's active runs. If one of them is not overdue,
     answer the 409 exactly as today, naming that run. Otherwise, for each overdue run, call a
     new `stop_overdue_run`, then continue to the launch. Extend the comment at `:3862-3867`
     with the case and the decision (`QA TC-1702 (SRC-39)`, D10).
  5. `stop_overdue_run(state, connector_id, run, now) -> Result<(), ApiError>` (doc with
     `# Errors`): call `state.dagster.cancel_run_now(&run.run_id)`; a transport error or a
     refusal is written with `tracing::warn!` and is not an error of the request (the error's
     text goes to the log only, never into an `ApiError`). For `RanTooLong`, call
     `connector_run::end_timed_out_run` with the run limit and `now`; a store error is
     returned with `?`. Then one `tracing::info!` line with the connector id and the run id.
  6. Tests that say "a run is going" must say it with a recent start: `ingest_run_for`
     (`ingest_run.rs:56-61`) and `dg_run` (`connectors.rs:4103`) use a fixed start time in
     September 2026, which is now older than the limit. Make both use the current time minus
     60 seconds, and add `creationTime` / `creation_time` with the same value. This does not
     weaken those tests: it makes the fixture say what the test name says. Write that in the
     commit body (rule 2).
- Check:
  - Unit, in `connectors.rs` `mod tests`:
    `a_run_started_longer_ago_than_the_limit_is_overdue_and_a_younger_one_is_not`,
    `a_run_that_never_started_is_overdue_after_the_start_limit`,
    `a_run_with_no_time_recorded_is_never_overdue`, `a_finished_run_is_never_overdue`.
  - Integration, in `rust/crates/lakehouse-api/tests/ingest_run.rs` (not verified locally, CI):
    `a_run_older_than_the_limit_is_cancelled_and_the_next_run_starts` (the orchestrator lists
    one `STARTED` run of the connector that started seven hours ago, and a record of it exists
    from `start_run`; the answer is 200 with the new run id; the orchestrator received a
    request containing `MARK_AS_CANCELED_IMMEDIATELY` and the old run's id; the old record is
    `failed` with `timed_out`),
    `a_run_that_never_started_stops_locking_the_connector_after_the_start_limit` (`NOT_STARTED`,
    created 20 minutes ago, no record: 200, the cancel was asked, no record was written), and
    `an_orchestrator_that_refuses_the_cancel_does_not_keep_the_connector_locked` (the cancel
    answers `TerminateRunFailure`: still 200, and the response body holds none of the
    orchestrator's message). `ingest_run_refuses_a_second_run_while_one_is_going` stays a 409.

### T9. A rule's target is returned only to someone who may manage the rule (`LH-SRC-7-04`; `SRC-42`, `DEC-16`)

- Anchors: `rust/crates/lakehouse-api/src/routes/alerts.rs:42-56` (the doc of `list`),
  `:89-131` (`listed_rules`; the serialisation is `:116-119`), `:3485-3505` (`engineer()`,
  `admin()`: how a test builds a `Principal`), `routes/chart_alerts.rs:308-317`
  (`may_manage`), `rust/crates/lakehouse-api/tests/ai_alert_tools.rs:52-61`, `:140-200`
  (`rule_row`, `fixture(permissions, ..)`, `call_tool`: the fixture to copy).
- Change: a private function in `alerts.rs`,
  `rule_json_for(principal: Option<&Principal>, rule: &AlertRule) -> Result<Value, serde_json::Error>`:
  serialise the rule; when `!crate::routes::chart_alerts::may_manage(principal, Some(rule))`,
  remove the key `target` from the object and insert `"targetHidden": true`. `listed_rules`
  calls it in place of `serde_json::to_value(&rule)` at `:116`; its error handling stays.
  Comment at the function: `DEC-16`, QA `LH-SRC-7-04`, why (a webhook address can carry a
  token), and that this is the one place (D15). Update the doc of `list` (`:42-50`) and of
  `listed_rules` with one sentence each. Nothing else: `AlertRule`, `create`, `update`,
  `POLICY_TABLE` and the tool schemas are not touched.
- Check:
  - Unit, a new `#[cfg(test)]` module at the end of `alerts.rs` (plain `#[test]`, no
    database): `a_caller_with_alert_read_only_gets_the_rule_without_its_target` (no `target`
    key, `targetHidden` is `true`, `channel` is still `"webhook"`, and the serialised text does
    not contain the address), `a_caller_with_alert_write_gets_the_target` (`target` present,
    no `targetHidden` key), `the_owner_of_a_chart_alert_gets_their_own_target_without_alert_write`,
    `a_caller_with_no_principal_gets_no_target`.
  - Integration, new file `rust/crates/lakehouse-api/tests/alert_targets_hidden.rs`, built
    like `ai_alert_tools.rs` (a recording `ClickHouse` fake that answers the rule read with one
    mart rule whose target is `https://hooks.example.com/secret-token`, and
    `create_assistant_principal(&pool, permissions)` for the two callers:
    `"alert:read"` and `"alert:read, alert:write"`). Not verified locally, CI.
    - `get_alerts_hides_the_target_from_a_caller_without_alert_write` and
      `get_alerts_returns_the_target_to_a_caller_with_alert_write` (`GET /api/alerts`).
    - `the_assistants_rule_list_hides_the_target_from_a_caller_without_alert_write` and
      `the_assistants_rule_list_returns_the_target_to_a_caller_with_alert_write`
      (`POST /api/ai/tool`, tool `list_alert_rules`).
    - Two pins that pass today and are there so a later field cannot add the address:
      `the_incident_list_never_carries_a_rule_target` (`GET /api/overview/alerts`, for both
      callers, after one instance is written with `overview::insert_from_fired_rule`: no item
      has a `target` key and the body does not contain the address) and
      `the_alert_status_holds_one_boolean_and_nothing_else` (`GET /api/alerts/status`, both
      callers).

### T10. Run monitoring is on, and the limits reach both services (`TC-1702`, `LH-SRC-11-11`; `SRC-39`)

- Anchors: `dagster/dagster.yaml` (19 lines, `storage` only), `dagster/Dockerfile:103-105`
  (the file is copied into the image), `docker-compose.yml:527-532` (the API's
  `CONNECTOR_AUTO_DISABLE_AFTER` block), `:2293-2299` (the code location's
  `INGEST_RETRY_DELAYS_SECONDS` block), `ops/g6/src11_connector_operations_test.py:382-387`
  (`dagster_status`, a GraphQL call to copy), `:255` (`poll`), `:838-865` (`run`),
  `.github/workflows/ci.yml:2602-2603` (job `g6-ops`, "Rust · SRC-11 connector operations").
- Change:
  1. `dagster/dagster.yaml`: append, under a comment block that says why (D8, D9, the QA
     cases, and that these fields cannot be read from the environment):
     ```yaml
     run_monitoring:
       enabled: true
       start_timeout_seconds: 600
       cancel_timeout_seconds: 300
       max_resume_run_attempts: 0
       poll_interval_seconds: 60
     ```
     No `max_runtime_seconds` (D9).
  2. `docker-compose.yml`: after `:532`, `INGEST_RUN_MAX_RUNTIME_SECONDS: ${INGEST_RUN_MAX_RUNTIME_SECONDS:-}`
     and `INGEST_RUN_START_TIMEOUT_SECONDS: ${INGEST_RUN_START_TIMEOUT_SECONDS:-}`; after
     `:2299`, `INGEST_RUN_MAX_RUNTIME_SECONDS: ${INGEST_RUN_MAX_RUNTIME_SECONDS:-}`. Each with
     a comment in the style of its neighbour: what it is, the default the code keeps when it
     is unset or unreadable, `SRC-39`, not a credential. Unique keys; no default value in the
     file.
  3. Gate: a new step in `ops/g6/src11_connector_operations_test.py`,
     `step_stuck_runs_are_ended(run_id)`, called from `run()` as
     `named("h. run monitoring is on and a run carries its limit", ...)` with the run id that
     step b returns (`pg_run`). It asks the orchestrator
     `{ instance { daemonHealth { daemonStatus(daemonType: "MONITORING") { required healthy } } } }`
     and requires `required` true, polling with `poll` until `healthy` is true; then asks for
     that run's tags and requires a `dagster/max_runtime` tag whose value is a whole number of
     1 or more. A failure raises `OpsGateFailure` starting with the step's name. Add the step
     to the module text's list of steps (`:28-60`).
- Check: this is a compose and image change, so its proof is a real `up` (rule 8). On the
  build machine that is **not verified locally**; the CI gate that proves it is `g6-ops`
  ("Rust · SRC-11 connector operations"), which builds the Dagster image, brings the
  `dagster` profile up and runs the new step h after real runs of steps a to g under
  monitoring. `g3a-dagster` brings the same image up as well. Say so in the handoff.

### T11. The connector page says when a run was stopped for running too long (`TC-1702`; `SRC-39`)

- Anchors: `src/services/contracts/connectors.ts:997-998` (`RunEndReason`),
  `src/features/connectors/connector-ingest-panel.tsx:1336-1341` (`END_REASON_LABEL`),
  `src/features/connectors/connector-overview.tsx:45-54` (the "last run failed" message).
- Change: add `"timed_out"` to `RunEndReason`; add
  `timed_out: "The run did not finish within the time limit and was stopped."` to
  `END_REASON_LABEL`; in `connector-overview.tsx`, a failed run whose `endReason` is
  `"timed_out"` reads "The last ingest run did not finish within the time limit and was
  stopped." `STALE_RUN_MS` and "No end reported" are not changed: they still cover a record
  nobody closed.
- Check: in `src/features/connectors/connector-ingest-panel.test.tsx`, a failed run with
  `endReason: "timed_out"` shows the D19 sentence; in
  `src/features/connectors/connector-overview.test.tsx`, the problem line for such a run is
  the sentence above.

### T12. The Alerts pages show "Webhook" or "E-mail" when the address is hidden (`LH-SRC-7-04`; `SRC-42`, `DEC-16`)

- Anchors: `src/services/contracts/alerts.ts:55-73` (`AlertRule`, `target: string`),
  `src/features/alerts/rules-columns.tsx:258-274` (Delivery cell), `:275-299` (the switch),
  `:300-355` (actions), `src/features/alerts/alerts-page.tsx:158` (search fields),
  `src/features/alerts/alert-rule-page.tsx:138-168` (`alertRuleSaveInput`, three
  `f.target.trim()`), `:461-477` (the not-found state), `:506`, `:690`.
- Change:
  1. Contract: `target?: string` and `targetHidden?: boolean`, with a doc that mirrors T9
     (omitted, with `targetHidden: true`, when the caller may not manage the rule).
  2. `rules-columns.tsx`: an exported function `deliveryLabel(rule)` that returns
     `"E-mail"` or `"Webhook"` by `channel` when `rule.targetHidden`, else `rule.target ?? ""`;
     the Delivery cell shows it. When `rule.targetHidden`: the switch is rendered `disabled`,
     and the actions cell renders nothing (D19).
  3. `alert-rule-page.tsx`: read the target as `f.target ?? ""` at the five sites. After the
     not-found block, when the rule has `targetHidden`, return an `EmptyState` with the D19
     title and description and the same "Back to Alerts" action; the form is not rendered.
  4. `alerts-page.tsx:158`: keep the search on the target; make it type-check with the
     optional field.
- Check: in `src/features/alerts/alerts-page.test.tsx`: a rule with `targetHidden: true` and
  `channel: "webhook"` shows "Webhook", its switch is disabled and it has no "Test run now"
  button; a rule with `channel: "email"` and `targetHidden` shows "E-mail"; a rule with a
  target shows the address, an enabled switch and its actions. In
  `src/features/alerts/alert-rule-page.test.tsx`: the page for a `targetHidden` rule shows
  "You cannot change this rule" and no Save button; the page for a rule with a target is
  unchanged.

### T13. Documents

- `docs/core/features/load-failure-alerts.md`: in the Decisions table, after row 9, add
  `| 10 | Beyond the spec: who sees where a rule sends (`DEC-16`, QA `LH-SRC-7-04`). | Only a person with `alert:write`, and the owner of a chart alert for their own rule. A person with `alert:read` sees the rule, its state and whether it sends to a webhook or an e-mail address, not the address. | 2026-11-12 |`.
  In the acceptance checklist, after row 12, add
  `| 13 | As a user with `alert:read` and without `alert:write`, open the Alerts page and read `GET /api/alerts` | Each rule shows "Webhook" or "E-mail" and no address; the answer holds no webhook address and no e-mail address | Not run |`.
- `docs/core/features/connector-operations.md`: in the Decisions table, after row 14, add
  `| 15 | Beyond the spec: a run that never finishes (`SRC-39`, QA `TC-1702`). | A run that has not started after 10 minutes, or is still running after 6 hours *(proposed)*, is stopped and recorded as failed; the connector can run again. The 6 hours are a setting. | |`
  (the Signed cell stays empty for the owner). Under "Limits to tell a customer" add
  `- A load that needs more than 6 hours is stopped unless `INGEST_RUN_MAX_RUNTIME_SECONDS` is raised.`
  In the acceptance checklist, after row 11, add
  `| 12 | Stop the orchestrator's worker while a run is going, and wait for the limit (a test setting may lower it; say so in the result) | The run ends as failed with "did not finish within the time limit", the connector's health follows, and "Run now" starts a new run | Not run |`.
- `docs/CONFIGURATION.md`: after line 241, two rows in the table's own format:
  `INGEST_RUN_MAX_RUNTIME_SECONDS` (how long a connector run may run before it is stopped and
  recorded as failed, read by `lakehouse-api` and `dagster-code-location`; default `21600`)
  and `INGEST_RUN_START_TIMEOUT_SECONDS` (how long the API waits for a queued run to start
  before it stops holding the connector; must equal `start_timeout_seconds` in
  `dagster/dagster.yaml`; default `600`).
- Check: none (documents). The developer does not edit `BACKLOG.md`, `PRODUCT.md` or any
  other document.

## 3. Verification

For this build machine (11 GB RAM, little disk). Never `cargo clean`. Never a bare
`cargo test -p lakehouse-api <filter>`.

Per commit, only for what the commit touched:

```bash
# Python (T1, T2, T3); the interpreter is the Dagster environment's
(cd dagster && python -m pytest dispar_orchestrate/test_ingest_errors.py -q)
(cd dagster && python -m pytest dispar_orchestrate/test_ingest_retries.py -q)
(cd dagster && python -m pytest dispar_orchestrate/test_ingest_run_limit.py -q)

# Rust (T4 to T9)
(cd rust && cargo fmt --check)
/home/shiro/rantai/bin/cargo-locked check -p lakehouse-dagster -j 2    # T4
/home/shiro/rantai/bin/cargo-locked check -p lakehouse-store -j 2      # T5
/home/shiro/rantai/bin/cargo-locked check -p lakehouse-api -j 2        # T4 (the dg_run field), T6 to T9
/home/shiro/rantai/bin/cargo-locked test -p lakehouse-dagster --lib terminate -j 2   # T4, wiremock only

# TypeScript (T11, T12)
bun run typecheck && bun run lint
bun test src/features/connectors/connector-ingest-panel.test.tsx src/features/connectors/connector-overview.test.tsx
bun test src/features/alerts/alerts-page.test.tsx src/features/alerts/alert-rule-page.test.tsx
```

Before handoff, in the foreground, on the final commit: the blocks above once more, the whole
Dagster suite because T1 changes how deep every failure is read
(`(cd dagster && python -m pytest dispar_orchestrate -q)`), and:

```bash
python3 ops/lint/check_bare_iceberg_count.py && python3 ops/lint/check_intra_package_imports.py
git status   # nothing generated staged
```

Listed in the handoff as **not verified locally, CI**, each by name: the `#[sqlx::test]` tests
of T5 and T7; the unit tests of T6, T8 and T9 that live in `lakehouse-api` (they may be run as
`cargo-locked test -p lakehouse-api --lib <test name> -j 2` if memory allows; say which were);
the integration tests in `rust/crates/lakehouse-api/tests/ingest_run.rs` and
`alert_targets_hidden.rs`; `cargo clippy`; and T10, whose gate is `g6-ops`.

For QA's retest of `LH-SRC-6-53` (D7), to be written in the pull request by the planner: with
the storage down the run's attempts read "The source could not be reached" and are tried
again after 1, 5 and 15 minutes; when the run has failed, wait one minute, check that the
Alerts page does not say run alerts are off, and read the connector's health: Degraded after
the first failed run. If it still reads Healthy with reports on, that is a new defect with
its own backlog row.

## 3a. Change of machine (planner, 2026-11-12)

The developer works on the ARM64 build machine, in the checkout
`/home/rismana/rantai/lakehouse-agents-4` (20 CPUs, 121 GB of RAM), not on the machine
section 3 describes. Section 3's list of what to run stays; how to run it changes:

- Before the first cargo command of a shell: `export CARGO_TARGET_DIR=/home/rismana/rantai/cargo-target-4 CARGO_BUILD_JOBS=6`.
  There is no `/home/shiro/rantai/bin/cargo-locked` here: where section 3 writes
  `/home/shiro/rantai/bin/cargo-locked <args> -j 2`, run `cargo <args>`.
- Per Rust commit, for each crate the commit touched: `cargo fmt --check`,
  `cargo clippy -p <crate> --all-targets -- -D warnings`, and the unit tests the task names,
  as `cargo test -p <crate> --lib <test name>`. These are no longer left to CI.
- An integration test file under `rust/crates/lakehouse-api/tests/` that a task adds to or
  changes is run as `cargo test -p lakehouse-api --test <file>` (Docker is running here). Never
  a bare `cargo test -p lakehouse-api <filter>`. If a container image has no ARM64 build or a
  test cannot start for a reason of the machine, write *not verified locally, CI* with the
  error's first line; do not change the test.
- Before the first TypeScript check: `bun install --frozen-lockfile`.
- CI on the pull request (x86_64) is still the final word. The handoff quotes each command
  with its counts and says *not verified* for anything not run (rule 7).
- The machine is shared with another project: start no container you do not remove, stop no
  process you did not start, and work only under `/home/rismana/rantai/`.

## 4. Out of scope

- A wrong access key or a missing bucket at the object store (`s3fs` raises Python's
  `PermissionError` or `FileNotFoundError`): still `other`. Only "cannot be reached" and
  timeouts are classified here.
- Making health follow runs when run reports are off (`PIPELINE_RUN_TOKEN` unset), or a notice
  for it on the connector page: `SRC-7` decision 9 stands.
- A row in the run history for a run that never started (D13).
- A limit per connector, and a limit for any job other than `ingest_job` (uploads, pipelines,
  exports). Run monitoring's start timeout does apply to every job; nothing else does.
- `SRC-18` (second half) and anything in `record_run_once` (`SRC-32`): see D14.
- The "New rule" button for a person without `alert:write`, and who holds `alert:read`
  (`ADM-13`, migration `0133`): `DEC-16` decided the targets only.
- The assistant's tool schemas and prompt.
- The gate does not make a run hang until it is stopped; it proves that monitoring is on and
  that runs carry the limit (T10). The behaviour past the limit is the orchestrator's own.

## Open questions for the owner

None of these blocks the build; each has a planned default.

1. **The two numbers (D9)**: 6 hours for a running run, 10 minutes for a run that has not
   started. Both are *(proposed)*. The 6 hours are the bound the console already uses.
2. **A run cancelled at "Run now" is not counted as a failed run (D12)**, while a run the
   orchestrator stops by itself is. Planned as written, after `SRC-11` decision 3.
3. **A run that never started is not listed in the history (D13)**; it is counted and
   alerted. Should it get a row ("Did not start")? That is a follow-up item if yes.

## 5. Handoff

Developer: a non-Opus coding agent, on the ARM64 build machine, in
`/home/rismana/rantai/lakehouse-agents-4`, branch
`fix/qa-round-connectors-p1` off `origin/fix/qa-round-connectors-p1` at
`eb209d04`. Every commit was pushed after the checks below. No pull request was
opened. Migration number: `git fetch origin && git ls-tree --name-only
origin/main rust/migrations/ | tail -3` answered `0133`, `0134`, `0135`,
`0136`, so `0137` was free and is used as planned.

### Commits

| Commit | Title |
| --- | --- |
| `f5229f15` | fix(dagster): read a failure chain sixteen exceptions deep |
| `faf6f7c6` | fix(dagster): an object store that is down is unreachable and retried |
| `6831fa70` | feat(dagster): every ingest run carries its time limit |
| `52e6c658` | feat(dagster): report when a run was created and cancel one for good |
| `e28f2447` | feat(store): a run record can end as timed out |
| `a5a8a0d8` | feat(api): the two run limits are settings of the API |
| `70c85a6a` | feat(api): a failed run past the limit closes as timed out |
| `1e0b0658` | feat(api): an overdue run no longer locks the connector |
| `9febaba0` | feat(api): a rule's target is returned only to whoever may manage it |
| `3ab35d23` | feat(compose,dagster): run monitoring is on and the limits reach both services |
| `7fd71e12` | feat(console): the connector page says when a run was stopped for running too long |
| `69d4ca07` | feat(console): the Alerts pages say Webhook or E-mail when the address is hidden |
| `27d75539` | docs: the pages and the config table name the run limits and the hidden target |

Tasks T1 to T13, in the plan's order, one task per commit. T14 does not exist;
the documents are T13.

### Tests added, by task

T1 (`dagster/dispar_orchestrate/test_ingest_errors.py`):
`test_a_failure_five_wrappers_deep_is_still_found`,
`test_a_failure_under_implicit_context_is_found_three_links_deep`,
`test_the_walk_stops_after_sixteen_exceptions`. All three answered `other`
before the change (3 failed, 66 deselected) and pass after it.

T2 (`test_ingest_errors.py`, `test_ingest_retries.py`):
`test_an_object_storage_endpoint_that_is_down_is_unreachable_and_retried`,
`test_each_object_storage_connection_error_gets_its_class`,
`test_a_certificate_failure_at_the_object_store_is_not_called_unreachable`
(a pin: `other` before and after; its first run failed only because
`SSLError` needs its `error=` argument, which the test now passes),
`test_the_classifier_imports_no_object_storage_library`,
`test_an_unreachable_object_store_is_retried_after_one_five_and_fifteen_minutes`.
The first, second and last failed before the change (`other`, no wait).

T3 (new `dagster/dispar_orchestrate/test_ingest_run_limit.py`):
`test_an_ingest_run_is_limited_to_six_hours_by_default`,
`test_the_run_limit_comes_from_the_setting`,
`test_a_run_limit_that_cannot_be_read_falls_back_to_six_hours` (`"abc"`,
`"0"`, `"-5"`, `"1.5"`, `"  "`; the warning never echoes the value, compared
with the default removed from the log because `21600` contains `0`),
`test_the_ingest_job_and_its_runs_carry_the_limit_tag`. All 8 cases errored
before the change (no `RunLimitSettings`, no `_ingest_job_tags`).

T4 (`lakehouse-dagster/src/lib.rs` tests):
`list_runs_for_job_with_config_returns_when_each_run_was_created`,
`cancel_run_now_asks_for_the_run_to_be_marked_cancelled_at_once`,
`terminate_run_still_asks_for_a_safe_termination`. The first two did not
compile before the change; the three existing `terminate_run_*` tests stay
green unchanged.

T5 (`lakehouse-store/tests/connector_run.rs`):
`a_run_past_its_limit_ends_as_failed_with_the_limit_in_its_last_log_line`,
`a_run_that_already_ended_or_has_no_record_is_left_alone_by_the_time_limit`.
Both did not compile before the change.

T6 (`lakehouse-api/src/config.rs` tests):
`the_run_limits_default_to_six_hours_and_ten_minutes_and_ignore_garbage` (did
not compile before the change).

T7 (`lakehouse-api/src/routes/load_alerts.rs` tests):
`a_failed_run_that_ran_past_the_limit_closes_its_record_as_timed_out`,
`a_failed_run_inside_the_limit_still_ends_as_a_failed_table`,
`a_run_with_a_missing_time_is_never_called_timed_out` (a plain `#[test]`; it
did not compile before the change, and it was the one run before the fix).

T8 (`lakehouse-api/src/routes/connectors.rs` tests and
`lakehouse-api/tests/ingest_run.rs`):
`a_run_started_longer_ago_than_the_limit_is_overdue_and_a_younger_one_is_not`,
`a_run_that_never_started_is_overdue_after_the_start_limit`,
`a_run_with_no_time_recorded_is_never_overdue`,
`a_finished_run_is_never_overdue` (the first did not compile before the
change); integration:
`a_run_older_than_the_limit_is_cancelled_and_the_next_run_starts`,
`a_run_that_never_started_stops_locking_the_connector_after_the_start_limit`,
`an_orchestrator_that_refuses_the_cancel_does_not_keep_the_connector_locked`.
`ingest_run_refuses_a_second_run_while_one_is_going` stays a 409.
`ingest_run_for`/`dg_run` now say "a run is going" with a start of 60 seconds
ago plus `creationTime`; that is in the commit body (rule 2: no assertion was
weakened).

T9 (`lakehouse-api/src/routes/alerts.rs` `mod hidden_targets`, and new
`lakehouse-api/tests/alert_targets_hidden.rs`):
`a_caller_with_alert_read_only_gets_the_rule_without_its_target`,
`a_caller_with_alert_write_gets_the_target`,
`the_owner_of_a_chart_alert_gets_their_own_target_without_alert_write`,
`a_caller_with_no_principal_gets_no_target` (the first did not compile before
the change); integration:
`get_alerts_hides_the_target_from_a_caller_without_alert_write`,
`get_alerts_returns_the_target_to_a_caller_with_alert_write`,
`the_assistants_rule_list_hides_the_target_from_a_caller_without_alert_write`,
`the_assistants_rule_list_returns_the_target_to_a_caller_with_alert_write`,
`the_incident_list_never_carries_a_rule_target`,
`the_alert_status_holds_one_boolean_and_nothing_else`. The two "hides" tests
were also run with the redaction taken out of `listed_rules` and failed with
the address in the body (5 passed, 2 failed), then passed with it.

T11 (`src/features/connectors/connector-ingest-panel.test.tsx`,
`connector-overview.test.tsx`): "says a run that outlived its limit was
stopped" and "says a last run stopped by the time limit was stopped for
running too long" (both failed before the change).

T12 (`src/features/alerts/alerts-page.test.tsx`,
`alert-rule-page.test.tsx`): "says Webhook for a hidden webhook rule, disables
its switch and shows no actions", "says E-mail for a hidden e-mail rule",
"shows the address, an enabled switch and the actions when the target is
returned", "says a rule whose target is hidden cannot be changed", "still
renders the form for a rule whose target is returned" (the first, second and
fourth failed before the change; the switch's disabled state is base-ui's
`aria-disabled`, not `.disabled`).

### Commands run, with their counts

Python, per commit (the interpreter is a venv built for this task with
`uv venv /home/rismana/rantai/venv-agents-4 --python 3.12` and
`uv pip install -e . pytest` in `dagster/`):

- `(cd dagster && python -m pytest dispar_orchestrate/test_ingest_errors.py dispar_orchestrate/test_ingest_retries.py -q)`
  → 81 passed (T1), 86 passed (T2).
- `(cd dagster && python -m pytest dispar_orchestrate/test_ingest_errors.py dispar_orchestrate/test_ingest_retries.py dispar_orchestrate/test_ingest_run_limit.py -q)`
  → 94 passed (T3).

Rust, per commit, with `export CARGO_TARGET_DIR=/home/rismana/rantai/cargo-target-4 CARGO_BUILD_JOBS=6`:

- `cargo fmt --check` → OK after every Rust commit.
- `cargo test -p lakehouse-dagster --lib terminate` → 4 passed; `--lib cancel_run_now`
  → 1 passed; `--lib list_runs_for_job_with_config_returns_when_each_run_was_created`
  → 1 passed; `cargo clippy -p lakehouse-dagster --all-targets -- -D warnings`
  → clean; `cargo check -p lakehouse-api` → OK (T4).
- `cargo test -p lakehouse-store --test connector_run` → 11 passed;
  `cargo clippy -p lakehouse-store --all-targets -- -D warnings` → clean (T5).
- `cargo test -p lakehouse-api --lib the_run_limits_default_to_six_hours` → 1
  passed; `cargo clippy -p lakehouse-api --all-targets -- -D warnings` → clean
  (T6).
- `cargo test -p lakehouse-api --lib "routes::load_alerts::tests"` → 25 passed
  (T7).
- `cargo test -p lakehouse-api --test ingest_run` → 25 passed;
  `cargo test -p lakehouse-api --lib "routes::connectors::tests"` → 68 passed
  (T8).
- `cargo test -p lakehouse-api --test alert_targets_hidden` → 7 passed;
  `cargo test -p lakehouse-api --lib "routes::alerts::"` → 42 passed (T9).
  (`Docker` is running here, so the `#[sqlx::test]` tests of T5, T7, T8 and
  T9 ran locally, not only in CI.)

TypeScript (`bun install --frozen-lockfile` once first):

- `bun test src/features/connectors/connector-ingest-panel.test.tsx src/features/connectors/connector-overview.test.tsx`
  → 108 passed (T11).
- `bun test src/features/alerts/alerts-page.test.tsx src/features/alerts/alert-rule-page.test.tsx`
  → 43 passed (T12).
- `bun run typecheck` → clean; `bun run lint` → 0 errors, 11 warnings, all
  pre-existing in files this slice does not touch.

Compose and Python lints (T10):

- `sh ops/init-env.sh` once (no `.env` existed; it is untracked and not
  staged), then `docker compose --profile '*' config --quiet` → OK.
- `python3 ops/lint/check_intra_package_imports.py` → OK;
  `python3 ops/lint/check_bare_iceberg_count.py` → OK.

Before handoff, on the final commit `27d75539`, in the foreground:

- `(cd dagster && python -m pytest dispar_orchestrate -q)` → **2540 passed, 1
  skipped, 41 subtests passed** in 837.83s.
- `python3 ops/lint/check_intra_package_imports.py` and
  `check_bare_iceberg_count.py` → OK; `docker compose --profile '*' config --quiet`
  → OK.
- `(cd rust && cargo fmt --check)` → OK.
- `(cd rust && cargo clippy --workspace --all-targets --all-features -- -D warnings)`
  → clean (`Finished dev profile`).
- `(cd rust && cargo test --workspace --no-fail-fast)` → every target and every
  doc-test passed except one test (below). One failing test in the whole
  workspace: `a_pdf_failure_is_logged_for_the_operator_and_the_record_stays_fixed`
  (`lakehouse-api --test report_runner`: 23 passed, 1 failed).
- `bun run typecheck` → clean; `bun run lint` → 0 errors, 11 warnings;
  `bun run test` → **4000 passed, 1 skipped, 0 failed** (4001 tests, 380
  files).
- `git status` → clean (the `.env` is ignored; no `__pycache__/`, `*.pyc`,
  `rust/target/` or other generated file is staged or tracked).

### One failing test, and it fails on `main` too

`a_pdf_failure_is_logged_for_the_operator_and_the_record_stays_fixed`
(`rust/crates/lakehouse-api/tests/report_runner.rs`) reads a tracing capture
from `tracing::subscriber::set_default` and is a flake under parallel test
execution: it passes alone (`cargo test -p lakehouse-api --test report_runner
a_pdf_failure_is_logged_for_the_operator_and_the_record_stays_fixed` → 1
passed) and fails when the whole file runs (`--test report_runner` → 23
passed, 1 failed). It is not caused by this branch: in a detached worktree at
`origin/main` (`75010df8`) with the same target directory, the same file
failed **5 of 5** runs, the same test. CI runs nextest, one process per test,
where it passes. It is unrelated to this slice (reports and PDF rendering) and
needs a backlog item — the planner's call; I did not edit `BACKLOG.md`.

### Not verified locally, CI

- **T10, its whole proof**: the real `docker compose up` this change needs
  (rule 8) and gate step h (`step_stuck_runs_are_ended`) were **not verified
  locally, CI**. The CI gate is `g6-ops` ("Rust · SRC-11 connector operations"),
  which builds the Dagster image, brings the `dagster` profile up and runs the
  new step h after real runs of steps a to g under monitoring; `g3a-dagster`
  brings the same image up as well. `docker compose config --quiet` is all that
  was run here.
- Nothing else is unverified. The `#[sqlx::test]` tests of T5, T7, T8 and T9,
  the `lakehouse-api` unit tests of T6, T8 and T9, and the integration tests
  `ingest_run.rs` and `alert_targets_hidden.rs` all ran locally with their
  counts above; CI runs them again through nextest's partitions.

### Notes for the reviewer

- `launch_ingest` grew past clippy's `too_many_lines`; the guard was extracted
  as `guard_one_run_at_a_time` rather than allowed, and the one allow added is
  `clippy::cast_precision_loss` with a checkable reason (Unix seconds to `f64`
  to match `DgConfiguredRun`), in `connectors.rs` and in the `ingest_run.rs`
  test module.
- The 6 hours and the 10 minutes stay *(proposed)* in the feature page's
  decision 15, with the Signed cell empty for the owner, as the plan says.
- `tests/fixtures/tool_schemas.json` was not regenerated: no tool schema or
  description changed (D20).

### MISMATCH

None. Every anchor of the plan was found by its text; the only differences were
line numbers, and the plan's `:4103` `dg_run` fixture needed the `creation_time`
field the same task adds, as it says.

## 6. Review

### Round 1 (planner, 2026-11-12), at `90ff6cb7`

Read against the plan and `AGENTS.md`; nothing was run for this review, CI on the draft pull
request (#485) runs the suites and the `g6-ops` gate. 1 BLOCKER (R1) and 10 SHOULD-FIX. No
existing test was weakened and no route returns an alert target to a caller without
`alert:write`.

**Planner's rulings** (they replace "planner decision" in the findings below; where a ruling
and a finding's "Fix" differ, the ruling holds).

- **R1 (changes D10 and D13).** The plan was wrong about a queued run: the orchestrator queues
  every run, and its monitoring never ends a queued one. New rule for `overdue`:
  - `QUEUED`: overdue only when it was created longer ago than the **running** limit plus the
    grace (a run that waited in the queue longer than a run may run). Until then the route
    answers 409 as before this branch.
  - `NOT_STARTED`: the start limit plus the grace, by `creationTime`, as built.
  - `STARTING`: never overdue here; the route answers 409 and leaves it to the orchestrator's
    monitoring, which fails it after the start limit.
  - `STARTED`: the running limit plus the grace, by `startTime`, as built.
  Correct the comments and the document lines R1 names so they say this, and reword D13 and
  decision 15 of `connector-operations.md` for the queued case (Signed cell: *(proposed, not
  yet signed)*).
- **R2.** As the finding says: an orchestrator that cannot be reached for the cancel answers
  503 and writes nothing; a typed refusal lets the launch go on but claims no stop; the record
  is closed before the cancel is asked for.
- **R3.** As the finding says: decide first, refuse what the request itself gets wrong, and
  stop the overdue run only immediately before the launch.
- **R7.** Row 10 of `load-failure-alerts.md` keeps only the words of `DEC-16` with its date;
  the chart-owner exception (D16) moves to its own row with the Signed cell *(proposed, not
  yet signed)*; fix the section's opening line to the number of rows it now has.
- **R8.** The API returns the running limit with the connector's run list (one number, in
  seconds, camelCase, read from the same setting), and the console uses it in place of
  `STALE_RUN_MS`; when the field is absent the console falls back to six hours.
- **R9.** Both services accept the same thing: surrounding spaces trimmed, then ASCII digits
  only, a value from 1 to 4 294 967 295; anything else falls back to six hours, and the API
  logs a warning that names the variable and not the value, as the orchestrator does.
- **R10.** The assistant does not stop a run. `ingest_run` called without a signed-in person
  (the assistant's `run_ingest`) answers the 409 for an overdue run, with a sentence that
  says the earlier run has run too long and can be replaced with "Run now" on the connector's
  page; it sends no `terminateRun`. No tool description or schema changes.
- **R11.** As the finding says.

### R1. BLOCKER: A healthy run that is only waiting in the orchestrator's queue is force-cancelled by "Run now" after 15 minutes, because the route applies the start limit to `QUEUED`, a state run monitoring never ends.

- Where: `rust/crates/lakehouse-api/src/routes/connectors.rs:4180-4192` (`overdue`), `:4140-4145` (`OVERDUE_GRACE_SECONDS` doc), `:3868-3870`; the statements that say otherwise: `docker-compose.yml:539-543`, `docs/CONFIGURATION.md:243`, `rust/crates/lakehouse-api/src/config.rs:1141-1147`, `dagster/dagster.yaml:19-23`, `docs/core/features/connector-operations.md:86`.
- What happens: `dagster/dagster.yaml` sets no `run_coordinator`, and in `dagster` 1.13.20 the default is `QueuedRunCoordinator` (`dagster/_core/instance/ref.py`, the defaults dict) with `max_concurrent_runs` 10 for the whole instance (`queued_run_coordinator.py`). So every run, from the API and from the schedule sensor, is `QUEUED` until the daemon takes it, and waits there whenever ten runs of any job are going. Run monitoring reads only `STARTING`, `STARTED`, `CANCELING` and `NOT_STARTED` (`run_monitoring.py`, `execute_run_monitoring_iteration`); `start_timeout_seconds` is never applied to `QUEUED`. The route, as T8.2 says, calls `QUEUED` with a `creationTime` older than 600 + 300 s `NeverStarted`. State: connector X's scheduled run (or a person's "clear"/"refresh" of one table) has been queued for 16 minutes behind ten running loads. Someone presses "Run now" on X (the page shows no run: the record starts with the start report). Before: 409 "already queued". Now: `terminateRun(MARK_AS_CANCELED_IMMEDIATELY)` on the healthy queued run, no row in the history (D13), no count, no alert, and a new run at the back of the queue. Pressing again 15 minutes later does it again. A queued table action is replaced by a plain run without a word. Three plan statements do not hold for `QUEUED`: D10 "the five minutes let D8 act first ... the route only acts when the orchestrator's monitoring did not" (monitoring never acts on it), D13 "it is counted as a failed run and alerted by D8's path" (nothing reports it), and D8 as the answer to "a queued run never started" for a scheduled connector (the sensor's `_busy_connector_ids` counts `QUEUED`, `ingest_factory.py:1631-1636`, and nothing but a person's "Run now" ends it). `STARTING` has a smaller version of the same fault: the route measures it from `creationTime`, the orchestrator from the launch time (`monitor_starting_run`), so a run that waited 20 minutes in the queue and was taken a second ago is already `NeverStarted` for the route. The handoff says "MISMATCH: None"; this is one (plan against the pinned library), and the developer built T8.2 as written.
- Fix: planner decision first, then code. In `overdue`, take `QUEUED` out of the start-limit arm: either keep the 409 for `QUEUED`, or give it its own, explicitly decided limit (for example the run limit: queued longer than a run may run). Keep the start limit for `NOT_STARTED`. For `STARTING`, do not judge by `creationTime` (keep the 409 and leave it to the orchestrator, which fails it 600 s after launch). Correct the five comments and document lines above so they say the start limit covers a run that was created or launched and did not start, not a queued one, and reword D13 and decision 15 of the feature page for the queued case.
- Check: unit `a_queued_run_is_not_overdue_after_the_start_limit` (status `QUEUED`, `creation_time = now - start_seconds - 1` gives `None`) and `a_starting_run_is_not_judged_by_when_it_was_created`; integration in `tests/ingest_run.rs`: a `QUEUED` run created 20 minutes ago answers 409 and the mock received no `terminateRun`.

### R2. SHOULD-FIX: When the forced cancel fails, the route still writes "was stopped" into the run record, logs "stopped an overdue run", and launches; an orchestrator that could not be reached is treated like a refusal.

- Where: `rust/crates/lakehouse-api/src/routes/connectors.rs:3893-3923` (`stop_overdue_run`).
- What happens: (a) `cancel_run_now` returns `Err` (time-out, connection reset) for a `STARTED` run 7 hours old whose worker is alive (monitoring is down, or the run predates the tag). The route warns, then closes the record as `failed` / `timed_out` with the line "run ended: failed, the run did not finish in 6 h and was stopped", logs "stopped an overdue run so the connector can run again", and goes on to launch. If the launch call gets through, a second run loads on top of a live one (Bronze is append-only, the reason the guard exists). If the old run then finishes, its own end report is refused (`end_run` updates only `status = 'running'`) and the history keeps "failed, was stopped" for a run that loaded its rows (principle 2). (b) The opposite order: the cancel succeeds and `end_timed_out_run` fails once; the request is a 5xx, the next request finds the run `CANCELED` (not active), so nothing ever closes the record and it stays `running`. T8.5 asked for the unconditional close, so this is the plan's text built literally; the transport case is not "the connector locked for good", it is retryable.
- Fix: on `Err` from `cancel_run_now` return `orchestrator_unreachable(&err)` (503, nothing written). On a typed refusal keep D10 (launch goes on) but do not claim a stop: skip the `info!` line, and either leave the record to the sensor or close it with a line that does not say "was stopped". Close the record before asking for the cancel, so a store error cannot leave a cancelled run `running`.
- Check: integration `an_orchestrator_that_cannot_be_reached_for_the_cancel_answers_503_and_starts_nothing` (the `terminateRun` mock answers 500: status 503, no `launchRun` received, the old record is still `running`).

### R3. SHOULD-FIX: A request that is then refused has already cancelled the overdue run and closed its record.

- Where: `rust/crates/lakehouse-api/src/routes/connectors.rs:3965-3986` (the guard runs before the two refusals of a run of one table), `:4003-4040`.
- What happens: `POST /api/connectors/{id}/ingest/run` with `{"object": "not_a_table", "action": "clear"}` on a connector with an overdue run: `guard_one_run_at_a_time` cancels the run and writes `timed_out`, then the request answers 404 "This connector does not load that table." (or 409 for a table that waits for a schema decision). The same when the launch itself is refused (422, 503 job not loaded). Before this branch the guard had no side effect, so its place did not matter.
- Fix: split the guard: decide first (409 or a list of overdue runs), run the request's own refusals, and call `stop_overdue_run` only immediately before `launch_run_with_config_and_tags`.
- Check: integration: an overdue `STARTED` run plus a body naming a table the connector does not load answers 404 and the mock received no `terminateRun`; the record is still `running`.

### R4. SHOULD-FIX: No test fails if the five-minute grace of D10 is removed.

- Where: `rust/crates/lakehouse-api/src/routes/connectors.rs:4255-4322` (the unit tests build their times from `limits.running_seconds` / `limits.start_seconds`, which already hold the grace), `rust/crates/lakehouse-api/tests/ingest_run.rs:637`, `:705` (7 hours, 20 minutes: far past either bound).
- What happens: set `OVERDUE_GRACE_SECONDS` to `0.0`, or drop the `+ OVERDUE_GRACE_SECONDS` in `RunAgeLimits::from_config`: every test still passes. The decision "each plus a fixed five minutes, so the orchestrator's monitoring acts first" is not pinned, and without it the route races the daemon at exactly the limit.
- Fix: add a unit test on absolute numbers from the default config.
- Check: `a_run_inside_the_grace_after_its_limit_still_holds_the_connector`: with the default config, a `STARTED` run with `start_time = now - 21_600 - 299` is `None` and `now - 21_600 - 301` is `RanTooLong`; a `NOT_STARTED` run at `now - 600 - 299` is `None` and at `now - 600 - 301` is `NeverStarted`.

### R5. SHOULD-FIX: No test fails if the run-list query stops asking for `creationTime`, which would silently switch the never-started path off.

- Where: `rust/crates/lakehouse-dagster/src/lib.rs:4036-4057` (the mock answers any POST), `rust/crates/lakehouse-api/tests/ingest_run.rs:50-58` (`mount_recent_runs` matches `runsOrError` only).
- What happens: remove `creationTime` from the query at `lakehouse-dagster/src/lib.rs:1223`: the wiremock fixtures still return the field, so all tests pass, while the real orchestrator returns none, `creation_time` is `None`, and `overdue` answers `None` for every queued or unstarted run (fail closed, so the QA case comes back unnoticed).
- Fix: add `.and(body_string_contains("creationTime"))` to the mock in `list_runs_for_job_with_config_returns_when_each_run_was_created` (as the two `terminateRun` tests do for their policy word).
- Check: that test fails when the field is taken out of the query string.

### R6. SHOULD-FIX: The "warning never echoes the value" test passes for `"0"` only because the warning sits on line 278.

- Where: `dagster/dispar_orchestrate/test_ingest_run_limit.py:57`.
- What happens: `caplog.text` is formatted with pytest's default `%(levelname)-8s %(name)s:%(filename)s:%(lineno)d %(message)s`, so it holds `ingest_factory.py:278`. The assertion `"0" not in caplog.text.replace("21600", "")` holds today because 278 has no zero. Any edit that moves `logger.warning(` in `RunLimitSettings.from_env` to a line with a `0` in its number (280, 290, 300 to 309, ...) fails the `bad="0"` case with no change of behaviour.
- Fix: assert on the records, not on the formatted text: for each record, `bad.strip()` is not in `record.getMessage().replace(str(SIX_HOURS), "")` and is not among `record.args`.
- Check: the test still passes after ten blank lines are inserted above `RunLimitSettings` (not to be committed).

### R7. SHOULD-FIX: A "Signed" cell dated 2026-11-12 cites `DEC-16` for wording the owner's decision does not contain.

- Where: `docs/core/features/load-failure-alerts.md:69` (Decisions row 10; the table's last column is "Signed"), against `docs/core/BACKLOG.md:711` (`DEC-16`) and `:161` (`SRC-42`).
- What happens: `DEC-16` reads "shown only to people with `alert:write`; `alert:read` shows the rule without them". Row 10 says "Only a person with `alert:write`, and the owner of a chart alert for their own rule", and carries the signed date. The chart-owner exception is plan decision D16, not the owner's. The developer copied T13 as written, so the correction is the planner's. (The section's opening line also still says "All nine defaults signed ... on 2026-10-09" above a table of ten.) Row 15 of `connector-operations.md` leaves its Signed cell empty, as it should.
- Fix: either cut row 10's Default to the words of `DEC-16` and move the chart-owner exception to its own unsigned row, or leave the Signed cell empty until the owner has seen the sentence.
- Check: none (document); the reviewer reads the row against `DEC-16`.

### R8. SHOULD-FIX: The run limit is a setting, but the console's six hours are fixed, so raising the limit brings the QA symptom back.

- Where: `src/features/connectors/connector-ingest-panel.tsx:1275` (`STALE_RUN_MS`), `docs/core/features/connector-operations.md:98` ("unless `INGEST_RUN_MAX_RUNTIME_SECONDS` is raised").
- What happens: an operator sets `INGEST_RUN_MAX_RUNTIME_SECONDS=43200` for a long first load. At hour 7 the console calls the run "no end reported" and enables "Run now" (its bound is six hours); the API answers 409 until hour 12 and five minutes, which is the case D9 itself names as the QA finding (`TC-1702`). T11 says `STALE_RUN_MS` is not changed, so this is a gap of the plan, not of the build.
- Fix: planner decision: return the limit to the console (for example beside the run list) and use it in place of the constant, or say in the feature page's limit that the page stops showing a run as going after six hours whatever the setting.
- Check: `connector-ingest-panel.test.tsx`: with a limit of 12 hours a run that started 7 hours ago is still shown as going and "Run now" is disabled.

### R9. SHOULD-FIX: The two services do not read `INGEST_RUN_MAX_RUNTIME_SECONDS` by the same rule, and the API falls back without a word.

- Where: `dagster/dispar_orchestrate/ingest_factory.py:210-214`, `:268-284` (`_env` strips, then `int()`); `rust/crates/lakehouse-api/src/config.rs:1492-1498` (`parse_positive_u32_or_default`: no trim, `i64` then `u32`).
- What happens: `"86_400"`, a value quoted with a space, or any number above 4 294 967 295 is accepted by Python (`int("86_400") == 86400`) and rejected by Rust. The orchestrator then lets a run go for a day, the API believes six hours, and "Run now" force-cancels a healthy run at 6 h 5 min (R8 makes the button available). Python warns about a value it cannot read; the API logs nothing.
- Fix: make `RunLimitSettings.from_env` as strict as the API (ASCII digits only after the strip, at most `u32::MAX`), and trim in the API before parsing; or name the accepted form in both comments and in `docs/CONFIGURATION.md`.
- Check: add `"86_400"` and `"4294967296"` to the parametrised cases of `test_a_run_limit_that_cannot_be_read_falls_back_to_six_hours`, and `" 900 "` (taken as 900) to `the_run_limits_default_to_six_hours_and_ten_minutes_and_ignore_garbage`.

### R10. SHOULD-FIX: The assistant's `run_ingest` tool can now cancel a run without approval, and the plan does not name it.

- Where: `rust/crates/lakehouse-api/src/routes/ai/tools/ingest.rs:101-108` (calls `connectors::ingest_run`, so `guard_one_run_at_a_time`), `rust/crates/lakehouse-api/src/routes/ai/registry.rs:2593-2596` (`run_ingest`: `Risk::WriteLow`), `:2448-2451` (`cancel_pipeline_run`: `Risk::WriteHigh`, "Needs human approval").
- What happens: "run the orders connector" through the assistant, with an overdue run listed, sends `terminateRun(MARK_AS_CANCELED_IMMEDIATELY)`. Stopping a run is an approval-gated action everywhere else in the assistant; here it happens inside a low-risk tool, and the tool's description says nothing of it. D20 speaks of the alert tools only.
- Fix: planner decision: accept it and say so in the plan and in `run_ingest`'s description (then `tests/fixtures/tool_schemas.json` is regenerated), or have `ingest_run` (the entry with no principal) answer the 409 for an overdue run and leave the stop to the person's route.
- Check: a tool test in `routes/ai/tools/ingest.rs` or `tests/ingest_run.rs` for whichever is decided (either "the mock received no `terminateRun`", or the description names the stop).

### R11. SHOULD-FIX: `timed_out` can now be written through the orchestrator's end report too, which D11 rules out.

- Where: `rust/crates/lakehouse-store/src/connector_run.rs:112-125` (`EndReason` derives `Deserialize`), `rust/crates/lakehouse-api/src/routes/connector_runs.rs:646-690` (`EndBody.reason: EndReason`, `report_end`).
- What happens: D11: "written in two places and nowhere else". `POST /api/connectors/{id}/run-reports/{runId}/end` with `{"status": "succeeded", "reason": "timed_out"}` is now accepted and stored (with the generic phrase, no limit in the line). The orchestrator's own list (`dagster/dispar_orchestrate/run_reports.py:51`, `END_REASONS`) does not know the word, so nothing sends it today.
- Fix: refuse `EndReason::TimedOut` in `report_end` with a 400 (the route's own vocabulary stays the four words), with a comment citing D11.
- Check: a test beside `connector_runs.rs:1093`: the end report with `reason: "timed_out"` answers 400 and the record stays `running`.


### Round 1 fixes (developer)

Base of this round: `67581e96` (the review). One commit per finding, R1 to R11 in order, each pushed
to `fix/qa-round-connectors-p1`. Every new or corrected test was run against the code **before** its
fix (the fix stashed, or the temporary wrong change the finding's Check names), seen to fail, and then
run green again; the failure text is quoted. Fix sites carry `review round 1, R<n>` (rule 13).

**R1** `138abf94` — `overdue` no longer judges `QUEUED` by the start limit (a queued run is overdue
only past the run limit plus the grace), `STARTING` is never overdue here and is left to the
monitoring, `NOT_STARTED` keeps the start limit by `creationTime`; the guard doc, the
`OVERDUE_GRACE_SECONDS` doc, `docker-compose.yml`, `docs/CONFIGURATION.md`, `config.rs` and
`dagster/dagster.yaml` now say so; D13 and decision 15 of `connector-operations.md` are reworded for
the queued case.
Proof: `a_queued_run_is_not_overdue_after_the_start_limit` and
`a_starting_run_is_not_judged_by_when_it_was_created` failed before the fix with
`assertion left: Some(NeverStarted), right: None`; integration
`a_queued_run_still_holds_the_connector_after_the_start_limit` failed with `left: 200, right: 409`
(`{"runId":"never"}` — the route cancelled and launched).

**R2** `071c0d0a` — the cancel is asked for first: an unreachable orchestrator is the retryable 503
with nothing written and nothing launched, and a typed refusal lets the launch go on without claiming
a stop (no `info!` line, no `timed_out` record).
Proof: `an_orchestrator_that_cannot_be_reached_for_the_cancel_answers_503_and_starts_nothing` failed
before the fix with `left: 200, right: 503` (`{"runId":"never"}`).
**MISMATCH: R2**, one clause. The ruling's "the record is closed before the cancel is asked for"
cannot hold together with its own "an orchestrator that cannot be reached for the cancel answers 503
and writes nothing" and this finding's Check "the old record is still `running`": closing the record
first writes on the 503 path. I implemented the half the Check pins (cancel, then close) and reported
the conflict rather than improvising. Consequence, as the finding's (b) says: if `end_timed_out_run`
fails after a cancel was accepted, the request is a 5xx and the record keeps `running` (the next
request sees the run as `CANCELED`, not active, so nothing closes it). A fix needs one of the two
requirements dropped; that is the planner's call.

**R3** `b5f8493e` — the guard is a decision only (`runs_holding_the_lock`: the 409 for a run still
going, or the overdue runs), the request's own refusals follow it with no side effect, and
`stop_overdue_run` is called immediately before the launch. The 409 for a run still going stays before
the body is judged, as `the_409s_for_a_run_going_and_a_paused_connector_are_answered_before_the_body_is_judged`
requires.
Proof: `a_table_the_connector_does_not_load_is_a_404_and_stops_no_run` failed before the fix with the
`terminateRun` body in the received requests (`left: ["{…terminateRun…runId:\"run-typo\"}"], right: []`).

**R4** `91615ca6` — a unit test on absolute numbers from the default config.
Proof: `a_run_inside_the_grace_after_its_limit_still_holds_the_connector` failed with
`OVERDUE_GRACE_SECONDS` set to `0.0` (`assertion left: 21600.0, right: 21900.0`).

**R5** `1dd3fe8e` — the run-list mock matches the request body on `creationTime`.
Proof: `list_runs_for_job_with_config_returns_when_each_run_was_created` failed with `creationTime`
taken out of the query (`called Result::unwrap() on an Err value: Status(404)`).

**R6** `5b4a08f2` — the "value never echoed" assertion reads each record's `getMessage()` and `args`.
Proof: with `logger.warning(` moved from line 278 to line 280 (two blank lines, not committed) and
the old assertion put back, `test_a_run_limit_that_cannot_be_read_falls_back_to_six_hours[0]` failed
with `AssertionError: assert '0' not in 'WARNING  di...e default \n'`; the new assertion passes at
line 280.

**R7** `b951624f` — row 10 of `load-failure-alerts.md` holds only `DEC-16`'s words with its date; the
chart-owner exception (D16) is row 11 with `*(proposed, not yet signed)*`; the opening line says ten
signed rows and eleven in all.
Proof: none (document); the reviewer reads row 10 against `DEC-16`.
Note: R1's ruling asks for `*(proposed, not yet signed)*` in row 15 of `connector-operations.md`'s
Signed cell, which R7 calls empty "as it should". R1's explicit instruction was followed (no date was
written); if the planner prefers the cell empty, it is a one-line revert.

**R8** `94e0985d` — `GET /api/connectors/{id}/runs` answers `maxRuntimeSeconds` from
`ingest_run_max_runtime_seconds` (camelCase, documented on `RunsResponse` and on the console's
`ConnectorRunList`); the Runs card, the last-run tile and the OAuth sign-in check use it and fall back
to six hours when it is absent. No route added, `POLICY_TABLE` unchanged.
Proof: `keeps a run going for the limit the API sends, not for six hours` (12-hour limit, run started
7 hours ago) failed before the fix (the run showed "No end reported" and "Run now" was enabled); the
Rust list test also asserts `maxRuntimeSeconds` is `21600`.

**R9** `656ec725` — both services trim, take ASCII digits only and a value from 1 to 4294967295, and
both warn, naming the variable and never the value; the API reads both run limits through
`parse_run_limit`.
Proof: the Python fallback cases gained `"86_400"`, `"4294967296"`, `"+900"` and the setting case
`" 900 "`; they failed before the fix with `assert 86400 == 21600`, `assert 4294967296 == 21600`,
`assert 900 == 21600`. The Rust config test gained the same rejected values and `" 900 "` as 900; it
failed before the fix with `"+900"` `left: 900, right: 21600` (the shared helper accepts a leading
`+`).

**R10** `4e2652fe` — `runs_holding_the_lock` takes `may_stop`; `launch_ingest` passes
`principal.is_some()`, so the assistant's `run_ingest` answers the 409 for an overdue run with a
sentence naming "Run now" on the connector's page and sends no `terminateRun`. No tool description or
schema change; `tests/fixtures/tool_schemas.json` untouched.
Proof: `the_assistants_run_ingest_answers_409_for_an_overdue_run_and_stops_nothing` failed before the
fix with `{"tool":"run_ingest","outcome":"executed","result":{"runId":"never"}}`.

**R11** `7c0fc58b` — `report_end` refuses `EndReason::TimedOut` with a 400, citing D11.
Proof: `the_end_report_cannot_write_the_timed_out_reason` failed before the fix with
`assertion left: 200, right: 400` (`{"recorded":true}`).

**Follow-up** `1f85c157` — R8's client change left `ConnectorRun` unused in
`src/services/clients/connectors.ts`, the one lint warning this round added (12 to 11 warnings once
removed); `bun run typecheck`, `bun run lint` and `bun test src/features/connectors/` (451 passed) ran
on it, and the block below was re-run on it.

#### Commands run on the final commit (`1f85c157`), with their counts

```
cd rust && cargo fmt --check
  clean (no output)
cargo clippy -p lakehouse-api -p lakehouse-store -p lakehouse-dagster --all-targets -- -D warnings
  Finished `dev` profile [unoptimized + debuginfo] target(s) in 8.97s — no warning
cargo test -p lakehouse-api --test ingest_run
  test result: ok. 29 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.47s
(cd dagster && python -m pytest dispar_orchestrate/test_ingest_run_limit.py -q)
  11 passed, 218 warnings in 0.64s
(cd dagster && python -m pytest dispar_orchestrate/test_ingest_errors.py dispar_orchestrate/test_ingest_run_limit.py -q)
  84 passed, 218 warnings in 0.92s
(cd dagster && python -m pytest dispar_orchestrate/test_ingest_factory.py dispar_orchestrate/test_ingest_retries.py
 dispar_orchestrate/test_ingest_run_reports.py dispar_orchestrate/test_ingest_single_table.py
 dispar_orchestrate/test_run_reports.py -q)
  171 passed, 218 warnings in 2.69s
bun run typecheck
  clean (no output)
bun run lint
  ✖ 11 problems (0 errors, 11 warnings) — all pre-existing (none in a file this round changed)
bun run test
  4001 pass, 1 skip, 0 fail, 11993 expect() calls, 4002 tests across 380 files [82.11s]
docker compose --profile '*' config --quiet
  ok
python3 ops/lint/check_bare_iceberg_count.py
  R11 guard: no bare count() against a Bronze Iceberg table found.
python3 ops/lint/check_intra_package_imports.py
  OK: all intra-package dispar_orchestrate imports resolve
```

Per commit, only the touched language's scoped checks ran: `cargo fmt --check`,
`cargo clippy -p lakehouse-api --all-targets -- -D warnings` (or `-p lakehouse-dagster`) and the
touched tests (`cargo test -p lakehouse-api --lib <name>`, `cargo test -p lakehouse-api --test
ingest_run`, `cargo test -p lakehouse-dagster --lib <name>`); for Python the touched test file; for
TypeScript `bun run typecheck`, `bun run lint` and `bun test <the touched files>`.

#### Not verified, and why

- `cargo test --workspace` was not run: the developer's brief says CI runs the full suite, and the
  block above is the one required for handoff. The crates whose code changed (api, dagster) ran their
  own tests; `lakehouse-store` was compiled by clippy but its tests were not re-run (no store change).
- `docker compose up` was not run for this round's compose edit: it is a comment only (no key, value
  or service changed), so `config --quiet` is the whole proof. The `up` for T10's settings belongs to
  the `g6-ops` gate, as the handoff of the build round says.
- Nothing else was skipped.
