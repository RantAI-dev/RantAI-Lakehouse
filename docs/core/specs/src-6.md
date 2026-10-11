# `SRC-6` Fix the broken connectors

| | |
| --- | --- |
| Backlog | `SRC-6` in [BACKLOG.md](../BACKLOG.md) |
| Module | Data |
| Size | S (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |

## Why

Three connector types are advertised but cannot be tested or edited, and the Sources page promises features that do not exist.

## What users get

Every connector type shown as supported can be created, tested and edited.

## Target specs

"Today" is `main` at `f3a3196`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Object-storage connectors from the wizard | Wizard and test disagree on the host format; keys cannot be changed | Create, test and change keys work for S3-compatible storage |
| Oracle | Credential change refused with 422; every test reads as failed | Credential change works; the test reports the real result |
| Google Sheets | Listed as supported, works nowhere | Shown as unsupported with a reason until it works (principle 2) |
| Sources page header | Promises SaaS and federation | Describes only what exists |

## Benchmark

Bugs found by reading the code during the Data audit.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `SRC-6-AC1` Each connector type in the picker can be created, tested and edited on a real deployment
- `SRC-6-AC2` Google Sheets shows 'not supported yet'
- `SRC-6-AC3` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- New connector types: SRC-3

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed to the AI team in [`AI-16`](ai-16.md).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
