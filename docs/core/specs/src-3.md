# `SRC-3` Connectors for business apps

| | |
| --- | --- |
| Backlog | `SRC-3` in [BACKLOG.md](../BACKLOG.md) |
| Area | Data (Sources) |
| Who builds it | Data |
| When | Later |
| Size | L (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Waits for | `DEC-9` (decision) |
| Status | Spec. Not planned, not built |

## Why

There are no SaaS connectors: no CRM, ads, support or finance sources.

## What users get

Bring in data from business apps without building each connector by hand.

## Target specs

"Today" is `main` at `f3a3196`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Reuse route | None | Decision DEC-9: dlt verified sources (our loader already runs on dlt) or another library |
| First wave | None | Databricks' ten GA SaaS connectors: Salesforce, ServiceNow, Workday Reports, Google Analytics 4, HubSpot, Zendesk, Confluence, Dynamics 365, NetSuite, SharePoint |
| Second wave | None | Match Databricks' beta list (about 45), ordered by customer demand |
| Each connector | n/a | OAuth where offered (SRC-12), table and column choice, incremental loads, schema change handling (SRC-8), a gate test (SRC-9) |

## Benchmark

Databricks Lakeflow Connect GA: Salesforce, ServiceNow, Workday Reports, Google Analytics 4, HubSpot, Zendesk, Confluence, Dynamics 365, NetSuite, SharePoint, plus about 45 in beta. Airbyte: 602 sources. Snowflake Openflow: about 25.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- [ ] Every Target row above works as written
- [ ] Each first-wave connector connects with one sign-in, lists its objects and loads incrementally on a real account
- [ ] Each has a passing gate
- [ ] A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- A no-code builder for custom connectors *(proposed drop)*

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed to the AI team in [`AI-16`](ai-16.md).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
