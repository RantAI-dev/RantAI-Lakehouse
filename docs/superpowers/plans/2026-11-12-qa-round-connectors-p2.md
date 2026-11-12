# QA round, Connectors and alerts (P2): `SRC-40`, `SRC-41`

Fixes for two backlog rows of the QA round of 2026-11-02 to 11-12, read against the head of
branch `fix/qa-round-connectors-p1` at `5e316b426` on 2026-11-12 (pull request #485, reviewed
with no open blocker, about to merge). Every anchor below was verified there; the backlog rows
were read at `657c2502` and their anchors were re-read.

- `SRC-40` (QA `TC-1695`, `TC-1696`, `LH-SRC-7-10`), the Alerts page: a "run succeeded" alert
  is delivered but never shown as sent; the rule form prints the API's raw refusal
  (`permission_denied: connector:manage`); "New rule" is offered to a person who may not
  create a rule.
- `SRC-41` (QA `LH-SRC-3-11`, `LH-SRC-3-08`, `LH-SRC-6-16`, `LH-SRC-6-39`, `LH-SRC-6-54`,
  `LH-SRC-6-19`, `LH-SRC-6-50`, `LH-SRC-6-55`, `TC-1712`), the new-connector screens: eight
  small findings, one commit each.

Base: `main` **after #485 is merged**. Do not start before it is; branch from `main` then.
Branch: `fix/qa-round-connectors-p2`. One pull request for the slice, opened as a draft by the
planner once the first commits are pushed (CI on the pull request, x86_64, is the final
word). Push after every commit.

Planner and reviewer: Claude Opus. Developer: another agent (not Opus). The developer writes
the code and tests of section 2, in order, one task per commit, and changes documents only
where a task says so. If an anchor or a statement below does not match the code, stop and
report it in the handoff instead of improvising.

No route is added and no route's policy changes, so `POLICY_TABLE` and
`rust/crates/lakehouse-api/tests/route_auth.rs` stay as they are. One migration is added (T1).

## 1. Decisions

### Both rows

- **D1. One task per commit, in the order of section 2** (Rust together, then Python, then
  the console, then documents). Each commit body cites the QA case IDs and the backlog ID.
  Each fix site carries a short comment naming the case (rule 13), for example
  `// QA TC-1695 (SRC-40): ...` or `# QA TC-1712 (SRC-41): ...`.
- **D2. No new helper where one exists** (rule 4). Named in the tasks:
  `overview::DeliveryRecord` (`rust/crates/lakehouse-store/src/overview.rs:153`),
  `load_alerts::delivery_record` (`rust/crates/lakehouse-api/src/routes/load_alerts.rs:375`),
  `alerts::forget_state` (`routes/alerts.rs:811`), `alerts::listed_rules` (`:137`),
  `chart_alerts::may_manage` (`routes/chart_alerts.rs:308`), `check_dial_ssrf` and
  `check_oauth_addresses` (`routes/connectors.rs:3145`, `:3270`), `useAuth().hasPermission`
  (`src/features/auth/auth-provider.tsx:169`), `readableErrorMessage`
  (`src/lib/notify-message.ts:65`), `deliverySummary`
  (`src/features/overview/alert-delivery.tsx:14`), `formatDateTime` (`src/lib/format.ts:94`),
  `usesNoCredential` (`src/features/connectors/connector-form-parts.tsx:382`), `EmptyState`
  and `ErrorState` (`src/components/patterns/page-states.tsx:44`, `:77`), `SAAS_VENDORS`
  (`src/lib/connectors/saas-vendors.ts:47`).
- **D3. The assistant, in one place.** Four tasks touch code the assistant's tools also call
  (`rust/crates/lakehouse-api/src/routes/ai/tools/`). None changes a tool's name, description
  or parameters, so `rust/crates/lakehouse-api/tests/fixtures/tool_schemas.json` is not
  regenerated (it holds the tools' input schemas only). What each tool sees is said in its
  task: `list_alert_rules` and `delete_alert_rule` (T2), `create_connector` and
  `set_ingest_spec` (T4), `test_connector` (T6). `discover_connector` goes through
  `connector_discover.rs`, which T5 does not touch.

### `SRC-40`: the Alerts page

- **D4. Where a "run succeeded" alert is recorded: on its rule, in a table of its own.** Read
  at `5e316b426`: the only store of a delivery's outcome is the incident table
  (`alert_instance.delivery_status`, `delivery_reason`, migration `0068`), written by
  `persist_instances` (`load_alerts.rs:388-416`) when `deliver_with_rules` is given a source
  (`:332-334`). The success arm passes `None` (`:528`), so nothing is written. An
  `alert_instance` row is an incident: it is listed under "Active Incidents", it is `open` at
  birth, and every open row lights the bell (`routes/notifications.rs:63-76`), the home page
  and the system status page. `list_alerts` reads the whole table with no limit
  (`overview.rs:167-174`), on every bell poll. A connector on a 15-minute schedule would add
  96 rows a day that are not incidents. The other per-rule table, `alert_rule_state`
  (migration `0113`), is the threshold alerts' state machine and its `state` has a CHECK of
  three words. So: a new table `alert_rule_delivery`, one row per rule, holding the rule's
  **latest** delivery attempt: when, delivered or not, and the fixed reason when not. This is
  the narrowest record that makes "sent at … / not delivered: …" true for a success rule, and
  it opens no incident. It needs a migration (T1).
- **D5. What is written, and when.** After the deliveries of a success event,
  `deliver_with_rules` records one row per rule a delivery was attempted for. Best effort,
  like `persist_instances`: a Postgres failure is logged and does not turn a delivered alert
  into an error. A newer attempt replaces the row; an older `at` never replaces a newer one.
  A silenced rule is not attempted, so nothing is written for it. A failed delivery with no
  reason writes nothing (`delivery_record` returns `None`, `load_alerts.rs:375-381`:
  no invented reason). The reason is one of the ten fixed sentences of
  `lakehouse_notify::DeliveryFailure::message` (`rust/crates/lakehouse-notify/src/lib.rs:76-89`);
  none holds an address or text from the other side. Only the success kind writes this row:
  every other kind's delivery is on its incident, where it is today. The row is removed when
  the rule is deleted, and when an edit changes what the rule watches (`forget_state`).
- **D6. Who reads it.** `listed_rules` (`routes/alerts.rs:137-164`) adds
  `"lastDelivery": { "at": "...", "delivered": true }` or
  `{ "at": "...", "delivered": false, "reason": "..." }` to a rule that has a row, for every
  caller who may list the rule (the incident list already shows the same two facts to
  `alert:read`). A rule with no row has no key. A store that cannot be read is logged and
  leaves the key out; the list still answers.
- **D7. Text on screen for the record (English, as the console).** In the Rules list, under
  the Delivery cell's address or channel: **"Last sent {date and time}"** when it arrived;
  **"Last attempt {date and time} · Not delivered: {reason}"** when it did not (the words
  after the dot are `deliverySummary`'s, the same ones the incident list uses).
- **D8. The rule form says a refusal in words.** The connector picker of the rule form reads
  `GET /api/connectors`, which takes `connector:manage`
  (`rust/crates/lakehouse-api/src/policy.rs:937`); a person with `alert:write` and without it
  sees the raw text. For that refusal the form says **"You do not have permission to choose a
  connector for this rule."** Every other error text the form and the list print (marts,
  boards, measures, the save, the list's red banner) goes through `readableErrorMessage`, the
  shared translation, so no text that starts with one of the API's codes is shown.
- **D9. "New rule" and the switch.** "New rule" is rendered only for a person with
  `alert:write`: the page it opens already requires it
  (`src/components/app-shell/nav-config.ts:348`). The status switch is rendered only for a
  rule the caller may manage, which is the API's own per-rule answer (`targetHidden` absent:
  `alert:write`, or the owner of a chart alert for their own rule, #485 D16). For any other
  rule the cell shows the state as text, "Active" or "Disabled", and no switch. **This
  replaces one sentence of #485's D19** ("the switch is shown but disabled"). Keying the
  switch on `alert:write` alone would take it from the owner of a chart alert, whom the API
  lets switch their own rule. #485's hidden target and hidden actions stay as they are.
- **D10. The server already refuses each of those writes; one pin is added.** `POST`, `PUT`
  and `DELETE /api/alerts` are `RequiresAuth` in `POLICY_TABLE` (`policy.rs:428-430`) and the
  handler decides: `chart_alerts::authorise_save` for a save, `may_manage` for a delete
  (`routes/alerts.rs:785-792`). Proven today: `POST` without `alert:write` is 403
  (`tests/route_auth.rs:403-410`), `PUT` of a rule that is not the caller's own chart alert
  is 403 (`tests/chart_alerts.rs:545-552`). `DELETE` of such a rule has no test; T3 adds it.

### `SRC-41`: the new-connector screens

- **D11. The connection settings are checked before the connector is created.** Today the
  wizard creates the row (`POST /api/connectors`) and then saves the settings
  (`PUT /api/connectors/{id}/ingest-spec`); the settings are checked only in the second call
  (`routes/connectors.rs:3472-3473`, then `check_oauth_addresses`, then `check_dial_ssrf`), so
  a refusal leaves a connector with no settings. `POST /api/connectors` gains an optional
  body field `connection: { adapter, dial }`. When it is present the handler runs the same
  three checks **before** `connectors::create_connector` (`:811`); a refusal is a 400 and no
  row exists. The console always sends it. The second call stays and checks again (it is the
  one that stores the settings). A save that fails for another reason after a good check (the
  database) still leaves the row, and the created screen says so as today.
- **D12. The sentence.** A refused `dial` answers **"The connection settings are not valid:
  {reason}"**, where `{reason}` is the validator's own text: one of the fixed sentences
  written in `rust/crates/lakehouse-store/src/ingest_spec.rs` (they start with the field's
  name), or serde's field text (``missing field `host` ``). The words "dial for adapter" and
  the adapter's name are gone. Nothing has been dialled when this is produced, so the reason
  holds no text from a source and no resolved address. It can repeat a value the caller typed
  in the same request (a host that is not a hostname, `ingest_spec.rs:2330`), as
  `resolve_checked`'s refusal does (`SEC-15`, `connector_probe.rs:1401-1405`); that is not
  changed. For the
  business-app account the reason is **"account is not in the form this vendor uses"**: the
  doubled "account: account …" (`ingest_spec.rs:1470`) is corrected. An unknown adapter
  answers **"The connection settings name a connector type this build does not know."**
  An internal or unresolvable address keeps `resolve_checked`'s own sentence
  (`rust/crates/lakehouse-api/src/connector_probe.rs:1382-1390`, `:1421`).
- **D13. The form disables Next where it can know.** The console holds the vendor table with
  each vendor's account rule (`saas-vendors.ts`, `account.kind`). For a business-app
  connector the Connection step's Next is disabled while the account does not fit the rule,
  and, once something is typed, the reason is shown under the form: **"This is not in the
  form {vendor label} uses."** The rule is a copy of `SaasVendor::validate_account`
  (`rust/crates/lakehouse-store/src/saas_vendors.rs:245-315`), given in T11; the server stays
  the authority (D11). For every other connector type the form cannot know whether an
  address is internal (it is the installation's setting), so Next is not changed there; the
  server's refusal on Create now comes before anything exists.
- **D14. The created screen waits for the save.** `handleSubmit` sets `createdId` and only
  then awaits the save (`src/features/connectors/connector-create-page.tsx:242-247`). The
  created screen mounts `ConnectorIngestPanel` at once (`:349-353`), which reads
  `GET .../ingest-spec` once (`connector-ingest-panel.tsx:372`) and never again. When that
  read answers before the `PUT` lands, the panel says "This connector has no connection
  settings saved yet" (`:387-398`) and stays that way. Which request wins is a race, which is
  why it is intermittent. Fix: the panel (and the sign-in panel) are mounted only when the
  save has answered; until then the card reads **"Saving the connection settings…"**.
- **D15. One truth about a connector with no credential.** The connector page's Credential
  fact is built from `credentialSource` alone (`connector-detail-page.tsx:67-80`, `:94`). A
  Kafka connector with authentication "none" is created with `source: "managed"` and no
  value (`connector-create-page.tsx:209-225`), so the fact reads "Stored by lakehouse,
  encrypted" while the review step says "No credential is stored". The page already loads the
  ingest spec (`:238`); when it says the connection reads no credential (`usesNoCredential`)
  the fact reads **"None: this connection authenticates with none"**. While the spec is
  loading the fact is **"—"** (not a claim it may have to take back); if the spec cannot be
  read the fact is today's.
- **D16. A source that does not answer is given up on after 30 seconds** *(proposed, see Open
  questions)*. What the code shows, per source of `TC-1712`:
  - Object storage is listed by the API. The S3 client is built on a pinned HTTP client with
    no connect or read timeout (`connector_probe.rs:1096-1112`: `object_store`'s own 5 s and
    30 s defaults are in the options this connector ignores), so a silent endpoint is waited
    for until `LIST_TIMEOUT`, 120 s (`rust/crates/lakehouse-api/src/connector_listing.rs:78`,
    `:1137`). Fix: the first answer of a listing or a preview must arrive within
    `FIRST_ANSWER_TIMEOUT`, 30 s; past it the answer is the class `timeout`, the same class
    and sentence the 120 s limit gives today. The 120 s for the whole listing stays (`SRC-13`
    decision 9, a bucket of 10,000 files).
  - SFTP is listed by the orchestrator. `paramiko`'s `connect` is called with no `timeout`
    (`dagster/dispar_orchestrate/adapters/sftp.py:80-84`; `paramiko` 3.5.0's default is
    `None`), so a silent host is waited for until the operating system gives up (about two
    minutes on Linux). Fix: `timeout=30`. `paramiko` re-raises a connect timeout as it is
    (`paramiko/client.py:383-397` in 3.5.0), Python's `TimeoutError`, which is the class
    `timeout` for a listing and for a load (`ingest_errors.py:510`), where it is retried. The same function opens a load's session, so a load gets the same bound.
  - A REST preview is read by the orchestrator with `timeout=30`
    (`adapters/rest.py:487`) and a token request with 10 s (`:110`): already bounded.
  - Why 30: it is the number the code already uses for one request to a source
    (`rest.py:487`; Kafka's `request_timeout_ms=30_000`, `discover.py:494`; and `sqlx`'s
    30 s `acquire_timeout`, under which a SQL listing's connect already ends,
    `connector_listing.rs:806-851`). The connection test's 5 s (`DIAL_TIMEOUT`) is for a test,
    not for a listing over a slow link.
  - *Not verified*: QA's 3 min 40 s for a REST preview and the part of SFTP's 4 minutes
    beyond the connect are not explained by any dial timeout in the code. A listing the
    orchestrator reads is a run of `discover_job` (queued, started, the code location
    imported), and the console waits for it up to five minutes
    (`rust/crates/lakehouse-store/src/connector_discovery.rs:104`). How long that start took on
    the QA instance was not measured and is not changed here. Also not explained: QA's 3
    minutes for object storage, when the API's own limit for a listing is 120 s, inside a
    request limit of 130 s (`routes/mod.rs:326`).
- **D17. A run's status follows the orchestrator's report; the cause is written down and the
  page says so. Nothing is retimed.** Read in the code: a batch run does not report its own
  end. Each table's rows are reported as its step ends, and the record is closed only when the
  orchestrator's success sensor posts the run (`ingest_factory.py:118-123`;
  `pipeline_events.py:254-260`; `connector_run_event` → `settle_run`,
  `load_alerts.rs:476-483`). That report waits for three things in the pinned
  `dagster==1.13.20`: the sensor is evaluated at most every 30 seconds
  (`minimum_interval_seconds`, not set at `pipeline_events.py:254`, default
  `DEFAULT_SENSOR_DAEMON_INTERVAL = 30`); one evaluation handles at most 5 ended runs of
  **all** jobs of the code location (`DAGSTER_RUN_STATUS_SENSOR_PROCESS_LIMIT`, default 5;
  every listing or preview the orchestrator reads, and every Oracle or MongoDB connection
  test, is a `discover_job` run and counts); and the sensor daemon evaluates the sensors one
  after another (`sensors.use_threads` in `dagster/dagster.yaml`, not set, default `false`).
  And it needs `PIPELINE_RUN_TOKEN`. *Not verified*: which of the three made five to six minutes on the QA
  instance; it was not measured, so no number is changed (principle 5). The Runs card says,
  while a run is going and at least one attempt has ended: **"The status changes when the
  orchestrator reports the end of the run. That can be a few minutes after the last table's
  rows are counted."** The three settings are named on the feature page (T17).
- **D18. One is singular.** "1 file found", "1 endpoint found", and so on for every kind.
- **D19. A MariaDB connector's test names MariaDB.** The label is one of two constants,
  chosen by whether the connector's type is exactly `MariaDB` (the type's name in migration
  `0035`); the type's text itself is never put into the sentence.
- **D20. The New Connector page shows a refusal instead of the form.** When the list of
  types cannot be read, the page shows only the header and the state: for a refused read,
  **"You cannot create a connector"** / **"Creating a connector needs the connector:manage
  permission. Ask a workspace administrator."**; for any other failure, `ErrorState` with
  Retry. No Name, Direction, Previous or Next. The server refuses the create for the same
  person already (`POST /api/connectors` is `connector:manage`, `policy.rs:959`).

## 2. Tasks

### T1. A table for a rule's latest delivery (`TC-1695`; `SRC-40`)

- Anchors: `rust/migrations/` (highest at `5e316b426`: `0137_connector_run_timed_out.sql`);
  `rust/migrations/0068_alert_instance_delivery_outcome.sql` (the two words and the reason
  rule to copy); `rust/crates/lakehouse-store/src/overview.rs:20` (`iso_millis`), `:146-148`
  (`DELIVERED`, `NOT_DELIVERED`), `:153-160` (`DeliveryRecord`);
  `rust/crates/lakehouse-store/src/alert_state.rs` (the module to model on);
  `rust/crates/lakehouse-store/src/lib.rs:36`.
- Change:
  1. A new migration with the next free number: `0138_alert_rule_delivery.sql` at
     `5e316b426`. Another slice of this round also adds a migration, so run
     `ls rust/migrations | tail -1` on the branch first, and again after any rebase; take
     the next number, and never edit a migration that is already on `main`. A why-header
     that says what D4 says (why not `alert_instance`, why not `alert_rule_state`, that
     `rule_id` names a ClickHouse row and so has no foreign key, that only the latest attempt
     is kept, that the reason is a fixed sentence and never the other side's text). Then:

     ```sql
     CREATE TABLE alert_rule_delivery (
         rule_id         TEXT PRIMARY KEY,
         at              TIMESTAMPTZ NOT NULL,
         delivery_status TEXT NOT NULL,
         delivery_reason TEXT,
         CONSTRAINT alert_rule_delivery_status_check
             CHECK (delivery_status IN ('delivered', 'not_delivered')),
         CONSTRAINT alert_rule_delivery_reason_check
             CHECK (delivery_reason IS NULL OR delivery_status = 'not_delivered')
     );
     ```
  2. In `overview.rs` make `iso_millis`, `DELIVERED` and `NOT_DELIVERED` `pub(crate)`.
     Nothing else in that file changes.
  3. A new module `rust/crates/lakehouse-store/src/alert_delivery.rs`, added to `lib.rs` in
     alphabetical place (before `alert_state`), with a module doc that says D4 and D5, and:
     - `pub struct LastDelivery { pub at: String, pub delivered: bool, pub reason: Option<String> }`,
       `Serialize` with `rename_all = "camelCase"`, `reason` skipped when `None`. `at` is
       `overview::iso_millis` of the stored time.
     - `pub async fn record(pool, rule_id: &str, delivery: DeliveryRecord<'_>, at: OffsetDateTime) -> Result<(), StoreError>`:
       one `INSERT ... ON CONFLICT (rule_id) DO UPDATE SET at = EXCLUDED.at, delivery_status = EXCLUDED.delivery_status, delivery_reason = EXCLUDED.delivery_reason WHERE alert_rule_delivery.at <= EXCLUDED.at`.
       Values bound; the status words are the two constants.
     - `pub async fn last_for(pool, rule_ids: &[String]) -> Result<HashMap<String, LastDelivery>, StoreError>`:
       one `SELECT ... WHERE rule_id = ANY($1)`. A stored status other than the two words is
       left out of the map (not known, as `overview.rs:131-135` does).
     - `pub async fn delete(pool, rule_id: &str) -> Result<(), StoreError>`.
     Every `Result` function has `# Errors`.
- Check: a new file `rust/crates/lakehouse-store/tests/alert_delivery.rs`
  (`#[sqlx::test(migrations = "../../migrations")]`, `use lakehouse_test_support as _;`, as
  `tests/alert_state.rs`):
  - `a_rule_never_delivered_to_has_no_row`: `last_for` of an unknown id is an empty map.
  - `the_latest_attempt_replaces_the_one_before_it`: record `NotDelivered("the webhook server
    answered with an error")` at `t`, then `Delivered` at `t + 60 s`; the map holds
    `delivered: true`, no reason, and `at` of the second.
  - `an_older_attempt_never_replaces_a_newer_one`: record `Delivered` at `t + 60 s`, then
    `NotDelivered(..)` at `t`; the row is still the delivered one. (Fails if the `WHERE` of
    the upsert is removed.)
  - `a_deleted_rules_row_is_gone`: record, `delete`, `last_for` is empty.
  - `the_table_refuses_a_reason_on_a_delivered_row`: a raw `INSERT` with
    `('r', now(), 'delivered', 'x')` is an error. (Fails if the second CHECK is dropped.)

### T2. A "run succeeded" alert is recorded on its rule and listed with it (`TC-1695`; `SRC-40`)

- Anchors: `rust/crates/lakehouse-api/src/routes/load_alerts.rs:262-292` (`deliver_event` and
  its doc), `:303-336` (`deliver_with_rules`; the branch at `:332-334`), `:375-381`
  (`delivery_record`), `:383-416` (`persist_instances`, the posture to copy), `:519-531` (the
  success arm, `None` at `:528`); `rust/crates/lakehouse-api/src/routes/alerts.rs:120-164`
  (`listed_rules` and its doc), `:805-817` (`forget_state`).
- Change:
  1. `load_alerts.rs`: a new private `async fn record_last_deliveries(pool, attempted: &[RuleDelivery])`
     beside `persist_instances`: for each attempted delivery, `delivery_record(result)`; when
     it is `Some`, `lakehouse_store::alert_delivery::record(pool, &rule.id, record, now)`; a
     failure is `tracing::warn!` with the rule id and nothing else (the delivery already
     happened). In `deliver_with_rules`, the `if let Some(source) = persist_as { .. }` gains
     an `else { record_last_deliveries(pool, &attempted).await; }`. Rewrite the doc comments
     of `deliver_event` and `deliver_with_rules`: every attempted delivery leaves a record,
     an incident when `persist_as` names a source, the rule's latest delivery when it is
     `None`; and `None` is passed by the success event only (verified: the callers outside
     this file, `schema_changes.rs:568`, `:617` and `capture_lag.rs:266`, all pass `Some`).
     The success arm keeps `None`; add the QA comment there.
  2. `alerts.rs`, `listed_rules`: after `visible_rules`, when `state.pg` is `Some`, read
     `alert_delivery::last_for` once for the ids of the visible rules. For each rule that
     has an entry insert `"lastDelivery"` with the serialized `LastDelivery` into the rule's
     JSON object (beside the `chartTitle` insert). No pool, or a read that fails: log
     `tracing::warn!` and insert nothing. Add D6 to the function's doc.
  3. `alerts.rs`, `forget_state`: after the `alert_state::delete`, also
     `alert_delivery::delete(pg, id)`, logged the same way when it fails. Extend its doc.
- Assistant: `list_alert_rules` returns `listed_rules`' JSON
  (`routes/ai/tools/alerts.rs:108-121`), so each rule it lists gains `lastDelivery` when
  there is one. `delete_alert_rule` calls `forget_state` (`routes/ai/tools/alerts.rs:197`),
  so it also removes the row. No tool schema changes.
- Check:
  - In `load_alerts.rs`'s own tests (`#[sqlx::test]`, the `harness` of `:753`; model on
    `a_success_resets_the_streak_and_alerts_only_a_rule_for_that_connector`, `:1265`):
    - `a_success_alert_is_recorded_as_the_rules_last_delivery_and_opens_no_incident`: one
      `connector_success` rule `al-s` for the connector; the run is `SUCCESS`; post
      `run_finished_event`. Then `alert_delivery::last_for(&pool, &["al-s".to_owned()])`
      holds `delivered: true` and no reason, and `overview::list_alerts(&pool)` is empty.
      (Fails on today's code: no row. Fails if the `else` branch is removed, or if the
      success arm is given a source, which would list an incident.)
    - `a_success_alert_whose_webhook_answers_an_error_is_recorded_as_not_delivered`: the
      same with the webhook answering 500 for `/al-s` (the mock of `:983-988`, with a body
      `upstream-secret-detail`); the row is `delivered: false` with the reason
      `the webhook server answered with an error`, and the serialized row does not contain
      `upstream-secret-detail`.
    - `a_failed_run_still_opens_an_incident_and_records_no_last_delivery`: a
      `connector_failure` rule `al-f`, a `FAILURE` run; `list_alerts` has one row for `al-f`
      and `last_for(["al-f"])` is empty. (Fails if the new branch also runs for a source.)
  - In `rust/crates/lakehouse-api/tests/alert_targets_hidden.rs` (the fixture of `:76`,
    rule `al_1`):
    - `the_rule_list_and_the_assistants_list_carry_a_rules_last_delivery`: call
      `lakehouse_store::alert_delivery::record(&f.app.pool, RULE, DeliveryRecord::NotDelivered("the webhook server answered with an error"), now)`;
      then for the reader and for the writer, `GET /api/alerts` and `list_alert_rules` both
      hold `lastDelivery.delivered == false`, the reason, and a string `at`; and for the
      reader the body still does not contain `ADDRESS`. (Fails if the insert in
      `listed_rules` is removed.)
    - `a_rule_never_delivered_to_has_no_last_delivery_key`: with no row,
      `rule.get("lastDelivery")` is `None`.

### T3. A person without `alert:write` cannot delete a rule (a pin) (`TC-1696`; `SRC-40`)

- Anchors: `rust/crates/lakehouse-api/src/routes/alerts.rs:773-804` (`delete`; the refusal at
  `:785-792`); `rust/crates/lakehouse-api/tests/alert_targets_hidden.rs:47-66` (`FakeCh`, its
  `log`), `:97-116` (`get`).
- Change: tests only. Keep the fake's `log` reachable from the fixture (add a field to
  `Fixture`), and add a small `send(f, who, method, uri)` beside `get` (or give `get` a
  method argument).
- Check: `a_caller_without_alert_write_cannot_delete_a_rule`: as the reader,
  `DELETE /api/alerts?id=al_1` is 403 and the log holds no statement containing
  `INSERT INTO console.alert_rule` (a delete is that statement,
  `rust/crates/lakehouse-alerts/src/lib.rs:1444-1451`); as the writer the same request is
  200 and the log holds one. This is a pin: it passes on today's code. It fails if the
  `may_manage` check at `alerts.rs:785-792` is removed. If the reader's answer is not 403,
  stop and report it; do not change the assertion.

### T4. Connection settings are checked before the connector is created, and a refusal names the field (`LH-SRC-3-11`; `SRC-41`)

- Anchors: `rust/crates/lakehouse-store/src/ingest_spec.rs:1460-1470`
  (`validate_saas_dial_post_parse`; the doubled wording at `:1470`), `:1598-1629`
  (`IngestSpecError`); `rust/crates/lakehouse-api/src/routes/connectors.rs:611-639`
  (`CreateConnectorBody`), `:768-811` (`create` up to `create_connector`), `:3145`
  (`check_dial_ssrf`), `:3270` (`check_oauth_addresses`), `:3472-3473` (the save's
  `Dial::parse`); `rust/crates/lakehouse-api/tests/sec11_guard.rs:37` (`ALLOWED`), `:266-270`
  (the existing `"dial: {err}"` entry), `:281-285`.
- Change:
  1. `ingest_spec.rs:1470`: `refuse(&format!("account: {err}"))` becomes
     `refuse(&err.to_string())`. `InvalidAccount`'s text already names the field
     (`saas_vendors.rs:246`).
  2. `connectors.rs`: a private `fn dial_refusal(err: &IngestSpecError) -> ApiError` with the
     two sentences of D12, both `ApiError::BadRequest`: for `UnknownAdapter` the fixed
     sentence; for `InvalidDial { source, .. }`
     `format!("The connection settings are not valid: {source}")`. Its doc says what D12
     says about what `source` can hold. Use it at `:3473` in place of
     `ApiError::BadRequest(err.to_string())`.
  3. `connectors.rs`: `CreateConnectorBody` gains
     `#[serde(default)] connection: Option<NewConnectionBody>`, with
     `struct NewConnectionBody { adapter: String, #[serde(default)] dial: serde_json::Value }`
     (`rename_all = "camelCase"`, documented). A private
     `async fn check_new_connection(state: &AppState, connection: &NewConnectionBody) -> Result<(), ApiError>`:
     `Dial::parse(&connection.adapter, &connection.dial).map_err(|err| dial_refusal(&err))?`,
     then `check_oauth_addresses(..)`, then `check_dial_ssrf(..)`, with the same arguments as
     the save passes (`:3497-3502`, `:3542-3547`). In `create`, call it right after the
     direction check (`:780-785`) and before `check_create_credential` (`:787`), when
     `body.connection` is `Some`. Add to `create`'s `# Errors`: 400 when `connection` is
     sent and refused, and nothing is created.
  4. `tests/sec11_guard.rs`: the guard flags the `{source}` line. Add one `ALLOWED` entry:
     file `connectors.rs`, substring `The connection settings are not valid: {source}`,
     reason: "IngestSpecError::InvalidDial's source: a fixed sentence of ingest_spec.rs or
     serde's field text about the caller's own dial; nothing is dialled before it, so it
     holds no upstream text and no resolved address (it can repeat a host the caller typed)".
     This is the guard's own procedure for text written in this repository, not a weakening
     (rule 2). The existing entry
     `.map_err(|err| ApiError::BadRequest(err.to_string()))?;` still matches `:3466`
     (`validate_load_modes`), so it does not go stale.
- Assistant: `create_connector` passes its arguments to `routes::connectors::create` as they
  are (`routes/ai/tools/connectors.rs:69-100`). Its schema offers no `connection`
  (`routes/ai/registry.rs:1363-1382`), so its calls are unchanged; if a model sent one, it
  would be checked like the console's. `set_ingest_spec` calls `ingest_spec_put`
  (`routes/ai/tools/ingest.rs:72`) and now gets the D12 sentence for a refused `dial`.
- Check:
  - In `ingest_spec.rs`'s tests (beside `:4759`):
    `a_refused_saas_account_names_the_field_once`: `Dial::parse("saas", ..)` for
    `dynamics365` with the account `127.0.0.1` is `InvalidDial` whose `source.to_string()`
    is exactly `account is not in the form this vendor uses`. (Fails on today's code:
    "account: account …".)
  - A new file `rust/crates/lakehouse-api/tests/connector_create_checks.rs`, with a fixture
    and a `send` of the shape of `tests/connector_repoint.rs:57-75` and `:107-132` (the
    seeded Data Engineer's cookie, `CONNECTOR_SECRETS_DIR` in a temporary directory,
    `CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS` on request). A create body that is valid without
    `connection`: name, `"type": "PostgreSQL"`, `"direction": "source"`, `"host": "h"`,
    `"credential": {"source": "managed", "primary": "password", "values": {"primary": "pw"}}`,
    `"environment": "production"`, `"tenant": "g6"`, the seeded tenant id as `tenantId`
    (`tests/connector_secret_reference.rs:35`, `:416-427`), `"residency": ""`,
    `"capabilities": []`. Count rows with `SELECT count(*) FROM connector WHERE name = $1`.
    If the control test's body is not accepted as written (a 400 for another reason), read
    `create`'s own checks, correct the body, and say what you changed; the three refusal
    assertions are not to be changed.
    - `a_business_app_account_that_is_refused_creates_nothing_and_the_refusal_names_the_field`:
      `"type": "Dynamics 365"` and
      `"connection": {"adapter": "saas", "dial": {"vendor": "dynamics365", "account": "127.0.0.1", "auth": {"type": "oauth2_authorization_code", "clientId": "app"}}}`
      → 400; `error` is exactly
      `The connection settings are not valid: account is not in the form this vendor uses`;
      it contains neither `dial for adapter` nor `saas` nor `127.0.0.1`; the count is 0.
      (Fails on today's code: the field is ignored and the row is created, 201. Fails if the
      check is moved after `create_connector`: the count is 1. Fails if `err.to_string()` is
      used: the text.)
    - `a_database_at_an_internal_address_creates_nothing`: internal hosts not allowed;
      `"connection": {"adapter": "sql", "dial": {"driver": "postgres", "host": "127.0.0.1", "port": 5432, "database": "d", "user": "u"}}`
      → 400, `error` starts with `refusing to dial`, the count is 0.
    - `good_connection_settings_create_the_connector`: the same body with internal hosts
      allowed → 201, the count is 1.
    - `a_create_without_connection_settings_still_creates_the_connector`: no `connection`
      key → 201 (the assistant's path).
    - `saving_refused_connection_settings_answers_the_same_sentence`: after a good create,
      `PUT /api/connectors/{id}/ingest-spec` with the refused `saas` dial of the first test
      → 400 with the same `error`.
  - `cargo test -p lakehouse-api --test sec11_guard` passes.

### T5. An object store that does not answer is given up on after 30 seconds (`TC-1712`; `SRC-41`)

- Anchors: `rust/crates/lakehouse-api/src/connector_listing.rs:11-18` (the module doc's
  timing line), `:76-78` (`LIST_TIMEOUT`), `:980-1032` (`list_s3`; the stream at `:996`, the
  loop at `:998`), `:1062-1073` (`preview_s3`; the `head` at `:1070`), `:1122-1158`
  (`files_answer`), `:236-239` (`logged`).
- Change:
  1. A constant beside `LIST_TIMEOUT`:
     `pub const FIRST_ANSWER_TIMEOUT: Duration = Duration::from_secs(30);`, with a doc that
     says D16's first bullet and where 30 comes from.
  2. `list_s3` and `preview_s3` each gain a last parameter `first_answer: Duration`.
     - `list_s3`: make the stream peekable and wait for its first item under the timer,
       before the loop, which stays exactly as it is:

       ```rust
       let mut stream = client.list(prefix.as_ref()).peekable();
       // QA TC-1712 (SRC-41): ...
       if tokio::time::timeout(first_answer, std::pin::Pin::new(&mut stream).peek())
           .await
           .is_err()
       {
           return Err(logged("an S3 listing", ErrorClass::Timeout));
       }
       ```

       If this does not compile as written, any form that waits for the first item under
       the timer and leaves the loop's body untouched is fine; say in the handoff what you
       used. Do not restructure the loop (it has `continue` branches).
     - `preview_s3`: wrap the `head` call:
       `tokio::time::timeout(first_answer, client.head(&path)).await.map_err(|_| logged("an S3 read", ErrorClass::Timeout))?.map_err(fail)?.size`.
  3. `files_answer` passes `FIRST_ANSWER_TIMEOUT` to both. The outer
     `tokio::time::timeout(LIST_TIMEOUT, ..)` stays.
  4. The module doc's line about `LIST_TIMEOUT` gains one sentence about the first answer.
  Not changed: `connector_probe::s3_client`, the retry settings, the SQL listing, and
  `connector_discover.rs` (its dial is bounded by `DIAL_TIMEOUT`, 5 s, `:504`).
- Assistant: none. `discover_connector` calls `routes::connectors::discover`
  (`routes/ai/tools/ingest.rs:96`), which is `connector_discover.rs`.
- Check, in `connector_listing.rs`'s tests. The stand-in is a
  `tokio::net::TcpListener` on `127.0.0.1:0` whose task accepts every connection and keeps
  it open without writing a byte. The client is
  `connector_probe::s3_client(&format!("http://127.0.0.1:{port}"), "bucket", "127.0.0.1", &approved, "ak", "sk")`
  with `approved` from
  `connector_probe::resolve_checked("127.0.0.1", port, &InternalHosts::ALL)`.
  - `a_bucket_that_never_answers_a_listing_ends_as_a_timeout_at_the_first_answer_limit`:
    `list_s3(.., Duration::from_millis(300))` inside the test's own
    `tokio::time::timeout(Duration::from_secs(10), ..)` returns `Err(ErrorClass::Timeout)`.
  - `a_bucket_that_never_answers_a_preview_ends_as_a_timeout_at_the_first_answer_limit`:
    the same for `preview_s3` (the runner is `unused_runner()`, `:1270`).
  Both fail if the timer is removed: the call then waits for the silent server and the
  test's ten-second limit ends it. The existing tests of this file and
  `cargo test -p lakehouse-api --test connector_s3_preview` stay green unchanged.

### T6. A MariaDB connector's test says MariaDB (`LH-SRC-6-19`; `SRC-41`)

- Anchors: `rust/crates/lakehouse-api/src/connector_probe.rs:567` (the call), `:591-638`
  (`probe_mysql`; the three sentences at `:626`, `:629`, `:634`), `:2913` (a test that calls
  it); `rust/migrations/0035_connector_type.sql:25-27` (the type names).
- Change: a private `fn mysql_family_label(kind: &str) -> &'static str` that returns
  `"MariaDB"` when `kind == "MariaDB"` and `"MySQL"` otherwise (so `MySQL CDC` and any
  other text read MySQL; `kind` itself is never returned). `probe_mysql` gains a parameter
  `label: &'static str` and builds its three sentences with it:
  `Connected via {label} and ran SELECT 1.`, `{label} connection failed: {class}`,
  `{label} connection timed out after {n}s`. The call at `:567` passes
  `mysql_family_label(&info.kind)`. The existing test at `:2913` passes `"MySQL"`.
- Assistant: `test_connector` returns the probe's outcome
  (`routes/ai/tools/connectors.rs:129`), so it reads the same sentence.
- Check, in `connector_probe.rs`'s tests (model on
  `wizard_postgres_connector_dials_from_its_dial_not_its_host`, `:2957-2985`: port 1 on the
  loopback address, `InternalHosts::ALL`, `EnvSecretResolver::with_map`):
  - `a_mariadb_connectors_failed_test_names_mariadb_and_a_mysql_connectors_names_mysql`:
    `probe(..)` of a `sql` connector with `"driver": "mysql"` and `kind: "MariaDB"` has a
    message that contains `MariaDB connection failed` and does not contain `MySQL`; with
    `kind: "MySQL"` it contains `MySQL connection failed`. (Fails on today's code. Fails if
    the call passes the constant `"MySQL"`.)
  - `the_label_is_one_of_two_constants`: `mysql_family_label` of `"MariaDB"`, `"MySQL"`,
    `"MySQL CDC"` and `"mariadb; x"` is `MariaDB`, `MySQL`, `MySQL`, `MySQL`.
  The sentence of a passed test is built from the same label; a passed test needs a real
  server and is *not verified* by a unit test.

### T7. An SFTP host that does not answer is given up on after 30 seconds (`TC-1712`; `SRC-41`)

- Anchors: `dagster/dispar_orchestrate/adapters/sftp.py:68-93` (`_connect_kwargs`; the dict
  at `:80-84`), `:105-125` (`open_session`, used by a load and by `discover.py:503-505`);
  `dagster/dispar_orchestrate/ingest_errors.py:510`;
  `dagster/dispar_orchestrate/test_adapters_sftp.py:50-84` (a fake client that captures
  `connect`'s arguments).
- Change: a module constant `CONNECT_TIMEOUT_SECONDS = 30` with a comment that says D16's
  second bullet, and `"timeout": CONNECT_TIMEOUT_SECONDS` in the dict at `:80-84`. Nothing
  else: `banner_timeout` and `auth_timeout` keep `paramiko`'s own defaults.
- Check, in `test_adapters_sftp.py`:
  `test_the_connection_is_given_thirty_seconds_to_be_answered`: with the fake client of
  `:50-84`, `build_source` calls `connect` with `timeout == 30`. (Fails on today's code: no
  such key. Fails if the key is removed.) No network.

### T8. The Rules list shows when a success rule last sent (`TC-1695`; `SRC-40`)

- Anchors: `src/services/contracts/alerts.ts:74-82` (`target`, `targetHidden`);
  `src/features/alerts/rules-columns.tsx:58-66` (`deliveryLabel`), `:268-284` (the Delivery
  cell); `src/features/overview/alert-delivery.tsx:14-28` (`deliverySummary`).
- Change:
  1. Contract: `AlertRule` gains
     `lastDelivery?: { at: string; delivered: boolean; reason?: string }`, documented (the
     latest delivery of a "run succeeded" rule, which opens no incident; absent when none
     was attempted).
  2. `rules-columns.tsx`: an exported pure function
     `lastDeliveryLabel(rule: Pick<AlertRule, "lastDelivery">): string | null`: `null` with
     no `lastDelivery`; `Last sent ${formatDateTime(at)}` when delivered; otherwise
     `Last attempt ${formatDateTime(at)} · ${deliverySummary({ delivered: false, deliveryReason: reason }).text}`.
     The Delivery cell renders it as a second line (`block text-xs`, `text-destructive` when
     not delivered, `text-muted-foreground` when delivered) under the address or channel.
- Check, in `src/features/alerts/alerts-page.test.tsx` (the `cell(..)` helper of `:386-390`):
  - `shows when a success rule last sent, under its delivery`: a rule with
    `lastDelivery: { at: "2026-11-12T03:04:05.000Z", delivered: true }` renders
    `Last sent ${formatDateTime("2026-11-12T03:04:05.000Z")}` (build the expected text with
    `formatDateTime`, never a typed date: the zone differs by machine).
  - `says a success alert was not delivered, with the reason`: `delivered: false`,
    `reason: "the webhook server answered with an error"` renders a text that ends
    `· Not delivered: the webhook server answered with an error`.
  - `shows no second line for a rule with no recorded delivery`: `lastDeliveryLabel` is
    `null` and the cell holds no "Last sent" and no "Last attempt".
  The first two fail if the cell does not render the label.

### T9. The rule form and the list say a refusal in words (`LH-SRC-7-10`; `SRC-40`)

- Anchors: `src/features/alerts/alert-rule-page.tsx:437` (the measures' error), `:506-507`
  (`connectorsError`), `:509` (`martsError`), `:516` (`boardsError`), `:548` (the form's
  error), shown at `:272`, `:292`, `:376`, `:661`; `src/features/alerts/alerts-page.tsx:205-209`
  (the list's red banner); `src/lib/notify-message.ts:65-74`.
- Change: import `readableErrorMessage` from `@/lib/notify-message`.
  - `connectorsError`: when `connectorList.error.code === "permission_denied"`, the D8
    sentence; otherwise `readableErrorMessage(connectorList.error.message)`.
  - `martsError`, `boardsError`, the measures' error at `:437`, and
    `saveAction.error?.message` at `:548`: each through `readableErrorMessage`. `formErr`
    (the form's own sentences) stays as it is.
  - `alerts-page.tsx:207`: `readableErrorMessage(mutationError.message)`.
- Check, in `src/features/alerts/alert-rule-page.test.tsx` (it spies the services, `:41-50`):
  - `says in words that the connector cannot be chosen, not the API's code`: 
    `connectorService.listConnectors` rejects with
    `new ServiceError("permission_denied", "permission_denied: connector:manage", 403)`;
    choose the type "Connector run failed"; the page shows "You do not have permission to
    choose a connector for this rule." and no text matching `/permission_denied/`.
  - `shows the shared sentence when the save is refused`: `alertRuleService.createRule`
    rejects with `new ServiceError("permission_denied", "permission_denied: alert:write", 403)`;
    after Create the page shows "This account does not have permission. Ask a workspace
    admin." and no text matching `/permission_denied/`.
  Both fail on today's code, and fail if a site prints `error.message` itself.

### T10. "New rule" and the switch are shown only to a person who may use them (`TC-1696`; `SRC-40`)

- Anchors: `src/features/alerts/alerts-page.tsx:82-84` (the page's hooks), `:194-198` ("New
  rule"); `src/features/alerts/rules-columns.tsx:286-306` (the Status cell; the switch at
  `:293-300`); `src/features/alerts/alerts-page.test.tsx:378-414` (#485's tests of a hidden
  rule); `src/features/alerts/alerts-page-test-run.test.tsx:62-105` (`setupFetch`, with
  `permissions` at `:76`).
- Change:
  1. `alerts-page.tsx`: `const { hasPermission } = useAuth()` (import from
     `@/features/auth/auth-provider`); render the "New rule" button only when
     `hasPermission("alert:write")`. Comment: the page it opens takes the same permission
     (`nav-config.ts:348`) and the API refuses the save (D10).
  2. `rules-columns.tsx`, Status cell: when `r.targetHidden === true`, render no `Switch`,
     only the text "Active" or "Disabled". Otherwise the switch as today, without the
     `disabled` prop #485 added. Rewrite #485's comment at `:296-297` to say D9.
- Server: the writes these controls make are refused for the same person already (D10; T3
  adds the missing pin).
- Check:
  - `alerts-page-test-run.test.tsx`: give `setupFetch` a second parameter for the
    permissions (default: today's list). `offers New rule only to a person with alert:write`:
    with `["dashboard:read", "alert:read"]` the page, once the rule's name is shown, has no
    button named "New rule"; with the default list it has one. (Fails on today's code.)
  - `alerts-page.test.tsx`: the test "says Webhook for a hidden webhook rule, disables its
    switch and shows no actions" (`:392-400`) asserted #485's D19. Change it to the new
    decision and say so in the commit (rule 2: the decision changed, the test is not
    weakened): rename it "... shows its state without a switch and shows no actions";
    assert `screen.queryByRole("switch")` is `null` and the text "Active" is shown. The test
    at `:408-414` (a rule with its target: an enabled switch) stays unchanged.

### T11. The wizard sends the settings with the create and disables Next for an account the vendor cannot have (`LH-SRC-3-11`; `SRC-41`)

- Anchors: `src/services/contracts/connectors.ts:424-441` (`CreateConnectorInput`);
  `src/lib/connectors/saas-vendors.ts:15-17` (`SaasAccountRule`), `:360-367`;
  `src/features/connectors/connector-create-page.tsx:188-201` (`problems`, `canProceed`),
  `:226-237` (the create request), `:420-426` (`DialFormFor`), `:531-533` (the create error);
  `rust/crates/lakehouse-store/src/saas_vendors.rs:179-229`, `:245-315` (the rule to copy).
- Change:
  1. Contract: `CreateConnectorInput` gains
     `connection?: { adapter: string; dial: Record<string, unknown> }`, documented (checked
     by the API before the connector is created; nothing is created when it is refused).
  2. `connector-create-page.tsx`, the create request: add
     `connection: { adapter, dial: dial ?? {} }`.
  3. `saas-vendors.ts`: an exported pure function
     `saasAccountProblem(vendor: SaasVendor, account: string): string | null`, `null` when
     the account fits, else `` `This is not in the form ${vendor.label} uses.` ``. The rule,
     by `vendor.account.kind`, in ASCII only (a character outside the classes fails):
     - helpers: a *label* is 1 to 63 of `[A-Za-z0-9-]`, not starting or ending with `-`. A
       *tenant word* is 1 to 64 of `[A-Za-z0-9_-]`. An *Entra tenant* is a GUID (36
       characters: hexadecimal digits, with `-` at positions 8, 13, 18 and 23, counted from
       0), or a domain: at most 253 characters, two or more labels separated by `.`, and a
       letter somewhere in the last label.
     - `label`: a label.
     - `digits`: 1 to 20 of `[0-9]`.
     - `account_id`: 1 to 40 of `[A-Za-z0-9_-]`, first and last character a letter or digit.
     - `dataverse_host`: lower-cased, it ends with `.dynamics.com`; what is before that
       splits at its **last** `.` into an organisation and a region; the region is `crm`
       followed by at most two digits and nothing else; the organisation is a label.
     - `label_tenant`: split at the **first** `/` into an inner part and a tenant (no `/`
       fails); the tenant is an Entra tenant; the inner part follows `label`.
     - `dataverse_host_tenant`: the same, the inner part follows `dataverse_host`.
     - `host_tenant`: split at the first `/` into a host and a tenant; the tenant is a tenant
       word; the lower-cased host ends with one of `vendor.account.suffixes`, and what is
       before the suffix is one to three labels separated by `.`.
     A `switch` over the kind with a `never` check in the default branch, so a new kind in
     the table does not compile until it has a rule.
  4. `connector-create-page.tsx`: for `adapter === "saas"` with a vendor
     (`saasVendorForType(selectedType?.name)`), compute
     `saasAccountProblem(vendor, typeof dial?.account === "string" ? dial.account : "")`.
     Step 1 of `canProceed` also requires it to be `null`. Under `DialFormFor`, when the
     account is not empty and there is a problem, render
     `<p role="alert" className="text-xs text-destructive">{problem}</p>` (the same rule as
     the credential fields: nothing red before something is typed, `:519`). Other adapters:
     unchanged.
- Server: the same account is refused by the API before anything is created (T4).
- Check:
  - `src/lib/connectors/saas-vendors.test.ts`, the cases of the Rust tests
    (`saas_vendors.rs:650-660`, `:707-733`, `:737-797`, `:800-880`), so the two copies are
    held to the same examples:
    - `accepts the account each vendor's own form takes`: `acme` for `salesforce`,
      `servicenow`, `zendesk`, `confluence`; `123456789` for `ga4`, `hubspot`;
      `1234567_SB1` for `netsuite`; `wd2-impl-services1.workday.com/acme_tenant` and
      `a.b.c.myworkday.com/t_1` for `workday`;
      `acme/00000000-0000-0000-0000-000000000001` for `sharepoint`;
      `acme.crm4.dynamics.com/contoso.example.invalid` and
      `Acme.CRM.dynamics.com/00000000-0000-0000-0000-000000000001` for `dynamics365`.
    - `refuses what no vendor's form takes`: for every vendor, each of
      `evil.example.invalid`, `https://acme`, `acme@evil`, `user:pass@acme`, `acme:8443`,
      `acme/path`, `acme.evil`, `" acme"`, `"acme "`, `""`, `-acme`, `acme-`, `"ac me"`,
      and `127.0.0.1`.
    - `refuses each vendor's near misses`: `workday`: `wd2.workday.com`,
      `wd2.workday.com/a/b`, `a.b.c.d.workday.com/t`, `wd2.workday.com.evil.invalid/t`,
      `wd2.workday.org/t`; `dynamics365` (each with
      `/00000000-0000-0000-0000-000000000001` appended): `acme.dynamics.com`,
      `acme.crm123.dynamics.com`, `acme.crmx.dynamics.com`, `a.b.crm.dynamics.com`; and
      `127.0.0.1/00000000-0000-0000-0000-000000000001`; `netsuite`: `1234567_`, `_1234567`;
      as the tenant part, after `acme/` for `sharepoint` and after
      `acme.crm4.dynamics.com/` for `dynamics365`: `common`, `contoso`, `1.2.3.4`,
      `00000000-0000-0000-0000-00000000000g`; and no tenant at all (`acme` and `acme/`
      for `sharepoint`).
    Each of these fails if its rule is loosened or tightened.
  - `src/features/connectors/connector-create-page.test.tsx` (types stubbed with
    `{ name: "Dynamics 365", adapter: "saas", supported: true, .. }`):
    - `keeps Next disabled and says why for an account the vendor cannot have`: on the
      Connection step, with the account `127.0.0.1` and the credential fields filled, Next
      is disabled and "This is not in the form Dynamics 365 uses." is shown; with
      `acme.crm4.dynamics.com/contoso.example.invalid` the sentence is gone and Next is
      enabled. (Fails on today's code: Next is enabled.)
    - `sends the connection settings with the create request`: after Create, the body of
      `POST /api/connectors` holds `connection.adapter` and `connection.dial` equal to what
      the `PUT .../ingest-spec` body holds as `adapter` and `dial`.
    - `shows the API's refusal and stays on the form when the create is refused`: the
      `POST` answers 400 `{ "error": "The connection settings are not valid: account is not in the form this vendor uses" }`;
      that sentence is shown, no `PUT` and no `/test` request was made, and "Connector
      created" is not shown.
  If an existing test of a business-app connector types an account outside its vendor's
  form, correct the fixture's account and say so in the commit.

### T12. The "Connector created" screen waits for the save (`LH-SRC-6-39`, `LH-SRC-6-54`, `LH-SRC-6-16`; `SRC-41`)

- Anchors: `src/features/connectors/connector-create-page.tsx:146-157` (`saveSpec`),
  `:241-266` (the order after the create), `:289-323` (the status block; the raw
  `saveSpec.error.message` at `:293`), `:331-342` ("Run test again"), `:347` (the sign-in
  panel), `:349-353` (the ingest panel);
  `src/features/connectors/connector-ingest-panel.tsx:372`, `:387-398`.
- Change, in the created screen only (`handleSubmit`'s order stays):
  1. `const settingsSaved = saveSpec.status === "success"`. The ingest panel's card (`:349`)
     and the sign-in panel (`:347`) are rendered only when `!testSkipped && settingsSaved`.
  2. The status block gains a branch, after `testSkipped` and before `test.status`: while
     `saveSpec.status` is `"idle"` or `"pending"`, or `setCredentialAfterCreate.status` is
     `"pending"`: `<p className="text-sm text-muted-foreground">Saving the connection settings…</p>`.
  3. "Run test again" is also disabled in that state.
  4. `:293`: `readableErrorMessage(saveSpec.error.message)`.
  `connector-ingest-panel.tsx` is not changed.
- Check, in `connector-create-page.test.tsx`. Widen the `extra` callback of `stubFetch`
  (`:53-56`) to allow a `Promise<Response>`, and hold the `PUT .../ingest-spec` answer on a
  promise the test resolves:
  - `does not read the connection settings before the save has answered`: after Create and
    while the `PUT` is held, the page shows "Saving the connection settings…", no
    `GET .../ingest-spec` request was made, and no text matching
    `/has no connection settings saved yet/` is shown. Then resolve the `PUT`: a
    `GET .../ingest-spec` is made and the text is still absent. (Fails on today's code: the
    `GET` is made while the `PUT` is held. Fails if the panel is mounted on `createdId`
    alone.)
  The existing tests of this file stay green; the one at `:128` already asserts the `PUT`
  comes before the test.

### T13. The Credential fact says "None" for a connection that reads no credential (`LH-SRC-6-39`; `SRC-41`)

- Anchors: `src/features/connectors/connector-detail-page.tsx:62-80` (`credentialFact`),
  `:89-94` (`connectorFacts`), `:238` (`ingestSpec`), `:387` (the call);
  `src/features/connectors/connector-form-parts.tsx:381-384` (`usesNoCredential`);
  `src/features/connectors/connector-create-page.tsx:517-518` (the review step's sentence);
  `src/features/connectors/connector-detail-page.test.tsx:443-453`.
- Change: `connectorFacts` gains a third argument,
  `credential: "none" | "unknown" | "stored"`, computed at `:387` from `ingestSpec`:
  `"unknown"` while its status is `"loading"`; `"none"` when it is `"success"` and
  `usesNoCredential(ingestSpec.data.adapter ?? null, ingestSpec.data.dial)`; `"stored"`
  otherwise (a read that failed included). The Credential fact is the D15 sentence for
  `"none"`, `"—"` for `"unknown"`, and `credentialFact(c.credentialSource)` for `"stored"`.
  Import `usesNoCredential` from `./connector-form-parts`.
- Check, in `connector-detail-page.test.tsx`:
  - `says None for a connector that authenticates with none`: the spec is
    `{ adapter: "kafka", ingestMode: "batch", dial: { auth: { type: "none" } }, .. }` and
    the connector's `credentialSource` is `"managed"`; the Credential fact becomes "None:
    this connection authenticates with none" and "Stored by lakehouse, encrypted" is not
    shown. (Fails on today's code. Fails if the fact ignores the spec.)
  - The existing assertion at `:453` reads the fact right after the heading appears; the
    fact is now "—" until the spec has answered. Wrap that one expectation in `waitFor` and
    say so in the commit (the page gained a loading state; the expected text is unchanged).

### T14. The Runs card says the status follows the orchestrator's report (`LH-SRC-6-16`; `SRC-41`)

- Anchors: `src/features/connectors/connector-ingest-panel.tsx:1508-1512` (the line shown
  while a run is going), `:1638-1643` (`run.attempts`).
- Change: under the line at `:1508-1512`, when `activeRun.attempts > 0`, a second line
  `<p className="text-xs text-muted-foreground">` with the D17 sentence. A comment that
  names the cause in one sentence and points to the feature page's limit (T17). No timing
  constant changes.
- Check, in `src/features/connectors/connector-ingest-panel.test.tsx`:
  `says the status follows the orchestrator's report once an attempt has ended`: with the
  runs list answering one `running` run with `attempts: 1` the sentence is shown; with
  `attempts: 0` it is not; with a `succeeded` run it is not. (Fails on today's code. Fails
  if the line is shown for a finished run.)

### T15. "1 file found" (`LH-SRC-6-50`, `LH-SRC-6-55`; `SRC-41`)

- Anchors: `src/features/connectors/table-finder.tsx:55-64` (`FINDER_WORDS`), `:495`.
- Change: each entry of `FINDER_WORDS` gains `one`: `table`, `table`, `file`, `file`,
  `endpoint`, `collection`, `topic`, `object` (in the order of the record). Line `:495`
  uses `words.one` when `baseObjects.length === 1`, else `words.many`. The other uses of
  `words.many` (`:471`, `:478`, `:486`, `:489`, and `connector-ingest-panel.tsx:655`) are
  about the kind, not a count of one, and stay.
- Check, in `src/features/connectors/table-finder.test.tsx`:
  `says one file found and two files found`: a `files` listing of one object shows "1 file
  found"; of two, "2 files found"; a `rest` listing of one endpoint shows "1 endpoint
  found". (Fails on today's code: "1 files found".)

### T16. The New Connector page shows a refusal instead of the form when the types cannot be read (`LH-SRC-3-08`; `SRC-41`)

- Anchors: `src/features/connectors/connector-create-page.tsx:56` (`types`), `:358-368` (the
  header), `:369-377` (`FormStepLayout`), `:387-393` (the three states inside step 0).
- Change: before the wizard's `return` (`:358`), when `types.status === "error"`, return the
  page header (title and the Cancel link, as at `:360-368`) and only the state: for
  `types.error.code === "permission_denied"`, `EmptyState` with the title and the
  description of D20; otherwise `<ErrorState error={types.error} onRetry={types.reload} />`.
  The `types.status === "error"` branch inside step 0 (`:389-390`) is then unreachable:
  remove it.
- Server: `POST /api/connectors` and `GET /api/connectors/types` both take
  `connector:manage` (`policy.rs:959`, `:954`), walked by the loops of
  `tests/route_auth.rs`.
- Check, in `connector-create-page.test.tsx`:
  - `shows the missing permission and no form when the types are refused`:
    `GET /api/connectors/types` answers 403 `{ "error": "permission_denied: connector:manage" }`;
    the page shows "You cannot create a connector" and "Creating a connector needs the
    connector:manage permission. Ask a workspace administrator."; there is no field labelled
    "Name", no button "Next" and no button "Previous"; no text matches `/permission_denied/`.
    (Fails on today's code: the form is there.)
  - `offers Retry and no form when the types cannot be loaded`: the same with a 503; a
    button "Retry" is shown and no "Next".

### T17. Documents

- `docs/core/features/load-failure-alerts.md`:
  - In the paragraph under "## Decisions": `row 11 is *(proposed, not yet signed)*` becomes
    `rows 11 and 12 are *(proposed, not yet signed)*`, and `Rows 6 to 11 are beyond` becomes
    `Rows 6 to 12 are beyond`.
  - In the Decisions table, after row 11, add
    `| 12 | Beyond the spec: where a "run succeeded" alert is recorded (QA `TC-1695`). | On its rule: the Rules list shows when the rule last sent, or that the message did not arrive and why. A success opens no incident, so it is not listed under Active Incidents and does not light the bell. Only the latest one is kept. | changed 2026-11-12 (`SRC-40`), *(proposed, not yet signed)* |`.
  - Under "Limits to tell a customer" add
    `- A "run succeeded" alert is shown on its rule (last sent, or not delivered with the reason), not as an incident.`
  - In the acceptance checklist, after row 13, add
    `| 14 | With a success rule for a connector, run the connector; open Alerts, Rules | The rule shows "Last sent" with the time; Active Incidents has no new row and the bell does not change | Not run |`
    and
    `| 15 | As a user with `alert:read` and without `alert:write`, open Alerts, Rules; then open `/alerts/rules/new` | No "New rule" button and no switch on a rule; the address shows "You don't have access" | Not run |`.
- `docs/core/features/connector-operations.md`, under "Limits to tell a customer", add
  `- A run's status changes when the orchestrator reports the end of the run, which can be minutes after the last table's rows are counted. The report is the run-status sensor's: it is evaluated at most every 30 seconds (`minimum_interval_seconds` of `pipeline_run_finished_sensor`), handles at most 5 ended runs of all jobs per evaluation (`DAGSTER_RUN_STATUS_SENSOR_PROCESS_LIMIT`), and waits its turn behind the other sensors (`sensors.use_threads` in `dagster/dagster.yaml` is off). Which of these made the five to six minutes QA saw (`LH-SRC-6-16`) was not measured.`
- `docs/core/features/table-discovery-every-source.md`, under "Limits to tell a customer",
  add
  `- A source that does not answer is given up on after 30 seconds (object storage, SFTP, a REST preview) and the listing says it took too long. For the sources the orchestrator reads, the time to start its short job comes on top; that time was not measured (`TC-1712`).`
- `docs/core/features/connectors-that-work.md`, in the acceptance checklist, after row 10,
  add
  `| 11 | In the wizard, enter connection settings the product refuses (a business-app account that is not in the vendor's form, or a database at an internal address) and create | Refused with a sentence that names the field; no connector appears in the list | Not run |`
  and
  `| 12 | Create a Kafka connector with no authentication; open its page | The Credential fact reads "None: this connection authenticates with none" | Not run |`.
- Check: none (documents). The developer does not edit `BACKLOG.md`, `PRODUCT.md`,
  `HANDOFF.md` or any other document, and never writes that the product owner signed a
  wording.

## 3. Verification

The developer works on a strong ARM64 build machine (section 3a). Never `cargo clean`. Never
`cargo test --workspace` and never a bare `cargo test -p lakehouse-api <filter>`: CI runs the
full suite on the pull request (x86_64) and is the final word.

Per commit, only for what the commit touched:

```bash
# Rust (T1 to T6)
(cd rust && cargo fmt --check)
(cd rust && cargo clippy -p lakehouse-store --all-targets -- -D warnings)   # T1, T4
(cd rust && cargo clippy -p lakehouse-api --all-targets -- -D warnings)     # T2 to T6
(cd rust && cargo test -p lakehouse-store --test alert_delivery)            # T1
(cd rust && cargo test -p lakehouse-store --lib a_refused_saas_account_names_the_field_once)   # T4
(cd rust && cargo test -p lakehouse-api --lib a_success_alert)              # T2 (two tests)
(cd rust && cargo test -p lakehouse-api --lib a_failed_run_still_opens_an_incident_and_records_no_last_delivery)   # T2
(cd rust && cargo test -p lakehouse-api --test alert_targets_hidden)        # T2, T3
(cd rust && cargo test -p lakehouse-api --test connector_create_checks)     # T4
(cd rust && cargo test -p lakehouse-api --test sec11_guard)                 # T4
(cd rust && cargo test -p lakehouse-api --lib a_bucket_that_never_answers)  # T5 (two tests)
(cd rust && cargo test -p lakehouse-api --test connector_s3_preview)        # T5
(cd rust && cargo test -p lakehouse-api --lib a_mariadb_connectors_failed_test)   # T6
(cd rust && cargo test -p lakehouse-api --lib the_label_is_one_of_two_constants)  # T6

# Python (T7), in a virtual environment under /home/rismana/rantai/
(cd dagster && python -m pytest dispar_orchestrate/test_adapters_sftp.py -q)

# TypeScript (T8 to T16)
bun install --frozen-lockfile        # once
bun run typecheck && bun run lint
bun test src/features/alerts/alerts-page.test.tsx src/features/alerts/alert-rule-page.test.tsx src/features/alerts/alerts-page-test-run.test.tsx
bun test src/lib/connectors/saas-vendors.test.ts
bun test src/features/connectors/connector-create-page.test.tsx src/features/connectors/connector-detail-page.test.tsx
bun test src/features/connectors/connector-ingest-panel.test.tsx src/features/connectors/table-finder.test.tsx
```

`sqlx::test` embeds the migrations at compile time: after T1's migration every later Rust
test build includes it; if its number changes after a rebase, the next build picks that up.

Always, on the final commit, in the foreground:

```bash
python3 ops/lint/check_bare_iceberg_count.py && python3 ops/lint/check_intra_package_imports.py
git status   # nothing generated staged
```

The handoff quotes each command it ran with its counts, and lists as *not verified* anything
it did not run, with the reason (rule 7). Already known to be *not verified* by any local
test: a passed MariaDB test's sentence (T6, needs a server); the timing of a real silent
object store or SFTP host (T5 and T7 test the bound with a stand-in and an argument); every
screen in a browser; and D16's and D17's unmeasured times. For QA's retest, to be written in
the pull request by the planner: `TC-1695` is read on Alerts, Rules, under the success rule's
Delivery ("Last sent …"), not under Active Incidents.

## 3a. Machine

- Checkout: `/home/rismana/rantai/lakehouse-agents-4`. Work only under
  `/home/rismana/rantai/`.
- Before any cargo command:
  `export CARGO_TARGET_DIR=/home/rismana/rantai/cargo-target-4 CARGO_BUILD_JOBS=6`.
- The machine is shared (20 CPUs, 121 GB RAM, Docker running). Start no container you do
  not remove: the Rust tests start a Postgres through `testcontainers`; note `docker ps`
  before the first test run and after the last, and remove only what your runs left. No
  `docker compose up` is needed for this slice (nothing in compose changes).
- Python: a virtual environment under `/home/rismana/rantai/` with the `dagster/` package
  and `pytest` installed; reuse one that exists for this checkout, and say in the handoff
  which one was used.

## 4. Out of scope

- The delivery of every other rule kind on its rule ("Last sent" for failure, schema-change,
  threshold and digest rules): theirs is on the incident, as today. A history of a rule's
  deliveries beyond the latest one.
- Changing the orchestrator's timing for the run status: the sensor's interval, the
  process limit, threaded sensors, or having a batch run report its own end (D17). See Open
  questions.
- The time the orchestrator takes to start `discover_job`, and the console's five-minute
  wait for it (D16).
- A first-answer timer for the SQL Server listing's TCP connect
  (`connector_listing.rs:871`): it stays under the 120 s limit. Not in QA's evidence, and it
  has no stand-in a unit test can reach.
- The S3 client's missing connect and read timeouts themselves (`connector_probe.rs:1096-1112`)
  and its retry settings: T5 bounds the wait from outside.
- Disabling Next for an internal address of a database, file, REST or SFTP connector: the
  form cannot know the installation's setting (D13). Next being enabled with an empty host
  for those types is also not changed here; the create is now refused before anything
  exists (T4).
- The edit page's use of the business-app form (its save answers the D12 sentence; nothing
  is half-made there).
- Saving the settings inside the create request (one call instead of two). A save that
  fails after a good check still leaves the connector, with today's message.
- The other places that print "connector's dial is invalid: …" for a **stored** dial that no
  longer parses (`connector_probe.rs:537`, `:785`, `:1778`, `connector_probe/saas.rs:90`,
  `connector_discover.rs:149`, `:201`).
- The doc comment on `IngestSpecError::InvalidDial`'s `source` (`ingest_spec.rs:1625-1626`),
  which says it never echoes an input value; `InvalidHostname` does repeat the host it was
  given (`:1318-1323`, `:2330`). Reported here, not changed.
- The raw error texts of other connector screens (for example `runNow.error.message`,
  `connector-ingest-panel.tsx:1523`).
- The assistant's tool schemas and prompt.

## Open questions for the owner

None of these blocks the build; each has a planned default.

1. **Where a success alert shows (D4, D7).** Planned: on its rule, in the Rules list ("Last
   sent …"), with no incident. New decision row 12 on the feature page, *(proposed, not yet
   signed)*.
2. **30 seconds (D16)** for a source that does not answer a listing, and for an SFTP connect
   in a load as well as in a listing. *(proposed)*.
3. **The run status delay (D17).** Planned: say it on the page and write the cause down.
   Should a follow-up item make it short, either by having a successful batch run report its
   own end from a last step, or by changing the three orchestrator settings after measuring
   which one costs the time on the QA instance?
4. **The switch (D9)** is not shown at all for a rule the person may not manage, where #485
   showed it disabled. Planned as written, after "show them only with the permission".

## 5. Handoff

Developer: opencode-go/deepseek-v4.1-flash. Branch `fix/qa-round-connectors-p2`, off
`5e316b426` (the head of `fix/qa-round-connectors-p1`, PR #485). Pushed after every commit;
no force-push, no pull request opened.

**Migration number: `0139_alert_rule_delivery.sql`.** `0138` belongs to the other slice of this
round (#487), and `origin/main`'s highest was `0136` when the file was created and when the
handoff was written, so `0139` is free and nothing is renumbered.

### Commits

| Commit | Title |
| --- | --- |
| `dd4406aa` | `fix(alerts): a table for a rule's latest delivery (TC-1695; SRC-40)` |
| `fa7b4213` | `fix(alerts): record a "run succeeded" alert on its rule and list it (TC-1695; SRC-40)` |
| `22e2c9e8` | `test(alerts): pin that a caller without alert:write cannot delete a rule (TC-1696; SRC-40)` |
| `c8e26f70` | `fix(connectors): check the connection settings before the connector exists (LH-SRC-3-11; SRC-41)` |
| `7c8499d1` | `fix(connectors): give up on a silent object store after 30 seconds (TC-1712; SRC-41)` |
| `fc29551b` | `fix(connectors): a MariaDB connector's test names MariaDB (LH-SRC-6-19; SRC-41)` |
| `f8256a6b` | `fix(sftp): give a silent host thirty seconds to answer (TC-1712; SRC-41)` |
| `7fd152a5` | `feat(alerts): the Rules list shows when a success rule last sent (TC-1695; SRC-40)` |
| `05a9c7fc` | `fix(alerts): the rule form and the list say a refusal in words (LH-SRC-7-10; SRC-40)` |
| `287e407f` | `fix(alerts): "New rule" and the switch only for a person who may use them (TC-1696; SRC-40)` |
| `3fe29f86` | `feat(connectors): the wizard sends the settings with the create and checks a business-app account (LH-SRC-3-11; SRC-41)` |
| `e79604b6` | `fix(connectors): the created screen waits for the save (LH-SRC-6-39, LH-SRC-6-54, LH-SRC-6-16; SRC-41)` |
| `91e7e534` | `fix(connectors): the Credential fact says None for a connection that reads no credential (LH-SRC-6-39; SRC-41)` |
| `5aa44d5c` | `feat(connectors): the Runs card says the status follows the orchestrator's report (LH-SRC-6-16; SRC-41)` |
| `b2ba9e7c` | `fix(connectors): "1 file found" (LH-SRC-6-50, LH-SRC-6-55; SRC-41)` |
| `94b77f70` | `fix(connectors): the New Connector page shows a refusal instead of the form (LH-SRC-3-08; SRC-41)` |
| `784b90c3` | `fix(alerts): the D8 sentence matches the refusal this client really reports (LH-SRC-7-10; SRC-40)` |
| `9b020210` | `docs(features): the QA round's decisions and limits for alerts and connectors (SRC-40, SRC-41)` |
| `151cfa1e` | `test(connectors): the column-choice helper waits for the singular count (LH-SRC-6-50; SRC-41)` |

Tasks ran in plan order (T1 … T17), T17 as `9b020210`; `784b90c3` and `151cfa1e` are two
follow-ups the checks below turned up (see "MISMATCH / deviations").

### Tests added, and the failure each had before the fix

- T1 `rust/crates/lakehouse-store/tests/alert_delivery.rs`: `a_rule_never_delivered_to_has_no_row`,
  `the_latest_attempt_replaces_the_one_before_it`, `an_older_attempt_never_replaces_a_newer_one`,
  `a_deleted_rules_row_is_gone`, `the_table_refuses_a_reason_on_a_delivered_row`. Before the fix
  (source stashed, test kept): `error[E0432]: unresolved import
  lakehouse_store::alert_delivery … could not find alert_delivery in lakehouse_store`.
- T2 `routes/load_alerts.rs`: `a_success_alert_is_recorded_as_the_rules_last_delivery_and_opens_no_incident`
  (before the fix, `else` branch removed: `panicked …: the delivered success is recorded`),
  `a_success_alert_whose_webhook_answers_an_error_is_recorded_as_not_delivered`,
  `a_failed_run_still_opens_an_incident_and_records_no_last_delivery`.
  `tests/alert_targets_hidden.rs`: `the_rule_list_and_the_assistants_list_carry_a_rules_last_delivery`,
  `a_rule_never_delivered_to_has_no_last_delivery_key`.
- T3 `tests/alert_targets_hidden.rs`: `a_caller_without_alert_write_cannot_delete_a_rule` — a pin,
  it passed on the code before it by design (the plan says so).
- T4 `ingest_spec.rs`: `a_refused_saas_account_names_the_field_once` — before the fix:
  `assertion left == right failed: left: "account: account is not in the form this vendor uses"`.
  `tests/connector_create_checks.rs`: `a_business_app_account_that_is_refused_creates_nothing_and_the_refusal_names_the_field`
  (before the fix: `left: 201 right: 400`, with the created connector body in the message),
  `a_database_at_an_internal_address_creates_nothing`, `good_connection_settings_create_the_connector`,
  `a_create_without_connection_settings_still_creates_the_connector`,
  `saving_refused_connection_settings_answers_the_same_sentence`.
- T5 `connector_listing.rs`: `a_bucket_that_never_answers_a_listing_ends_as_a_timeout_at_the_first_answer_limit`
  (before the fix, timer removed: `panicked …: the first-answer limit ends the listing: Elapsed(())`),
  `a_bucket_that_never_answers_a_preview_ends_as_a_timeout_at_the_first_answer_limit`.
- T6 `connector_probe.rs`: `a_mariadb_connectors_failed_test_names_mariadb_and_a_mysql_connectors_names_mysql`
  (before the fix: `panicked …: MySQL connection failed: connection refused`),
  `the_label_is_one_of_two_constants`.
- T7 `test_adapters_sftp.py`: `test_the_connection_is_given_thirty_seconds_to_be_answered`
  (before the fix: `E  KeyError: 'timeout'`).
- T8 `alerts-page.test.tsx`: `shows when a success rule last sent, under its delivery`,
  `says a success alert was not delivered, with the reason` (both failed with the second line
  removed), `shows no second line for a rule with no recorded delivery`.
- T9 `alert-rule-page.test.tsx`: `says in words that the connector cannot be chosen, not the API's code`
  (before the fix: the raw `permission_denied: connector:manage` was rendered),
  `shows the shared sentence when the save is refused`.
- T10 `alerts-page-test-run.test.tsx`: `offers New rule only to a person with alert:write` (before
  the fix the button was rendered for `alert:read` alone); `alerts-page.test.tsx`'s hidden-rule
  test renamed to `says Webhook for a hidden webhook rule, shows its state without a switch and
  shows no actions`, now asserting `queryByRole("switch")` is null and "Active" is text (D9).
- T11 `saas-vendors.test.ts`: `accepts the account each vendor's own form takes`,
  `refuses what no vendor's form takes`, `refuses each vendor's near misses` (they cannot compile
  before `saasAccountProblem` exists).
  `connector-create-page.test.tsx`: `keeps Next disabled and says why for an account the vendor
  cannot have` (before the fix: Next was enabled and no sentence was shown),
  `sends the connection settings with the create request`,
  `shows the API's refusal and stays on the form when the create is refused`.
- T12 `connector-create-page.test.tsx`: `does not read the connection settings before the save has
  answered` (before the fix the `GET .../ingest-spec` went out while the `PUT` was held).
- T13 `connector-detail-page.test.tsx`: `says None for a connector that authenticates with none`
  (before the fix the fact read "Stored by lakehouse, encrypted"); the existing fact assertion now
  `waitFor`s (the page gained a loading state, the expected text is unchanged).
- T14 `connector-ingest-panel.test.tsx`: `says the status follows the orchestrator's report once an
  attempt has ended` (before the fix: no sentence).
- T15 `table-finder.test.tsx`: `says one file found and two files found` (before the fix:
  "1 files found").
- T16 `connector-create-page.test.tsx`: `shows the missing permission and no form when the types are
  refused`, `offers Retry and no form when the types cannot be loaded` (before the fix: the form
  was there).

### Commands run, with their counts

Every command was run in the foreground, from the checkout, with
`CARGO_TARGET_DIR=/home/rismana/rantai/cargo-target-4 CARGO_BUILD_JOBS=6`.

Per commit (the scoped lines of section 3), all on the final code:

- `cargo fmt --check` → no diff.
- `cargo clippy -p lakehouse-store -p lakehouse-api --all-targets -- -D warnings` → `Finished` (no
  warning). T1 and T4 also ran `-p lakehouse-store` alone, clean.
- `cargo test -p lakehouse-store --test alert_delivery` → 5 passed.
- `cargo test -p lakehouse-store --lib a_refused_saas_account_names_the_field_once` → 1 passed
  (319 filtered).
- `cargo test -p lakehouse-api --lib a_success_alert` → 2 passed; `… a_failed_run_still_opens_an_incident_and_records_no_last_delivery`
  → 1 passed; `… a_bucket_that_never_answers` → 2 passed; `… a_mariadb_connectors_failed_test`
  → 1 passed; `… the_label_is_one_of_two_constants` → 1 passed; `… connector_listing` → 21 passed
  (T5, the whole module, to prove the existing cases stayed green).
- `cargo test -p lakehouse-api --test alert_targets_hidden` → 10 passed;
  `--test connector_create_checks` → 6 passed; `--test sec11_guard` → 4 passed;
  `--test connector_s3_preview` → 5 passed.
- `(cd dagster && …/venv-agents-4/bin/python -m pytest dispar_orchestrate/test_adapters_sftp.py -q)`
  → 9 passed. The full `pytest dispar_orchestrate -q` → **2544 passed, 1 skipped, 41 subtests
  passed in 838.60s** (venv `/home/rismana/rantai/venv-agents-4`).
- `bun install --frozen-lockfile` → "Checked 779 installs across 753 packages (no changes)".
- `bun run typecheck` → clean. `bun run lint` → 0 errors, 11 warnings, all pre-existing (the rule
  forbids adding any; none was added).
- `bun run test` (the whole console) → **4068 pass, 1 skip, 0 fail, 4069 tests across 380 files**.
  (A first run showed 7 failures in `connector-ingest-panel.test.tsx` — the "1 tables found"
  helper, fixed in `151cfa1e` — and one flaky `schema-browser.test.tsx` failure that passed alone
  and in the rerun.)
- `python3 ops/lint/check_bare_iceberg_count.py` → `R11 guard: no bare count() against a Bronze
  Iceberg table found.`
- `python3 ops/lint/check_intra_package_imports.py` → `OK: all intra-package dispar_orchestrate
  imports resolve`.
- `git status` → clean; nothing generated staged.
- `docker ps` before the first Rust test and after the last: 32 containers both times (31
  `clickhouse/clickhouse-server:26.8`, 1 `postgres:16-alpine`); nothing was left behind.

### Not verified

- **`cargo test --workspace` / the full Rust suite**: not run, per this plan's section 3 and the
  developer brief; CI on the pull request (x86_64) is the final word.
- `rust/crates/lakehouse-api/tests/route_auth.rs`: not run. No route was added and no route's
  policy changed (`POLICY_TABLE` untouched), so nothing there is affected.
- A passed MariaDB test's sentence (`Connected via MariaDB and ran SELECT 1.`): it needs a real
  server; only the failed and timed-out sentences are covered by a unit test (T6).
- The real timing of a silent object store and of a silent SFTP host: both are proved with a
  stand-in and an argument (300 ms bounds in T5, the `timeout` keyword in T7), not against a real
  silent endpoint.
- Every screen in a browser: not run; the console is covered by the component tests above only.
- D16's and D17's unmeasured times (QA's 3 min 40 s for a REST preview, the five to six minutes
  for the run status): unchanged, as the plan says.
- `docker compose config`: nothing in `docker-compose.yml` changed.

### MISMATCH / deviations

- **MISMATCH (handled, `784b90c3`):** the plan's D8/T9 predicate, `connectorList.error.code ===
  "permission_denied"`, never matches this client. `src/services/clients/connectors.ts::getJson`
  maps every non-404 4xx — a 403 included — to `invalid_request` and keeps `status`, so the D8
  sentence was dead code in the browser. The predicate is now
  `code === "permission_denied" || status === 403`, the pattern `src/lib/home-cards.ts::cardRead`
  already uses, and the test rejects with the shape the client produces
  (`invalid_request`, status 403). No sentence or scope changed. T16 uses the same predicate
  (`94b77f70`).
- **T15 collateral (`151cfa1e`):** seven existing tests in `connector-ingest-panel.test.tsx` waited
  for "1 tables found", the line D18 changed. They now wait for "1 table found". The decision
  changed; the assertion followed it and nothing was loosened (rule 2).
- **T5:** the plan's `peek` snippet compiled as written (`Pin::new(&mut stream).peek()`), so no
  alternative form was needed.
- **T4:** the create body in the plan was accepted as written; no field had to be corrected.
- **T7:** the new test is at the end of `test_adapters_sftp.py` rather than inside the pinned-host-key
  block; it uses the same fake client shape.
- **T12:** the `testSkipped` branch keeps its own wording; its save error text now also goes through
  `readableErrorMessage` (the plan's `:293`).
- The migration is `0139`, not `0138` (see above); the migration numbering on `origin/main` was
  `0136` throughout.

## 6. Review

### Round 1 (planner, 2026-11-12), at `b09b4210`

Read against the plan and `AGENTS.md`, against the base `fix/qa-round-connectors-p1`; nothing
was run for this review, CI on the draft pull request (#490) runs the suites. No BLOCKER, 5
SHOULD-FIX, all to be fixed as the findings say. No defect found in the delivery record
(tenant scope, the concurrent upsert, fixed failure sentences, no address in `lastDelivery`),
in the check before a connector is created (it runs before anything is written and dials
nothing), in the 30-second limits (connect and negotiation only, not reads) or in the
`status === 403` predicate; no existing test was weakened.

The developer's handled MISMATCH (a 403 reaches this client as `invalid_request` with
`status`, not as `permission_denied`) is accepted as built.

### R1. SHOULD-FIX: after a failed save, "Run test again" removes the only statement that the connection settings were not saved (T12)

- Where: `src/features/connectors/connector-create-page.tsx:316-331` (the status block), `:364-368`
  ("Run test again"), `:380` (the ingest panel's condition).
- What happens: the create answers 201 and `PUT .../ingest-spec` fails (D11's "a save that fails for
  another reason"). The screen first says, correctly, "The connection test was not run: the
  connection settings could not be saved. ..." with the Edit link. "Run test again" is enabled
  there (`settingsPending` is false for `saveSpec.status === "error"`), and its click sets
  `testSkipped` to false. From then on the status block shows only the test's result, the Edit
  link is gone, and the ingest panel is not mounted because `settingsSaved` is false. Nothing on
  the screen says the settings are missing. Before this branch the panel mounted at that point
  and said "This connector has no connection settings saved yet". The first state is right and
  the path needs a failed save, which is why this is not a blocker.
- Fix: keep the fact on screen while `saveSpec.status === "error"`. Either disable "Run test again"
  in that state (a test of a connector with no settings says nothing useful, and the sentence
  already sends the person to Edit), or render the "could not be saved" block with its Edit link
  on `saveSpec.status === "error"` whatever `testSkipped` is.
- Check: a new test in `connector-create-page.test.tsx`: the `PUT` answers 500; the sentence and
  the Edit link are shown, no `GET .../ingest-spec` and no `/test` request is made, and "Run test
  again" is disabled (or, if it stays enabled, the sentence is still shown after the click). The
  created screen's failed-save state has no test at all today (`grep "could not be saved"` in the
  test file finds nothing). Temporary wrong change: remove the new `saveSpec.status === "error"`
  condition; the test must fail.

### R2. SHOULD-FIX: no test proves that a rule's delivery row is removed with the rule or when an edit changes what it watches (D5)

- Where: `rust/crates/lakehouse-api/src/routes/alerts.rs:851-860` (`forget_state`). The only test of
  a delete is `rust/crates/lakehouse-store/tests/alert_delivery.rs` `a_deleted_rules_row_is_gone`,
  which calls the store function itself.
- What happens: delete the `alert_delivery::delete` call at `alerts.rs:856` and every test of the
  branch stays green. A deleted rule then keeps its row, and a success rule moved to another
  connector keeps showing the old connector's "Last sent". The code is right today; nothing holds
  it. The sibling table has these tests (`tests/ai_alert_tools.rs:365-388`,
  `tests/chart_alerts.rs:744-764`).
- Fix: tests only. (a) In `tests/alert_targets_hidden.rs`
  `a_caller_without_alert_write_cannot_delete_a_rule`: `alert_delivery::record` a row for `al_1`
  first; after the reader's 403 `last_for` still holds it; after the writer's 200 it is empty.
  (b) In `tests/ai_alert_tools.rs`, beside the `alert_rule_state` assertion at `:388`, the same for
  `alert_rule_delivery` after `delete_alert_rule`. (c) One test of an edit that changes
  `connector` (model on `tests/chart_alerts.rs:744-764`): the row is gone; an edit of the name
  alone keeps it.
- Check: remove the `alert_delivery::delete` call from `forget_state`; (a), (b) and (c) must fail.

### R3. SHOULD-FIX: the handoff's "seven existing tests" is not what the code shows; five tests use the changed helper

- Where: plan section 5, "Commands run" ("A first run showed 7 failures in
  `connector-ingest-panel.test.tsx`") and "MISMATCH / deviations" ("seven existing tests ...
  waited for `1 tables found`"); `src/features/connectors/connector-ingest-panel.test.tsx:1316`
  (`openColumns`) and its callers at `:1329`, `:1345`, `:1364`, `:1381`, `:1410`.
- What happens: the base file holds "1 tables found" once, in `openColumns`, and five tests call
  it. So either two of the seven failures had another cause that the handoff does not name, or
  the count is wrong. Rule 7 wants the counts that were seen, with what they were. The change
  itself is clean: one expected text in the helper, no assertion removed, nothing skipped.
- Fix: the developer lists the seven test names of that first run in the handoff. If two do not
  call `openColumns`, say why they failed and why they pass now.
- Check: `bun test src/features/connectors/connector-ingest-panel.test.tsx` on `b2ba9e7c6` (T15,
  before `151cfa1e4`) prints the failing names.

### R4. SHOULD-FIX: the console's copy of the account rule lower-cases with Unicode rules, the server with ASCII rules, so the form passes an account the server refuses

- Where: `src/lib/connectors/saas-vendors.ts:405` and `:426` (`toLowerCase()`);
  `rust/crates/lakehouse-store/src/saas_vendors.rs` `validate_account` (`to_ascii_lowercase`). The
  plan's T11 says "in ASCII only (a character outside the classes fails)".
- What happens: U+212A (KELVIN SIGN) lower-cases to `k` in JavaScript and stays as it is in Rust.
  For Workday `wd2.worKday.com/t`, or for Dynamics 365
  `aK.crm4.dynamics.com/00000000-0000-0000-0000-000000000001`, `saasAccountProblem` returns
  `null`: Next is enabled and no sentence is shown. The server then refuses the create with the
  D12 sentence. Nothing is created, so the cost is only that the form does not say it first; it is
  the one input where the two copies disagree.
- Fix: lower-case ASCII letters only in both places, for example
  `account.replace(/[A-Z]/g, (c) => c.toLowerCase())`.
- Check: add both accounts to "refuses each vendor's near misses" in
  `src/lib/connectors/saas-vendors.test.ts` (not `null`). Temporary wrong change: put
  `toLowerCase()` back; the test must fail.

### R5. SHOULD-FIX: three T9 fix sites carry no comment naming the case (rule 13, plan D1)

- Where: `src/features/alerts/alerts-page.tsx:215` (the list's red banner),
  `src/features/alerts/alert-rule-page.tsx:439` (the measures' error) and `:565` (the save's
  error). The one comment of T9 is at `alert-rule-page.tsx:509-515`, above `connectorsError`.
- What happens: a reader of the banner in `alerts-page.tsx` finds `readableErrorMessage` with no
  word on why or which QA case asked for it.
- Fix: one line at each site, for example
  `// QA LH-SRC-7-10 (SRC-40 D8): never the API's code-first text.`
- Check: none (comments).

