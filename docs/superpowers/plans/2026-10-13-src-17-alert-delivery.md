# `SRC-17` Load-failure alerts: delivery that is recorded honestly, and measured — Implementation Plan

**Status:** ready to build. `SRC-17` is a Task: what `SRC-7` (In Acceptance)
still lacks against its spec, `docs/core/specs/src-7.md`, found by the
delivery review of 2026-10-12
(`docs/superpowers/plans/2026-10-12-delivery-review.md`, `SRC-7`
SHOULD-FIX). The product owner chose it on 2026-10-13 as the one spec gap of
Phase 1 to close now. It changes no signed decision of the feature page
(`docs/core/features/load-failure-alerts.md`); it makes decision 1 and
checklist rows 2, 5 and 7 provable.
Written 2026-10-13 by the planner (Claude Opus) for a developer agent, under
the role split in `AGENTS.md`.

**Base and branch:** `main` at `432a3c9`; branch `fix/src-17-alert-delivery`.

## 1. What exists today (anchors at `432a3c9`; the developer re-verifies each before changing it)

| # | Spec row / checklist row | Code | Gap |
| --- | --- | --- | --- |
| F1 | Checklist row 5: "the failed delivery is shown with an honest message and no server text" | `lakehouse-alerts/src/lib.rs` about line 1665-1691 (the function `evaluate_connector_event` uses): the result of each delivery is discarded and the rule is returned as "delivered" either way; `routes/load_alerts.rs` then saves an alert instance for it. | A webhook that answers 500 leaves an alert that reads as sent. |
| F2 | Spec "every failed connector run … raises an alert" | `routes/load_alerts.rs` `connector_run_event` (about 274-346): the dedupe row (`record_pipeline_run_event`) is written first, then the rules are read from ClickHouse. If that read fails the route answers 503, the orchestrator retries, and the retry is skipped as "already recorded". | One rule-store failure loses the alert for that run, silently. The same order must be checked in the upload path (`upload_failure`, about 370-411). |
| F3 | Spec "Events: … schema change (with `SRC-8`)" | `src/features/alerts/alerts-page.tsx` and the doc of the kind in `lakehouse-alerts/src/lib.rs` say nothing raises `connector_schema_change`. `SRC-8` raises it. | The page tells the user a working alert does not work. (`connector_disabled` is still raised by nothing; that text stays until `SRC-11`.) |
| F4 | Spec "within 5 minutes *(proposed)*", signed as decision 1; checklist rows 2 and 7 | No gate receives a webhook or an e-mail; `docs/plans/SRC-7-RESULT.md` does not exist. | The number has never been measured (principle 5). |

## 2. Decisions (planner; none changes a signed decision)

- **D-a. A delivery's outcome is kept and shown.** Each attempted delivery
  ends as delivered or not delivered, with one of a fixed set of reasons the
  product words itself (for example: the address was refused, the server
  could not be reached, the server answered an error, e-mail is not set up,
  the mail server refused the message). Never the server's text, status
  line or body (principles 2 and 4). The alert instance stores it; the
  Alerts page shows "Not delivered" with the reason on that alert. An alert
  whose delivery failed is still listed: the event happened.
- **D-b. A rule-store failure does not lose the alert.** Recording the run
  (the dedupe row and the health facts, which must stay once-per-run) and
  delivering its alerts are separated, so that a retry of the same report
  after a failed delivery step delivers and does not count the run twice.
  How (a second marker written only after the rules were read, or another
  way that keeps both properties) is the developer's, with a test for each
  property: a rule-store failure followed by a retry delivers once; a retry
  after a full success delivers nothing and counts nothing.
  The orchestrator retries a report three times, three seconds apart
  (`pipeline_events.py`); a rule store down for longer still loses the
  alert. That limit is written on the feature page, not hidden.
- **D-c. The Alerts page says what is true.** `connector_schema_change` is
  described as raised by schema changes at the source; `connector_disabled`
  keeps its "nothing raises this yet" note.
- **D-d. The 5 minutes are measured by a gate.** A gate step, in a job of
  its own so no existing job grows (the SQL job is at 19m54s of 20):
  1. a webhook rule and an e-mail rule for failed runs of one connector; a
     run that fails (a wrong password on the MySQL source); the gate
     receives the webhook and the e-mail and prints the seconds from the
     run's end to each arrival;
  2. a webhook target that answers 500: the alert is listed as not
     delivered, with a fixed reason and none of the server's text;
  3. an upload that fails to load, with an upload rule: the webhook
     arrives; seconds printed.
  The receiver is part of the gate (a listener the gate starts, or a small
  container in the gate's compose profile); an SMTP sink container may be
  added to that profile only. Alert webhooks refuse internal addresses
  (`SEC-10`): the gate uses the documented opt-in for its own stack and
  says so in a comment; it must not weaken the default. The step fails if
  an arrival takes more than 300 seconds. Lines are printed as
  `[g…] SRC-17 …`.
  **Stop and report** instead of improvising if e-mail cannot be pointed at
  a plain SMTP sink without changing product behaviour, or if the opt-in of
  `SEC-10` does not exist in a form the gate can use.

### D-d as changed by the planner on 2026-10-13, after the developer stopped on both stop conditions

Found by the developer: `SEC-10` is not built (backlog: Todo), so alert
webhooks have no internal-address check and no opt-in; and the e-mail sender
has no mode without verified TLS, so it cannot reach a plain SMTP sink
without a product change.

- **Webhook: built now.** Steps 1 (webhook only), 2 and 3 of D-d. The
  gate's receiver is on the gate's own network; a comment at the step and
  in the job says that `SEC-10` will refuse such an address by default and
  must give this gate its documented opt-in when it lands. A line is added
  to `SEC-10`'s backlog row by the planner.
- **E-mail: not built, not measured.** No test-only plain-SMTP mode is
  added: a switch that turns off TLS verification for mail is a product
  and security decision, not a gate detail. `SRC-7-RESULT.md` says e-mail
  arrival is *not measured* and why. The product owner decides whether a
  measured e-mail path is wanted and how.

## 3. Tasks

One task per commit; the Rust tasks sit together. Cite `SRC-17` and the
finding at each site.

1. **Keep the outcome (D-a, store and crate).** The delivery function
   returns each rule with its outcome; the alert instance stores delivered
   or not and the reason. If a column is needed, a migration with the next
   free number and a why-header (ClickHouse or Postgres, wherever alert
   instances live; follow that store's existing migration pattern). Unit
   tests: a webhook answering 500, an unreachable host, a refused address,
   e-mail not configured: each gives its fixed reason and no upstream text.
2. **Show it (D-a, API and console).** The alerts list route returns the
   outcome; the Alerts page shows it. Route test and component test. The
   TypeScript contract mirrors the serde names.
3. **Do not lose the alert (D-b).** Connector runs and uploads. Route tests
   for both properties named in D-b, and for the "3 in a row" rule firing
   once when the third run's first report hit a rule-store failure.
4. **True texts (D-c).** Console and crate docs; the component test that
   pins the old sentence is corrected, and the commit says why.
5. **Gate (D-d).** The step, its job in `ci.yml`, `ci-required`,
   `required_gate.sh` and its self-test, `ops/gates/coverage.json` if the
   coverage check needs it, `docs/CI.md`.
6. **Docs (planner).** `docs/plans/SRC-7-RESULT.md` with the seconds CI
   measured, the feature page's limits, `CHANGELOG.md`, `BACKLOG.md`.

## 4. Build limits on this machine

No `cargo test`, `cargo build`, `cargo nextest`, `cargo clean`, no
containers. `df -h /` before cargo (stop under 12 GB free). Shared target
`/home/hv/.cache/lakehouse-src6-target`, `CARGO_BUILD_JOBS=2`,
`CARGO_INCREMENTAL=0`; touch every Rust source before the workspace clippy
(other worktrees share the directory); run it detached and poll. Rust tests
and the gate are *not verified* until CI on the pull request.

## 5. Out of scope

- A durable queue that re-sends an alert after a long rule-store outage.
- `connector_disabled` (raised by `SRC-11`).
- `SRC-18` and `SRC-19`.

## Handoff

**Developer (Claude Sonnet 5.5), 2026-10-13, branch `fix/src-17-alert-delivery`, base `432a3c9`.**
Tasks 1 to 4 built; **task 5 (the gate) STOPPED on both stop conditions of
D-d**, nothing of it was written. Task 6 is the planner's.

### Commits

1. `4167acd` task 1, D-a store and crate (migration `0068`).
2. `4703e85` task 2, D-a API and console.
3. `15c7519` task 3, D-b (migration `0069`).
4. `3e5b550` task 4, D-c true texts.

### D-a: where the outcome lives

- `lakehouse-notify`: `DeliveryFailure`, eight fixed sentences (webhook
  address refused / server could not be reached / server answered with an
  error; e-mail not set up / address refused; mail server could not be reached
  / refused the message; channel not supported). `DeliverResult.error` is now
  always one of them, so the status code, the driver's text and the SMTP
  reply are dropped. This also stops `/api/alerts/run` returning driver text
  in `delivered.error`.
- `lakehouse-alerts`: `evaluate_connector_event` returns `RuleDelivery`
  (`rule`, `result`) instead of the bare rules.
- Postgres `alert_instance`: migration `0068` adds `delivery_reason` and CHECKs
  the existing, never-written `delivery_status` (`delivered` | `not_delivered`;
  NULL = not known, never filled in for older rows).
  `overview::insert_from_fired_rule_with_delivery`; `AlertItem` carries
  `delivered?` and `deliveryReason?`; both persist paths (load alerts and
  `/api/alerts/run`) pass the outcome. Console: a Delivery column and a
  Delivery row in the drawer (`features/overview/alert-delivery.tsx`).
- Known limit: `insert_from_fired_rule` still dedupes 15 minutes per rule, so
  a failed delivery inside the window of an earlier instance of the same rule
  is not shown on its own row (unchanged primitive; the delivery itself is
  attempted).

### D-b: mechanism

Two rows, two steps. `connectors::record_run_once` writes the `(run_id, kind)`
row and the connector's health facts in one transaction and keeps the streak
the run produced (`pipeline_run_event.failure_streak`, migration `0069`).
Then the rules are read; only then is `(run_id, "<kind>_alerted")` written,
and delivery runs from the rules already in memory
(`lakehouse_alerts::load_rules` + `deliver_connector_event_to`), so the rule
store cannot fail after the run is counted.

- Rules unreadable: 503, run counted once, no `_alerted` row; the retry counts
  nothing again, delivers, and decides "3 in a row" from the run's own stored
  streak (never from the connector's current one), so it fires once.
- Everything went through: both rows exist; a retry delivers and counts
  nothing.
- Uploads (no health facts): the rules are read before the single dedupe row.
- Delivery is at most once: `_alerted` is written before the deliveries. A
  crash between that row and the sends loses the alert (the limit D-b already
  accepts; no durable queue, section 5).

### Stop conditions hit (task 5 not built)

1. `SEC-10` has no opt-in, it has no implementation: `BACKLOG.md` has it
   `Todo`; `lakehouse-notify::send_webhook` checks only the `http(s)` prefix
   and `deliver_event` uses a plain `reqwest::Client::new()`. There is no
   setting to name in a comment. Today a gate webhook to a container would
   work only because the check does not exist yet, and would break when
   `SEC-10` lands.
2. E-mail cannot be pointed at a plain SMTP sink without changing product
   behaviour: `EmailSender::send` uses `starttls_relay` (`SMTP_SECURE=false`)
   or `relay` (`true`); both need TLS with a certificate verified against the
   host name, and there is no plain mode and no way to trust a gate-made CA
   without changing the image or the sender.

Needed from the planner before task 5: `SEC-10` with its documented opt-in
landing first (or the gate waits for it), and a decision on e-mail (a plain
mode for tests is a product change; a TLS sink with a CA mounted into the
API image is a compose/image change).

### Commands run (all from `/home/hv/lakehouse-sec27`)

- `cd rust && cargo fmt --check`: clean.
- `cargo check -p lakehouse-notify -p lakehouse-alerts -p lakehouse-store`,
  `cargo check -p lakehouse-api`, `cargo check -p lakehouse-store -p
  lakehouse-api --tests` (shared `CARGO_TARGET_DIR`, jobs 2, no incremental):
  finished.
- `find crates -name '*.rs' | xargs touch`, then `cargo clippy --workspace
  --all-targets --all-features --locked -- -D warnings`, detached: `EXIT=0`
  (test code included, so every Rust test compiles).
- `bun install --frozen-lockfile`; `bun run typecheck`: clean; `bun run lint`:
  0 errors, 6 warnings (all in files this work did not touch, except the
  existing `rules` warning in `alerts-page.tsx`); `bun run test`: 1110 pass, 1
  skip, 0 fail (1111 tests, 114 files).
- `python3 ops/lint/check_intra_package_imports.py`, `check_bare_iceberg_count.py`,
  `check_connector_gate_coverage.py`: OK. `bash scripts/ci/tests/test_required_gate.sh`:
  30 passed, 0 failed. `ci.yml` parses.
- No Python or `dagster/` file changed, so no pytest and no `py_compile`.

### Not verified

- Every Rust test (`cargo test` is forbidden here). They compile (clippy
  `--all-targets`), none ran. CI is the proof.
- The mail-server-refused classification (`is_permanent() || is_transient()`
  to `MailServerRefused`) has no test: it needs an SMTP server.
- Migrations `0068` and `0069` ran against no database.
- The console was not opened in a browser.

### Existing tests changed

- `lakehouse-notify` tests that pinned upstream-shaped text:
  `webhook_rejects_non_http_url`, `webhook_reports_non_2xx_status`,
  `email_reports_unconfigured_smtp`, `email_rejects_invalid_address`,
  `deliver_dispatches_email_channel_and_still_validates_smtp`,
  `deliver_rejects_unknown_channel` now pin the fixed sentences.
- `lakehouse-alerts`: `evaluate_connector_event_matches_its_connector_star_and_kind_only`
  and `..._delivers_connector_disabled_and_schema_change_rules` read
  `.rule.id` (new return type); the doc of the second is corrected.
- `load_alerts.rs`: `a_success_run_of_a_connector_with_a_waiting_schema_change_is_a_success_and_never_a_failure`
  expected exactly one `success` row; it now expects `success` and
  `success_alerted` (still no failure row). The harness field `_clickhouse`
  is renamed `clickhouse` because tests now use it.
- `alerts-page.test.tsx`: the schema change sentence (task 4, reason in the
  commit).
- Greped and unchanged: `a_sensor_retry_neither_delivers_nor_counts_a_second_time`
  (still counts 1 `failure` row), the third-failure test, the
  `/api/alerts/run` tests (`delivered` stays absent when silenced).

### Tests added

notify: unreachable webhook, unreachable mail server, every sentence pinned.
alerts: `evaluate_connector_event_reports_each_deliverys_outcome_with_a_fixed_reason`.
store: `an_instance_records_whether_its_delivery_arrived`,
`record_run_once_counts_a_run_once_and_hands_a_retry_the_runs_own_streak`,
`record_run_once_leaves_no_dedupe_row_for_an_unknown_connector`.
api (`load_alerts.rs`): a webhook answering an error leaves a "not delivered"
instance; both D-b properties; "3 in a row" after a failed first report; the
upload path. Console: `alert-delivery.test.tsx`.

### Most likely to fail in CI

The `load_alerts.rs` route tests that make the mocked rule store answer 500
once (`up_to_n_times(1)` with `with_priority(1)`): they rely on wiremock
falling through to the harness mock afterwards, and on the Dagster/ClickHouse
mock servers staying alive (they do: held in `Harness`). Next, the migration
`0068` CHECK against any code path that writes `delivery_status`.


## Review

*(planner)*
