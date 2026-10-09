# Alerts when a load fails

| | |
| --- | --- |
| Module | Data (Sources) |
| Backlog | `SRC-7` |
| Spec | `docs/core/specs/src-7.md`. This page follows it row by row; anything the spec does not say is under "Beyond the spec" |
| Status | Decisions signed 2026-10-09. Built, not verified by CI, not accepted |
| Plan | `docs/superpowers/plans/2026-10-08-src-7-load-failure-alerts.md` |

## Problem

From the spec: a failed connector run or upload raises nothing; someone has
to open the console to notice. Checked in the code on 2026-10-08: the
orchestrator already reports every failed run to the API, but the API
recognises only pipelines a user authored and answers "unknown job" for a
connector run; a connector's health changes only when someone presses Test;
an upload's failure is recorded only when someone opens its page. Evidence is
in the plan, section 1.

## What the user can do when this is done

One statement per row of the spec's Target table.

| Spec row | Spec target | What the user can do |
| --- | --- | --- |
| Failure alert | Every failed connector run and upload load raises an alert within 5 minutes *(proposed)* | 1. Get an alert within 5 minutes of a connector run failing. 2. Get an alert within 5 minutes of an upload's load failing, without anyone opening its page. |
| Events | Failed run, repeated failures (3 in a row, proposed), connector disabled, schema change (with `SRC-8`); success alert optional per connector | 3. Choose, per rule, which event it is for: failed run, 3 failures in a row, connector disabled, schema change, or success. 4. Success alerts exist only for a connector someone switched them on for. |
| Channels | Email and webhook, reusing the alert channels that exist (`SEC-10` applies) | 5. Send each rule to an email address or a webhook, through the sender the other alerts use. |
| Connector health | Updated from every real run: last success, last failure, failure streak | 6. See on Sources, without pressing Test, each connector's last success, last failure and how many runs in a row have failed. |

Two of the five events have nothing that raises them today: nothing detects a
schema change (that is `SRC-8`) and nothing disables a connector (that is the
auto-disable of `SRC-11`). The spec itself ties schema change to `SRC-8`.
This work builds both as rule kinds with their delivery, and `SRC-8` and
`SRC-11` raise them. All three are on the Phase 1 branch, which is not
released until the phase is done, so no build ships a rule that cannot fire.

## Not included

From the spec:

- Slack and Teams apps (owner: out of scope). A Slack incoming webhook works
  as a webhook.

## Asking the assistant

From the spec: not part of the dashboards assistant; AI work for the Data
module is `AI-16`. The assistant's connector tools return the connector with
its new run fields, so the AI team reviews the tool schema snapshot.

## Decisions

All nine defaults signed by the product owner on 2026-10-09 ("do as recommended"). Rows 1 and 2 are the spec's *(proposed)*
numbers. Rows 3 to 5 settle how a spec row is met. Rows 6 to 9 are beyond
the spec.

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Spec *(proposed)*: how fast a failure alert arrives. | Within 5 minutes of the run ending. | 2026-10-09 |
| 2 | Spec *(proposed)*: repeated failures. | 3 failed runs in a row. Fires once, on the third; a success resets the count. | 2026-10-09 |
| 3 | Spec "Events": where a person chooses who is told. | Alert rules on the Alerts page, the list the other alerts use: one kind per event, each with a channel and a target. | 2026-10-09 |
| 4 | Spec "success alert optional per connector". | A success rule must name one connector; there is no "all connectors" success rule. With no such rule, no success alert is sent. | 2026-10-09 |
| 5 | Spec "connector disabled" and "schema change". | Built here as rule kinds with delivery; raised by `SRC-11` and `SRC-8`, on the same branch. | 2026-10-09 |
| 6 | Beyond the spec: what a run does to the health badge. The spec asks for three facts, not for a badge rule; its checklist asks that the list "shows the failure". | Success: healthy. 1 or 2 failures in a row: degraded. 3 or more: unhealthy. A manual Test still sets healthy or unhealthy as today. | 2026-10-09 |
| 7 | Beyond the spec: who may create a rule for all connectors, or for uploads. Rules are one list for the installation, so such a rule carries every tenant's connector names. | Only an administrator who sees every tenant. Others with `alert:write` may create a rule for one connector in their own tenant. | 2026-10-09 |
| 8 | Beyond the spec: what an alert says (principle 4). | The connector's or file's name, the run, a link into the console. Never the error text from the source system. | 2026-10-09 |
| 9 | Beyond the spec: the orchestrator reports runs only when `PIPELINE_RUN_TOKEN` is set, and it is optional in compose. | Keep it optional; the Alerts page says plainly when it is not set, so "no alerts" is never silent (principle 2). | 2026-10-09 |

## Limits to tell a customer

- A change-capture (CDC) connector streams through Debezium and has no runs,
  so it raises no failure alert here; its lag alert is `SRC-4`.
- Alerts need `PIPELINE_RUN_TOKEN` set on the API and the orchestrator.
- `SEC-10` applies to webhook targets until it is fixed (spec).
- Rules are shared by the whole installation, not per tenant.

## Acceptance checklist

Run on a real deployment. Mark each Pass, Fail, or Not run with the reason.
Rows 1 to 5 are the spec's checklist, in its order. Rows 6 to 12 make "every
Target row works as written" checkable and cover the decisions above.

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | Spec: every Target row above works as written | Rows 6 to 10 pass | |
| 2 | Spec: break a connector's password; run it, with a failed-run rule to an email address and one to a webhook | The run fails and an email and a webhook arrive within 5 minutes | |
| 3 | Spec: open Sources without pressing Test | The connector list shows the failure | |
| 4 | Spec: with no success rule, run a healthy connector | No success alert. Create a success rule for that connector; run again: one arrives | |
| 5 | Spec: as a user without `alert:write`, create a rule; and cause a delivery to fail (a webhook that answers 500) | The user is refused; the failed delivery is shown with an honest message and no server text | |
| 6 | Run the broken connector twice more, with a "3 in a row" rule | The rule fires once, after the third run | |
| 7 | Upload a file that fails to load, with an upload rule in place; do not open the upload | An alert arrives within 5 minutes | |
| 8 | Read the connector's row after rows 2 and 6, then fix the password and run | Last failure time and a count of 3; then last success time and a count of 0 | |
| 9 | Create a "connector disabled" rule and a "schema change" rule | Both saved. They fire in the `SRC-11` and `SRC-8` checklists, not here | |
| 10 | Read an alert from row 2 | It names the connector and the run and links to the console; no database error text | |
| 11 | As a user who is not an administrator: create a rule for all connectors, then for another tenant's connector | Both refused with a plain message | |
| 12 | (operator) Unset `PIPELINE_RUN_TOKEN`, restart, open the Alerts page | The page says run alerts are not being reported | |

**Accepted by:** __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` item moved to Done; follow-ups added
- [ ] `CHANGELOG.md` entry a customer can read
