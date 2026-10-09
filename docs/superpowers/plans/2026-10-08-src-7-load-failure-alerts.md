# `SRC-7` Alerts when a load fails — Implementation Plan

> **Migration numbers, 2026-10-10.** `main` took `0061` after this plan was
> written. This branch's migrations were renumbered in `36f2a2a`, before any
> was applied: `0061`→`0062` (`SRC-6`), `0062`→`0063` (`SRC-7`),
> `0063`→`0064` and `0064`→`0065` (`SRC-8`). Numbers below are the old ones
> where they record what was done at the time.

**Status:** ready to build. Decisions D1–D9 on the feature page were signed
by the product owner on 2026-10-09, all as proposed.
Written 2026-10-08 by the planner (Claude Opus) for a developer agent, under
the role split in `AGENTS.md`.

**Spec:** `docs/core/specs/src-7.md`. Backlog `SRC-7`.
**Feature page:** `docs/core/features/load-failure-alerts.md`. Decision
numbers below (D1–D9) are its rows. The page maps each row of the spec's
Target table to what is built; section 2a below maps each to its tasks.

**Base and branch:** `feat/phase-1-connector-reliability`, on top of `SRC-6`
(`6ab12f5`). The product owner asked for Phase 1 on one branch, not pushed
until the phase is done. Consequence, stated once: no Rust test of `SRC-6`
has run yet; this work is built on code whose tests CI has not seen.

**Why.** A failed connector run or upload tells nobody, and connector health
changes only on a manual test.

---

## 1. What exists today (anchors verified at `6ab12f5`)

| # | Spec row | Code | Verdict |
| --- | --- | --- | --- |
| F1 | Failure alert: none | `dagster/dispar_orchestrate/pipeline_events.py:181` and `:196`: the failure and success sensors watch every job (`monitored_jobs=None`) and post `{runId, jobName}`. `routes/pipelines.rs:1744` (`run_failed_event`) maps `jobName` only through `job_name_to_pipeline_id` (`:1835`), which knows authored pipelines. `ingest_job` and `file_ingest_job` answer `{"matched": 0}`. | Confirmed. The trigger and the dedupe exist; the mapping does not. |
| F2 | Channels: n/a | `lakehouse-alerts/src/lib.rs:188` `AlertChannel` is webhook or email; `deliver_unless_silenced` (`:1659`) sends through `lakehouse-notify` and honours silences. | Exists. Reuse it. |
| F3 | Events: none | `AlertKind` (`lib.rs:128`) has four pipeline kinds scoped by a `pipeline` column that must be a `pl-…` id or `*` (`validate_pipeline_scoped`, `:595`). No connector or upload kind. | Confirmed. |
| F4 | Connector health changes only on a manual test | `lakehouse-store/src/connectors.rs:1261` `record_test_result` is the only writer of `connector.health`. No column holds a last run time or a streak. | Confirmed. |
| F5 | *Not in the spec* | A connector run does not carry its connector in the job name: every connector runs `ingest_job`, and only the run config (`ops.run_ingest.config.connector_id`) says whose it is (`routes/connectors.rs`, `connector_runs`). `DgClient` (`lakehouse-dagster/src/lib.rs`) can read a run's config only by listing a job's recent runs (`list_runs_for_job_with_config`, `:909`). | Needs a by-id read. |
| F6 | *Not in the spec* | An upload's failure is written only when somebody reads it: `routes/uploads.rs:711` `Settler::settle` asks the orchestrator when the list or the page is opened. `file_upload.run_id` ties an upload to its run. | An unopened failed upload stays "ingesting". The alert path must settle it. |
| F7 | *Not in the spec* | Alert rules are one list for the installation (`console.alert_rule` in ClickHouse, no tenant column); `routes/alerts.rs:76` `create` and `:90` `update` take no principal. | A rule for "all connectors" would carry every tenant's connector names. D7. |
| F8 | *Not in the spec* | The console can create `alert`, `digest` and `freshness` rules only (`src/features/alerts/alerts-page.tsx:101`). The four pipeline kinds have no form. | The new kinds need a form; the pipeline kinds stay as they are. |
| F9 | *Not in the spec* | `PIPELINE_RUN_TOKEN` is `${X:-}` in compose (`docker-compose.yml:258`, `:1877`); unset, the sensors post nothing. | D9. |

## 2. Decisions already made by the planner

- **Six new rule kinds**, one per event the spec lists plus the upload
  half of "Failure alert": `connector_failure`,
  `connector_repeated_failure`, `connector_disabled`,
  `connector_schema_change`, `connector_success`, `upload_failure`. Scope
  lives in a new additive column `console.alert_rule.connector` (a connector
  id or `*`), added the way `pipeline` was (`lib.rs:702`). `upload_failure`
  always stores `*`. `connector_success` must name one connector, never `*`
  (D4: optional per connector).
- **Two kinds have no trigger in this work (D5).** `connector_disabled` and
  `connector_schema_change` are saved, listed and delivered by one public
  entry point, `evaluate_connector_event`, which `SRC-11` and `SRC-8` call.
  Nothing in `SRC-7` raises them, and the form says so beside each.
- **One event route pair, three job families.** `run_failed_event` and
  `run_finished_event` dispatch on `jobName`: `ingest_job` → connector,
  `file_ingest_job` → upload, anything else → the existing pipeline lookup,
  unchanged.
- **Dedupe reuses `pipeline_run_event`** (`0049`), with `pipeline_id` set to
  `connector:<id>` or `upload:<id>`. No new table.
- **Run facts live on the connector row**: `last_run_success_at`,
  `last_run_failure_at`, `failure_streak`. One store function writes them
  and `health` together (D6) and returns the new streak.
- **The repeated-failure rule fires when the streak becomes exactly 3**, so
  it fires once per episode.
- **A fired failure is also written to `alert_instance`**
  (`overview::insert_from_fired_rule`) so the Alerts page shows it.
- **Message text is built here, never taken from the orchestrator**: name,
  id, run id, a relative console link (principle 4).

## 2a. Spec row to tasks

| Spec Target row | Tasks |
| --- | --- |
| Failure alert: every failed connector run and upload load, within 5 minutes | 2, 4, 5; measured by 10 and written up in 11 |
| Events: failed run, repeated failures, connector disabled, schema change, optional success per connector | 3, 4, 8 |
| Channels: email and webhook, reusing what exists | 3 (no new sender) |
| Connector health: last success, last failure, failure streak, from every real run | 1, 4, 9 |

Spec acceptance lines: "break a password" → tasks 4 and 10; "list shows the
failure" → 1 and 9; "success alerts off unless switched on" → 3 and 4; "a
user without the permission is refused, a failure shows an honest message" →
6 and the existing `alert:write` gate. Tasks 6 and 7 are beyond the spec
(D7, D9).

## 3. Tasks

One task per commit, in order. Rust tasks 1–7 sit together; the migration
lands with task 1. Cite `SRC-7` and the finding at each site.

1. **Store: run facts on the connector.** Migration, next free number (0062
   at `6ab12f5`), why-header: three columns on `connector`
   (`last_run_success_at TIMESTAMPTZ`, `last_run_failure_at TIMESTAMPTZ`,
   `failure_streak INTEGER NOT NULL DEFAULT 0 CHECK (failure_streak >= 0)`).
   `connectors::record_run_result(pool, id, succeeded, at) ->
   Result<RunHealth, StoreError>` in one `UPDATE … RETURNING`: success sets
   healthy and streak 0; failure adds 1 and sets degraded below 3, unhealthy
   from 3 (constant `REPEATED_FAILURE_STREAK = 3`). `Connector` and its
   TypeScript contract gain `lastRunSuccessAt`, `lastRunFailureAt`,
   `failureStreak`.
   *Check:* store tests: three failures give 1 degraded, 2 degraded, 3
   unhealthy; a success resets; an unknown id is `NotFound`;
   `record_test_result` still behaves as before.
2. **Orchestrator client: a run's config by id.** `DgClient::run_config(run_id)
   -> Result<Option<Value>, DgError>`. *Check:* unit test against the mocked
   GraphQL the neighbouring tests use; an unknown run is `None`.
3. **Alerts crate: the kinds and their evaluators.** `AlertKind` variants,
   `as_str`, `normalize_input`, a `validate_connector_scoped` beside
   `validate_pipeline_scoped`, the `connector` column in `ensure`, `COLS`,
   `row_to_rule` and `save_rule`. `evaluate_connector_event(ch, http, email,
   kind, connector_id, text, title, silence)` and
   `evaluate_upload_failure(…)`, built on the same filter-and-deliver shape as
   `deliver_one_kind` (`:1448`); reuse it if its `pipeline` filter can take
   the scope column as a parameter rather than writing a second copy (rule 4).
   *Check:* tests like `evaluate_pipeline_failure_delivers_a_message_naming_pipeline_and_run`
   (`:2530`): a scoped rule matches its connector only; `*` matches all; a
   disabled or silenced rule does not deliver; `ensure` sends the additive
   column statement; a `connector_success` rule with `*` is refused with a
   fixed message; `evaluate_connector_event` delivers a
   `connector_disabled` and a `connector_schema_change` rule when called
   (the callers come with `SRC-11` and `SRC-8`).
4. **API: failed and finished runs of `ingest_job`.** In `run_failed_event`:
   read the connector id from the run's config (task 2), confirm the run's
   status as the handler already does, dedupe, call `record_run_result`,
   deliver `connector_failure`, deliver `connector_repeated_failure` when the
   returned streak is exactly 3, persist the instance. In
   `run_finished_event`: same for success, delivering `connector_success`. A
   run whose config names no connector, or an unknown connector, answers
   `{"matched": 0}` with a reason, as an unknown job does today.
   *Check:* route tests beside `run_failed_event_route`: failure delivers
   and stamps the row; a sensor retry does not deliver twice or count twice;
   the third failure delivers the repeated rule once and the fourth does not;
   a non-service, non-admin caller is refused; no response or message holds
   orchestrator error text.
5. **API: failed runs of `file_ingest_job`.** Store
   `uploads::get_by_run_id`. The handler settles the upload as failed with
   the existing fixed reason, then delivers `upload_failure` and persists the
   instance. Reuse `Settler`'s recording path; do not write a second one.
   *Check:* route test: an upload still `ingesting` whose run failed becomes
   `failed` and one alert is delivered, with nobody having read it.
6. **API: who may save a rule (D7, F7).** `create` and `update` take the
   principal. For the six new kinds: `*` needs
   `routes::catalog::is_unrestricted`; a connector id must pass
   `require_connector_in_tenants`. Other kinds are unchanged. Fixed 403
   messages.
   *Check:* route tests for each refusal and each allowed case; `tests/route_auth.rs`
   still asserts the routes both ways.
7. **API: say when runs are not being reported (D9).** `GET /api/alerts`
   gains `runEventsConfigured: bool`, true when `PIPELINE_RUN_TOKEN` is set.
   *Check:* route test both ways.
8. **Console: rule form.** `alerts-page.tsx`: the six kinds in the type
   select, a line under "connector disabled" and "schema change" saying
   which later work raises them (D5), no "All connectors" choice for the
   success kind (D4), a connector select (the user's connectors, plus "All connectors"
   for an administrator) and no mart, measure or board for them;
   `rules-columns.tsx` shows the scope; a notice when
   `runEventsConfigured` is false. Contract `AlertRuleKind` and `AlertRule`.
   *Check:* tests for `alertRuleFormFields`, the notice, and a saved rule's
   request body.
9. **Console: run health on Sources.** `connectors-columns.tsx`: a "Last
   run" column (last success or failure, whichever is later) and the streak
   beside the health badge when above 0; the connector page's overview shows
   the three values. *Check:* component tests.
10. **Gate.** `ops/g6/g6_ingest_matrix_test.py`: a connector with a wrong
    password is run; within 5 minutes of the run ending
    `GET /api/connectors/{id}` shows `failureStreak: 1`, `health: "degraded"`
    and a last failure time, and `GET /api/overview/alerts` lists the
    failure. The step prints the measured seconds. Needs
    `PIPELINE_RUN_TOKEN` in the gate's environment: check the g6 override
    and CI `.env` first; if it is not set there, stop and report (no compose
    edit without the planner).
    *Check:* the g6 job in CI.
11. **Docs (planner).** `docs/plans/SRC-7-RESULT.md` with the seconds the
    gate measured (the "5 minutes" claim cites it, principle 5),
    `CHANGELOG.md`, `docs/OPERATIONS.md`, the spec's "Today" rows.

## 4. Assistant

`routes/ai/tools/connectors` returns `Connector`; its three new fields may
change `tests/fixtures/tool_schemas.json`. If the snapshot changes, the
developer reports it and the AI team reviews it. No tool is added.

## 5. Build limits on this machine

As `SRC-6`: shared target dir
`/home/hv/.cache/lakehouse-src6-target`, `CARGO_BUILD_JOBS=2`, `df -h /`
before each cargo command, no `cargo test`, no `cargo clean`, no docker.
Every Rust test and the gate step are *not verified* until CI.

## 6. Out of scope

- Detecting a schema change (`SRC-8`) and disabling a connector (`SRC-11`):
  only their rule kinds and delivery are here. Slack and Teams apps (spec).
  CDC connectors; per-tenant rule lists; `SEC-10`.
- Retries and the 30-failure auto-disable (`SRC-11`).
- Any change to the pipeline alert kinds.

## Handoff

*(developer)*

### Tasks 1–7 (Rust, plus the TypeScript contract fields of task 1)

Commits on `feat/phase-1-connector-reliability`, base `6ab12f5`:

| Task | Commit |
| --- | --- |
| 1 store run facts, migration `0062_connector_run_facts.sql`, TS contract | `a92997a` |
| 2 `DgClient::run_config` | `8e3ac4e` |
| 3 six alert kinds, `connector` column, `evaluate_connector_event` | `4349cdc` |
| 4 `ingest_job` failed/finished events | `4482421` |
| 5 `file_ingest_job` failed event, upload settled | `a8c83c8` |
| 6 who may save a rule (D7) | `8f84a2f` |
| 7 `runEventsConfigured` | `4eb1928` |

Commands run (all foreground, `df -h /` showed 59 GB free before each cargo
command; `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-src6-target
CARGO_BUILD_JOBS=2`):

- Per commit: `cargo fmt --check` (clean) and `cargo clippy -p <crates>
  --all-targets -- -D warnings` for the crates touched: store (1), dagster (2),
  alerts (3), alerts+api (3), api (4, run with the task-5 files reverted so
  the commit compiles alone), store+api (5), api (6, 7). Every one finished
  with no warnings.
- Final: `cargo fmt --check` clean; `cargo clippy --workspace --all-targets
  --all-features -- -D warnings` finished, no warnings.
- `bun run typecheck`: clean. `bun run lint`: 0 errors, 6 warnings (none in
  files touched). `bun run test`: 891 pass, 1 skip, 0 fail (892 tests, 100
  files).
- Not run: `cargo test`, `cargo build`, docker, Python checks (no Python
  change).

Existing assertions changed:

- `routes/ai/tools/alerts.rs`, test `update_delete_and_run_require_id`: the
  call is `update_alert_rule(&state(), None, &Map::new())` because the tool now
  takes the state and the caller's principal (task 6). Expected value
  unchanged (`{"error": "id is required"}`).
- Five TypeScript test fixtures gain the three new `Connector` fields
  (`connector-create-page`, `-detail-page`, `-edit-page`, `-overview`,
  `connectors-page` tests). No assertion changed.

*Not verified* (no Rust test has been run; CI is the first run): every Rust
test below.

- store `tests/connectors.rs`:
  `record_run_result_counts_failures_and_a_success_resets_them`,
  `record_run_result_refuses_an_unknown_id_and_a_manual_test_leaves_run_facts`;
  unit test `connector_serializes_without_host_or_secret_ref` (extended with
  the three keys). `tests/uploads.rs`:
  `get_by_run_id_finds_the_upload_of_a_run_or_none`. Migration `0062` applies
  only under `sqlx::test`.
- dagster: `run_config_returns_the_config_of_a_run_and_none_for_an_unknown_run`.
- alerts: `src7_kinds_have_snake_case_strings_in_as_str_and_on_the_wire`,
  `normalize_input_accepts_connector_kinds_with_an_id_or_star`,
  `normalize_input_refuses_a_connector_rule_without_a_valid_connector`,
  `normalize_input_refuses_a_connector_success_rule_for_star_with_a_fixed_message`,
  `normalize_input_stores_star_for_an_upload_failure_rule_whatever_was_sent`,
  `row_to_rule_maps_the_connector_column`,
  `ensure_sends_an_additive_connector_column_statement`,
  `save_rule_sends_the_connector_scope_as_a_quoted_literal`,
  `evaluate_connector_event_matches_its_connector_star_and_kind_only`,
  `..._delivers_the_message_text_unchanged`, `..._does_not_deliver_a_silenced_rule`,
  `..._delivers_connector_disabled_and_schema_change_rules`,
  `..._delivers_an_upload_failure_rule_for_any_upload`,
  `..._ignores_a_non_connector_kind`.
- api `routes/load_alerts.rs`: `a_failed_connector_run_delivers_stamps_the_row_and_persists_an_instance`,
  `a_sensor_retry_neither_delivers_nor_counts_a_second_time`,
  `the_third_failure_delivers_the_repeated_rule_once_and_the_fourth_does_not`,
  `a_success_resets_the_streak_and_alerts_only_a_rule_for_that_connector`,
  `a_success_with_no_success_rule_sends_nothing`,
  `a_user_who_is_not_the_orchestrator_is_refused`,
  `a_run_that_is_not_failed_is_refused_and_leaves_the_row_alone`,
  `a_run_with_no_known_connector_answers_matched_zero_with_a_reason`,
  `orchestrator_error_text_never_reaches_the_response`,
  `a_failed_upload_run_settles_the_upload_and_delivers_one_alert`,
  `a_failed_run_of_no_upload_answers_matched_zero`.
- api `routes/alerts.rs` (`rule_scope_authorisation`, `run_events_configured`):
  five scope tests (own-tenant connector allowed; `*`/uploads refused for a
  non-admin; other tenant's connector refused like an unknown one; admin
  allowed; fail closed with no principal, other kinds unchanged; update cannot
  move a rule) and `the_flag_is_true_with_a_token_and_false_without_or_with_an_empty_one`.
- Route tests that rely on the seeded connectors `conn-pg-lakehouse` and
  `conn-s3-warehouse` and tenants `…0001`/`…0002` existing after all
  migrations (the store's own tests use the first).

Plan/code mismatches and decisions to review:

1. **`AlertKind` serde.** `rename_all = "lowercase"` makes the existing
   `PipelineFailure` etc. serialize as `"pipelinefailure"` on the wire (only
   `as_str` and the stored string are snake_case). Out of scope to change
   (pipeline kinds). The six new variants carry an explicit
   `#[serde(rename = "connector_failure")]` etc., so they serialize correctly.
   Task 8 must not copy the pipeline kinds' wire string.
2. **No connector-id validator exists in the store.** `connectors::slug_id`
   only mints `[a-z0-9-]`. The alerts crate checks shape only: ASCII
   alphanumeric or `-`, 1 to 128 characters (`is_connector_id_shape`).
3. **AI tools were a way around D7.** `routes/ai/tools/alerts.rs`
   `create_alert_rule` / `update_alert_rule` call `save_rule` directly and
   take `type`/`connector` from model arguments. They now run the same
   check (`authorise_rule_scope`, `authorise_rule_update` in `routes/alerts.rs`)
   with the chat user's principal; no principal fails closed for the six kinds.
4. **`DELETE /api/alerts` is unchanged**: any `alert:write` holder can delete
   any rule, including another tenant's connector rule or the `*` rule. Not in
   the plan; flagged for the planner. `GET /api/alerts` still lists every rule
   (connector ids and webhook targets) to every authenticated caller, as before
   (limit "Rules are shared by the whole installation").
5. **Files outside the plan:** `routes/load_alerts.rs` (new, holds the
   connector/upload event logic so `pipelines.rs`, already very large, gets
   only the dispatch), `routes/mod.rs` (`mod load_alerts;`),
   `routes/connectors.rs` and `routes/uploads.rs` (`INGEST_JOB` and
   `FILE_INGEST_JOB` made `pub(super)`; `settle_loading_upload`), and
   `routes/ai/tools/{alerts,mod}.rs` (point 3).
6. **Order and loss.** The dedupe row is written before `record_run_result`
   (required so a retry cannot double count). If the write after it fails the
   retry is skipped and that one event is lost; documented in the module doc.
   For uploads the dedupe is written only after the upload is `failed`, so an
   unreachable orchestrator or `ClickHouse` leaves it `ingesting` and the
   retry tries again.
7. **`alert_instance` dedupe** is per `rule_id` within 15 minutes: two
   connectors failing under one `*` rule both deliver, but only the first gets
   an Alerts-page row. Commented at `deliver_event`. Success events write no
   instance.
8. **Cancelled runs.** Verified in `pipeline_events.py`: the failure sensor is
   a `run_failure_sensor` and the success sensor a `run_status_sensor(SUCCESS)`,
   so a cancelled run is never posted; if one were, the status check refuses it
   with 409 before any write.
9. **Fixed-text errors.** The new paths answer orchestrator and `ClickHouse`
   failures with fixed text (`the orchestrator could not be asked about this
   run`, `the alert rules could not be read`) and log the cause; the existing
   pipeline path still uses `js_error`.
10. **Parity corpus.** `rust/tests/parity/corpus/alerts-list.json` records
    `{"rules": []}` from the TypeScript original; the Rust body now also has
    `runEventsConfigured`. Not touched (the corpus is the original's capture);
    the planner decides whether the parity run needs an exception.
11. `tests/fixtures/tool_schemas.json` **does not change**: it snapshots tool
    input schemas, which hold no `Connector` output fields (grep for
    `lastTestAt`/`failureStreak` in it and in `registry.rs`: 0). No tool
    schema was edited.
12. `GET /api/alerts` is an object (`{"rules": [...]}`), not a bare array, so
    `runEventsConfigured` sits beside `rules` and `listRules` in
    `src/services/clients/alerts.ts` keeps working unchanged. It is true when
    the API's `PIPELINE_RUN_TOKEN` is non-empty; it cannot see the
    orchestrator's copy.

For tasks 8–10: rule wire shape (`POST`/`PUT /api/alerts`, camelCase for the
request fields that are one word): `{ "name", "type", "channel", "target",
"enabled"?, "severity"?, "connector"?, "id"? (PUT) }`.

- `type` is one of `connector_failure`, `connector_repeated_failure`,
  `connector_disabled`, `connector_schema_change`, `connector_success`,
  `upload_failure`. Send no `mart`, `measure`, `board`, `pipeline`.
- `connector`: a connector id, or `"*"`. Required for the five `connector_*`
  kinds; `connector_success` refuses `"*"` (400, `connector_success must name
  one connector, not "*".`); `upload_failure` ignores it and stores `"*"`.
- Responses and `GET /api/alerts` rules carry `"type": "<kind>"` and
  `"connector": "<id or *>"` (omitted when empty, like `pipeline`).
- Errors: 403 `permission_denied: only an administrator who sees every tenant
  can save a rule for all connectors or for uploads` (a non-`*:*` caller with
  `*`/uploads); 404 `Connector <id> not found` (unknown or another tenant's,
  identical); 401 with no principal; 400 for a missing or malformed connector.
- `GET /api/alerts` → `{ "rules": AlertRule[], "runEventsConfigured": boolean }`.
- `Connector` (and `ConnectorDetail`) gain `lastRunSuccessAt: string | null`,
  `lastRunFailureAt: string | null`, `failureStreak: number` (ISO strings, UTC
  millis). `health` is `degraded` for streak 1–2, `unhealthy` from 3,
  `healthy` after a success.
- Alert text: `Connector <name> (<id>) run <runId> failed. View:
  /connectors/<id>`; upload: `File <name> (upload <id>) failed to load in run
  <runId>. View: /connectors/upload?id=<id>`. Instances show on the Alerts page
  with source `Connector runs` / `File uploads`.
- The gate (task 10) needs the failing run to be an `ingest_job` run whose
  config carries `ops.run_ingest.config.connector_id`, and `PIPELINE_RUN_TOKEN`
  set on both the API and the Dagster code location.

### Review fixes and tasks 8–10 (second pass)

Commits on `feat/phase-1-connector-reliability`, base `4eb1928`:

| Item | Commit |
| --- | --- |
| BLOCKER 1 list filter (`visible_rules`, store `connector_ids_in_tenants`, assistant list tool) | `bae7f7c` |
| BLOCKER 2 delete check (`authorise_existing_rule`, assistant delete tool) | `a72aed2` |
| SHOULD-FIX 3 `GET /api/alerts/status`, list body restored | `78508b1` |
| SHOULD-FIX 4 bounded in-tick retry, 503 for an unsettled upload, comments | `73e6136` |
| Task 8 rule form | `72aafa7` |
| Task 9 run health on Sources | `5c338e2` |
| Task 10 gate step, `PIPELINE_RUN_TOKEN` in the g6 CI `.env` | `9088b69` |

Commands run (foreground; `df -h /` 58-59 GB free before each cargo run;
`CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-src6-target CARGO_BUILD_JOBS=2`):

- Per Rust commit: `cargo fmt --check` clean; `cargo clippy -p lakehouse-api
  (-p lakehouse-store for BLOCKER 1) --all-targets -- -D warnings`: no warnings
  each time. Final: `cargo fmt --check` clean; `cargo clippy --workspace
  --all-targets --all-features -- -D warnings`: finished, no warnings.
- `python3 ops/lint/check_intra_package_imports.py`: OK.
  `python3 ops/lint/check_bare_iceberg_count.py`: OK.
  `python3 -m py_compile ops/g6/g6_ingest_matrix_test.py`: compiles.
- `python3 -m unittest dispar_orchestrate.test_pipeline_events` (from
  `dagster/`, with a **stub `dagster` module** on `PYTHONPATH`, because no
  Dagster is installed on this machine and there is no venv): 22 tests, OK.
  *Not verified* against the real package.
- `bun run typecheck`: clean. `bun run lint`: 0 errors, 6 warnings (the same
  six as before). `bun run test`: 911 pass, 1 skip, 0 fail (912 tests, 101
  files).
- Not run: `cargo test`, `cargo build`, docker, the g6 gate.

**What Dagster does** (found by downloading the pinned wheel,
`pip download dagster==1.13.20`, since none is installed; read
`dagster/_core/definitions/run_status_sensor_definition.py`, `_wrapped_fn`):
lines 910-959 call the function inside `user_code_error_boundary`;
lines 961-965 catch `RunStatusSensorExecutionError` (a raise) and turn it into
a `serializable_error` for the tick; line 967 `context.update_cursor(...)`
runs after that and also after a normal return; lines 977-983 yield a
`DagsterRunReaction`. So raise or return, the cursor moves past the run and
nothing asks again: **Dagster does not retry.** The default sensor evaluation
limit is 60 s (`_grpc/utils.py`, `_DEFAULT_GRPC_TIMEOUT_IF_NO_ENV_VAR_SET =
60`, overridable by `DAGSTER_SENSOR_GRPC_TIMEOUT_SECONDS`). The review was
right. `post_run_failed` / `post_run_finished` now try 3 times, 3 s apart,
for connection errors, timeouts and 5xx only (never 4xx); the request timeout
went from 30 s to (3 s connect, 10 s read) so the worst case for one run, 45 s,
fits the limit. Remaining limit, stated in `pipeline_events.py` and
`load_alerts.rs`: one report per run; if the API is unreachable for the whole
window the alert is not sent and the connector's health catches up at its next
run; a sensor evaluation covers every run since the last tick, so a long
outage with many runs can still exceed the limit. `load_alerts.rs`: an upload
that cannot be settled yet now answers 503 with fixed text (was 200
`{"matched": 0}`).

Existing assertions changed:

- `routes/ai/tools/alerts.rs::update_delete_and_run_require_id`:
  `delete_alert_rule(&state(), None, &Map::new())` (the tool takes state and
  principal now); expected value unchanged.
- `routes/alerts.rs` test mod `run_events_configured`: now tests the `status`
  handler (flag moved to its own route); same truth table.
- `test_pipeline_events.py`: four `timeout=30` assertions now assert
  `pipeline_events.POST_TIMEOUT` (the timeout changed with the retry budget);
  the two always-failing-post tests also patch the pause.
- `alerts-page.test.tsx`: the three `alertRuleFormFields` tests gain the keys
  `connector`, `allConnectors`, `note` (false/false/null) because `toEqual`
  compares every key.
- Behaviour change, not an assertion: the rules table's Type column used the
  label "Dashboard digest" for everything but `alert` (a freshness rule read
  "Dashboard digest"); it now reads "Dataset freshness".

*Not verified* (CI is the first run): every Rust test, new and old, including
`list_shows_a_caller_only_the_connector_rules_of_their_tenants`,
`list_returns_other_kinds_untouched_and_refuses_new_kinds_without_a_principal`,
`delete_applies_the_scope_check_to_the_stored_rule` (route tests in
`routes/alerts.rs`), store
`connector_ids_in_tenants_keeps_only_the_ids_of_the_callers_tenants`, the
`status` test, and the new 503 for an unsettled upload (no test drives that
path: `Settler` fails only when the orchestrator or `ClickHouse` is
unreachable, and the same orchestrator call precedes it). `tests/route_auth.rs`
needed no edit: its two loops walk `POLICY_TABLE`, and
`every_registered_route_has_a_policy_entry` reads `routes/mod.rs`, so the new
route is asserted both ways by construction once it is in both. The g6 step has
not run.

BLOCKER 1 design notes: a rule of the six kinds is visible to a non-
administrator only if its connector id is returned by one query
(`connector_ids_in_tenants`, `id = ANY($1) AND tenant_id = ANY($2)`); `*`,
`upload_failure` and a connector that is gone (or has no tenant) are hidden. No
principal: 401 only when such a rule exists (other kinds as before, the policy
gate already requires auth). BLOCKER 2: `authorise_existing_rule` is the block
`authorise_rule_update` had inline; update and delete both call it. An
unrestricted caller skips the read. Another tenant's connector and an unknown
one both answer 404 `Connector <id> not found`; a `*` or upload rule is the 403
text a save of one gives.

Task 8 and "All connectors": the console can know. `useAuth().user.permissions`
(from `/api/auth/me`) contains the literal `*:*` for an administrator who sees
every tenant, the exact token the API's `is_unrestricted` checks
(`seesEveryTenant`). "All connectors" is offered only then, and never for the
success kind. The "Upload failed" kind is still listed for everyone (the plan
names six kinds), so a non-administrator who picks it gets the API's 403 message
shown in the dialog. Status notice: if `GET /api/alerts/status` cannot be read,
no notice is shown (the list reports its own failure) rather than a guess.

Task 10: what the step asserts: a real `ingest_job` run with a wrong password
fails in Dagster; then `GET /api/connectors/{id}` shows `failureStreak == 1`,
`health == "degraded"` and a `lastRunFailureAt`, within 300 s of the run's
`endTime`; it prints `[g6] SRC-7 failed run <id> -> connector failureStreak=1
health=degraded lastRunFailureAt=<t> <n>s after the run ended (polled every
3s)`. What it does **not** assert: that `GET /api/overview/alerts` lists the
failure and that a webhook arrives. `ops/g6/rest_stub.py` has only `do_GET`
(a webhook POST would get a 501), so a receivable webhook needs a change
outside the test file; skipped, with a comment in the step, not faked. The
failing connector is not in the matrix's `run_ids`, so no later step expects
its run to succeed. Verified by reading: `docker-compose.yml:258` and `:1877`
pass `${PIPELINE_RUN_TOKEN:-}` to `lakehouse-api` and `dagster-code-location`;
`main.rs:98` `bootstrap_pipeline_run_service` seeds the identity at start; the
g6 override does not touch the variable. Only the g6 job's CI `.env` heredoc
changed. Side effect to know: with the token set, the g6 stack's other runs now
also report to the API (matrix successes set their connectors healthy), and the
authored-pipeline factory is active there.

Files outside the plan: `rust/crates/lakehouse-store/src/connectors.rs` and
`tests/connectors.rs` (the batch tenant helper, BLOCKER 1),
`routes/ai/tools/mod.rs` (dispatch), `src/features/connectors/
connectors-columns.test.ts` (new test file for the pure helpers).

Plan/code mismatches: none that stop the work. The plan's "retry" wording in
`load_alerts.rs` and `pipeline_events.py` (finding 4) is corrected.

## Review

### Tasks 1–7, 2026-10-09, at `4eb1928`

Read `routes/load_alerts.rs` in full, the store and migration changes, the
rule authorisation in `routes/alerts.rs`, and the run sensors the events
come from.

Re-run by the planner on `4eb1928`: `cargo fmt --check` clean;
`cargo clippy --workspace --all-targets --all-features -- -D warnings` no
warnings (shared target dir, 2 jobs); `bun run typecheck` clean;
`bun run lint` 0 errors, 6 warnings in untouched files; `bun run test` 891
pass, 1 skip, 0 fail, 892 tests in 100 files. No Rust test was run by
anyone.

Checked by reading: every query that loads a `ConnectorRow` goes through
`CONNECTOR_COLUMNS`, so the three new columns cannot be missing at run time
(`connectors.rs:342`, `:428`, `:789`, `:1833`).

Findings:

- `BLOCKER` 1. `GET /api/alerts` returns every rule to every signed-in
  caller. With the new kinds a rule holds a connector id, so one tenant's
  user reads another tenant's connector ids (the class of leak `SEC-16`
  closed). For the six new kinds the list must show a caller only rules for
  connectors in their tenants, and `*` and upload rules only to an
  administrator who sees every tenant. The assistant's rule-listing tool
  must apply the same filter. Other kinds: unchanged.
- `BLOCKER` 2. `DELETE /api/alerts` lets any `alert:write` holder delete any
  rule, including another tenant's connector rule or an administrator's `*`
  rule: a user can silence another tenant's failure alerts. Deleting a rule
  of the six kinds needs the check `authorise_rule_update` applies to the
  existing rule. Same for the assistant's delete tool.
- `SHOULD-FIX` 3. `runEventsConfigured` was added to `GET /api/alerts`,
  whose recorded body in `rust/tests/parity/corpus/alerts-list.json` is
  `{"rules": []}`. The parity harness is the cutover gate and must not be
  widened for this. Move the flag to its own route, `GET /api/alerts/status`
  (`Policy::RequiresAuth`, in `POLICY_TABLE`, asserted both ways in
  `tests/route_auth.rs`), and leave the list body as it was.
- `SHOULD-FIX` 4. Two comments promise a retry that the code does not show
  (rule 1). `load_alerts.rs` says "the sensor's retry tries again" and
  answers `{"matched": 0}` with 200 when an upload is not yet settled;
  `pipeline_events.py:147` catches the request error, logs it and returns, so
  the tick succeeds and nothing asks again. Read what the installed Dagster
  does with a run-status sensor whose function raises or returns normally,
  then make code and comments agree. Whatever it does, one refused or
  unreachable call must not lose an alert silently: give the two posts a
  bounded retry inside the tick (three attempts, a few seconds apart, within
  the sensor's time limit), and state the remaining limit where the reader
  looks.
- Accepted: the copilot's alert tools now apply decision D7 (they called
  `save_rule` directly). Found by the developer; it would have been a way
  around the rule.
- Noted, not this work: `AlertKind`'s `rename_all = "lowercase"` writes the
  pipeline kinds as `pipelinefailure` on the wire. Existing behaviour; needs
  its own backlog item.
- Plan correction (planner's error): task 10 says to stop if
  `PIPELINE_RUN_TOKEN` is not in the gate's environment. It is not
  (`ops/g6/*.yml`, `.github/workflows/ci.yml`). Compose already passes
  `${PIPELINE_RUN_TOKEN:-}` to both services, so the gate needs only the
  variable in the g6 job's CI-only `.env`; the developer may add that line.

### Review fixes and tasks 8–10, 2026-10-09, at `9088b69`

Read the list filter, the delete check, the status route, the sensor retry
and the gate step. No open `BLOCKER`.

- `BLOCKER` 1 fixed in `bae7f7c`, `BLOCKER` 2 in `a72aed2`, `SHOULD-FIX` 3
  in `78508b1`, `SHOULD-FIX` 4 in `73e6136`. The Dagster claim behind
  `SHOULD-FIX` 4 was read by the developer in the pinned `dagster==1.13.20`
  (`run_status_sensor_definition.py`, lines 910–983): the sensor moves past a
  run whether its function raises or returns. The planner did not re-read
  that source.

Re-run by the planner on `9088b69`: `cargo fmt --check` clean;
`cargo clippy --workspace --all-targets --all-features -- -D warnings` no
warnings; both `ops/lint` scripts pass; `py_compile` of the gate script and
`pipeline_events.py` passes; `bun run typecheck` clean; `bun run lint` 0
errors, 6 warnings in untouched files; `bun run test` 911 pass, 1 skip, 0
fail, 912 tests in 101 files.

Not verified, by anyone:

- Every Rust test of `SRC-7` and migration `0062` on PostgreSQL.
- `test_pipeline_events.py` against the real Dagster package: no Dagster is
  installed here, and the developer's 22 passing tests ran against a stub
  module. CI's "Dagster · unit tests" job is the first real run.
- The g6 step, and what adding `PIPELINE_RUN_TOKEN` to the g6 job does to
  the gate's other steps: every run in that stack now reports to the API.
- The 503 for an upload that cannot be settled yet has no test.
- Nothing was opened in a browser.

Known limits of what was built, for the feature page:

- The gate proves the failed run reaches the connector's row within 5
  minutes. It does not prove an email or a webhook arrives: the gate's stub
  server accepts no POST. Acceptance row 2 is proven only by the product
  owner's QA.
- `docs/plans/SRC-7-RESULT.md` cannot be written until CI prints the measured
  seconds (task 11, principle 5). Until then "within 5 minutes" is a target,
  not a measurement.
- Two different connectors failing under one "all connectors" rule within 15
  minutes both deliver, but only the first is listed on the Alerts page.
