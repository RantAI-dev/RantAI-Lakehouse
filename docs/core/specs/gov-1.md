# `GOV-1` Audit trail covers every console change

| | |
| --- | --- |
| Backlog | `GOV-1` in [BACKLOG.md](../BACKLOG.md) |
| Module | Governance |
| Size | L, in two parts (planner's estimate) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md) |
| Spec checked | Against the code at `10263dd7` on 2026-11-14. Feature page [`audit-trail-coverage.md`](../features/audit-trail-coverage.md) |

## Why

The audit trail (`audit_event`, migration `0024`) records about 87 of the 218 routes that change something. Identity, policies, governance, connectors, pipelines, settings (part), sharing and the assistant's tool calls are recorded. Saved queries and folders, dashboards and their parts (about 45 routes), alerts, report subscriptions, query limits and reporting settings, storage restore, workload cancel, table maintenance, role creation, service-identity creation and agent suspend/resume/revoke are not. Nothing checks that a new route records itself. The audit page loads the newest 200 events and filters them in the browser, so an older change cannot be found, and there is no export.

## What users get

Every change made in the console, by a person, the assistant, a schedule or a service identity, leaves one row: who, what action, on what, when, and whether it succeeded or was refused. An auditor can find a change from any date by person, action, object and time range, and export the result.

## Target specs

| Capability | Today | Target |
| --- | --- | --- |
| Routes that change state and record a row | About 87 of 218 | Every one, except a listed exclusion with its reason (Decision 1) |
| A new route that records nothing | Not caught | A test over `POLICY_TABLE` fails until the route records a row or is added to the exclusion list with a reason, like `tests/sec11_guard.rs` |
| A refused change (permission, validation) | Recorded by some routes | Recorded with outcome `refused` for every covered route *(proposed)* |
| What a row says about the change | Free-form `args`, per route | The names of the fields that changed, always; old and new values only for roles, policies, table access and grants, never a secret *(proposed)* |
| A failed audit write | Logged, the change stands (two paths fail closed) | Unchanged, plus a counter on the Operations health page *(proposed)* |
| Finding an event | Newest 200, browser-side filter | Server-side filter by person, action, object kind and id, outcome and time range, paged, any age |
| Export | None | CSV of the filtered result, up to 100,000 rows per file *(proposed)* |
| Who reads the audit page | `governance:read`, which every role holds (`0133_pages_per_role.sql`) | `audit:read` only (Governance Admin, `*:*`) |
| Changing or deleting a row | No code does it ("append-only by convention") | The database refuses `UPDATE` and `DELETE` on `audit_event` (trigger) *(proposed)* |
| Retention | Forever, no setting | Forever; a retention setting is left for later *(proposed)* |

## Benchmark

| Product | Audit trail |
| --- | --- |
| Databricks | `system.access.audit` system table, workspace and account events, kept 365 days free; queried with SQL |
| Metabase (Pro, Enterprise) | `audit_log`, `query_execution`, `view_log` behind Usage analytics; `MB_AUDIT_MAX_RETENTION_DAYS`, minimum 30 days, 0 keeps all |
| Superset | `logs` table of actions; no audit page |

Target: Databricks' coverage (every console change, searchable at any age, exportable) without its 365-day cut-off; Metabase's retention setting later.

## Acceptance checklist

On the feature page, [`audit-trail-coverage.md`](../features/audit-trail-coverage.md).

## Not included

- Reads (opening a page, running a query: `query.run` stays as it is), downloads stay as they are.
- A retention or purge setting.
- Sending audit events to an outside system (SIEM, webhook).
- Personal UI state: favourites, recent items, home layout, chat session titles (Decision 1).

## Asking the assistant

The assistant's own tool calls are already recorded (`routes/ai/audit.rs`). Asking the assistant "who changed this dashboard yesterday" is not included; it is a Governance read the AI team may add later (`AI-15` does not apply).

Sources for the benchmark: Databricks system tables page (`docs.databricks.com/aws/en/admin/system-tables`, `system.access.audit`, 365 days); Metabase environment variables (`metabase.com/docs/latest/configuring-metabase/environment-variables`, `MB_AUDIT_MAX_RETENTION_DAYS`).
