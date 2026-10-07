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

## BI feature matrix: Tableau and Metabase

Compiled 2026-10-05 and updated the same day after PR #68 (map charts,
custom SQL in the chart builder). Our side is a code reading of `main` at
`47a826d`, not a test run, and no feature has been accepted by the product
owner yet. The competitor sides come from the official documentation and
pricing pages (help.tableau.com; metabase.com/docs and
metabase.com/pricing/compare-plans) as of October 2026. Metabase **Pro+**
means Pro or Enterprise, which have the same features; **OSS** is the free
edition. Tableau's tiers are its Creator/Explorer/Viewer roles and its
Standard, Enterprise, Cloud+ and Tableau+ editions.

Summary: 148 capabilities compared. We have 39, have 39 partly and lack 68.
A competitor has it fully where we are partial or missing in 106 rows; we
lead both in 8 (BI, lakehouse and pipelines in one product; AI that acts
with human approval; rose, calendar and radar charts; query cost and plan
panels). Metabase is the realistic benchmark; Tableau is the ceiling.

### The product owner's decisions (2026-10-06)

| Area | Decision | Backlog |
| --- | --- | --- |
| Where charts read data | Only data in the lakehouse, for now | `BI-21` (decided) |
| Result caching | Build it | `BI-4` |
| Query timeouts and limits | Build configurable limits | `BI-5` |
| No-code chart builder | Match the competitors' power. No drag and drop: the assistant covers that | `BI-6` |
| Joins without writing SQL | Build it | `BI-7` |
| Calculated fields and custom expressions | Build it | `BI-8` |
| Group by day, week, month, year | Build it | `BI-9` |
| Bins, groups, hierarchies | Build them. Sets: no | `BI-10`; `BI-23` (rejected) |
| Parameters | Build them | `BI-11` |
| Reusable metrics | Build them | `BI-2` |
| Curated datasets for authors | No: Gold tables are the curated datasets | `BI-12` (rejected) |
| Combine two sources on one chart | Build it | `BI-13` |
| Write-back from dashboards | No: we are a lakehouse, not an ERP | `DEC-7` (rejected) |
| Lineage from a chart to its sources | Build it | `BI-14` |
| SQL editor | Make it very good | `BI-15` |
| Chart types | Cover every type Tableau and Metabase have, including spatial analysis and Gantt | `BI-16` |
| Chart formatting | Reach both competitors' level, including forecasting and clustering | `BI-17` |
| Dashboards | Reach both competitors' level, except automatic dashboards (the assistant does that) | `BI-18`; `BI-25` (rejected) |
| Sharing and delivery | Cover everything except integrations with Slack, Teams, Office and Google, and except PowerPoint export | `BI-19`, `RPT-1`, `BI-1`; `BI-24` (rejected) |
| Embedding | Make it better without overdoing it: no SDK, no editing inside embeds | `BI-26` |
| Data preparation inside BI | No: the Build module is our data preparation | `BI-22` (rejected) |
| Everything else in the matrix | Noted; to be considered later | `BI-20` |

### The matrix

Status words: **Have**, **Partial**, **Missing**, **Not verified**, **n/a**.
**Have** means we match or beat the best competitor that offers it; anything
less is **Partial** (product owner, 2026-10-07).

### Data and modelling

| Capability | RantAI Lakehouse | Tableau | Metabase |
| --- | --- | --- | --- |
| Where charts read data | **Partial**. By design, charts read the lakehouse's curated Gold tables in ClickHouse; sources connect through Data → Sources and pipelines, not from the BI layer | **Have**. About 94 connectors, live or extract | **Have**. About 20 official drivers plus community drivers (self-hosted) |
| Result caching | **Missing**. Every request runs fresh; no server cache | **Have**. Extracts (Hyper), query cache, data freshness policy | **Have**. Adaptive caching on all plans; per-dashboard and scheduled caching Pro+ |
| Query timeouts and limits | **Partial**. SQL-source charts 2,000 rows and 30 s; mart charts 1–100 groups, 60 s API timeout | **Have**. Configurable | **Have**. 20 min default, configurable on self-hosted; row limits configurable |
| No-code chart builder | **Partial**. One dimension, 1–3 measures, optional breakdown, five aggregates, order and limit | **Have**. Drag-and-drop shelves, Show Me | **Have**. Notebook editor: joins, filters, custom columns, multiple stages |
| Joins without writing SQL | **Missing**. Only by writing SQL, now possible inside the chart builder | **Have**. Relationships, joins, unions | **Have**. Notebook joins, including multi-condition |
| Custom SQL as a chart source | **Have**. Write SQL directly in the chart builder and run it in place, or reuse a saved SQL source; needs dashboard:sql (new in #68) | **Have**. Custom SQL, Initial SQL, stored procedures (C) | **Have**. SQL questions; reference other questions as CTEs |
| Calculated fields / custom expressions | **Missing**. Only plain column names accepted | **Have**. Row-level, aggregate, LOD expressions, table calculations | **Have**. About 100 functions: math, text, dates, window, conditional |
| Group by day / week / month / year | **Missing**. No time grain anywhere in the builder | **Have** | **Have** |
| Bins, groups, sets, hierarchies | **Missing** | **Have**. All four | **Partial**. Binning and temporal units; no sets or hierarchies |
| Parameters | **Missing** | **Have**. Including dynamic parameters and parameter actions | **Have**. SQL variables, field filters, time-grouping parameters |
| Reusable metrics (semantic layer) | **Partial**. Built-in cards defined once in a deployment JSON file; users cannot define metrics (BI-2) | **Have**. Pulse metric definitions; Tableau Semantics on Next / T+ | **Have**. Metrics, segments, measures on all plans; library and verified metrics Pro+ |
| Curated datasets for authors | **Partial**. Gold marts are curated in Build; SQL sources act as saved datasets | **Have**. Published data sources, certification | **Have**. Models with column metadata and version history |
| Field metadata in the BI layer (display names, types, value remapping) | **Not verified**. Catalog descriptions exist in Data; not checked whether charts use them | **Have**. Aliases, default formats, roles, folders | **Have**. Data Studio metadata editor on all plans |
| Data preparation | **Partial**. Build module pipelines with five transformation steps | **Have**. Tableau Prep Builder and Conductor | **Partial**. SQL transforms; Python transforms are a paid add-on |
| Upload a file as data | **Missing**. Backlog DATA-9 | **Have**. Excel, CSV, JSON, PDF, spatial and more | **Have**. CSV upload (create, append, replace), all plans |
| Combine two sources on one chart | **Missing**. Each chart reads one mart or one SQL source | **Have**. Data blending | **Have**. Visualizer combines questions, even across databases |
| Write-back from dashboards | **Missing** | **Partial**. External Actions trigger Salesforce Flows | **Have**. Actions on all plans; editable tables Pro+ |
| Certified / verified content | **Missing** | **Have**. Certified data sources; Catalog certification (Ent) | **Have**. Verified content Pro+ |
| Lineage from a chart to its sources | **Partial**. Governance → Lineage exists but is shallow; not linked from dashboards | **Have**. Catalog lineage and impact analysis (Ent) | **Have**. Dependency graph Pro+ |
| Row-level security on dashboard queries | **Have**. Applied to tiles, previews, drill-down, embeds, public links and digests; not gate-tested for dashboards | **Have**. User filters, entitlement calculations, data policies (Ent) | **Have**. Row and column security, Pro+ only |
| Column masking | **Have**. Same policy engine; proven end to end for Query Studio (gate g8) | **Partial**. Through calculations or virtual-connection policies | **Have**. Column security, Pro+ only |

### SQL editor

| Capability | RantAI Lakehouse | Tableau | Metabase |
| --- | --- | --- | --- |
| Editor with highlighting and format | **Partial**. Highlighting and a small formatter; no tabs, themes or formatting rules | **Partial**. Custom SQL dialog only; no ad-hoc SQL workspace | **Have** |
| Autocomplete | **Partial**. Tables and keywords; no columns | **Missing** | **Have**. Tables, fields, snippets |
| Variables in SQL | **Missing** | **Have**. Parameters in custom and initial SQL | **Have**. Text, number, date, multi-value, field filters, optional clauses |
| Snippets | **Missing** | **Missing** | **Have**. Snippets; folder permissions Pro+ |
| Query history | **Have**. Per user | **n/a** | **Missing**. Not found as a feature |
| Saved queries | **Partial**. List and create only; no edit or delete; shared with everyone who has query:read | **n/a**. Workbooks instead | **Have**. Saved questions with 15-version history |
| Two engines to choose from | **Have**. ClickHouse or Trino in Query Studio (charts use ClickHouse only) | **n/a** | **n/a** |
| Cost estimate and query plan panels | **Have** | **Missing**. Not found | **Missing**. Not found |
| Stop a running query | **Partial**. Aborts the browser request; the server query keeps running | **Not verified** | **Not verified** |
| AI writes or fixes SQL | **Partial**. Writes SQL but skips governance (see findings); cannot fix a failing query | **Missing**. Tableau Agent writes calculations, not SQL | **Have**. Metabot, bring your own key on OSS |

### Chart types

| Capability | RantAI Lakehouse | Tableau | Metabase |
| --- | --- | --- | --- |
| Bar, line, area, stacked | **Have** | **Have** | **Have** |
| Combo / dual axis | **Have**. Bar plus line on two y-axes | **Have** | **Have**. Split y-axis |
| Pie / donut | **Have** | **Have** | **Have** |
| Scatter and bubble | **Have** | **Have** | **Have** |
| Heatmap | **Have** | **Have** | **Partial**. Only as pivot-table heatmap |
| Treemap | **Have** | **Have** | **Have**. Since v63 |
| Funnel | **Have** | **Partial**. Built by technique | **Have** |
| Waterfall | **Have** | **Partial**. Gantt technique or viz extension | **Have** |
| Sankey | **Have** | **Have**. Viz extension | **Have** |
| Sunburst | **Have** | **Have**. Viz extension | **Have** |
| Box plot | **Have** | **Have** | **Have** |
| Radar | **Have** | **Partial**. Partner viz extension | **Partial**. Custom visualization, Pro+ |
| Calendar heatmap | **Have** | **Partial**. Built by technique | **Partial**. Custom visualization, Pro+ |
| Rose (nightingale) | **Have** | **Missing** | **Missing** |
| Gauge | **Have** | **Partial**. Technique or extension | **Have** |
| Progress bar | **Missing** | **Partial**. Technique | **Have**. Goal from a constant, column or query |
| Histogram | **Missing** | **Have** | **Have** |
| Gantt | **Missing** | **Have** | **Partial**. Custom visualization, Pro+ |
| Bullet graph | **Missing** | **Have** | **Missing** |
| KPI with comparison, trend or sparkline | **Partial**. Plain big number only | **Have** | **Have**. Number with conditional colour; Trend with period-over-period |
| Table of raw rows | **Missing**. Tables are always grouped, at most 100 rows | **Have** | **Have**. Column formats, links, images, mini bars |
| Pivot table with totals | **Missing** | **Have**. Crosstab with totals and subtotals | **Have** |
| Record detail view | **Partial**. Drill-down shows up to 100 rows behind a value | **Have**. View data | **Have**. Object detail panel or page |
| Region (choropleth) map | **Partial**. Jakarta, Indonesia provinces and Indonesia kabupaten/kota (new in #68); no world map and no way to upload other boundaries; unmatched rows are counted on the tile | **Have**. Any geography, custom geocoding, spatial files | **Have**. World and US built in, custom GeoJSON |
| Point / pin map | **Have**. From latitude and longitude columns, sized and coloured by the measure; top 5,000 rows (2,000 on a SQL source) and says when capped (new in #68) | **Have** | **Have** |
| Density map | **Have**. Density heatmap from latitude and longitude (new in #68) | **Have** | **Have**. Grid map |
| Map pan and zoom | **Have**. Drag to pan, 1x–20x zoom buttons; outlines are bundled, no outside map server (new in #68) | **Have** | **Have**. Pin and grid maps |
| Spatial analysis (spatial joins, drive time) | **Missing** | **Have** | **Missing** |
| Text / markdown card | **Partial**. Plain markdown; no values from filters | **Have** | **Have**. With variables from filters |
| Image card | **Missing** | **Have** | **Have**. Images through markdown and table columns |
| Web page / iframe card | **Missing** | **Have** | **Have** |
| Custom chart plugins | **Missing** | **Have**. Viz extensions | **Have**. Custom visualization SDK, Pro+ |

### Chart formatting

| Capability | RantAI Lakehouse | Tableau | Metabase |
| --- | --- | --- | --- |
| Number formats (decimals, currency, %) | **Partial**. The stored format is ignored; numbers always round to whole numbers; tables and charts use different locales | **Have** | **Have** |
| Choose colours | **Missing**. One fixed 10-colour palette | **Have**. Custom palettes, dynamic colour ranges | **Have**. Per series; instance palette Pro+ |
| Axis titles, ranges, log scale | **Missing** | **Have** | **Have** |
| Data labels on points | **Missing** | **Have** | **Have** |
| Conditional formatting | **Missing** | **Have** | **Have**. Tables and number cards |
| Goal / reference lines | **Missing** | **Have**. Lines, bands, distributions | **Have**. Goal lines |
| Trend lines | **Missing** | **Have**. Linear, log, exponential, polynomial, power | **Have**. Time series |
| Forecasting | **Missing** | **Have**. Exponential smoothing | **Missing** |
| Clustering | **Missing** | **Have**. k-means | **Missing** |
| Annotations and events | **Missing** | **Have**. Point, mark, area annotations | **Partial**. Events and timelines on time series |
| Custom tooltips | **Missing** | **Have**. Including viz in tooltip | **Have**. Multiple metrics |
| Dark mode | **Have**. Charts follow the app theme | **Partial**. Dark viewer theme (Cloud 2026.2) | **Have**. Per user |
| Custom themes and fonts | **Missing** | **Have** | **Partial**. Light/dark on OSS; custom themes and fonts Pro+ |

### Dashboards

| Capability | RantAI Lakehouse | Tableau | Metabase |
| --- | --- | --- | --- |
| Many dashboards: create, rename, delete | **Have** | **Have** | **Have** |
| Grid layout with drag and resize | **Have**. 12-column grid, saved automatically | **Have**. Tiled and floating | **Have** |
| Tabs | **Missing** | **Partial**. Workbook sheets and dynamic zone visibility | **Have**. Tabs, move cards between tabs |
| Phone and tablet layouts | **Missing**. Tiles scale but do not stack on small screens | **Have**. Device-specific layouts | **Partial**. Responsive web only |
| Filter by a list of values | **Partial**. Multi-select only; 200 values, no search; values only from Gold marts | **Have** | **Have**. Dropdown, search box or input; values from field, question or custom list |
| Date-range and relative-date filters | **Missing**. A year filter exists but is hard-coded to a column named tahun and is not saved | **Have** | **Have** |
| Number and text filters | **Missing** | **Have** | **Have** |
| Linked (cascading) filters | **Missing** | **Have**. Only relevant values | **Have** |
| Default filter values | **Partial**. Any editor's ad-hoc change is silently saved as the shared default, which also changes public and embed views | **Have**. Custom views | **Have**. Defaults and required values |
| Cross-filtering by clicking a chart | **Partial**. Only for charts users created; built-in tiles ignore it | **Have**. Filter and highlight actions | **Have** |
| Drill down to underlying records | **Partial**. Up to 100 rows; not for SQL-source charts or nine chart types | **Have** | **Have**. Many drill-through actions |
| Click to another dashboard or URL | **Missing** | **Have**. Go-to-sheet and URL actions | **Have**. Custom destinations passing filter values |
| Parameter and set actions | **Missing** | **Have** | **Partial**. Click behaviour can set filters |
| Show or hide parts of a dashboard | **Missing** | **Have**. Dynamic zone visibility, show/hide buttons | **Partial**. Hide a card when it is empty |
| Auto-refresh | **Partial**. In the browser only; not saved with the board | **Partial**. Follows extract refresh | **Have**. 1–60 minutes, settable in the URL |
| Full-screen / presentation | **Partial**. A page overlay, not real full-screen | **Have**. Presentation mode | **Have** |
| Stories / written reports | **Missing** | **Have**. Stories | **Have**. Documents (all plans) |
| Templates and starters | **Missing** | **Have**. Dashboard starters, Exchange accelerators | **Have**. Section templates |
| Automatic dashboards from a table | **Missing** | **Partial**. Tableau Agent dashboard insights (beta) | **Have**. X-ray, all plans |
| Duplicate a dashboard | **Partial**. Filters are not copied; fails on the built-in board | **Have** | **Have** |
| Folders / collections | **Have**. Nested up to 4 levels; tree shows only in the board switcher | **Have**. Projects | **Have**. Collections, personal collections |
| Favourites / bookmarks | **Missing**. Only 'last visited', per browser | **Have** | **Have** |
| Search for dashboards | **Partial**. Search on the browse page only | **Have** | **Have**. Command palette and advanced search |
| Version history and revert | **Missing** | **Have**. Revision history | **Have**. 15 versions |
| Trash with restore | **Missing** | **Have**. Recycle bin, 30 days | **Have** |
| Personal space for drafts | **Missing** | **Have** | **Have** |

### Sharing and delivery

| Capability | RantAI Lakehouse | Tableau | Metabase |
| --- | --- | --- | --- |
| Public link | **Partial**. Random token, revocable; no expiry or password; built-in board cannot be shared | **Partial**. Only through Tableau Public | **Have**. All plans; admins can list and disable |
| Embed in another website (iframe) | **Partial**. Whole board or one chart, but the single-chart view is not enforced | **Have** | **Have** |
| Signed embedding | **Partial**. JWT with locked filters; expiry optional, no revocation | **Have**. Connected apps (JWT or OAuth) | **Have**. Guest embeds; 'Powered by Metabase' badge on OSS |
| Embedding SDK (JavaScript / React) | **Missing** | **Have**. Embedding API v3 web components | **Have**. Modular embedding and React SDK, Pro+ |
| Edit dashboards inside an embed | **Missing** | **Have**. Embedded web authoring | **Have**. Pro+ |
| White-labelling | **Missing**. 'Rantai Lake' is hard-coded in public and embed views | **Partial**. Logo, domain, toolbar; Tableau branding stays | **Have**. Pro+ |
| Permissions per dashboard or folder | **Missing**. Only global per role; the seeded Analyst role cannot view dashboards | **Have**. Project and content permissions | **Have**. Collection permissions, all plans |
| Comments on dashboards | **Missing** | **Have**. With @mentions | **Partial**. Only in Documents |
| Scheduled report emails | **Missing**. Backlog RPT-1 | **Have**. PNG/PDF, on schedule or on data refresh, skip if empty | **Have**. Email and Slack; CSV, XLSX and PDF attachments |
| Digests | **Partial**. Text-only list of KPI tiles; no schedule per digest, so each goes out every 15 minutes | **Have**. Pulse digests by email, Slack, Teams (Cloud) | **Have**. Subscriptions |
| Threshold alerts | **Partial**. One aggregate over a whole mart, no filter; re-fires every check while true | **Have**. Data-driven alerts (Explorer and up) | **Have**. Goal lines, results returned, custom cron |
| Alert channels | **Partial**. Email and a webhook shaped for Slack/Discord; no Teams | **Have**. Email, Slack; Teams for Pulse | **Have**. Email, Slack, webhooks |
| Export CSV | **Have**. In the browser; a server CSV/Parquet download exists without a button | **Have** | **Have** |
| Export Excel | **Missing** | **Have**. Crosstab XLSX | **Have** |
| Export PDF | **Partial**. Browser print with print styles | **Have** | **Have**. Dashboard PDF |
| Export image (PNG/SVG) | **Missing** | **Have**. PNG and SVG | **Have**. PNG |
| Export PowerPoint | **Missing** | **Have** | **Missing** |
| Slack / Teams apps | **Missing** | **Have**. Slack and Microsoft 365 apps | **Partial**. Metabot in Slack; no Teams |
| Office / Google add-ins | **Missing** | **Have**. Word, PowerPoint, Google Workspace | **Missing** |
| Dashboards as code (export / import, git) | **Partial**. YAML export of all boards; no import | **Partial**. Workbook files, Content Migration Tool (Ent) | **Have**. Serialization and Git remote sync, Pro+ |

### AI

| Capability | RantAI Lakehouse | Tableau | Metabase |
| --- | --- | --- | --- |
| Ask a question in plain language, get an answer or chart | **Have**. Needs a model endpoint | **Have**. Tableau Agent (Cloud+/T+) | **Have**. Metabot, bring your own key |
| AI creates charts | **Have**. Draft card with preview, save or open in builder | **Have** | **Have** |
| AI creates dashboards | **Have**. create_board tool | **Partial**. Tableau Next Data Pro | **Partial**. Dashboards as code through CLI and agents, Pro+ |
| AI explains a dashboard or chart | **Partial**. The assistant reads tile titles, first rows and filters from the page | **Have**. Explain Data; dashboard overview and insights (beta) | **Have**. Chart analysis and summaries |
| Automatic insights and anomaly alerts | **Missing** | **Have**. Pulse insights; Inspector agent on Next | **Partial**. X-ray |
| AI creates alerts | **Have**. Alert tools in the assistant | **Not verified** | **Have**. Metabot in Slack |
| Human approval before risky AI actions | **Have**. Approvals queue; delete_chart goes through it | **Missing**. Not found | **Missing**. Not found |
| AI respects data permissions | **Partial**. Assistant tools yes; the natural-language query endpoints do not (see findings) | **Have** | **Have**. Within the user's permissions |
| Choose your own model provider | **Have**. Any configured endpoint | **Partial**. Einstein Trust Layer; own LLM on Server | **Have**. Seven providers, plus a paid Metabase AI service on Cloud |
| MCP server for outside AI tools | **Missing** | **Have**. Open-source and managed | **Have**. All plans |
| AI usage controls and audit | **Partial**. Every AI tool call is audited; no per-user limits | **Have** | **Have**. Limits, system prompts, usage audit, Pro+ |

### Admin and platform

| Capability | RantAI Lakehouse | Tableau | Metabase |
| --- | --- | --- | --- |
| Audit of dashboard changes and views | **Missing**. Only Query Studio runs and AI tool calls are audited | **Have**. Activity Log, Admin Insights | **Have**. Usage analytics, Pro+ |
| Dashboard usage analytics | **Missing** | **Have** | **Have**. Pro+ |
| Single sign-on | **Partial**. Built, never tested with a real identity provider | **Have**. SAML, OIDC, Google, Salesforce | **Have**. Google and basic LDAP on all plans; SAML, OIDC, JWT Pro+ |
| SCIM provisioning | **Missing** | **Have** | **Have**. Pro+ |
| Two-factor sign-in | **Missing** | **Have** | **Have**. Pro+ |
| Several organisations in one install | **Partial**. Dashboard tables are not separated by tenant | **Have**. Sites | **Have**. Tenants, Pro+ |
| API for dashboards | **Have**. Full create, read, update, delete for boards, charts, folders, sources | **Have**. REST, Metadata (GraphQL) | **Have**. REST with OpenAPI spec |
| Webhooks on content events | **Missing** | **Have**. Workbook, data source, refresh events | **Partial**. For alerts only |
| Interface languages | **Missing**. English, with some Indonesian strings mixed in | **Have**. About 15 languages | **Have**. 34 languages |
| Accessibility | **Partial**. Labels present; drag and resize need a mouse | **Have**. Targets WCAG 2.2 AA; keyboard-only authoring | **Partial**. Not yet fully WCAG 2.1 AA |
| Native mobile app | **Missing** | **Have**. iOS and Android | **Missing** |
| Self-hosted | **Have** | **Have**. Tableau Server | **Have**. Open source or Pro |
| BI and lakehouse in one product | **Have** | **Partial**. Tableau Next with Salesforce Data 360 | **Missing** |
| Pipelines in the same product | **Have**. Build module | **Partial**. Tableau Prep | **Partial**. Transforms |
| Price | **Not verified**. Not set | **n/a**. Standard: Creator $75, Explorer $42, Viewer $15 per user per month; Enterprise $115 / $70 / $35 | **n/a**. Open source free; Starter $100/mo (5 users); Pro $575/mo (10 users); Enterprise from $20,000/yr |

## Data module matrix: Databricks, Snowflake and Airbyte

Compiled 2026-10-07. Our side is a code reading of `main` at `f3a3196` (after
PR #71), not a test run. Competitors come from docs.databricks.com (AWS),
docs.snowflake.com and docs.airbyte.com with Airbyte's live connector
registry, October 2026. Airbyte is compared for Sources only. **Have** means we
match or beat the best competitor that offers it; anything less is
**Partial**. The product owner has not yet decided this module area by area;
the task list is backlog `SEC-14` to `SEC-18`, `SRC-3`, `SRC-6` to `SRC-13`
and `DATA-11` to `DATA-19`, with edge cases in `DEC-10`.

Summary: 69 capabilities. We have 25, have 23 partly and lack 19.

#### Catalog

| Capability | RantAI Lakehouse | Databricks | Snowflake | Airbyte |
| --- | --- | --- | --- | --- |
| Browse by namespace and layer | **Have**. Namespaces page; Raw, Bronze, Silver and Gold quick filters | **Have**. Catalog, schema, table tree | **Have**. Horizon Catalog Explorer | **n/a** |
| Search | **Partial**. Substring match over name, namespace, description, owner and id; no column or tag search | **Have**. Names, comments, column names, tags; filters and search syntax | **Have**. Universal Search over metadata, fuzzy | **n/a** |
| Filter, sort and group assets | **Have**. 14 operators, and/or, group-by with summaries | **Have** | **Have** | **n/a** |
| Asset page with tabs | **Have**. Seven tabs: Overview, Schema, Sample, Quality, Access, Lineage, Activity | **Have**. Columns, Sample, Details, Permissions, Policies, History, Lineage, Insights, Quality | **Have**. Details, Columns, Data Preview, Copy History, Lineage, Data Quality | **n/a** |
| Edit description, owner, steward, tags | **Have**. Audited as catalog.annotate | **Have** | **Have**. Contacts with purposes (steward, approver) | **n/a** |
| Governed tags (allowed values, policies) | **Missing**. Free-text tags only | **Have**. Governed tags with tag policies | **Have**. Tag propagation and tag-based masking (Ent+) | **n/a** |
| Data preview | **Partial**. 25–100 rows, masked and row-filtered; Databricks shows about 1,000 and answers questions over them | **Have**. About 1,000 rows, plus plain-language questions over the sample | **Have**. 100 rows | **n/a** |
| Column profiling | **Have**. Nulls, distinct count, range, top values over up to 100,000 rows | **Partial**. Statistics in the results grid; profiling as a separate monitor | **Have**. Data Profile tab (Ent+) | **n/a** |
| Schema versions over time | **Have**. Iceberg metadata, plus Silver and Gold versions recorded every 15 minutes | **Partial**. History tab; no schema-change view | **Partial**. Schema evolution in SQL; no UI | **n/a** |
| Table-level lineage | **Partial**. Recorded edges only (pipelines, builds, exports, views); nothing inferred | **Have**. Automatic, across workspaces | **Have**. Automatic (Ent+) | **n/a** |
| Column-level lineage | **Missing**. Always empty | **Have** | **Have**. Ent+ | **n/a** |
| Lineage from outside systems | **Missing** | **Have**. External lineage | **Have**. OpenLineage ingest | **n/a** |
| What uses this table | **Have**. Saved queries and dashboards whose SQL reads it | **Have**. Insights: frequent queries, users, dashboards | **Partial**. Lineage only | **n/a** |
| Usage and popularity | **Partial**. Queries, users and latency over 7 days; Databricks shows 30 days, frequent users and column popularity | **Have**. 30-day usage, column popularity | **Missing**. Not documented | **n/a** |
| Quality rules on an asset | **Partial**. Create, run and delete rules, health badge; no built-in metric library or anomaly checks like Snowflake's | **Have**. Anomaly detection and data profiling | **Have**. Data metric functions and expectations (Ent+) | **n/a** |
| Automatic freshness and volume anomaly checks | **Partial**. Freshness against an SLA target; no anomaly detection | **Have**. Per schema (PP) | **Have**. Ent+ | **n/a** |
| Quality dashboard with incidents | **Missing** | **Have** | **Have**. Preview | **n/a** |
| Classification and masking on the asset | **Have**. Badges, rule add/remove, inline masking and row-filter policy creator | **Have**. Policies tab (ABAC) | **Have**. Masking policy per column (Ent+) | **n/a** |
| Automatic sensitive-data scanning | **Not verified**. Classification rules exist in Governance; automatic scanning not checked | **Have**. About 90 classes, auto-tagging | **Have**. Classification profiles (Ent+) | **n/a** |
| Change history and activity | **Have**. Audit events plus schema versions | **Have**. History tab | **Partial**. Copy history; access history in SQL | **n/a** |
| Freshness and SLA | **Have**. Lag against target; set from the Activity tab | **Have** | **Have**. Freshness metric | **n/a** |
| Snapshot history | **Have**. Iceberg snapshots with a row-count chart | **Have** | **Partial**. SQL only | **n/a** |
| Query a past version (time travel) | **Partial**. Opens Query Studio with version syntax; not proven to pass the masking rewrite | **Have**. VERSION and TIMESTAMP AS OF, RESTORE | **Have**. 1 day on all editions, up to 90 days on Ent+ | **n/a** |
| Undrop a table | **Missing** | **Have** | **Partial**. SQL only | **n/a** |
| Storage details | **Have**. Iceberg files and partitions; ClickHouse engine and parts | **Have** | **Have** | **n/a** |
| Request access | **Have**. Decided in Approvals | **Have**. PP | **Have**. Preview | **n/a** |
| Grant and revoke from the asset | **Partial**. Access tab shows classification and policies; no grants | **Have**. Permissions tab | **Have**. Privileges section | **n/a** |
| Certification and deprecation marks | **Missing** | **Have** | **Have**. Certification-status tag | **n/a** |
| Business glossary or domains | **Missing** | **Partial**. Domains (PP), Pages (Beta) | **Partial**. Semantic views | **n/a** |
| Entity relationship diagram | **Missing** | **Have**. From declared keys | **Missing** | **n/a** |
| Favourites | **Missing** | **Missing**. Not documented for tables | **Missing**. Not documented | **n/a** |
| Open table format readable by other engines | **Partial**. Raw data in Iceberg; Gold only as a published copy for tables switched on (merged, not accepted) | **Have**. Delta and Iceberg | **Have**. Iceberg, Horizon REST catalog | **n/a** |
| Browse other systems' catalogs (federation) | **Missing**. Backlog QRY-1 | **Have**. Foreign catalogs | **Have**. Catalog-linked databases | **n/a** |

#### Data Explorer

| Capability | RantAI Lakehouse | Databricks | Snowflake | Airbyte |
| --- | --- | --- | --- | --- |
| Asset table with filters, grouping and CSV export | **Have**. Server-side paging, layer quick filters, row menu | **Partial**. Search results page | **Partial**. Explorer lists per schema | **n/a** |

#### Sources

| Capability | RantAI Lakehouse | Databricks | Snowflake | Airbyte |
| --- | --- | --- | --- | --- |
| Working connector types | **Partial**. PostgreSQL, MySQL/MariaDB, SQL Server, S3 files, REST, Kafka (batch), MongoDB, PostgreSQL CDC; Oracle, SFTP and MySQL/SQL Server CDC never gate-tested; Sheets does not work | **Have**. About 60 managed connectors, 10 SaaS ones generally available | **Have**. Openflow, about 25 connectors | **Have**. 602 sources, 66 certified |
| Database change capture (CDC) | **Partial**. PostgreSQL gate-tested; MySQL and SQL Server untested; Debezium added by an operator | **Have**. SQL Server GA; MySQL and PostgreSQL in preview | **Have**. PostgreSQL, MySQL, SQL Server, Oracle | **Have**. PostgreSQL, MySQL, SQL Server, MongoDB |
| SaaS connectors | **Missing**. Backlog SRC-3 | **Have** | **Have** | **Have** |
| Files from object storage | **Partial**. S3 csv/json/parquet; testing is broken for wizard-made connectors | **Have** | **Have** | **Have**. Compression and glob patterns |
| Streaming sources | **Partial**. Kafka batch poll | **Have** | **Have** | **Partial**. Community connector |
| Create wizard | **Have**. Type, connection, tenant, review, then a real test | **Have** | **Have** | **Have**. Form generated from the connector spec, docs alongside |
| Test connection | **Partial**. Real for five types; broken for object storage; Oracle always reads as failed | **Have** | **Partial** | **Have**. With suggested fixes |
| Write-only credentials | **Partial**. Write-only and never returned, but stored as plain files at rest; no secret manager | **Have**. Connection objects | **Have**. Secret objects | **Have** |
| External secret managers | **Missing** | **Have**. Beta | **Have**. Preview | **Have**. AWS, GCP, Azure, Vault |
| OAuth sign-in to a source | **Missing**. REST OAuth2 answers unsupported | **Have** | **Have** | **Have**. One-click on cloud; own app on Core |
| Change credentials | **Partial**. Works, except Oracle and object storage | **Have** | **Have** | **Have** |
| Discover the source's tables | **Partial**. SQL and CDC sources only | **Have** | **Have** | **Have**. Asynchronous for large schemas |
| Choose tables and columns | **Partial**. Tables; column choice not checked | **Have**. Plus row filtering | **Have** | **Have**. Top-level fields |
| Load modes | **Partial**. Replace, append, incremental by cursor; no deduplication or history (SCD) modes | **Have**. SCD type 1 and 2, full refresh | **Have** | **Have**. Five sync modes |
| Deduplicate on a key | **Missing** | **Have** | **Have** | **Have**. Append + deduped |
| Schema changes at the source | **Partial**. Default schema evolution; nobody is told | **Have**. New columns added, removed ones marked inactive | **Have** | **Have**. Propagate, approve or pause; breaking changes pause |
| Schedules | **Have**. Cron per connector | **Have** | **Have**. Tasks | **Have**. Interval or cron |
| Run history with per-table results | **Partial**. Per-table results and test history; no log view found | **Have**. Event logs | **Have**. Copy history | **Have**. Timeline with logs |
| Alert when a load fails | **Missing**. Failures raise nothing; health changes only on a manual test | **Have** | **Have** | **Have**. Email (cloud) and webhook |
| Automatic retries | **Not verified**. Not checked for ingest jobs | **Have** | **Not verified** | **Have**. Backoff and attempt limits |
| Connector page | **Have**. Overview, Ingest and Tests tabs | **Have** | **Have** | **Have** |
| Delete and clean up | **Have**. Blocked while pipelines depend on it; CDC slot dropped | **Have** | **Have** | **Have** |
| Build your own connector | **Missing** | **Partial**. Custom connectors | **Partial**. NiFi flows | **Have**. No-code builder and SDKs |

#### File upload

| Capability | RantAI Lakehouse | Databricks | Snowflake | Airbyte |
| --- | --- | --- | --- | --- |
| Upload a file from the console | **Have**. Merged in #71, not accepted | **Have** | **Have** | **n/a** |
| Formats | **Partial**. Delimited text only (CSV, TSV and similar) | **Have**. CSV, TSV, JSON, Avro, Parquet, text | **Have**. CSV, TSV, JSON, Avro, ORC, Parquet, XML | **n/a** |
| Size | **Partial**. 50 MB and 2,000,000 rows per file | **Have**. Up to 2 GB | **Have**. 250 MB per file | **n/a** |
| Several files at once | **Missing** | **Have**. Up to 10 | **Have**. Up to 250 | **n/a** |
| Preview with detection | **Have**. 20 rows; encoding, delimiter and header detected, each overridable | **Have**. 50 rows | **Have** | **n/a** |
| Set column names and types before loading | **Missing**. Every column stored as text; names cleaned automatically | **Have** | **Have** | **n/a** |
| Append or replace | **Have** | **Partial**. Create or overwrite only | **Have** | **n/a** |
| Duplicate-file notice | **Have**. Same file uploaded twice is flagged | **Missing** | **Missing** | **n/a** |
| Protection against overwriting another table | **Have**. Table claims refuse connector and other-tenant tables | **Missing** | **Missing** | **n/a** |
| Load progress and failure reasons | **Have**. Live run view; a fixed set of seven reasons | **Have** | **Have**. Copy history with errors | **n/a** |
| Delete an upload | **Have**. File only; the loaded table stays | **Have** | **Have** | **n/a** |
| Use uploaded tables in pipelines and dashboards | **Missing** | **Have** | **Have** | **n/a** |

#### AI in the Data module (input for the AI team)

| Capability | RantAI Lakehouse | Databricks | Snowflake | Airbyte |
| --- | --- | --- | --- | --- |
| AI-written descriptions | **Missing** | **Have**. Per object and bulk for all columns, language choice | **Have**. Generate with Cortex, bulk up to 50 columns | **n/a** |
| Plain-language search for tables | **Missing** | **Have**. Semantic search (PP) | **Have**. Natural-language queries in Universal Search | **n/a** |
| Ask questions about a table's sample | **Missing** | **Have**. Sample Data Explorer turns a question into SQL over the sample | **Missing**. Not on the catalog page | **n/a** |
| Assistant answers questions about catalog, lineage and quality | **Have**. describe_dataset, get_lineage and get_quality tools | **Have**. Genie Code in Catalog Explorer | **Have**. Cortex Code answers catalog and governance questions | **n/a** |
| Assistant drafts classification, quality and masking rules | **Have**. draft_classification_rule, draft_quality_rule and draft_policy tools | **Partial**. Classification review pre-fills a mask policy | **Partial**. Suggests quality checks (preview) | **n/a** |
| AI-suggested quality checks | **Partial**. Only when asked, through the assistant | **Partial**. Anomaly detection, not AI suggestions | **Have**. Suggested checks from Cortex (preview) | **n/a** |
| AI investigates quality incidents | **Missing** | **Partial**. Root cause from lineage, not AI | **Have**. 'Investigate' opens Cortex Code with the incident (preview) | **n/a** |
| AI sensitive-data classification | **Missing**. Classification rules are written by hand or drafted by the assistant | **Have**. Agentic model plus patterns, about 90 classes | **Have**. AI mode (preview) | **n/a** |
| Assistant creates, tests and changes connectors | **Have**. list, create, test, delete, discover, ingest spec, rotate credentials | **Missing**. Not documented | **Missing**. Not documented | **Partial**. Connector Setup Assistant (cloud, beta, certified connectors) |
| AI builds a connector from API docs | **Missing** | **Missing** | **Missing** | **Partial**. AI Assist in the Connector Builder (cloud, beta) |
| Assistant runs and watches ingestion | **Have**. run_ingest, list_ingest_runs and get_cdc_health tools | **Missing** | **Missing** | **Partial**. Remote MCP server (private beta) |

## Query Studio matrix: Databricks, Snowflake and Metabase

Compiled 2026-10-07, same method. Query Studio is a main feature of its own,
shared by BI and the lakehouse (product owner, 2026-10-07), so it is compared
with the lakehouse platforms' SQL workspaces and with Metabase's SQL editor.
The task list is backlog `QS-1` to `QS-5` and `SEC-9`, `SEC-19` to `SEC-21`.

Summary: 29 capabilities. We have 4, have 11 partly and lack 11.

| Capability | RantAI Lakehouse | Databricks | Snowflake | Metabase |
| --- | --- | --- | --- | --- |
| Editor basics | **Partial**. Highlighting, line numbers, folding, run shortcut; no tabs, themes or command palette | **Have**. Themes, command palette, folding | **Have**. Split panes, folding | **Have** |
| Schema browser beside the editor | **Not verified**. Not checked | **Have**. Search, type filter, insert names | **Have**. Database Explorer pane | **Have**. Data reference sidebar |
| Autocomplete | **Partial**. Table names only | **Have**. Keywords, tables, columns, aliases | **Have**. Objects, columns, functions | **Have**. Tables, fields, snippets |
| Format SQL | **Partial**. A small built-in formatter; no formatting rules | **Have**. With custom rules | **Have** | **Have**. Not for every database |
| Several statements and run selection | **Not verified**. Not checked | **Have**. Result per statement | **Have**. Run selected or all | **Partial**. Run selection; one statement per question |
| Several query tabs | **Not verified**. Not checked | **Have** | **Have** | **Missing** |
| Draft kept across reloads and shareable links | **Have**. Draft in the browser; links open a query or a saved query | **Have**. Autosave | **Have** | **Partial**. Saved questions only |
| Choice of engine | **Have**. ClickHouse or Trino | **Have**. SQL warehouses | **Have**. Warehouses | **Have**. Any connected database |
| Masking and row filters on queries | **Have**. Rewritten before either engine | **Have** | **Have**. Ent+ | **Have**. Row and column security, Pro+ |
| Parameters / variables | **Missing** | **Have**. Named parameters with widgets | **Missing**. Not supported in Workspaces | **Have**. Text, number, date, field filters, optional clauses |
| Reuse another saved query inside SQL | **Missing** | **Missing** | **Missing** | **Have**. Reference a question as a CTE |
| Snippets | **Missing** | **Have** | **Missing** | **Have**. Folder permissions on Pro+ |
| Results grid | **Partial**. Sortable and typed, with run statistics; no filters, column statistics or cell inspector | **Have**. Filters and column profiling | **Have**. Column stats, inspector, selection stats | **Have**. Conditional formatting, mini bars |
| Result size | **Partial**. 2,000 rows; the full result is still computed and buffered | **Have**. Up to 64,000 rows or 10 MB | **Have** | **Partial**. 2,000 by default, configurable |
| Download results | **Partial**. Browser CSV of the 2,000 rows; a server CSV/Parquet route exists with no button | **Have**. CSV, TSV, Excel up to about 5 GB | **Have**. Full CSV export | **Have**. CSV, Excel, JSON |
| Chart from results | **Partial**. Through 'save as SQL source' or the assistant | **Have**. About 20 chart types, add to dashboard | **Missing**. Retired with worksheets | **Have**. Any chart type |
| Explore a SQL result without SQL | **Missing** | **Missing** | **Missing** | **Have**. Explore results in the query builder |
| Stop a running query | **Partial**. Browser only; the server query keeps running | **Have**. Admins can also cancel others' queries | **Not verified** | **Not verified** |
| Query history | **Partial**. Own last 200 in a side panel | **Have**. History page with filters | **Have**. 14 days with filters | **Partial**. Query log in usage analytics, Pro+ |
| Query profile / plan | **Missing**. Plan always 'Not measured' | **Have**. Operator graph with insights | **Have**. Operator graph with insights | **Missing** |
| Cost estimate and freshness before running | **Have**. EXPLAIN ESTIMATE with cost range and data age | **Missing** | **Missing** | **Missing** |
| Saved queries | **Partial**. Create and list only; no edit or delete; listed for everyone | **Have**. Folders and sharing levels | **Have**. Shared workspaces with publish history | **Have**. Saved questions in collections with permissions |
| Version history of a query | **Missing** | **Have** | **Have**. Shared workspaces | **Have**. Last 15 versions with revert |
| Real-time co-editing | **Missing** | **Have** | **Missing** | **Missing** |
| Git integration | **Missing** | **Have** | **Have** | **Partial**. Remote sync, Pro+ |
| Configurable timeouts | **Missing**. Fixed 60 s | **Have**. Up to 2 days | **Have** | **Have**. 20 minutes by default |
| Result caching | **Missing** | **Partial**. Warehouse result cache | **Partial**. Result cache | **Have**. Adaptive caching on all plans |
| Time-travel picker | **Partial**. In the UI; not proven through the masking rewrite | **Partial**. SQL only | **Partial**. SQL only | **Missing** |
| Alerts on a query result | **Missing**. Alerts exist only on dashboard metrics | **Have**. SQL alerts with schedules | **Have**. Alerts on a schedule or new data | **Have**. Alerts on saved questions |

#### AI in Query Studio

| Capability | RantAI Lakehouse | Databricks | Snowflake | Metabase |
| --- | --- | --- | --- | --- |
| Write SQL from plain language | **Partial**. Exists, but skips masking and permissions (finding 1) | **Have**. Genie Code | **Have**. Cortex Code | **Have**. Metabot, bring your own key on OSS |
| Ask a question; the AI writes, runs, corrects and summarises | **Partial**. Agentic Ask exists, ungoverned | **Partial**. Genie agents are a separate product | **Partial**. Cortex Code in Workspaces | **Have**. Metabot answers with a chart |
| Fix a failing query | **Missing** | **Have**. Quick Fix, Diagnose Error | **Have**. Fix failed SQL | **Have**. 'Have Metabot fix it' |
| Inline AI completion while typing | **Missing** | **Have** | **Have**. Accept with Tab | **Partial**. Inline edit shortcut |
| Explain or optimise a query | **Missing** | **Have**. /optimize | **Have**. Explain and optimise | **Missing** |
| Filter results by describing the filter | **Missing** | **Have** | **Missing** | **Missing** |
| Assistant knows the current query and results | **Have**. Query Studio sends its context to the assistant | **Have** | **Have**. @-mention tables | **Have** |
