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

*(developer)*

## Review

*(planner)*
