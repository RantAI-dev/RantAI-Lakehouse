# Read access per schema and table

| | |
| --- | --- |
| Module | Governance |
| Backlog | `GOV-7` |
| Kind | Feature |
| Spec | `docs/core/specs/gov-7.md` |
| Status | In Acceptance |
| Priority | P1 |
| Owner | In the base |
| Acceptor | In the base |
| Plan | `docs/superpowers/plans/2026-11-09-gov-7-table-access-per-role.md` |
| Started | 2026-11-09 |
| Shipped | 2026-11-11 |
| Evidence | PR #459 (rule, enforcement, routes), PR #468 (console, docs), PR #471 (`GOV-8` Create role, `GOV-9` rules on Edit role) |

## Problem

`query:read` reads every table. Masks and row filters can hide columns and rows, but nothing can hide a table, so a role cannot be limited to one schema.

## What the user can do when this is done

1. An administrator creates or opens a role and chooses "All tables" or "Only these", then picks whole schemas or single tables (`GOV-8` added it to Create role).
2. A person whose roles are all narrowed sees and queries only those tables, in Query Studio, dashboards, embeds and public links, alerts, reports, authored pipelines, Gold export, the assistant, the catalog and its search.
3. A table they may not read does not appear anywhere, and a query naming it gets the same "not found" as a missing table.
4. Existing roles keep reading every table until an administrator narrows them.
5. Changing a role's tables is recorded in the audit log.
6. On Edit role, the administrator sees the masks and row filters that name the role, each linking to the table's Access tab where it is changed (`GOV-9`).

## Not included

- Column hiding (masks do that).
- Data access for one person. It is set on roles only (owner, 2026-11-12).
- Making or changing a mask or row filter from the role form.
- Temporary grants adding tables.
- Service identities and the Iceberg door.

## Asking the assistant

The assistant's data map, SQL and metric answers follow the rule. Changing a role's tables through the assistant is not included.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| D1 | Existing and new roles read every table until narrowed | As stated | Owner, 2026-11-09 |
| D2 | A table that may not be read is hidden entirely; a query gets "not found" | As stated | Owner, 2026-11-09 |
| D3 | Several roles combine as a union; any "All tables" role means all tables | As stated | Planner |
| D4 | A role holding `*:*` cannot be narrowed | As stated | Planner |
| D5 | Embeds and public links follow the Dashboard Viewer role's tables | As stated | Planner |
| D6 | Access check budget under 5 ms at p95 | *(proposed)* | |

## Limits to tell a customer

- Service identities read what their scopes allow, regardless of table access.
- A temporary access grant gives a permission, not a table.
- Dagster's own builds and quality checks run as the system and are not narrowed.
- Narrowing the Dashboard Viewer role also narrows every embed and public link.
- A scheduled agent run has no person behind it and is not narrowed either (`SEC-68`).
- Names are compared without regard to case, as masks are. Two tables that differ only by case share one grant.
- The check costs one database read per governed request: about 3.4 ms at the median and 4.6 to 5.1 ms at p95, measured in an unoptimised test build on a shared machine (plan, S1 handoff, T5). A release build is not measured.
- A Bronze table is granted by its registry table name under the schema `bronze`. A Bronze dataset whose registry records no table name cannot be named by a grant, so it stays hidden from a narrowed role's catalog and search.
- Saving a role writes its tables only when they were changed on the page. The two are separate requests; if the second fails the page says the role was saved and its tables were not.
- Creating a role is two requests as well. If the tables are not saved, the new role reads every table until they are saved on its Edit page, which says so; the picked tables are not carried over.
- The list of masks and row filters on Edit role needs `policy:read`, and leaves out a rule written as plain text, which the engine does not enforce.

## Acceptance checklist

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `GOV-7-AC1` | Narrow a role "Finance" to schema `finance` and give it to a user with only that role | The user's catalog and search show only `finance` tables | Not run |
| `GOV-7-AC2` | As that user, run `SELECT * FROM silver.orders` in Query Studio | "Table not found or not available to your organisation" | Not run |
| `GOV-7-AC3` | As that user, run a query on a `finance` table | Rows come back as before, with masks applied | Not run |
| `GOV-7-AC4` | As that user, open a dashboard with one tile on `finance` and one on `silver.orders` | The first tile shows data; the second shows the not-found message | Not run |
| `GOV-7-AC5` | As that user, ask the assistant "what tables can I see" and "count rows in silver.orders" | Only `finance` tables listed; the second is refused as not found | Not run |
| `GOV-7-AC6` | Open `/data/assets/<id>` of a hidden table by address | Not found | Not run |
| `GOV-7-AC7` | Add a second role on "All tables" to the user | All tables visible again | Not run |
| `GOV-7-AC8` | Try to narrow Platform Admin | Refused with a plain message | Not run |
| `GOV-7-AC9` | Narrow Dashboard Viewer to `gold`; open an embed whose tile reads `silver` | That tile shows not found | Not run |
| `GOV-7-AC10` | An alert owned by the narrowed user on a hidden table runs | It fails with not found, recorded on the alert's history | Not run |
| `GOV-7-AC11` | Look at the audit log after narrowing | An entry names the role and the before/after tables | Not run |
| `GOV-7-AC12` | Upgrade a deployment with existing roles | Every role still reads every table | Not run |
| `GOV-7-AC13` | Create a role, choose "Only these", tick schema `finance`, Create; open the role again | The role's tables show `finance`, and the Roles list says "1 schema, 0 tables" | Not run |
| `GOV-7-AC14` | Create a role and leave "All tables" | The role is created and the Roles list says "All tables" | Not run |
| `GOV-7-AC15` | Open Edit role for a role that a mask or row filter names | The rule is listed with its table, what it does and its status; the table links to its Access tab | Not run |
| `GOV-7-AC16` | Open Edit role for a role no rule names | "No mask or row filter names this role" | Not run |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` status set to **Released**, with the PR in the PR column; follow-ups added
- [ ] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo
- [ ] `CHANGELOG.md` entry a customer can read
