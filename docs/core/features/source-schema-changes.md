# Schema changes at the source

| | |
| --- | --- |
| Module | Data (Sources) |
| Backlog | `SRC-8` |
| Spec | `docs/core/specs/src-8.md`. This page follows it row by row; anything the spec does not say is under "Beyond the spec". The spec has no *(proposed)* numbers |
| Status | Decisions signed 2026-10-09 and 2026-10-10. Built, not verified by CI, not accepted |
| Plan | `docs/superpowers/plans/2026-10-09-src-8-source-schema-changes.md` |

## Problem

From the spec: when a source table gains, loses or changes a column, the load
evolves silently and nobody is told. Checked in the code on 2026-10-09: the
loader hands each table to dlt with its default behaviour, which adds a new
column by itself and keeps an old one; nothing compares a table's columns
with what they were at the last run, nothing is recorded, and a connector has
no setting for it. A Bronze table's page shows past schemas from Iceberg
metadata, after the fact. Evidence is in the plan, section 1.

## What the user can do when this is done

One row per row of the spec's Target table.

| Spec row | Spec target | What the user can do |
| --- | --- | --- |
| Detection | Added, removed, renamed-as-removed-and-added, and type-changed columns detected on every run | 1. After any run, see which columns of a source table were added, removed or changed type since the run before. A rename shows as one removed and one added. |
| Per-connector policy | Four choices, matching Airbyte: apply non-breaking automatically (default), apply all, ask first, pause | 2. Choose one of four policies per connector; a new connector starts on "apply non-breaking automatically". |
| Breaking changes (column removed, type narrowed, primary key changed) | Always pause the affected table and ask, whatever the policy | 3. When a column is removed, a type is narrowed or the primary key changes, that table stops loading and waits for a person, under every policy. The connector's other tables keep loading. |
| Removed columns | Kept in the table and marked inactive, never dropped (Databricks behaviour) | 4. After approving a removal, keep the column and its old values in the table, marked inactive. |
| Notice | A notice on the connector page listing each change, and an alert (`SRC-7`) | 5. Read each change on the connector's page, and approve the ones that wait. 6. Get the "schema changed" alert that `SRC-7` built. |

## Not included

From the spec:

- Backfilling a newly added column's history (later).

## Asking the assistant

From the spec: not part of the dashboards assistant; AI work for the Data
module is `AI-16`. The assistant's connector tools return the connector with
its new policy and pause fields, so the AI team reviews the tool schema
snapshot.

## Decisions

Decisions 1, 2 and 4 to 8 were signed by the product owner on 2026-10-09 ("do as you proposed"). Decisions 3 and 9 were signed that day too, then changed by a measurement; the changed text and decision 10 were signed on 2026-10-10 ("run your recommendation"). Loading an incompatible SQL type into a second column on approval is follow-up work, `SRC-15`. Rows 1 to 4 settle how a spec row is met.
Rows 5 to 9 are beyond the spec.

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Spec "four choices, matching Airbyte": what each does to a non-breaking change (a column added, a type widened). | **Apply non-breaking automatically**: applied, listed, alert sent. **Apply all**: the same, and a table that appears in a schema the connector already loads from is added to it (Airbyte's "all field and stream changes"). **Ask first**: listed and alerted, not applied; the table keeps loading its known columns until someone approves. **Pause**: the whole connector stops loading until someone approves. | 2026-10-09 |
| 2 | Spec "apply all" needs the orchestrator to list a source's tables. | Built for PostgreSQL, MySQL, MariaDB and SQL Server. For other source types "apply all" behaves as "apply non-breaking automatically", and the policy choice says so. | 2026-10-09 |
| 3 | Spec "type-changed": which changes count as widened, which as narrowed. **Changed 2026-10-09 after measurement** (`docs/plans/SRC-8-RESULT.md`): "anything to text" and "more decimal digits" are no longer widenings for a SQL source, because the load fails. | Widened: a larger integer, float to double, a longer text. The reverse of each is narrowed. Every other change of type is breaking. | 2026-10-10 (as changed) |
| 4 | Spec "detected on every run": for which sources. | Batch SQL sources (PostgreSQL, MySQL, MariaDB, SQL Server, Oracle): all four kinds of change, checked before the table loads, so a policy can hold a change back. Files, REST, MongoDB, Kafka and SFTP: added columns and changed types, read from what was loaded, so the change is listed and alerted but already in the table; a removed column cannot be told from a column that is empty in this batch and is not reported. | 2026-10-09 |
| 5 | Beyond the spec: what "approve" does, and whether there is a "reject". | Approve only. Approving a removal marks the column inactive and resumes the table; approving a narrowed type or a changed key resumes it. A user who does not want the change removes the table from the connector. | 2026-10-09 |
| 6 | Beyond the spec: who approves and who sets the policy. | A user with `connector:manage` on a connector of their own tenant. | 2026-10-09 |
| 7 | Beyond the spec: a table that waits does not load. Is its run a failure? | No. The run reports that table as "waiting for a decision"; it raises no failure alert and does not count toward the failure streak of `SRC-7`. | 2026-10-09 |
| 8 | Beyond the spec: where "marked inactive" shows. | On the connector's page and in the table's column list (the Schema tab of the Bronze table). | 2026-10-09 |
| 9 | Beyond the spec: a type change the table cannot hold in the same column. **Changed 2026-10-09 after measurement**: the signed text said the new values land in a second column. That is true only for files, REST, MongoDB, Kafka and SFTP. For a SQL source the load fails, and keeps failing after the source is put back. | Files, REST, MongoDB, Kafka, SFTP: the new values land in a second column named `<column>__v_text`; the notice names both. SQL sources: such a change always makes the table wait, and is never loaded into the existing column. | 2026-10-10 (as changed) |
| 10 | Beyond the spec, new on 2026-10-09: what Approve does for a SQL type change the column cannot hold (integer to text, text to number, a changed decimal scale). | Approve is refused with a plain message: the table keeps waiting until the column is changed back at the source, or the table is removed from the connector and added again under a new target. Loading the new type into a second column, or rebuilding the table, is follow-up work. | 2026-10-10 |

## Limits to tell a customer

- Change-capture (CDC) connectors stream through Debezium and have no runs;
  their schema changes are not handled here (`SRC-4`).
- For files, REST, MongoDB, Kafka and SFTP a policy cannot hold a change back,
  and a removed column is not reported (decision 4).
- "Apply all" adds new tables only for PostgreSQL, MySQL, MariaDB and SQL
  Server (decision 2).
- A SQL column whose type changes to one its Bronze column cannot hold
  (integer to text, text to number) cannot be loaded: the table waits until
  the column is changed back or the table is re-added under a new target
  (decisions 9 and 10).
- A column added at the source is filled from the run that first sees it;
  older rows are empty in it (spec: backfill is later).

## Acceptance checklist

Run on a real deployment. Mark each Pass, Fail, or Not run with the reason.
Rows 1 to 5 are the spec's checklist, in its order. Rows 6 to 12 make "every
Target row works as written" checkable and cover the decisions above.

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | Spec: every Target row above works as written | Rows 6 to 10 pass | |
| 2 | Spec: add a column at the source; run | It appears in the table after the run and a notice lists it | |
| 3 | Spec: drop a column at the source; run | The table pauses, the column stays, the notice asks for a decision | |
| 4 | Spec: set "pause"; change anything at the source; run | The load stops until approved | |
| 5 | Spec: as a user without `connector:manage`, approve a change or set a policy; and make the orchestrator unable to reach the API during a run | The user is refused; the run fails with an honest message and nothing is loaded unchecked | |
| 6 | Rename a column at the source; run | One removed and one added are listed; the table pauses | |
| 7 | Narrow a column's type, then change a table's primary key; run each | The table pauses each time, under the default policy and under "apply all" | |
| 8 | Approve the removal from row 3 | The table loads again; the column is still there with its old values, marked inactive on the connector page and in the Schema tab | |
| 9 | Set "ask first"; add a column at the source; run | The table loads without the new column; the notice lists it; after approval the next run has it | |
| 10 | With a "schema changed" alert rule (`SRC-7`), repeat row 2 | The alert arrives, naming the connector and table | |
| 11 | Set "apply all" on a PostgreSQL connector; create a table in a schema it loads from; run | The table is added to the connector and loaded; the notice lists it | |
| 12 | While one table waits (row 3), look at the connector's other tables and its health | They loaded; no failure alert; the failure count did not rise | |

**Accepted by:** __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` item moved to Done; follow-ups added
- [ ] `CHANGELOG.md` entry a customer can read
