# Glossary

Terms used in the product, its documents and the repository.

## Product terms

| Term | Meaning |
| --- | --- |
| **Lakehouse** | A data platform that keeps data as files in an open table format on object storage, and queries it like a database. |
| **Apache Iceberg** | The open table format the product uses. Any compatible engine can read it. |
| **Catalog** | The service that knows which tables exist and where their files are. Also the console page that browses it. |
| **Object storage** | File storage reached through the S3 interface. Where Iceberg data lives. |
| **Bronze** | Raw data as it arrived from a source. Stored in Iceberg. |
| **Silver** | Cleaned, row-level data. Stored in the analytics engine. |
| **Gold** | Curated tables ready for dashboards. Stored in the analytics engine. |
| **Mart** | One Gold table. |
| **Asset** | Any table or dataset listed in Data Explorer. |
| **Source / connector** | A registered external system data comes from. |
| **Change data capture (CDC)** | Continuously copying changes from a source database as they happen. |
| **Batch ingestion** | Loading data from a source on a schedule. |
| **Pipeline** | A defined, repeatable job that moves or transforms data. |
| **Run** | One execution of a pipeline. |
| **Orchestrator** | The component that schedules and runs pipelines and maintenance jobs. |
| **Publish / export** | Copying a Gold mart into an Iceberg table so outside tools can read it. |
| **Snapshot** | One version of an Iceberg table. Each write creates a new one. |
| **Compaction** | Merging many small data files into fewer large ones so queries stay fast. |
| **Maintenance** | Upkeep jobs on Iceberg tables: compaction, removing unreferenced files. |
| **Policy** | A rule about who may see which data. |
| **Classification** | A sensitivity label on a column. |
| **Masking** | Hiding a column's values from people not allowed to see them. |
| **Lineage** | The record of where a table's data came from and what uses it. |
| **Copilot / Ask AI** | The assistant that answers questions and performs actions through the console's tools. |
| **Digital employee** | An assistant configured to run on a schedule. Marked "Soon". |
| **Approval** | A person's decision on a destructive action an assistant requested. |
| **Service identity** | An account for a scheduled job, not a person. |
| **Tenant** | An organisation in the deployment. |
| **SQL source** | A saved read-only query a dashboard chart can be built on. |
| **Digest** | A scheduled text summary of a dashboard, sent by email or webhook. |
| **Not measured** | The console's wording for a value it does not know. Never shown as zero. |

## Process terms

| Term | Meaning |
| --- | --- |
| **Module** | A group in the console's navigation. |
| **Feature** | One capability a user would name. |
| **Sub-feature** | One observable thing within a feature. |
| **PRD** | Product requirements document. One per feature, in `docs/core/features/`. |
| **UAT** | User acceptance test. The script a product owner runs to accept a feature, in `docs/core/features/`. |
| **Plan** | The engineering plan for a feature, in `docs/superpowers/plans/`. |
| **Planner / reviewer** | The agent that writes the plan, reviews the work, and opens and merges the pull request. |
| **Developer** | A different agent that writes the code. |
| **Handoff** | The developer's record of what was done and what was verified. |
| **BLOCKER / SHOULD-FIX** | Review finding severities. A BLOCKER stops the merge. |
| **Ready / Done** | The two product gates; see [README.md](../README.md). |
| **Live** | In the navigation and backed by the real API. Not the same as accepted. |
| **Preview** | Hidden behind a "Soon" label. |

## Repository vocabulary

From `AGENTS.md`. These appear in code comments, commits and plans.

| Term | Meaning |
| --- | --- |
| **P0–P6** | Phases of the lakehouse foundation build. |
| **G1–G4** (and later gates) | Acceptance gates: repeatable tests under `ops/` that prove a capability end to end. |
| **R1–R11** | Entries in the engineering risk register. |
| **D1–D4** | Security fixes recorded in the changelog. |
| **ADR NNNN** | An architecture decision record in `docs/adr/`. |
| **RESULT document** | A recorded measurement in `docs/plans/*-RESULT.md`. Claims about performance cite one. |
| **Fail closed** | When a safeguard is not configured, refuse to run. |
| **Compose profile** | An optional set of components an operator can switch on. |

## Component names

| Name | Role |
| --- | --- |
| **ClickHouse** | Analytics engine. Holds Silver and Gold; reads Bronze. |
| **PostgreSQL** | The console's own application database. Also the main kind of source. |
| **Lakekeeper** | The Iceberg catalog. |
| **OpenFGA** | Authorization for the catalog. |
| **RustFS / SeaweedFS** | S3-compatible object stores. The first is the default; the second is the verified alternative. |
| **Dagster** | The orchestrator. |
| **Debezium** | Change data capture from PostgreSQL. |
| **dlt** | Batch ingestion. |
| **Trino** | Optional; used only to compact small files. |
