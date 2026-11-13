# QA round, leftovers (P3): a quality rule run by hand without the table check, a run record that stays "running"

Fixes for two backlog rows that came out of the planner's own reviews in the QA round of
2026-11-02 to 11-12, not out of a QA case: `GOV-12` (P2, security; #493 review, round 1 (R3))
and `SRC-43` (P3; #485 review, round 2 (R2)). There is **no QA case ID** for either. Fix sites
and commits cite the backlog ID and the review, and nothing else. Never invent a QA case ID.

Base: `main` at `bfce2c38d` (every anchor below was read there, in a checkout detached at
`origin/main`). Branch: `fix/qa-round-leftovers-p3`. One pull request for the whole slice,
opened as a draft by the planner as soon as T1 is pushed, so CI (x86_64) runs every test.

Planner and reviewer: Claude Opus. Developer: another agent (not Opus). The developer pushes
after every commit and does not change a decision below. If the code is not what a task says,
stop that task and report the mismatch in the handoff. If `main` has moved when the work
starts, **find each anchor by the text quoted, not by its line number**.

Both fixes are Rust only: `rust/crates/lakehouse-api` and, for `SRC-43`, one function in
`rust/crates/lakehouse-store`. No console file, no Dagster file, no migration, no route and no
`POLICY_TABLE` row changes.

What the reviews saw, and what the code says at this commit:

- `GOV-12` (#493 review, round 1 (R3)): `POST /api/governance/quality/{id}/run` (`run_rule`,
  `rust/crates/lakehouse-api/src/routes/quality.rs:1122-1143`) finds the rule and measures it.
  It asks neither the catalog's tenant rule nor the caller's table access. A caller outside
  the tenant that owns the shared catalog, or one whose role does not read the rule's table,
  holding `governance:write`, runs an existing rule and gets its measured value (for example
  "75.0% not null"). The rule's create and update have had both checks since `DATA-54` (#493).
- `SRC-43` (#485 review, round 2 (R2)): "Run now" cancels an overdue run in the orchestrator
  and then closes the stored record (`stop_overdue_run`,
  `rust/crates/lakehouse-api/src/routes/connectors.rs:3991-4028`: the cancel at `:3999-4003`,
  the close at `:4012-4021`). If the store fails between the two, that request answers 5xx. The
  next request sees the run as `CANCELED` in the orchestrator, which is not an active run, so
  nothing closes the record: it says `running` for good. Nothing else can close it either: the
  orchestrator's two run sensors report `FAILURE` and `SUCCESS` only
  (`dagster/dispar_orchestrate/pipeline_events.py:239-262`), and the API refuses any other
  status (`confirm_status`, `routes/load_alerts.rs:238-261`; the module comment at `:56-60` says
  so). No rows are lost; the history shows a run as going that is not.

## 1. Decisions

- **D1. One task per commit, in the order of section 2** (`GOV-12` first because it is the
  security fix, then `SRC-43`, then the documents). Each commit body cites the backlog ID and
  the review: `GOV-12 (#493 review, round 1 (R3))` or `SRC-43 (#485 review, round 2 (R2))`.
  Each fix site carries a short comment that starts the same way (rule 13), for example
  `// GOV-12 (#493 review, round 1 (R3)): …`. No QA case ID appears anywhere.
- **D2. No new guard and no second mapping** (rule 4). The helpers that exist and are used:
  `ensure_rule_table` (`routes/quality.rs:974-1041`), `catalog_tenant_refusal`
  (`routes/catalog.rs:210-236`) and `table_access_for` (`routes/catalog.rs:3200-3205`), both
  already called inside `ensure_rule_table`; `end_timed_out_run`, `settle_run`, `end_run`
  (`rust/crates/lakehouse-store/src/connector_run.rs:655`, `:694`, `:588`); `ran_at_least` and
  `run_end` (`routes/load_alerts.rs:183`, `:169`); `pipeline_run_status`
  (`rust/crates/lakehouse-dagster/src/lib.rs:2337`); `SETTLE_CALL_TIMEOUT`
  (`routes/uploads.rs:250`).

### GOV-12, running a rule by hand

- **D3. Every way a rule is run, and which of them a person drives.** Read at this commit;
  `evaluate` (`routes/quality.rs:1055`) has exactly three callers.

  | Entry | Who | Today | This plan |
  | --- | --- | --- | --- |
  | `POST /api/governance/quality/{id}/run` → `run_rule` (`quality.rs:1122`, route at `routes/mod.rs:663-666`, policy `governance:write` at `policy.rs:403`) | A person | No tenant gate, no table check | Checked (T1) |
  | The scheduled pass: the platform's tick `POST` or `GET /api/alerts/run` without `?id=` starts `spawn_scheduled_pass` (`routes/alerts.rs:1658-1659`, `quality.rs:1590`) → `scheduled_pass` → `evaluate(.., SCHEDULE_ACTOR)` (`quality.rs:1488`) | The system | Runs due rules in the background; the answer of the tick holds no rule's value | Not changed |
  | On load: `quality_load::after_load` (`routes/quality_load.rs:89-96`) → `run_on_load` → `evaluate(.., LOAD_ACTOR)` (`quality.rs:1570`) | The system | Runs the `on_load` rules of the loaded tables | Not changed |
  | The assistant | — | No tool runs a quality rule. `list_quality_rules` and `get_quality` read; `draft_quality_rule` creates through `create_quality_rule`, which is already checked (`routes/ai/tools/governance.rs:158-173`); `run_alert_rule` runs an alert rule, not a quality rule | Nothing to do |
  | A "run all" route | — | There is none (`routes/mod.rs:640-670`) | Nothing to do |

  The check goes into the handler `run_rule`, **not** into `evaluate`: the two system paths
  share `evaluate`, have no person behind them and must keep working whatever any role may
  read. `evaluate` is not edited.
- **D4. The check is `ensure_rule_table`, unchanged, called once.** `run_rule` calls
  `ensure_rule_table(&state, Some(&principal), &headers, &rule.asset).await?` after the rule is
  found and before `evaluate`. That is the call `update_rule` makes for a changed table
  (`quality.rs:1234-1236`) and `create_quality_rule` makes for a new rule
  (`routes/governance.rs:1698`). Inside it the order is already the required one (`:980-1040`):
  the asset's shape, then the catalog's tenant rule (`:1007-1012`), then the role's table access
  (`:1013-1016`), then whether the table exists (`:1018-1040`). Do not copy any of these steps
  into `run_rule` and do not write a second helper.
- **D5. The answers, exact.** `{"error": "<message>"}` bodies, as every `ApiError`.

  | Case | Status | Message |
  | --- | --- | --- |
  | No `governance:write` | 403 | the policy gate's, unchanged |
  | No rule has that id | 404 | `Quality rule not found`, unchanged |
  | Caller outside the catalog's tenant | 403 | `permission_denied: ` followed by the reason `catalog_tenant_refusal` returns; the same for every rule and every table |
  | The caller's role does not read the rule's table | 400 | `"<asset>" is not a table you can read. Pick the table as silver.<table>, serving.<table> or bronze.<table>.` |
  | The rule's table is not there (also: the asset is not a `silver.`, `serving.` or `bronze.` name) | 400 | the same sentence, the same status |
  | The table cannot be checked (the engine or the catalog did not answer) | 503 | `The table could not be checked. Try again.` |
  | The threshold is outside the grammar, or names a column the table does not have | 400 | as today (`evaluate`) |
  | The check query or saving its verdict fails | 503 | as today |
  | Permitted caller, runnable rule | 200 | `{"id", "status", "value"}`, as today |

  `<asset>` is the rule's own stored table text. "Not there" and "not yours" are one sentence
  and one status on purpose, so a run cannot be used to find out which tables exist. Two
  answers change for a caller who was served before: a rule whose table is gone read
  `… is not a table this can read. Name the asset as …` (`quality.rs:1058-1064`) and now reads
  the sentence above; a table the engine did not answer about was a 400 and is now the 503
  ("a check that cannot be made is not a refusal", as on create).
- **D6. The rule is looked up first, then checked.** The 404 for an unknown rule stays before
  the check, as in `update_rule` (`quality.rs:1225-1229`, then `:1234`). Reasons: the gate needs
  the rule's table; `tests/route_auth.rs:2620-2654`
  (`running_a_quality_rule_by_hand_is_gated_in_both_directions`) expects 404 for an unknown
  rule from a caller with `governance:write` and no tenant, and that test and the route's
  `POLICY_TABLE` row are **not** touched; and a rule's id is not a secret (the rule list shows
  every rule to whoever holds `governance:read`). Do not add a separate
  `catalog_tenant_refusal` call before the lookup.
- **D7. A refused run measures nothing and writes nothing.** The refusal comes before
  `evaluate`, so: no check query reaches the engine (`quality.rs:1077-1087`), no row is
  appended to `console.quality_run` (`record_run`, `:1091`), and no incident is opened or marked
  stale (`:1097-1105`). A caller refused by the tenant rule or by the role's access causes no
  question to the engine about the rule's table at all (the existence check is the last step
  of D4). A run writes no audit entry today and this plan adds none. `quality_rule.last_status`
  and `last_run_at` are written by nothing (`quality.rs:38-43`); the test still reads them
  before and after as a guard.
- **D8. What the plan accepts.** (a) A permitted run asks the engine for the table's columns
  twice (once in `ensure_rule_table`, once in `evaluate`'s `resolve`): one cheap metadata
  query more per run by hand. `evaluate` is not restructured to save it, because the system
  paths share it. (b) The refusal sentence names the rule's stored table text. That text is on
  the rule list already; the sentence does not say whether the table exists.
- **D9. No other surface changes for `GOV-12`.** The console already shows the error text of a
  refused run and shows "Run" only with `governance:write`. No assistant tool, schema or
  `tests/fixtures/tool_schemas.json` changes (D3: no tool runs a rule; this is not a dashboard
  behaviour).

### SRC-43, a run record that stays "running"

- **D10. The record is closed when the run list is read, in the API.** The backlog row offers
  two places; this plan chooses `GET /api/connectors/{id}/runs` (`list`,
  `routes/connector_runs.rs:111-119`). Reasons:
  - The failure sensor cannot do it. A run cancelled by "Run now" ends as `CANCELED`; the two
    sensors fire on `FAILURE` and `SUCCESS` only, and the API's report route refuses any other
    status with 409. A sensor fix would be a third sensor, a wider or new API route, a
    `POLICY_TABLE` row, loader tests and a second language. It would also report once: the
    sensor moves past a run whether its post succeeded or not (`load_alerts.rs:45-54`), so a
    store failure in that one report would leave the record `running` again.
  - A read repeats. Whatever went wrong the first time, the next read tries again.
  - The pattern exists: the upload list settles a loading upload from the orchestrator's
    status when it is read (`Settler`, `routes/uploads.rs:1418-1546`).
  - It is where a person sees the wrong word. The run route was the other candidate (it
    already holds the orchestrator's run list), but it runs only when someone presses "Run
    now", which a scheduled connector may never see.

  The cost: the list read does not ask the orchestrator today. After this plan it asks once per
  record that says `running` (normally none, or the one run that is going), bounded by D16.
  The console reads this list every 3 seconds while a run is going
  (`src/features/connectors/connector-ingest-panel.tsx:1261`, `:1403-1407`); the upload list
  pays the same price.
- **D11. A run has ended only when the orchestrator's own status word is `SUCCESS`, `FAILURE`
  or `CANCELED`.** Every other word leaves the record alone: `QUEUED`, `NOT_STARTED`,
  `STARTING`, `STARTED`, `MANAGED`, `CANCELING`, and any word not known. Judge by the raw word,
  as `overdue` does (`routes/connectors.rs:4314-4342`). **Do not** use `map_run_status`
  (`lakehouse-dagster/src/lib.rs:3030-3039`) for this: it folds `CANCELING` into `cancelled`,
  and a run that is still being cancelled has not ended.
- **D12. One mapping from the final status to the record, used by the sensor's report and by
  the list read.** #485 wrote it inline in `connector_run_event` (`load_alerts.rs:478-510`):
  a failed run that ran for the limit or longer closes as `timed_out`, then `settle_run`
  closes whatever is still `running`. T3 moves those lines, unchanged in effect, into one
  function, `close_run_record`, and adds the cancelled case. The result:

  | Orchestrator says | Ran for the limit or longer (`ran_at_least`) | Record becomes | Written by |
  | --- | --- | --- | --- |
  | `SUCCESS` | either | `succeeded`, `completed` | `settle_run(.., true, ..)` |
  | `FAILURE` | no | `failed`, `table_failed` | `settle_run(.., false, ..)` |
  | `FAILURE` | yes | `failed`, `timed_out`, last line names the limit | `end_timed_out_run` |
  | `CANCELED` | yes | `failed`, `timed_out`, last line names the limit | `end_timed_out_run` |
  | `CANCELED` | no | `failed`, `other` (D13) | `settle_cancelled_run` (T2) |

  The end time of the record is the orchestrator's `endTime` (`run_end`, `load_alerts.rs:169`).
  The fourth row is the failure this backlog row describes: "Run now" cancels only a run past
  the limit plus its grace, so the record it could not close gets exactly what
  `stop_overdue_run` would have written (D11 of #485). A record that already ended keeps its
  end: every one of these functions changes only a row that still says `running`.
- **D13. A run cancelled before its limit is recorded as `failed` with the reason `other`.**
  The record has no word for "cancelled": `connector_run.status` is `running`, `succeeded`,
  `failed` or `paused` (`rust/migrations/0072_connector_run_record.sql:52-56`), and #485
  already records the run it cancels as `failed` (its D12). The reason is `other`, whose fixed
  words are "no further reason is recorded" (`connector_run.rs:147`); the console shows "The
  run ended for another reason." (`connector-ingest-panel.tsx:1351`). `table_failed` is **not**
  used for a cancelled run: no table failed, and writing that would invent a cause
  (principle 2). A new status word would need a migration and a console change; that is not
  this row (see Open questions).
- **D14. A run the orchestrator does not know is left as it is.** `pipeline_run_status`
  answers `None` for a run it never had or has lost, and also for a GraphQL answer with no
  data (`lib.rs:2348-2354`); the two cannot be told apart. The record stays `running`, nothing
  is written and no result is guessed (principle 2). One `tracing::debug!` line, not a warning:
  the same record is asked about on every read. The upload list fails such an upload after an
  hour (`UNKNOWN_RUN_BOUND`, `uploads.rs:244`) because a loading upload can be neither deleted
  nor loaded again; a run record that stays `running` blocks nothing (the lock is the
  orchestrator's own run list), and past the run limit the console already shows it as "This
  run never reported its end." So no such bound is added here.
- **D15. The read always answers.** If the orchestrator cannot be asked, does not answer
  within `SETTLE_CALL_TIMEOUT` (5 seconds; the client has no timeout of its own,
  `lib.rs:1058-1076`), or the store fails while a record is closed: one `tracing::warn!` with
  the connector id and the run id, the loop stops for this read (so a dead orchestrator costs
  one failed call, not one per record), and the list is returned as it was read. The error is
  a field of the log macro only (`%err`); no part of it goes into a response, an `ApiError` or
  a `format!` (principle 4; `tests/sec11_guard.rs` must stay green).
- **D16. Bounded.** One read asks about at most 10 records that say `running`, newest first
  (`RECONCILE_PER_READ`). A list with no `running` record asks the orchestrator nothing. The
  rest wait for a later read. Accepted corner: more than ten newer `running` records that can
  never be closed (D14) would keep an older one from being asked about.
- **D17. Idempotent, safe under two requests at once, no new SQL, no migration.** Every close
  goes through `end_run_with_line` (`connector_run.rs:609-643`): it locks the run's row, then
  `UPDATE connector_run SET … WHERE run_id = $1 AND status = 'running'` with bound values
  (`:625-628`; the table's key is `run_id`), and writes the last log line only when that update
  changed a row. Two reads at once close the record once and write one line. The words written
  (`failed`, `succeeded`, `timed_out`, `other`, `completed`, `table_failed`) are all allowed by
  the checks of `0072` and `0137`, so no migration is needed (the highest is `0139`; do not add
  one).
- **D18. What still holds from #485, and what the list read never does.** A `QUEUED` run is
  overdue only past the running limit; a `STARTING` run is left to the orchestrator; a run
  cancelled by "Run now" is not counted as a failed run; a run that never started gets no
  history row. None of that code is touched (`overdue`, `runs_holding_the_lock`,
  `stop_overdue_run` stay as they are). The list read only walks records that exist and say
  `running`. It never inserts a record, never counts a run (`connectors::record_run_once` is
  not called), never delivers an alert, never cancels or launches a run, and writes no audit
  entry. Counting and alerting stay with the sensor's report, which is unaffected when the list
  read closed the record first: the same mapping gives the same record, and its own closes are
  then no-ops.
- **D19. Where `timed_out` is written, restated.** D11 of #485 says "in two places and nowhere
  else". After T3: by `close_run_record` (the sensor's failure report and the read of the run
  list) and by the run route. The run's own end report still may not send it. The two comments
  that say "two places" are corrected in T3.
- **D20. No other surface changes for `SRC-43`.** No console change (the status and reason
  words are ones the console already shows), no Dagster change, no assistant change (the
  assistant's `list_ingest_runs` reads the load results, not this record;
  `routes/ai/tools/ingest.rs:110-127`), no route or policy change.

## 2. Tasks

Rust first (T1 to T3), then the documents (T4). T1 is `GOV-12`; T2 and T3 are `SRC-43`.

### T1. Running a rule by hand checks the catalog's tenant and the table first (#493 review, round 1 (R3); GOV-12)

- Anchors: `rust/crates/lakehouse-api/src/routes/quality.rs:33-36` (module comment),
  `:857-862` (`rule_runs`: the parameter list to copy), `:974-1041` (`ensure_rule_table`),
  `:1109-1143` (`run_rule` and its doc), `:1225-1236` (`update_rule`: lookup, then check).
  Tests: `rust/crates/lakehouse-api/tests/quality_incidents.rs:30-76` (`app`), `:78-104`
  (`call`), `:106-120` (`rule`), `:124-128` (`admin`), `:133-157` (`narrow_to_orders`),
  `:159-170` (`incidents`), `:172-179` (`sent_to_clickhouse`), `:551-585`, `:593-663`,
  `:738-793` (the create refusals of #493); `tests/common/mod.rs:400`
  (`in_the_catalog_tenant`), `:532` (`create_principal_with_permissions`).
- Change, in `quality.rs` only:
  1. `run_rule` takes `headers: HeaderMap` between the `Extension` and the `Path`, exactly as
     `rule_runs` does (`:857-862`). `HeaderMap` is already imported (`:49`).
  2. After the rule is found (`:1128-1131`) and before `evaluate` (`:1132`), add one statement:
     `ensure_rule_table(&state, Some(&principal), &headers, &rule.asset).await?;` with the
     fix-site comment above it: `GOV-12 (#493 review, round 1 (R3))`, saying that a run by
     hand is a person's, so the catalog's tenant rule and the role's table access apply before
     anything is measured or written, and that the scheduled and on-load passes call `evaluate`
     directly and are the system's.
  3. Nothing else in `run_rule` changes. `evaluate`, `scheduled_pass`, `run_on_load`,
     `ensure_rule_table`, `policy.rs` and `routes/mod.rs` are not edited.
  4. Doc comments (rule 1: say only what the code does). `run_rule`'s `# Errors` (`:1112-1121`)
     follows D5: add the 403 for a caller outside the tenant that owns the shared catalog; say
     that a table that is not there and one the caller's role does not read are the same 400
     with one sentence; add the 503 for a table that cannot be checked. The module comment's
     paragraph at `:33-36` keeps its statement (the check query reads the table as the
     platform, only aggregates come back) and gains one sentence: who may start a check by
     hand is decided first, by the catalog's tenant rule and the caller's table access
     (`GOV-12`); the scheduled and on-load checks have no caller and are not narrowed.
- Check: five integration tests in `tests/quality_incidents.rs`, placed after
  `a_caller_outside_the_catalog_tenant_is_refused_before_the_table_is_asked_about`
  (`:738-793`). Reuse the file's fixtures; add **one** helper: `rule_on(pool, name, asset,
  threshold) -> String`, the body of today's `rule` with the asset as a parameter, and make
  `rule` call it with `"silver.orders"` (one creator, rule 4). "Not measured" below means: no
  body in `sent_to_clickhouse(&ch)` contains `countIf(isNotNull` and none contains
  `INSERT INTO console.quality_run`. "Not asked about" means: no body contains both
  `system.columns` and the table's name. "Nothing written" means: `SELECT count(*) FROM
  quality_incident` is 0, and `SELECT last_status || '|' || last_run_at::text FROM quality_rule
  WHERE id::text = $1` gives the same text before and after the refused run.
  1. `a_caller_outside_the_catalog_tenant_cannot_run_a_rule_and_nothing_is_measured`: two
     rules, one on `silver.orders` (`email not null >= 90%`) and one on `silver.no_such_table`
     (`rows >= 1`). The caller is made as at `:741-754` (`governance:write,governance:read`, a
     member of the second seeded tenant). Both runs answer 403 and the two bodies are equal
     (`assert_eq!` on the whole JSON). Not measured, neither table asked about, nothing written.
  2. `a_caller_whose_role_does_not_read_the_rules_table_cannot_run_it`: a rule on
     `silver.customers` (`email not null >= 90%`); the caller is made as at `:606-612`
     (`narrow_to_orders`, `in_the_catalog_tenant`). The run answers 400 and the error contains
     `is not a table you can read`. Not measured, `customers` not asked about, nothing
     written. Then the same person runs a rule on `silver.orders` (`email not null >= 70%`):
     200 and `status` is `passed` (their own table still runs).
  3. `running_a_rule_answers_the_same_for_a_table_that_is_not_there_and_one_the_caller_may_not_read`:
     **one** rule on `silver.customers`. First the narrowed caller of test 2 runs it: 400;
     assert `customers` was not asked about (this refusal is the role's access, not the
     existence check; comparing two refusals of the same branch would prove nothing). Then
     mount, with priority 1, the answer "no columns" for `system.columns` and `customers` (the
     mock at `:597-605`, with the other table name), and `admin` runs the same rule: 400. The
     two statuses are equal and the two whole bodies are equal.
  4. `a_permitted_caller_still_runs_a_rule_and_gets_its_value`: a person with
     `governance:write,governance:read`, in the catalog's tenant, not narrowed and not an
     administrator, runs a rule on `silver.orders` (`email not null >= 90%`): 200, `status`
     `failed`, `value` contains `75.0% not null`; a body containing
     `INSERT INTO console.quality_run` was sent; `incidents(&router, &cookie, "open")` has one
     entry.
  5. `running_a_rule_whose_table_cannot_be_checked_is_a_503_and_measures_nothing`: a rule on
     `silver.broken_table`, the mock of `:555-561`, run by `admin`: 503 with
     `The table could not be checked. Try again.`; not measured.
- Must fail on the base commit: tests 1, 2 and 3 (the run answers 200 with a value) and test 5
  (400). Test 4 passes on the base too: it is the guard that the gate does not shut out a
  permitted person; say so in the handoff.
- Must stay green, unchanged: every other test of `tests/quality_incidents.rs` (all run as
  `admin`), `tests/route_auth.rs` and `tests/sec11_guard.rs`. If a test of another file fails
  because a run by hand is now refused, do not change its assertion: report it.

### T2. The store can settle a cancelled run (#485 review, round 2 (R2); SRC-43)

- Anchors: `rust/crates/lakehouse-store/src/connector_run.rs:6-15` (module comment, "Who
  writes"), `:588-598` (`end_run`), `:684-710` (`settle_run`, the shape to follow);
  `rust/crates/lakehouse-store/tests/connector_run.rs:10-13` (imports), `:421-461`
  (`the_sensor_settles_only_a_run_still_running`, the test to model on).
- Change, in `connector_run.rs`, directly after `settle_run`:
  `pub async fn settle_cancelled_run(pool: &PgPool, connector_id: &str, run_id: &str, at:
  OffsetDateTime) -> Result<(), StoreError>`. It calls `end_run(pool, connector_id, run_id,
  RunStatus::Failed, EndReason::Other, at)` and maps the result exactly as `settle_run` does:
  `Ok(_)` and `Err(StoreError::NotFound)` are `Ok(())`, any other error is returned. No SQL is
  written. Its doc says: it settles a run that is still `running` and that the orchestrator
  ended as cancelled before the run's limit; the record has no word for cancelled, so it is
  `failed` with the reason `other`, never `table_failed` (D13); a run with no record and a run
  that already ended are left alone; with a `# Errors` section
  (`[`StoreError::Database`] on a failed statement`). Add to the module comment's "Who writes"
  one sentence: a run whose end was never reported is closed from the orchestrator's own final
  status by `settle_run`, `settle_cancelled_run` and `end_timed_out_run`.
- Check: one test in `rust/crates/lakehouse-store/tests/connector_run.rs`, after
  `the_sensor_settles_only_a_run_still_running`:
  `a_cancelled_run_still_running_is_settled_as_failed_with_no_reason_invented`. Three runs as
  in the model test: `run-open` (started only), `run-done` (ended `succeeded`, `completed`),
  `run-unknown` (never started). Call `settle_cancelled_run` on each, and twice on `run-open`.
  Assert: `run-open` is `failed` with `end_reason` `other` (read it with `list_runs`), its log
  has exactly one `run_ended` line and that line's text is `run ended: failed, no further
  reason is recorded` (`get_run_log`); `run-done` is still `succeeded` with `completed`;
  `run_status` of `run-unknown` is `None`.
- Must fail: the function does not exist on the base commit. To see the test bite, change
  `EndReason::Other` to `EndReason::TableFailed` for one run and see it fail; put it back.

### T3. Reading the run list closes a record whose run has ended (#485 review, round 2 (R2); SRC-43)

- Anchors: `rust/crates/lakehouse-api/src/routes/connector_runs.rs:85-119` (`RunsResponse`,
  `list`), `:691-696` and `:1153-1156` (the two "two places" comments), `:772-793` (the unit
  tests' `harness`); `routes/load_alerts.rs:56-60`, `:101-135` (`Outcome`), `:169-188`
  (`run_end`, `ran_at_least`), `:456-510` (`connector_run_event`; the lines to move are
  `:478-510`); `routes/uploads.rs:246-250` (`SETTLE_CALL_TIMEOUT`), `:1502-1546` (`settle_own`,
  the shape to follow); `rust/crates/lakehouse-api/tests/ingest_run.rs:42-46`
  (`spin_up_with_dagster`), `:91-107` (`seed_sql_connector`), `:576-583` (`recorded_runs`),
  `:588-651` (the overdue test of #485: how a record and the orchestrator's answers are set
  up), `:1217-1248` (`post_run`, the shape for a `GET` helper).
- Change 1, one mapping (`load_alerts.rs`), D12:
  1. A new `pub(super) enum FinalStatus { Success, Failure, Canceled }` (derive `Debug, Clone,
     Copy, PartialEq, Eq`) with `pub(super) fn of(dagster_status: &str) -> Option<Self>`:
     `"SUCCESS"`, `"FAILURE"`, `"CANCELED"` map to their variant, **every other word is
     `None`**. Its doc names the words that are not an end (`QUEUED`, `NOT_STARTED`,
     `STARTING`, `STARTED`, `MANAGED`, `CANCELING`) and says why `map_run_status` is not used
     (D11).
  2. `Outcome` gets `const fn final_status(self) -> FinalStatus` (`Failure` → `Failure`,
     `Success` → `Success`).
  3. A new `pub(super) async fn close_run_record(state: &AppState, pool: &PgPool,
     connector_id: &str, run_id: &str, ended: FinalStatus, info:
     &lakehouse_dagster::RunStatusInfo) -> Result<(), lakehouse_store::StoreError>`, with a
     `# Errors` section. Body, in this order:
     - `at` is `run_end(info.end_time)`; `limit` is
       `state.config.ingest_run_max_runtime_seconds`.
     - If `ended` is not `Success` and `ran_at_least(info.start_time, info.end_time, limit)`:
       `connector_run::end_timed_out_run(pool, connector_id, run_id, i64::from(limit),
       at).await?`.
     - Then by `ended`: `Success` → `connector_run::settle_run(pool, connector_id, run_id,
       true, at).await`; `Failure` → the same with `false`; `Canceled` →
       `connector_run::settle_cancelled_run(pool, connector_id, run_id, at).await`.

     Move the two comments of `:478-482` and `:499-502` into this function, and add the
     fix-site comment `SRC-43 (#485 review, round 2 (R2))` on the `Canceled` arm with D13's
     reason in one sentence.
  4. In `connector_run_event`, replace the block `:478-510` (the two comments, the
     `end_timed_out_run` call and the `settle_run` call) with one statement:
     `close_run_record(state, pool, &connector.id, &req.run_id, outcome.final_status(),
     &info).await?;`. Nothing else in that function changes. For `Success` and `Failure` the
     effect is what it was (the existing tests below prove it).
- Change 2, the list read (`connector_runs.rs`), D10, D14 to D16:
  1. In `routes/uploads.rs` make `SETTLE_CALL_TIMEOUT` `pub(super)` and add to its doc that the
     connector run list uses it too. Do not add a second timeout constant.
  2. A private `const RECONCILE_PER_READ: usize = 10;` with a doc giving D16.
  3. A private `async fn close_ended_runs(state: &AppState, pool: &PgPool, connector_id: &str,
     runs: &[RunSummary]) -> bool`. It returns whether it closed anything. Its doc carries the
     fix-site line `SRC-43 (#485 review, round 2 (R2))` and says in plain words: what is
     closed (D11, D12), what is left alone (D14), what a failure does (D15), and what it never
     does (D18). Body: for each run in `runs` whose `status` equals
     `RunStatus::Running.as_str()`, in the list's order, at most `RECONCILE_PER_READ` of them:
     - `tokio::time::timeout(super::uploads::SETTLE_CALL_TIMEOUT,
       state.dagster.pipeline_run_status(&run.run_id)).await`;
     - the call failed, or the time ran out: one `tracing::warn!` (fields `connector_id`,
       `run_id`, and `%err` for a failed call) and **stop the loop**;
     - `None`: one `tracing::debug!` and go on to the next run;
     - `Some(info)`: `let Some(ended) = FinalStatus::of(&info.status) else { continue; };` then
       `close_run_record(state, pool, connector_id, &run.run_id, ended, &info).await`; on `Ok`
       remember that something was closed; on `Err`, one `tracing::warn!` (`%err`) and stop the
       loop.
  4. `list` becomes: read the list as today (a failure of this first read is still the
     request's error, as today); if `close_ended_runs` returns `true`, read the list once more
     so the answer shows the closed record; if that second read fails, `tracing::warn!` and
     keep the first list. Then answer as today. The handler's doc gains one paragraph on this
     and keeps its `# Errors`.
  5. Correct the comments at `:691-696` and `:1153-1156` to D19's wording.
  6. The unit tests' harness (`:772-793`): its comment says these routes do not touch the
     orchestrator, which is no longer true for the list. In `harness_with`, before the loop
     over `extra`, set `DAGSTER_URL` to the dead address the integration harness uses (the
     same literal as `DEAD_UPSTREAM`, `tests/common/mod.rs:57`, set there at `:318`; that
     constant is not visible from a unit test, so repeat the literal with a comment naming
     it), and correct the comment: the list asks the orchestrator about a record still
     `running`; here it cannot be reached, so the list answers unchanged. Without this the
     unit tests would call whatever listens on the default orchestrator address of the
     developer's machine. No assertion of an existing test changes.
- Check, unit (`load_alerts.rs`, `mod tests`), a plain `#[test]`:
  `only_success_failure_and_canceled_end_a_run_and_a_run_being_cancelled_has_not_ended`:
  `FinalStatus::of` gives the three variants for the three words and `None` for `QUEUED`,
  `NOT_STARTED`, `STARTING`, `STARTED`, `MANAGED`, `CANCELING`, `unknown` and the empty string.
- Check, the move changed nothing: these existing unit tests stay green unchanged:
  `a_paused_run_is_not_counted_and_an_unclosed_run_is_settled` (`load_alerts.rs:1918`),
  `a_failed_run_that_ran_past_the_limit_closes_its_record_as_timed_out` (`:1982`),
  `a_failed_run_inside_the_limit_still_ends_as_a_failed_table` (`:2011`), and in
  `connector_runs.rs` `the_end_report_cannot_write_the_timed_out_reason` (`:1158`) and
  `the_history_route_answers_fifty_runs_of_this_connector_only` (`:1475`; 55 records say
  `running` there and the orchestrator is unreachable, so it also proves D15).
- Check, integration, in `rust/crates/lakehouse-api/tests/ingest_run.rs`, after
  `an_orchestrator_that_cannot_be_reached_for_the_cancel_answers_503_and_starts_nothing`
  (`:820-879`). Three helpers first:
  - `get_runs(app, cookie, id) -> (StatusCode, Value)`: `GET /api/connectors/{id}/runs`, built
    as `post_run` is.
  - `mount_run_status(server, run_id, answer: Value)`: a mock for `POST /graphql` whose body
    contains `pipelineRunOrError` and matches
    `body_partial_json(json!({"variables": {"rid": run_id}}))`, answering
    `{"data": {"pipelineRunOrError": answer}}`. A run that ended is
    `{"__typename": "Run", "status": …, "startTime": …, "endTime": …, "stepStats": []}`.
  - `run_status_requests(server) -> Vec<String>`: the received bodies that contain
    `pipelineRunOrError` (as `cancel_requests`, `:564-573`).

  Each test starts the mock server, the app with `spin_up_with_dagster`, signs in as the
  seeded user the file already uses (`:623`), seeds its own connector with
  `seed_sql_connector`, and makes records with `connector_run::start_run` as at `:625-634`.
  `now` is the current Unix time as at `:591`.
  1. `a_record_left_running_after_its_run_was_cancelled_is_closed_by_one_read_of_the_run_list`:
     the exact state after the two-step failure. One record, started 7 hours ago, `running`;
     the orchestrator answers `CANCELED`, `startTime` `now - 7 h`, `endTime` `now - 60`. One
     `GET`: 200, and **this answer's** `runs[0]` has `status` `failed`, `endReason`
     `timed_out`, `endedAt` not null. In the store: the same, and the last log line's text is
     `run ended: failed, the run did not finish in 6 h and was stopped`. A second `GET`: the
     same answer; the log has exactly one `run_ended` line; `run_status_requests` has exactly
     one entry over both reads. Not counted: `SELECT count(*) FROM pipeline_run_event WHERE
     run_id = $1` is 0 and the connector's `failure_streak` is 0.
  2. `a_run_cancelled_before_its_limit_is_closed_as_failed_with_no_reason_invented`:
     `CANCELED`, `startTime` `now - 600`, `endTime` `now - 300`, record started 10 minutes
     ago. After one `GET`: `failed`, `endReason` `other`, last line `run ended: failed, no
     further reason is recorded`.
  3. `a_record_left_running_after_its_run_succeeded_or_failed_is_closed_as_the_run_report_would_close_it`:
     two records on one connector; `SUCCESS` and `FAILURE`, each `startTime` `now - 120`,
     `endTime` `now - 60`. After one `GET`: `succeeded` with `completed`, and `failed` with
     `table_failed`. `pipeline_run_event` has no row for either run (the list read counts
     nothing).
  4. `a_run_that_is_still_going_keeps_its_record_running`: six records on one connector, one
     for each of `QUEUED`, `NOT_STARTED`, `STARTING`, `STARTED`, `MANAGED`, `CANCELING`
     (`endTime` null). After one `GET`: all six are `running` with no `endReason`, in the
     answer and in the store, and `run_status_requests` has six entries (they were asked
     about and left alone, not skipped).
  5. `a_record_whose_run_the_orchestrator_does_not_know_is_left_running`: the answer is
     `{"__typename": "RunNotFoundError", "message": "the run is gone from the store"}`. After
     one `GET`: 200, the record is `running` with no `endReason` and no `run_ended` line, the
     response does not contain the mock's message, and `run_status_requests` has one entry.
  6. `the_run_list_still_answers_when_the_orchestrator_cannot_be_asked`: two `running`
     records; every `pipelineRunOrError` request answers 500 with the text `upstream is down`.
     One `GET`: 200, both records `running`, the body does not contain `upstream is down`, and
     `run_status_requests` has exactly **one** entry (the loop stopped at the first failure).
  7. `two_reads_at_once_close_a_record_once`: the state of test 1; two `GET`s through
     `tokio::join!`. Both answer 200; afterwards the record is `failed` with `timed_out` and
     its log has exactly one `run_ended` line.
  8. `a_connector_with_no_running_record_asks_the_orchestrator_nothing`: one record that
     ended (`start_run`, then `end_run` as `succeeded`, `completed`) and no mock for
     `pipelineRunOrError`. One `GET`: 200, one run, `run_status_requests` is empty, and
     `SELECT count(*) FROM connector_run WHERE connector_id = $1` is still 1 (no row is
     invented).
  9. `one_read_asks_about_at_most_ten_running_records`: twelve `running` records; one mock
     that answers `STARTED` for any `pipelineRunOrError` request. One `GET`: 200,
     `run_status_requests` has exactly ten entries.
- Must fail on the base commit: tests 1, 2, 3 and 7 (the record stays `running`), and tests
  4, 5, 6 and 9 by their request counts (the base asks the orchestrator nothing). Test 8
  passes on the base; it is the guard for D16 and D18. To see test 4 bite on the status rule,
  make `FinalStatus::of` answer `Canceled` for `CANCELING` once and see it fail; put it back.
- Must stay green, unchanged: every other test of `tests/ingest_run.rs` (the overdue rules of
  #485 are not touched), `tests/route_auth.rs`, `tests/sec11_guard.rs`.
- Not covered by a test, read only: a store failure while a record is closed (the loop
  stops, the first list is returned). Say so in the handoff.

### T4. The documents say what is now true (GOV-12, SRC-43)

Documents only; no code. The developer does not edit `docs/core/BACKLOG.md`,
`docs/core/PRODUCT.md` or any Decisions table or "Signed" cell (the reviewer updates the
backlog after the merge).

- `docs/core/features/data-quality-in-depth.md`, in "Limits to tell a customer", directly
  after the point that ends "other table is refused (`DATA-54`)." (`:59-60`):

  > - A rule is run by hand only by a person who may read its table. Anyone else is refused
  >   with the same sentence a table that is not there gets, and nothing is measured or
  >   recorded. Scheduled and on-load checks run as the system and are not affected. Changed
  >   2026-11-12 (`GOV-12`), *(proposed, not yet signed)*.

- `docs/core/features/connector-operations.md`, at the end of "Limits to tell a customer"
  (after the point that starts "A run's status changes when the orchestrator reports the end
  of the run", `:99`):

  > - A run whose end was never recorded is closed when its connector's run list is next
  >   read: the list asks the orchestrator, and a run that has ended there is recorded as
  >   succeeded or as failed. A run that was stopped is recorded as failed, with the time
  >   limit as the reason when it ran that long and "The run ended for another reason."
  >   otherwise. This closes the record only: nothing is counted and no alert is sent by it.
  >   A run the orchestrator no longer knows stays as it is. Changed 2026-11-12 (`SRC-43`),
  >   *(proposed, not yet signed)*.

- `CHANGELOG.md`, under `## [Unreleased]`:
  - in `### Security`, directly after the `GOV-10` entry ("Running a data-quality rule by
    hand needs `governance:write` …"):

    > - Running a data-quality rule by hand checks the table first (backlog `GOV-12`). A
    >   person outside the organisation that owns the catalog, or whose role does not read
    >   the rule's table, could run an existing rule and see its measured value. They are now
    >   refused before anything is measured or recorded, with the same answer for a table
    >   that is not there and one they may not read. Scheduled and on-load checks are
    >   unchanged.

  - in `### Fixed`, as the first entry:

    > - A connector run no longer stays "running" in the history after it ended (backlog
    >   `SRC-43`). When a run's end was never recorded, reading the connector's run list now
    >   asks the orchestrator and closes the record with the run's real outcome. A run the
    >   orchestrator no longer knows is left as it is.

- Check: `git diff --stat` shows only these three files in this commit.

## 3. Verification

Section 3a names the machine this slice is built on and replaces the machine-specific lines
of this section. What holds on any machine:

- Per commit, only for what the commit touched. Never `cargo clean`. Never a bare
  `cargo test -p lakehouse-api <filter>` and never `cargo test --workspace`: a unit test of
  `lakehouse-api` runs with `--lib` and its name, an integration test file with `--test
  <file>`.
- On the planner's own machine (11 GB of memory, little disk) the rule is: per Rust commit
  `cargo fmt --check` and `/home/shiro/rantai/bin/cargo-locked check -p <crate> -j 2`; the
  integration tests under `rust/crates/lakehouse-api/tests/` are left to CI and listed in the
  handoff as *not verified locally, CI*. That rule applies to anyone who works there. The
  developer of this slice works on the machine of section 3a and runs the tests named below.
- Always, on the final commit: `python3 ops/lint/check_bare_iceberg_count.py && python3
  ops/lint/check_intra_package_imports.py`.
- No TypeScript, no Python and no compose file is touched: `bun`, `pytest` and `docker
  compose config` do not apply. No route is added or changed: `POLICY_TABLE` and
  `tests/route_auth.rs` are not edited (the file is still run, see 3a).

The tests of each task:

| Task | Crate | Tests |
| --- | --- | --- |
| T1 | `lakehouse-api` | `--test quality_incidents` (18 tests of its own after T1: 13 today plus 5) |
| T2 | `lakehouse-store` | `--test connector_run` (12 after T2: 11 today plus 1) |
| T3 | `lakehouse-api` | `--lib only_success_failure_and_canceled_end_a_run`; `--lib a_failed_run_`; `--lib a_paused_run_is_not_counted`; `--lib the_end_report_cannot_write_the_timed_out_reason`; `--lib the_history_route_answers_fifty_runs`; `--test ingest_run` (37 of its own after T3: 28 today plus 9) |
| T4 | — | `git diff --stat` only |

The handoff quotes each command with its counts, names every test added so it can be found
in the CI log, says which "must fail" checks were made and seen to fail, and says *not
verified* for anything not run (rule 7). The full `cargo test --workspace` is CI's on the
pull request; list it as *not verified locally, CI*.

What the planner read and did not run; the developer's runs must confirm it. If one of these
answers differently, report the answer and stop that task; the plan is corrected, not the
assertion.

- That the orchestrator gives a cancelled run an end time. Read in the pinned `dagster`
  1.13.20: `handle_run_event` sets `end_time` on the cancel event, and the GraphQL `endTime`
  returns it. If a cancelled run had no end time, `ran_at_least` would be false and the record
  of the `SRC-43` failure would close as `other`, not `timed_out`; the tests give the mock an
  `endTime`, so only a real run shows this. Not verified against a running orchestrator.
- That a 500 answer with a body that is not JSON makes `pipeline_run_status` return an error
  (T3 test 6), and that a `RunNotFoundError` answer makes it return `None` (test 5).
- That the seeded user `tests/ingest_run.rs` signs in as may read
  `GET /api/connectors/{id}/runs` of a connector seeded by `seed_sql_connector` (the role
  holds `connector:manage`; the connector is in that person's tenant).
- That two error bodies of the same refusal are equal as whole JSON values (T1 tests 1 and
  3): an `ApiError` body is `{"error": "<message>"}`.
- That nothing but the rule's own check sends a request containing `system.columns` and the
  table's name to the stand-in during T1's tests.
- That a role made by `create_principal_with_permissions` reads every table until it is
  narrowed (T1 test 4).
- That the unit tests of `connector_runs.rs` that list a record still `running` stay green
  with the orchestrator unreachable (T3, change 2.6).

## 3a. Change of machine

- The developer works on the ARM64 build machine, in the checkout
  `/home/rismana/rantai/lakehouse-agents-2`.
- Before the first cargo command of a shell:
  `export CARGO_TARGET_DIR=/home/rismana/rantai/cargo-target-2 CARGO_BUILD_JOBS=6`.
- Plain `cargo`; there is no `cargo-locked` there.
- Per Rust commit, from `rust/`: `cargo fmt --check`; `cargo clippy -p <crate> --all-targets
  -- -D warnings` (`lakehouse-api` for T1 and T3, `lakehouse-store` for T2); the named unit
  tests as `cargo test -p <crate> --lib <name>`; and each touched integration test file as
  `cargo test -p lakehouse-api --test <file>` (for T2: `cargo test -p lakehouse-store --test
  connector_run`). Docker runs there, so the integration tests run.
- ALWAYS also `cargo test -p lakehouse-api --test sec11_guard` and
  `cargo test -p lakehouse-api --test route_auth`, because `lakehouse-api/src` is touched
  (T1 and T3).
- Never `cargo test --workspace`, never a bare `cargo test -p lakehouse-api <filter>`, never
  `cargo clean`.
- If `dagster/` is touched, also `(cd dagster && python -m pytest dispar_orchestrate -q -m
  loader)` besides the touched test file. This plan touches nothing under `dagster/`; if a
  task seems to need it, stop and report.
- Each integration test binary of `lakehouse-api` also runs one test from `src/tls_proxy.rs`,
  so its count is one more than the file's own tests: expect 19 for `quality_incidents` after
  T1 and 38 for `ingest_run` after T3. The store's `connector_run` binary has no such extra
  test (12 after T2).
- CI on the pull request (x86_64) is the final word: a check that is green here and red
  there is red.
- Work only under `/home/rismana/rantai/`. Start no container you do not remove: before the
  first `cargo test --test …` run `docker ps -a` and write down what is there; after the last
  one remove what your runs started and nothing else, and say in the handoff which you
  removed.

Before handoff, once, in the foreground, on the final commit:

```bash
cd rust && cargo fmt --check
cargo clippy -p lakehouse-store --all-targets -- -D warnings
cargo clippy -p lakehouse-api --all-targets -- -D warnings
cargo test -p lakehouse-store --test connector_run
cargo test -p lakehouse-api --lib only_success_failure_and_canceled_end_a_run
cargo test -p lakehouse-api --lib a_failed_run_
cargo test -p lakehouse-api --lib a_paused_run_is_not_counted
cargo test -p lakehouse-api --lib the_end_report_cannot_write_the_timed_out_reason
cargo test -p lakehouse-api --lib the_history_route_answers_fifty_runs
cargo test -p lakehouse-api --test quality_incidents
cargo test -p lakehouse-api --test ingest_run
cargo test -p lakehouse-api --test sec11_guard
cargo test -p lakehouse-api --test route_auth
cd .. && python3 ops/lint/check_bare_iceberg_count.py && python3 ops/lint/check_intra_package_imports.py
git status   # nothing generated staged
```

## 4. Out of scope

- **The other quality routes that have no catalog tenant gate.** Seen while reading the
  handlers for T1, and not part of `GOV-12` (the row names the run): a `PUT
  /api/governance/quality/{id}` that keeps the rule's table (`quality.rs:1230-1236` checks only
  a changed table), `DELETE /api/governance/quality/{id}` (`:1290-1310`), the rule list `GET
  /api/governance/quality` (`routes/governance.rs:118`, `:210-224`), setting or resetting a
  schedule (`routes/quality_schedules.rs`: only the list is gated, `:61`) and resolving an
  incident (`routes/quality_incidents.rs`: only the list is gated, `:189`). Read at the
  handlers only. For the backlog; see Open questions.
- **`evaluate`, the scheduled pass and the on-load pass** (D3): not changed, not gated by any
  person's access.
- **A status word "cancelled" for a run record** (D13): a migration, a console label and the
  contract; its own item if the owner wants it.
- **Closing a record whose run the orchestrator no longer knows** (D14), and any time bound
  for it.
- **A sensor for cancelled runs, or any change under `dagster/`** (D10).
- **The order of cancel and close in `stop_overdue_run`** (#485 round 2 kept "cancel first,
  then close"); the overdue rules, the limits and the grace (D18).
- **Counting, the failure streak and alerts for a run closed by the list read** (D18): they
  stay with the sensor's report. A `FAILURE` whose sensor report was lost is still not
  counted, as today.
- **Other readers of the run record** that do not go through `GET
  /api/connectors/{id}/runs` (for example `routes/lineage/charts.rs:349`): they show the
  record as stored; the next read of the run list closes it.
- **`docs/core/BACKLOG.md` and `docs/core/PRODUCT.md`**: the reviewer sets `GOV-12` and
  `SRC-43` to Done after the merge.

### Open questions for the owner

None of these blocks the plan.

1. A run that was stopped before its time limit (cancelled in the orchestrator) will read
   "failed: The run ended for another reason." once its record is closed, because the record
   has no word for "cancelled" (D13). Today such a record says "running" until the limit and
   then "This run never reported its end." Is "failed, for another reason" the wording
   wanted, or should a cancelled run get its own word? The planner's proposal: keep D13 now,
   and open a backlog item for a "cancelled" status only if the wording is not acceptable.
2. The quality routes listed first under "Out of scope" let a person outside the catalog's
   tenant who holds `governance:write` loosen, delete or reschedule a rule and resolve an
   incident, and one who holds `governance:read` list every rule with its table. Should they
   get the same tenant gate? The planner's proposal: one backlog row (security, P2), planned
   on its own, because the rule list is also what the Data Quality page reads.
3. The two feature-page lines of T4 are *(proposed, not yet signed)*; they need the owner's
   signature like any limit told to a customer.

## 5. Handoff

Developer: a non-Opus agent, on the ARM64 machine in
`/home/rismana/rantai/lakehouse-agents-2`, branch `fix/qa-round-leftovers-p3`
tracking `origin/fix/qa-round-leftovers-p3`, based on `bfce2c38d` (`main` at
the plan's commit; the branch already held `ffa9b228`, the plan itself).
`CARGO_TARGET_DIR=/home/rismana/rantai/cargo-target-2`,
`CARGO_BUILD_JOBS=6`. Four commits, one per task, pushed after each.

### Commits

| Task | Commit | Title |
| --- | --- | --- |
| T1 | `c9cfd519` | `fix(quality): run a rule by hand only for a person who may read its table` |
| T2 | `2e32ca8d` | `feat(store): settle a cancelled run as failed with no reason invented` |
| T3 | `fc539800` | `fix(connectors): close a run record whose run has ended when the run list is read` |
| T4 | `0d2745b1` | `docs: say what the two fixes changed, for GOV-12 and SRC-43` |

### T1 — `GOV-12` (#493 review, round 1 (R3))

Changed: `rust/crates/lakehouse-api/src/routes/quality.rs` (module comment,
`run_rule`'s signature, one `ensure_rule_table` call, its `# Errors`) and
`rust/crates/lakehouse-api/tests/quality_incidents.rs`.

Tests added (after
`a_caller_outside_the_catalog_tenant_is_refused_before_the_table_is_asked_about`),
one helper `rule_on(pool, name, asset, threshold)` with `rule` now calling it
with `silver.orders`:

1. `a_caller_outside_the_catalog_tenant_cannot_run_a_rule_and_nothing_is_measured`
2. `a_caller_whose_role_does_not_read_the_rules_table_cannot_run_it`
3. `running_a_rule_answers_the_same_for_a_table_that_is_not_there_and_one_the_caller_may_not_read`
4. `a_permitted_caller_still_runs_a_rule_and_gets_its_value`
5. `running_a_rule_whose_table_cannot_be_checked_is_a_503_and_measures_nothing`

Run before the fix (`cargo test -p lakehouse-api --test quality_incidents`):
`FAILED. 15 passed; 4 failed` — tests 1, 2, 3 answered `200
{"status":"failed","value":"75.0% not null"}` where 403/403/400 were expected,
and test 5 answered `400` (`"silver.broken_table" is not a table this can read
…`) where 503 was expected. Test 4 passed on the base, as the plan says. After
the fix: `19 passed; 0 failed` (18 of the file plus the `tls_proxy` test).

### T2 — `SRC-43` (#485 review, round 2 (R2))

Changed: `rust/crates/lakehouse-store/src/connector_run.rs` (module comment's
"Who writes", new `settle_cancelled_run` after `settle_run`) and
`rust/crates/lakehouse-store/tests/connector_run.rs` (`settle_cancelled_run`
and `LogLine` added to the imports).

Test added (after `the_sensor_settles_only_a_run_still_running`):
`a_cancelled_run_still_running_is_settled_as_failed_with_no_reason_invented`.

Must-fail check: the function does not exist on the base, so the test cannot
compile there. The planned bite check was made instead: `EndReason::Other`
switched to `EndReason::TableFailed` in `settle_cancelled_run` and the test run
alone answered `FAILED. 0 passed; 1 failed` (`left: Some("table_failed")`,
`right: Some("other")`); the value was put back and the file then answered
`12 passed; 0 failed` (11 of the file plus the new one — the store's
`connector_run` binary has no extra `tls_proxy` test, as 3a says).

### T3 — `SRC-43` (#485 review, round 2 (R2))

Changed: `rust/crates/lakehouse-api/src/routes/load_alerts.rs` (`FinalStatus`,
`Outcome::final_status`, `close_run_record`, `connector_run_event` now calling
it), `rust/crates/lakehouse-api/src/routes/connector_runs.rs`
(`RECONCILE_PER_READ`, `close_ended_runs`, `list`, the two "two places"
comments, the unit-test harness's dead `DAGSTER_URL` and comment),
`rust/crates/lakehouse-api/src/routes/uploads.rs` (`SETTLE_CALL_TIMEOUT` is
`pub(super)` and says the run list uses it too), and
`rust/crates/lakehouse-api/tests/ingest_run.rs`.

Tests added: one unit test in `load_alerts.rs`,
`only_success_failure_and_canceled_end_a_run_and_a_run_being_cancelled_has_not_ended`;
three helpers in `tests/ingest_run.rs` (`get_runs`, `mount_run_status`,
`run_status_requests`); and nine integration tests after
`an_orchestrator_that_cannot_be_reached_for_the_cancel_answers_503_and_starts_nothing`:

1. `a_record_left_running_after_its_run_was_cancelled_is_closed_by_one_read_of_the_run_list`
2. `a_run_cancelled_before_its_limit_is_closed_as_failed_with_no_reason_invented`
3. `a_record_left_running_after_its_run_succeeded_or_failed_is_closed_as_the_run_report_would_close_it`
4. `a_run_that_is_still_going_keeps_its_record_running`
5. `a_record_whose_run_the_orchestrator_does_not_know_is_left_running`
6. `the_run_list_still_answers_when_the_orchestrator_cannot_be_asked`
7. `two_reads_at_once_close_a_record_once`
8. `a_connector_with_no_running_record_asks_the_orchestrator_nothing`
9. `one_read_asks_about_at_most_ten_running_records`

Must-fail check (the T3 `src/` files stashed, the tests kept):
`FAILED. 30 passed; 8 failed` — tests 1, 2, 3 and 7 failed on the record
staying `running`, and tests 4, 5, 6 and 9 on the request counts; test 8 passed
on the base, as the plan says. Test 4 was also seen to bite on the status rule:
`FinalStatus::of` made to answer `Canceled` for `CANCELING` once answered
`FAILED` (`left: String("failed")`, `right: "running"`); put back. After the
fix: `38 passed; 0 failed` (37 of the file plus the `tls_proxy` test).

Tests that had to stay green, run and green:
`--lib only_success_failure_and_canceled_end_a_run` (1), `--lib a_failed_run_`
(4), `--lib a_paused_run_is_not_counted` (1), `--lib
the_end_report_cannot_write_the_timed_out_reason` (1), `--lib
the_history_route_answers_fifty_runs` (1) — none of their assertions changed.

### T4 — documents

`docs/core/features/data-quality-in-depth.md`,
`docs/core/features/connector-operations.md` and `CHANGELOG.md`, exactly as the
task says. `git diff --stat` of that commit: the three files, 13 insertions.
`BACKLOG.md`, `PRODUCT.md` and every Decisions table are untouched.

### Before handoff (final commit `0d2745b1`, foreground)

```text
cargo fmt --check                                              OK
cargo clippy -p lakehouse-store --all-targets -- -D warnings   ok (finished)
cargo clippy -p lakehouse-api --all-targets -- -D warnings     ok (finished)
cargo test -p lakehouse-store --test connector_run             12 passed; 0 failed
cargo test -p lakehouse-api --lib only_success_failure_…       1 passed; 0 failed
cargo test -p lakehouse-api --lib a_failed_run_                4 passed; 0 failed
cargo test -p lakehouse-api --lib a_paused_run_is_not_counted  1 passed; 0 failed
cargo test -p lakehouse-api --lib the_end_report_cannot_…      1 passed; 0 failed
cargo test -p lakehouse-api --lib the_history_route_…          1 passed; 0 failed
cargo test -p lakehouse-api --test quality_incidents          19 passed; 0 failed
cargo test -p lakehouse-api --test ingest_run                 38 passed; 0 failed
cargo test -p lakehouse-api --test sec11_guard                 4 passed; 0 failed
cargo test -p lakehouse-api --test route_auth                 93 passed; 0 failed
python3 ops/lint/check_bare_iceberg_count.py                  OK
python3 ops/lint/check_intra_package_imports.py               OK
git status                                                    clean
```

`tests/route_auth.rs`, `tests/sec11_guard.rs`, `POLICY_TABLE`,
`routes/mod.rs`, `policy.rs`, `dagster/`, `src/`, migrations: not edited.

### Docker

Before the first `cargo test --test …`: one container,
`24d7d1d1667e postgres:16-alpine upbeat_engelbart Up 7 hours` (not mine).
After the last run: the same one container and nothing else — the testcontainers
runs left nothing behind, and nothing was removed. I started no container of my
own.

### Not verified

- `cargo test --workspace` and `cargo nextest run --workspace`: *not verified
  locally, CI* (rule 7; the plan leaves it to the pull request).
- The store failure while a record is closed (the loop stops and the first list
  is returned): read only, not covered by a test, as T3 says.
- That a real orchestrator gives a cancelled run an end time, and that a real
  500 with a non-JSON body and a `RunNotFoundError` answer read as the tests'
  mocks do: the mocks only; *not verified against a running orchestrator*.
- Cheaper checks I did not run: `bun`, `pytest`, `docker compose config` — this
  slice touches no TypeScript, Python or compose file (section 3).

### MISMATCH

None. Every anchor was found by its quoted text and behaved as the plan says.

## 6. Review

### Round 1 (planner, 2026-11-13, at `e54b1a27`)

Reviewed by reading the whole diff against sections 1 to 4 (an independent reviewer's notes,
then the planner's rulings). Nothing was built on the planner's machine; the run is CI on the
pull request (#496), which had no failed check on `e54b1a27` when this was written and was
still going. The code does what the plan says and nothing the plan forbids was touched:
`run_rule` calls the unchanged `ensure_rule_table` after the 404 and before `evaluate`, the
move into `close_run_record` keeps the sensor's behaviour for `SUCCESS` and `FAILURE`, the
list read is bounded and can only change a record that still says `running`, and no
assertion of an existing test changed. Test counts by attribute: 18, 37 and 12, as the
handoff says.

One BLOCKER, a comment. Six smaller items. The fixes go in ONE commit, `fix(review): round 1
of leftovers-p3 (R1, R2, R3, R4, R6)`, after `main` is merged into the branch (it moved:
`be2c92910`).

- **R1. BLOCKER (rule 1): the doc of `close_run_record` says the opposite of what "Run now"
  does.** `routes/load_alerts.rs`, the doc comment above `close_run_record`: "a `CANCELED`
  run (cancelled before its limit, which is what "Run now" does) closes as failed with the
  reason `other`". "Run now" cancels only a run that is past its limit plus the grace; such a
  run closes as `timed_out` by the branch above (D12, and test 1 asserts it). Replace the
  parenthesis so the sentence reads: "… and a `CANCELED` run that was stopped before its
  limit (by something other than "Run now", which cancels only a run already past its limit
  and so closes as `timed_out`) closes as failed with the reason `other`." Change nothing
  else in that comment.
- **R2. SHOULD-FIX: test 1's second read asserts less than "the same answer".**
  `tests/ingest_run.rs`, `a_record_left_running_after_its_run_was_cancelled_is_closed_by_one_read_of_the_run_list`:
  after the second `GET`, compare the whole answers, `assert_eq!(again, listed, "a second
  read answers the same")` (use the test's own variable names), and keep the existing
  assertions. Where the store's record is checked, also assert its end time is set.
- **R3. SHOULD-FIX: `two_reads_at_once_close_a_record_once` does not show the reads
  overlapped.** Every assertion in it also holds when the second read starts after the first
  finished. In this test only: mount the run-status answer inline with a delay
  (`.set_delay(Duration::from_millis(300))` on the `ResponseTemplate`, same matcher and body
  as `mount_run_status`), and add `assert_eq!(run_status_requests(&server).await.len(), 2,
  "both reads saw the record running")`. If that count is not 2, do NOT change the number:
  report what it was in the handoff (it would mean the reads do not overlap, which is a
  finding about the test, not about the code).
- **R4. SHOULD-FIX: "the same answer for every rule and every table" is wider than the code.**
  `routes/quality.rs`, `run_rule`'s `# Errors`, the 403 line. The asset's shape is checked
  before the tenant rule (D4), so a stored rule whose asset is not a table key answers the
  400 to everyone. New text for the end of that line: "— the same answer whatever the table,
  for every rule whose asset is a table key; an asset that is not one is the `400` below for
  everyone." The plan's D5 row carried the wide wording: the planner's error, corrected here.
  No behaviour changes and nothing leaks (the stored text is on the rule list already).
- **R5. Accepted, written down (no code).** A run marked `CANCELED` in the orchestrator while
  its worker is still alive is closed by the next read of the run list as `failed` / `other`;
  if the load then completes, the job's own end report finds the record closed and answers
  `recorded: false`. Before this change the record stayed `running` and that report closed it
  as `succeeded`. "Run now" already had this property for an overdue run. Whether a worker
  can outlive a cancel was not established from this repository. D13 stands; this is added
  to open question 1 for the owner.
- **R6. SHOULD-FIX: two comments not brought along.** (a) `routes/connector_runs.rs`, the doc
  of `report_end`: "an end after the run sensor already closed the run" becomes "an end after
  the run sensor or a read of the run list already closed the run". (b)
  `routes/load_alerts.rs`, in `close_run_record`: the moved `SRC-11` D7 comment sits inside
  the `Success` arm but describes `settle_run` in both arms; move it above the `match`.
- **R7. Note, no change.** The body of `fc5398000` says "eight" integration tests; there are
  nine, as the handoff says. The history is not rewritten for it.

The developer's handoff for this round, appended below this entry: the commit, each command
with its counts (`cargo fmt --check`; `cargo clippy -p lakehouse-api --all-targets -- -D
warnings`; `cargo test -p lakehouse-api --test ingest_run`; `--test quality_incidents`;
`--test sec11_guard`; `--test route_auth`; `--lib only_success_failure_and_canceled_end_a_run`),
the count R3 saw, and the result of merging `main` (conflicts, if any, by file).

### Round 1, developer's handoff

Developer, 2026-11-13, on the ARM64 build machine, checkout
`/home/rismana/rantai/lakehouse-agents-2`, `CARGO_TARGET_DIR=/home/rismana/rantai/cargo-target-2`
and `CARGO_BUILD_JOBS=6` in every shell that ran cargo.

#### Commits

- `9effb862` — "Merge remote-tracking branch 'origin/main' into
  fix/qa-round-leftovers-p3". `origin/main` had moved to `eaebacf2` (BLD-1 part 1 S1 plus its
  document commit). **No conflict**: git merged it with no conflicted file. The branch had
  nothing of its own in the files `main` touched there (`pipelines.rs`, `pipeline_compile.rs`,
  the pipeline test files, migration `0140`, `sql_guard.rs`, `route_auth.rs`, `sec11_guard.rs`).
  After the merge, from `rust/`: `cargo fmt --check` clean; `cargo clippy -p lakehouse-api
  --all-targets -- -D warnings` and `cargo clippy -p lakehouse-store --all-targets -- -D
  warnings` both finished without a warning.
- `6b3b842c` — "fix(review): round 1 of leftovers-p3 (R1, R2, R3, R4, R6)". Four files:
  `routes/load_alerts.rs` (R1, R6b), `tests/ingest_run.rs` (R2, R3),
  `routes/quality.rs` (R4), `routes/connector_runs.rs` (R6a). R5 and R7 needed no change.
  No assertion of an existing test changed and no test, lint or allowlist was weakened.

#### Checks (foreground, from `rust/`, on `6b3b842c`)

| Command | Result |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo clippy -p lakehouse-api --all-targets -- -D warnings` | finished, no warning |
| `cargo test -p lakehouse-api --lib only_success_failure_and_canceled_end_a_run` | 1 passed, 0 failed, 3623 filtered out |
| `cargo test -p lakehouse-api --test ingest_run` | 38 passed, 0 failed |
| `cargo test -p lakehouse-api --test quality_incidents` | 19 passed, 0 failed |
| `cargo test -p lakehouse-api --test sec11_guard` | 4 passed, 0 failed |
| `cargo test -p lakehouse-api --test route_auth` | 94 passed, 0 failed |

`ingest_run` and `quality_incidents` keep their one extra test from `src/tls_proxy.rs`, as
section 3a says. `sec11_guard` is 4 here: `main` removed one of its tests in the merge
(`tests/sec11_guard.rs`, 5 lines, in `be2c9291`'s slice), not this branch.

**R3: count was 2.** The reads do overlap with the 300 ms answer: both asked the orchestrator
before either closed the record, and the test passes with the assertion unweakened.

#### Not verified

- `cargo test --workspace` was not run (section 3a forbids it); CI on the pull request runs it.
- The rest of the section 3 block above (`--test connector_run`, the other four `--lib`
  filters, the two `ops/lint` scripts) was **not** re-run for this round: no file they cover
  changed after `e54b1a27`, and R1 to R6 touch docs, one test file and no query.
- Nothing under `dagster/` or `docker-compose.yml` was touched, so neither was run.

#### Docker

`docker ps -a` before the first integration test showed one container that was already
there and is not mine to remove: `24d7d1d1667e` `postgres:16-alpine`, up 7 hours,
`0.0.0.0:33908->5432/tcp`, name `upbeat_engelbart`. It is still there after the runs; the
test harness starts nothing of its own here, so this round started and removed no container.

### Round 2 (planner, 2026-11-13, at `7230334d`)

Read: the fix commit `6b3b842c` (four files, 45 insertions, 20 deletions) and the merge of
`main` (`9effb862`, no conflict).

- R1: fixed. The doc of `close_run_record` now says a `CANCELED` run stopped before its limit
  closes as `other`, and that "Run now" cancels only a run already past its limit.
- R2: fixed. The second read's whole answer is compared with the first, and the stored
  record's end time is asserted.
- R3: fixed. The run-status answer takes 300 ms and the test asserts two status requests; the
  developer's run saw 2, so the two reads do overlap and "closed once, one line" is now about
  the row lock.
- R4: fixed, as worded.
- R6: fixed, both parts.
- R5, R7: no change was asked.

No existing assertion changed in the fix commit. The developer's counts on `6b3b842c`:
`ingest_run` 38, `quality_incidents` 19, `sec11_guard` 4, `route_auth` 94 (one more than
before the merge: `main` added a route test), clippy and fmt clean. Not run by the planner;
CI on the pull request is the run that decides.

**No open BLOCKER.** The pull request is marked ready; it merges when CI is green on its
head and it is up to date with `main`.
