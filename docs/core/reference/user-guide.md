# User guide

What each part of the console is for, and who uses it.

**Scope of this version.** This is a purpose-level guide written from the
navigation definition and the repository's documents. It does not give
click-by-click steps, because those were not checked against a running
console, and an unverified instruction is worse than none. Verified steps
are backlog item `DOC-2`; each feature's UAT script is the starting point.

The console opens on Home. The left navigation has the modules below, in
this order.

## Home

**For:** everyone.
**Use it to:** start work. Ask a question or give an instruction, and see
what needs your attention.

## Ask AI

**For:** analysts, data engineers.
**Use it to:** ask questions about your data in plain language, and ask the
assistant to do things: build a chart, add it to a dashboard, run a
pipeline.

What to expect:

- The assistant uses the same tools the console does, limited to your own
  permissions.
- Before a low-risk change it asks you to confirm.
- A destructive action is not carried out. It is sent to Approvals for a
  person to decide.
- It needs a language-model endpoint set up by your operator. Without one,
  it is unavailable.
- It works over tables. It does not search documents.

## Dashboards

**For:** decision makers, analysts.
**Use it to:** view and build dashboards, filter and drill into figures,
and share them.

- Organise dashboards in folders.
- Share by public link or signed embed.
- A chart can be built on one curated table or on a saved read-only query
  ("SQL source"). SQL sources are limited to 2,000 rows and 30 seconds.
- "Export PDF" opens your browser's print dialog.

## Data

**For:** analysts, data engineers, stewards.

| Page | Use it to |
| --- | --- |
| **Catalog** | Browse the namespaces and tables the lakehouse holds. |
| **Data Explorer** | Browse data assets by layer, from raw to curated, and open one to see its schema, a sample, quality checks, policies, lineage and history. |
| **Query Studio** | Write SQL, or ask in plain language and review the generated SQL before running it. Save queries you reuse. Queries are read-only. |
| **Sources** | Register the systems data comes from, and test the connection. A real connection test exists for PostgreSQL and S3-compatible storage; other types say "unsupported". |

The layers:

| Layer | What it holds |
| --- | --- |
| Raw / Bronze | Data as it arrived from the source, stored in open Iceberg tables |
| Silver | Cleaned, row-level data |
| Gold | Curated tables ("marts") ready for dashboards |

## Build

**For:** data engineers.

| Page | Use it to |
| --- | --- |
| **Pipelines** | Create pipelines that move and transform data; schedule them; make one run after others succeed; see run history, logs and step timings; re-run failed steps; set limits for how long a run may take and get alerted when it is slow, late, failed or wrote unusually little. |
| **Exports** | Copy a Gold mart into an open Iceberg table so tools outside the console can read it. This page is being replaced by a per-mart "Publish in open format" option on the mart's own page. |

Pipelines need the orchestrator to be running; your operator enables it.

## Governance

**For:** stewards, security officers, approvers.

| Page | Use it to |
| --- | --- |
| **Policies** | Define who may see what. |
| **Classification & Masking** | Label sensitive columns and mask them for people who should not see the values. |
| **Data Quality** | See the checks on each asset and whether they pass. |
| **Lineage** | See where a table's data came from and what depends on it. |
| **Approvals** | Decide on actions waiting for a person: destructive actions requested through the assistant, and access requests. Approving runs exactly what was requested; rejecting runs nothing. |

## Monitoring

**For:** operators, data engineers.

| Page | Use it to |
| --- | --- |
| **Health** | See the overall state of the platform. |
| **Alerts** | Create threshold rules and scheduled digests, delivered by email or webhook, and see what has fired. |
| **Audit Log** | See what happened: pipeline runs and assistant actions. It does not yet cover every change made in the console. |
| **Activity** | See recent activity across the platform. |
| **Workloads** | See what is running and what it costs in resources. |
| **Observability** | See operational signals. There is no log or trace explorer. |
| **Services** | See whether each component is up. A component that was never configured shows "unknown", not "down". |
| **Capacity** | See storage growth. |
| **Table Maintenance** | See Iceberg tables and their upkeep; run a dry run to see what maintenance would do. |
| **Ingestion (CDC)** | See the health of live change capture from source databases, including how far behind it is. |

## Intelligence

Marked "Soon". Digital Employees (assistants that run on a schedule) and
Agent Runs are not yet available for general use.

## Administration

**For:** platform admins.

| Page | Use it to |
| --- | --- |
| **Users** | Add people and assign roles. |
| **Teams & Roles** | Define roles and their permissions. |
| **Tenants** | Manage organisations. One deployment serves one organisation's data. |
| **Service Identities** | Manage the accounts scheduled jobs use. |
| **SSO** | Configure sign-in through your identity provider. |
| **Sessions** | See active sign-ins. |

There is no default administrator account. Your operator sets the first one
at installation.

## Things the console will always tell you honestly

- A value it does not know reads "Not measured", never zero.
- A function it does not support says "unsupported".
- A failed action is shown as failed, with a reason.

If you see a number you doubt, that is worth reporting: a wrong figure is
treated as the most serious kind of problem.

## Where to get help

See [support-model.md](support-model.md). For installation and
operation, see `docs/OPERATIONS.md`.
