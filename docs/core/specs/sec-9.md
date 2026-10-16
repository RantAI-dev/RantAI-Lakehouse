# `SEC-9` Plain-language queries respect masking

| | |
| --- | --- |
| Backlog | `SEC-9` in [BACKLOG.md](../BACKLOG.md) |
| Module | Administration & Security |
| Also involves | AI Copilot |
| Size | S (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |

## Why

The agentic Ask and text-to-SQL endpoints run model-written SQL straight against ClickHouse. A user who may not see a column can get it by asking in plain language.

## What users get

Asking in plain language never shows more than the same user could see in Query Studio.

## Target specs

"Today" is the 2026-10-07 audits; items marked re-checked were confirmed in the code by the planner. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Model-written SQL goes through the masking and row-filter rewrite | Runs straight against ClickHouse | The same rewrite_sql_for_principal and sql_rewrite::enforce path as POST /api/query/run, including the table-function and system.* blocks |
| Permission to run | Any signed-in user | query:read, like Query Studio |
| Tenant gate | None | The catalog tenant gate applies, as in Query Studio |
| Error text | Raw model and ClickHouse errors returned | Fixed, classified messages only (principle 4) |

## Benchmark

Re-checked by the planner: routes/agent.rs, policy.rs. Both competitors run AI queries inside the user's permissions.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `SEC-9-AC1` A user without access to a masked column asks for it in plain language and gets the masked value
- `SEC-9-AC2` A user without query:read gets 403 from both endpoints
- `SEC-9-AC3` A prompt asking for url('http://169.254.169.254/') is refused before reaching ClickHouse
- `SEC-9-AC4` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Changes to the model prompts beyond what the fix needs

## Asking the assistant

Not an assistant feature. Where a fix touches the assistant's endpoints, the AI team reviews it.

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
