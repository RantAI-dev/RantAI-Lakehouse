# Competitive comparison

How alternatives approach problems this product also has. Compiled
2026-10-02 from vendor documentation and articles found by web search. It is
desk research: nothing here was tested hands-on, and vendor features change
often. Re-check before quoting it to a customer.

Positioning against these alternatives is in
`GTM/ON-PREM-SALES-PLAYBOOK.md` section 2. This document is about product
approach.

## Who the alternatives are

| Group | Examples | Why a buyer considers them |
| --- | --- | --- |
| Cloud data platforms | Databricks, Snowflake, Microsoft Fabric | Complete, mature, well known |
| Open lakehouse engines | Dremio, StarRocks | Open formats, can be self-hosted |
| Self-assembled stack | An analytics database plus an orchestrator plus a BI tool plus an access-control tool | Free components, familiar to engineers |
| BI tools alone | Metabase, Apache Superset, Tableau, Looker | Dashboards over an existing database |
| Data operating platforms | Palantir Foundry | Governed, document-style reporting |

Our distinguishing position is one self-hosted stack with one login,
governed by default, on open formats. See
[PRODUCT.md](../PRODUCT.md).

## Topic 1 — Curated data in an open format

Our approach: curated (Gold) tables live in the analytics engine's own
format; an open Iceberg copy is made by a separate publish step.

| Product | Approach | Separate export step for the user? |
| --- | --- | --- |
| Databricks | One copy of data files; Iceberg metadata is generated alongside so Iceberg tools read the same files. Also offers managed Iceberg tables | No |
| Microsoft Fabric | Warehouse and lakehouse store one copy in an open format; a setting exposes tables as Iceberg with metadata generated on demand | No |
| Dremio | Tables are Iceberg. Speed comes from precomputed copies, also Iceberg, chosen automatically and hidden from users | No |
| StarRocks | Base tables stay in Iceberg; native materialized views accelerate them, with queries rewritten automatically | No |
| Snowflake | Has the same split as us (closed native tables, open Iceberg tables). Solved by making managed Iceberg tables first-class, chosen when a table is created | No; a choice per table |

**What this means for us.** Most competitors make the open table the source
of truth and any fast copy a hidden accelerator. We do the reverse. The
short-term answer is the per-mart publish option (`DATA-1`); the longer-term
answer is to reverse the direction (`DATA-3`).

## Topic 2 — Reports

Our approach today: dashboards with share and embed; text digests by email
or webhook; PDF through the browser's print function. No scheduled reports
with attachments.

| Product | Approach | Worth noting |
| --- | --- | --- |
| Databricks AI/BI | Subscriptions on a published dashboard: PDF snapshot to email or chat tools | Page selection, tabular attachments per widget, applied filters attached, external recipients |
| Microsoft Fabric | Interactive reports and separate paginated reports for print | Many export formats; subscriptions; one report rendered per recipient under row-level security |
| Looker | A scheduler on any dashboard or query | File destinations beyond email; send only if results are non-empty or changed |
| Apache Superset | Alerts and Reports share one engine: reports on a schedule, alerts on a condition | PDF or image for dashboards, data files for charts |
| Metabase | Dashboard subscriptions; Documents mixing charts and prose | PDF attachments arrived recently; subscriptions on Documents were still a request |
| Palantir Foundry | Templated documents with live or frozen embeds | Periodic reports from a template; frozen point-in-time versions |
| Tableau Pulse | Metric digests with generated summaries | Digest per followed metric, with follow-up questions |
| Snowflake, Dremio | Little native reporting; defer to BI tools | The gap a single stack can fill |

**What this means for us.** A Reports module would be a scheduled, governed
document layer over dashboards and saved queries, not another chart builder
(`RPT-1`). Pinning a report to an Iceberg snapshot would let a past report
be reproduced exactly, which suits an audit-minded buyer. No decision has
been taken.

## Topic 3 — Where we are behind

Honest gaps against the mature platforms, from
[PRODUCT.md](../PRODUCT.md):

| Area | Gap |
| --- | --- |
| Scale and resilience | Single node; no high availability |
| Sources | Live change capture for one database type |
| Maintenance | Snapshot expiry missing; compaction optional |
| Enterprise security | Rate limiting, full audit coverage, verified single sign-on |
| AI | No document retrieval |
| Maturity | Version 0.1; no human security review |

## Topic 4 — Where we are ahead, for our buyer

| Area | Position |
| --- | --- |
| Runs on premises | The cloud platforms cannot |
| One stack, one login | Against a self-assembled stack |
| Authorization on by default | Against a self-assembled stack |
| Human approval of AI writes | Built into the product |
| Honesty about limits | Limits are documented and shown in the interface |
| Acceptance gates the customer can run on their own host | Proof, not a slide |

## Sources

- [Databricks: scheduled dashboard updates and subscriptions](https://docs.databricks.com/aws/en/dashboards/share/schedule-subscribe)
- [Databricks AI/BI release notes 2026](https://docs.databricks.com/aws/en/ai-bi/release-notes/2026)
- [Delta UniForm](https://www.databricks.com/blog/delta-uniform-universal-format-lakehouse-interoperability)
- [Databricks Unity Catalog and Apache Iceberg in 2026](https://dataengineerhub.blog/articles/databricks-unity-catalog-iceberg-2026)
- [Power BI paginated reports FAQ](https://learn.microsoft.com/en-us/power-bi/paginated-reports/paginated-reports-faq)
- [OneLake: Delta tables as Iceberg automatically](https://blog.fabric.microsoft.com/en-us/blog/new-in-onelake-access-your-delta-lake-tables-as-iceberg-automatically/)
- [OneLake overview](https://learn.microsoft.com/en-us/fabric/onelake/onelake-overview)
- [Report delivery: scheduling, broadcasting, bursting](https://inforiver.com/blog/enterprise/power-bi-report-delivery-scheduling-broadcasting-bursting/)
- [Looker: scheduling and sending dashboards](https://docs.cloud.google.com/looker/docs/scheduling-and-sending-dashboards)
- [Superset: Alerts and Reports](https://superset.apache.org/admin-docs/configuration/alerts-reports/)
- [Metabase 63](https://www.metabase.com/releases/metabase-63)
- [Metabase: subscriptions for Documents](https://github.com/metabase/metabase/issues/68439)
- [Palantir Notepad overview](https://www.palantir.com/docs/foundry/notepad/overview)
- [Tableau Pulse primer](https://thinklytics.com/insights/what-is-tableau-pulse)
- [Dremio Live Reflections on Iceberg](https://www.dremio.com/blog/dremio-live-reflections-on-iceberg/)
- [StarRocks: data lake query acceleration](https://docs.starrocks.io/docs/using_starrocks/async_mv/use_cases/data_lake_query_acceleration_with_materialized_views/)
- [Snowflake Iceberg tables (2026)](https://www.flexera.com/blog/finops/snowflake-iceberg-table/)
- [Unifying Iceberg Tables on Snowflake](https://www.snowflake.com/en/blog/unifying-iceberg-tables/)
- [ClickHouse: writing data to open table formats](https://clickhouse.com/docs/use-cases/data-lake/getting-started/writing-data)

## Keeping this current

Add a topic when a feature is researched. Date every addition. Remove a
claim rather than leave it unverified for more than two releases.
