# `SRC-7` Alerts when a load fails

| | |
| --- | --- |
| Backlog | `SRC-7` in [BACKLOG.md](../BACKLOG.md) |
| Module | Data |
| When | Next |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

A failed connector run or upload raises nothing; someone has to open the console to notice.

## What users get

People hear about a failed connector run or upload within minutes, without opening the console.

## Target specs

"Today" is `main` at `f3a3196`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Failure alert | None | Every failed connector run and upload load raises an alert within 5 minutes *(proposed)* |
| Events | None | Failed run, repeated failures (3 in a row, proposed), connector disabled, schema change (with SRC-8); success alert optional per connector |
| Channels | n/a | Email and webhook, reusing the alert channels that exist (SEC-10 applies) |
| Connector health | Changes only on a manual test | Updated from every real run: last success, last failure, failure streak |

## Benchmark

Airbyte notifications: failed sync, successful sync, schema change, action required, repeated failures, connection disabled; email and webhook/Slack.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `SRC-7-AC1` Break a connector's password; the next run fails and an email and a webhook arrive within 5 minutes
- `SRC-7-AC2` The connector list shows the failure without a manual test
- `SRC-7-AC3` Success alerts are off unless switched on
- `SRC-7-AC4` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Slack and Teams apps (owner: out of scope)

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed to the AI team in [`AI-16`](ai-16.md).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
