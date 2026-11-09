# `WEB-8` Forms offer the allowed values instead of free text

| | |
| --- | --- |
| Backlog | `WEB-8` in [BACKLOG.md](../BACKLOG.md) |
| Module | Platform |
| Size | M, in two parts |
| Spec checked | Against the code at `dcaae8a5` on 2026-11-09 |

## Why

Many forms take free text where the server accepts only a known list, so people need the docs or the code. The permission strings are spread through the API with no central list (`policy.rs` and handler literals), and the role form's example `query:run` does not exist (`roles-page.tsx:209-215`).

## What users get

Part 1: one central permission list in the API, served at `GET /api/identity/permissions`. Role permissions and service identity scopes are picked from it, and the server refuses unknown ones.

Part 2: invite roles and tenants as checkboxes (delivered in `ADM-11`), SSL mode lists in the SQL and Oracle connection forms, pipeline zone and column pickers from the namespaces and tables the page already loads, table and column pickers in data quality, freshness and alert rules, a time zone picker in report subscriptions, and a row filter hint that names the allowed functions.

## Target specs

| Capability | Today | Target |
| --- | --- | --- |
| Permission list | None; strings at each check site | `lakehouse_auth::permission_catalog`, with a test that every checked string, `POLICY_TABLE` entry and tool permission is in it |
| Role permissions, service identity scopes | Free text, comma-separated | Grouped checkboxes from the list; wildcards `area:*` and `*:*` offered |
| Unknown permission sent to the API | Stored silently, never matches | 400 naming the unknown strings; strings already on a role are kept |
| SSL mode (SQL, Oracle) | Free text | The list the connector accepts (`CDC_SSL_MODES`, `ORACLE_SSL_MODES`) |
| Pipeline zones | Free text | Picked from the namespaces the page loads |
| Pipeline source table | Free text with suggestions | Unchanged (`DATA-19` D4: may name a table that does not exist yet) |
| Pipeline columns | Free text | Picked from the table's columns when it exists, free text otherwise |
| Data quality, freshness, alert rule tables and columns | Free text | Picked from the catalog; columns from the table |
| Report subscription time zone | Free text | Picked from `listTimeZones()` |
| Row filter hint | Example only | Names the functions and operators the rewrite accepts |

## Benchmark

Metabase and Superset pick tables, columns and permissions from lists everywhere; neither asks for a permission string.

## Acceptance checklist

- `WEB-8-AC1` Create role: permissions are checkboxes grouped by area, and no `query:run` appears
- `WEB-8-AC2` Sending an unknown permission to `POST /api/identity/roles` (operator, curl) gives 400 naming it
- `WEB-8-AC3` Editing an old role that holds `notebook:run` keeps it and shows it as not recognised
- `WEB-8-AC4` Service identity scopes are picked from the same list
- `WEB-8-AC5` SQL and Oracle connection forms offer the SSL modes as a list
- `WEB-8-AC6` Adding a quality rule, a freshness target and a quality or freshness alert: the table and column are picked
- `WEB-8-AC7` Report subscription time zone is picked from a list
- `WEB-8-AC8` The row filter field names the allowed functions
- `WEB-8-AC9` Pipeline create: the zones are a list; the columns are a list once the table exists

## Not included

- Changing what the server accepts.

## Asking the assistant

The assistant already uses the server's lists through its tool schemas. Not changed.
