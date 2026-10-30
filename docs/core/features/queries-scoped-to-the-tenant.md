# Queries are scoped to the tenant

| | |
| --- | --- |
| Module | Administration & Security |
| Backlog | `SEC-20` |
| Kind | Task |
| Spec | `docs/core/specs/sec-20.md` (no *(proposed)* number) |
| Status | In Progress |
| Priority | P2 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Who runs the acceptance checklist; not the Owner. Held in the base. |
| Started | 2026-10-24; also in `BACKLOG.md` Dates |
| Shipped | Not yet |
| Evidence | |
| Plan | Plan files are not kept in the repository. |

## Problem

`POST /api/query/run` checks the caller's permission and role policies, but never the caller's tenant
(`rust/crates/lakehouse-api/src/routes/query.rs:462-520`). A user in tenant B can run SQL on a table that tenant A
uploaded. The catalog has a tenant gate (`routes/catalog.rs:86-112`, `catalog_tenant_refusal`), but the query path
never calls it. Saved queries have no tenant column (`rust/migrations/0005_queries.sql:21-29`), and every list
returns all of them: the console's `GET /api/query/saved` (`query.rs:1076`), the chat's `list_saved_queries`
(`routes/ai/tools/mod.rs:199`), `run_saved_query`'s lookup (`tools/queries.rs:91`), and the "used by" list on a
table (`routes/catalog_governance.rs:206`).

## What the user can do when this is done

1. A user in tenant B who runs SQL on tenant A's uploaded table is refused, and nothing reaches the engine.
2. The same refusal applies wherever a person's SQL runs through the query path: Query Studio's run, estimate and
   download, the chat's `run_sql`, `propose_sql` and `run_saved_query`, and a catalog table's sample and profile.
3. A user in tenant B sees only tenant B's saved queries, in Query Studio, in the chat, and in a table's "used by"
   list, and cannot run tenant A's saved query by its id.
4. A deployment with one tenant behaves as today.

## Not included

- The dashboards, the embeds, the public links and the alerts. They call the role rewrite directly
  (`dashboard.rs:331-388`, `embed.rs`, `alerts.rs:644`), not the query path. `SEC-50` gates them in its second
  slice; its first slice already gates the authored pipelines and the Gold export. Of those two, only the authored
  pipeline can name a `bronze.*` table — the Gold export reads the shared catalog's `serving.*` marts — so the
  authored pipeline is the one path in this service where another tenant's upload can be reached.
- Tenants on Silver and Gold tables. Nothing maps them to a tenant today; they stay shared catalog tables under the
  catalog's gate (decision 1).
- Query history. It is already per user (`lakehouse-store/src/queries.rs:367-384`).

## Asking the assistant

Not an assistant feature. The chat's tools go through the same gate. The only changes in the assistant's files are
the two dispatch lines in `routes/ai/tools/mod.rs` (the chat's `save_query` and `list_saved_queries`), so that the
chat's saved-query tools know who asks (owner's exception, 2026-10-23 for the one line, extended to both on
2026-10-24, while #204 holds that file). The AI team reviews those lines.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | The rule for each table a statement reads: an uploaded table claimed by another tenant is refused (`upload_table_claim`, `lakehouse-store/src/uploads.rs:679`); an uploaded table claimed by the caller's tenant is allowed; every other table is a shared catalog table and follows the catalog's existing gate (the caller's tenant must own the catalog when there is more than one tenant). A principal with every permission (`*:*`) passes, as it does in the catalog | none | Owner, 2026-10-24 |
| 2 | The refusal is a 404 with one fixed sentence, "Table not found or not available to your organisation: `<name as written>`", the same for a table of another tenant and for one that does not exist. Never a 403: Query Studio's Run all reads a 403 as "this user may not run anything" and would refuse the other statements too (`QS-1` review) | 404 | Owner, 2026-10-24 |
| 3 | The check runs inside the shared query rewrite (`rewrite_sql_for_principal`, `query.rs:78`), so every entry point in item 2 above gets it at once | none | Owner, 2026-10-24 |
| 4 | The caller's tenant is the `X-Tenant` header when sent, else the caller's first tenant (`tenant_scope::resolve`, as the catalog does). The chat sends no header, so it uses the first tenant | none | Owner, 2026-10-24 |
| 5 | Saved queries get a tenant column, set from the caller's tenant when a query is saved (console and chat). Saved queries from before this change have none; they belong to the tenant that owns the catalog, so that tenant's members still see them, and so does everyone in a one-tenant deployment | none | Owner, 2026-10-24 |
| 6 | A principal with no tenant (a service token) in a deployment with more than one tenant sees no saved queries, and its SQL on shared tables is refused, as the catalog refuses it today. With `*:*` it passes | none | Owner, 2026-10-24 |
| 7 | No on/off switch. Tenant scoping follows the `tenant` table, as the catalog does | none | Owner, 2026-10-24 |
| 8 | A statement that lists objects without naming one table (`SHOW TABLES`, `SHOW DATABASES`, `SHOW SCHEMAS`, `SHOW VIEWS`, and every other `SHOW` form the read-only guard lets through except `SHOW COLUMNS`) is refused for a member of a tenant that does not own the catalog, with a 422 and one fixed sentence, "This statement is not available to your organisation." It passes for a deployment with one tenant, a principal with every permission (`*:*`), and the catalog's own tenant | 422 | Owner, 2026-10-26 |
| 9 | `DESCRIBE`, `DESC` and `SHOW COLUMNS FROM <table>` are tenant-gated like a `SELECT`: the table they name must be the caller's to read, and the refusal is the same 404 sentence a refused `SELECT` gets | 404 | Owner, 2026-10-26 |

## Limits to tell a customer

- An authored pipeline is scoped now (`SEC-50`, its first slice): it acts as its own tenant, so a pipeline of one
  tenant cannot read another tenant's uploaded table. A pipeline that belongs to no tenant is refused with a 409
  until an administrator assigns one with `PUT /api/pipelines/{id}/tenant`.
- The Gold export is scoped now. A signed-in person who triggers it acts as themselves, so their own tenant must own
  the shared catalog; a job with no user (the scheduled export, a manual trigger done as a service) acts as the tenant
  that owns the shared catalog. On a deployment with several tenants and no `CATALOG_TENANT_ID`, the export is refused
  outright until an operator sets it.
- The dashboards, the embeds, the public links and the alerts are not yet scoped this way; they follow in the second
  slice of `SEC-50`.
- Silver and Gold tables belong to the shared catalog, not to a tenant. With more than one tenant, only the
  tenant that owns the catalog can query them.
- A refused table reads "not found or not available to your organisation", whether it exists or not.
- The chat sends no tenant header, so it acts as your first tenant. Query Studio can act as another tenant
  (the header), so the two can read different tables and different saved queries.
- A saved query that belongs to no tenant — written before tenants existed, or by a principal with no tenant —
  is listed for the tenant that owns the shared catalog.
- On a dataset's page, the "used by" list is resolved against your first tenant while the page's own gate is
  resolved against the header's tenant, so with a header naming a second tenant the two can disagree.
- A member of another tenant cannot list tables or databases (`SHOW TABLES`, `SHOW DATABASES`), and `DESCRIBE`
  of another tenant's table reads as not found, like a `SELECT` of it.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass. Needs two tenants and a user in
each (operator).

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `SEC-20-AC1` | As a user in tenant B, run `SELECT * FROM <tenant A's uploaded table>` in Query Studio | Refused with "Table not found or not available to your organisation", no rows | Not run |
| `SEC-20-AC2` | As the same user, open Saved Queries, and ask the chat "list my saved queries" | Neither shows tenant A's saved queries | Not run |
| `SEC-20-AC3` | As a user without `query:read`, run any SQL | Refused with a permission message; nothing runs | Not run |
| `SEC-20-AC4` | As a user in tenant A, run SQL on tenant A's uploaded table | Rows come back as before | Not run |
| `SEC-20-AC5` | As the tenant B user, ask the chat to run tenant A's saved query by its id | Refused; the chat says it cannot find it | Not run |
| `SEC-20-AC6` | In Query Studio, Run all three statements where only the second reads tenant A's table | 1 shows rows, 2 is refused with the sentence above, 3 says "Not run" (not "refused") | Not run |
| `SEC-20-AC7` | (operator) On a deployment with one tenant, run the same SQL as before the change | Same result as before | Not run |
| `SEC-20-AC8` | As a tenant-B user, run `DESCRIBE <tenant A's uploaded table>` and `SHOW TABLES FROM bronze` | The 404 sentence for the `DESCRIBE` and the 422 listing sentence for `SHOW TABLES` | Not run |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` status set to **Done**, with the PR in the PR column; follow-ups added
- [ ] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo (status, PR, dates, QA-case results)
- [ ] `CHANGELOG.md` entry a customer can read
