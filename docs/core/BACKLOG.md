# Backlog

The one list of known work. Look at it weekly; pick what is next.

- **When:** Now, Next, Later, or Decide (needs a yes/no first).
- **Source:** where the item comes from, so it can be checked. "PRODUCT §2"
  is the coverage table in `PRODUCT.md`.
- IDs are never reused. A finished or rejected item moves to the bottom with
  the date and reason; it is not deleted.
- **Specs:** every security, BI, assistant-for-dashboards, Data and Query
  Studio item has its own spec file in [`specs/`](specs/), indexed under
  "Specs" at the end of this file: target numbers against the best
  competitor, an acceptance checklist and what is left out. A spec is not a
  plan: each item still needs a feature page and a plan before anyone builds
  it (`AGENTS.md`).

## Now

| ID | Item | Area | Source |
| --- | --- | --- | --- |
| `DATA-1` | Gold tables publish to open format automatically, as an option per table. **Built and merged (PRs #60, #63 and the slice C+D PR); waiting for the product owner to run the acceptance checklist** | Build | `features/gold-publish-per-mart.md` |
| `QA-1` | Accept each built feature with a checklist, starting with what a demo shows | All | PRODUCT §4 blocker 1 |
| `SEC-1` | Confirm the leaked key is rotated; decide on rewriting git history | Security | `SECURITY.md` |
| `REL-1` | Protect the main branch | Delivery | `docs/CI.md` |
| `DOC-1` | Archive or rewrite the old product documents that contradict the build | Docs | PRODUCT §6 |
| `GOV-2` | Decide which role may publish a table in open format | Governance | PRODUCT §6 |
| `SEC-10` | Alert webhooks can be pointed at internal addresses; use the allowlisted resolver | Security | `lakehouse-notify` |
| `SEC-11` | Dashboard tiles and public/embed links return raw database error text. Same rule as `SEC-6` | Security | `routes/support.rs` |
| `SEC-14` | A stored connector password can be stolen by re-pointing the connector: a host change keeps the secret and the next test sends it to the new server | Security | `routes/connectors.rs`, `connector_probe.rs` |
| `SEC-15` | Connection tests can reach internal addresses: the block is off by default in compose, the REST test follows redirects, no address pinning, CDC delete skips the check | Security | `docker-compose.yml`, `connector_probe.rs` |
| `SEC-13` | Check `.env.staging`, pushed straight to `main` in `7ec3a81`, for real credentials or hostnames; rotate anything real | Security | commit `7ec3a81` |

## Next

| ID | Item | Area | Source |
| --- | --- | --- | --- |
| `RPT-1` | Scheduled reports with PDF and spreadsheet attachments | Dashboards | PRODUCT §2 |
| `BI-1` | Export a dashboard as a server-made PDF and as Excel | Dashboards | PRODUCT §2 |
| `BI-4` | Result caching for charts and queries; run a dashboard's tiles in parallel | Dashboards | `reference/competitive-comparison.md` (BI matrix) |
| `BI-5` | Configurable query timeouts and row limits; Stop cancels the query on the server | Dashboards | `reference/competitive-comparison.md` (BI matrix) |
| `BI-6` | A chart builder as capable as Metabase's and Tableau's: several dimensions, filters, sorting, multi-step questions. No drag and drop; the assistant covers that | Dashboards | `reference/competitive-comparison.md` (BI matrix) |
| `BI-7` | Joins in the chart builder without writing SQL | Dashboards | `reference/competitive-comparison.md` (BI matrix) |
| `BI-8` | Calculated fields and custom expressions: maths, text, dates, conditions, running totals | Dashboards | `reference/competitive-comparison.md` (BI matrix) |
| `BI-9` | Group by day, week, month, quarter, year | Dashboards | `reference/competitive-comparison.md` (BI matrix) |
| `BI-10` | Bins, groups and hierarchies | Dashboards | `reference/competitive-comparison.md` (BI matrix) |
| `BI-11` | Parameters, usable in charts, SQL and dashboards | Dashboards | `reference/competitive-comparison.md` (BI matrix) |
| `BI-13` | Combine two sources on one chart | Dashboards | `reference/competitive-comparison.md` (BI matrix) |
| `BI-14` | Lineage from a chart to its sources, reachable from the dashboard | Dashboards | `reference/competitive-comparison.md` (BI matrix) |
| `BI-16` | Every chart type Tableau and Metabase have that we lack: progress bar, histogram, Gantt, bullet graph, KPI with comparison and sparkline, raw-row table, pivot table with totals, record detail, world maps and uploaded boundaries, image and web-page cards, spatial analysis, custom chart plugins | Dashboards | `reference/competitive-comparison.md` (BI matrix) |
| `BI-17` | Chart formatting at both competitors' level: number formats, colours, axes, data labels, conditional formatting, goal and reference lines, trend lines, forecasting, clustering, annotations, custom tooltips, themes and fonts | Dashboards | `reference/competitive-comparison.md` (BI matrix) |
| `BI-18` | Dashboards at both competitors' level: tabs; phone and tablet layouts; date-range, relative-date, number, text and linked filters; filter defaults that do not change silently; drill-down for every chart; click to another dashboard or URL; show and hide; saved auto-refresh; stories; templates; favourites; version history; trash; personal space | Dashboards | `reference/competitive-comparison.md` (BI matrix) |
| `AI-1` | **Ready now**. The assistant manages the dashboards that exist today: rename boards, delete them (with approval), arrange and resize tiles, move charts between boards | Ask AI (dashboards) | `AGENTS.md` (assistant parity) |
| `AI-2` | **Waits for `BI-18`**. The assistant adds and changes dashboard filters ("only 2025, only Bali") | Ask AI (dashboards) | `AGENTS.md` (assistant parity) |
| `AI-3` | **Waits for `BI-9`**. The assistant picks the time grain ("monthly") | Ask AI (dashboards) | `AGENTS.md` (assistant parity) |
| `AI-4` | **Waits for `BI-8`**. The assistant writes calculated fields in the product's formula language, not raw SQL | Ask AI (dashboards) | `AGENTS.md` (assistant parity) |
| `AI-5` | **Waits for `BI-16`**, per chart type as it lands. The assistant creates every new chart type: raw and pivot tables, KPI with comparison, histogram, Gantt and the rest | Ask AI (dashboards) | `AGENTS.md` (assistant parity) |
| `AI-6` | **Waits for `BI-6`, `BI-7`, `BI-10`, `BI-11`, `BI-13`**, each part with its BI item. The assistant builds charts with several dimensions and filters, joins tables, uses bins, groups, hierarchies and parameters, and combines two sources | Ask AI (dashboards) | `AGENTS.md` (assistant parity) |
| `AI-7` | **Waits for `BI-2`**. The assistant uses named metrics before it writes SQL, so the same question gives the same number | Ask AI (dashboards) | `AGENTS.md` (assistant parity) |
| `AI-8` | **Waits for `BI-17`**. The assistant changes number formats, colours, axes and labels, and adds trend and forecast lines | Ask AI (dashboards) | `AGENTS.md` (assistant parity) |
| `AI-9` | **Waits for `BI-18`** (layout part). The assistant adds tabs and uses templates | Ask AI (dashboards) | `AGENTS.md` (assistant parity) |
| `AI-10` | **Waits for `RPT-1`, `BI-1`**. The assistant schedules reports ("send this to me every Monday as PDF") and produces a PDF or Excel file on request | Ask AI (dashboards) | `AGENTS.md` (assistant parity) |
| `AI-11` | **Waits for `BI-19`, `BI-26`, `DEC-8`**. The assistant creates share and embed links only with approval, respects dashboard permissions, and never grants permissions | Ask AI (dashboards) | `AGENTS.md` (assistant parity) |
| `AI-12` | **Ready now** for fixing and explaining SQL inside the editor; variables **Waits for `QS-5`** | Ask AI (dashboards) | `AGENTS.md` (assistant parity) |
| `AI-13` | **Waits for `BI-14`**. The assistant answers "where does this number come from" from the chart's lineage (`get_lineage` exists today) | Ask AI (dashboards) | `AGENTS.md` (assistant parity) |
| `AI-14` | Ongoing, with every BI item. What the dashboard page sends to the assistant grows with the features: filters, tabs, metrics, formats | Ask AI (dashboards) | `AGENTS.md` (assistant parity) |
| `AI-15` | **Ready now**. A standard set of about 30 typical dashboard requests, re-run whenever an assistant tool changes, so the assistant does not silently get worse | Ask AI (dashboards) | `AGENTS.md` (assistant parity) |
| `BI-26` | Better embedding, without overdoing it: a single-chart embed that really shows only that chart, light/dark theme and our logo optional, filters passed through the link or token, sizing that fits the host page. Token hardening is `SEC-12`. No embedding SDK and no editing inside embeds | Dashboards | `reference/competitive-comparison.md` (BI matrix) |
| `BI-19` | Sharing and delivery, except Slack, Teams, Office and Google integrations: public link expiry and password; permissions per dashboard and folder; comments; a schedule per digest; alerts with filters that fire once; PNG export; import of exported dashboards. Scheduled reports are `RPT-1`, PDF and Excel export `BI-1`, embedding `BI-26` | Dashboards | `reference/competitive-comparison.md` (BI matrix) |
| `BLD-1` | Joins and aggregations in the pipeline builder | Build | PRODUCT §2 |
| `VER-1` | Verify SFTP, Google Sheets and Oracle sources end to end; test row filters. The connector half is now `SRC-9`; row filters stay here | Data, Governance | PRODUCT §2 |
| `BI-2` | Metrics users can define once and reuse across dashboards (a semantic layer) | Dashboards | PRODUCT §2; `reference/competitive-comparison.md` (BI matrix) |
| `DATA-9` | Upload a file from the console. **Built and merged (PR #71); waiting for the product owner to run the acceptance checklist** | Data | `features/upload-file.md` |
| `OPS-8` | Usage and cost view | Monitoring | PRODUCT §2 |
| `SEC-2` | Login rate limiting. **Built and merged (PR #67); waiting for the product owner to run the acceptance checklist** | Security | `features/login-protection-and-session-cleanup.md` |
| `SEC-3` | Test single sign-on against a real identity provider; correct `README.md`, which says it is unbuilt | Security | PRODUCT §2 |
| `SEC-7` | A monitored security contact address | Security | `SECURITY.md` |
| `REL-2` | Adopt a version and release policy; cut a release from the unreleased work | Delivery | `reference/release-policy.md` |
| `GOV-1` | Audit trail covers every console change | Governance | PRODUCT §3 |
| `OPS-1` | Trim old versions of raw tables | Monitoring | `README.md` |
| `DATA-2` | Trim old copies in published Gold tables | Build | `features/gold-publish-per-mart.md` |
| `DATA-10` | A merge-proof "has this mart changed" signal for publishing, so background merges do not trigger extra copies | Build | Slice B review, gold-publish plan |
| `DATA-6` | A "latest copy only" view for outside readers of published tables | Build | `features/gold-publish-per-mart.md` |
| `SEC-4` | Human security review | Security | PRODUCT §4 blocker 3 |
| `SUP-1` | Decide a support commitment | Business | PRODUCT §4 blocker 5 |
| `OPS-7` | Test and time a restore from backup | Monitoring | PRODUCT §2 |
| `SEC-5` | Clean up old sessions and tokens. **Built and merged (PR #67); waiting for the product owner to run the acceptance checklist** | Security | `features/login-protection-and-session-cleanup.md` |
| `SRC-6` | Fix the broken connectors: object-storage test and key change, Oracle credential change, Sheets shown as unsupported, Sources header text | Data | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-7` | Alerts when a connector run or an upload load fails; health from real runs | Data | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-8` | Schema changes at the source: detect, notify, and a per-connector choice to apply, ask or pause | Data | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-9` | Gate tests for every advertised connector (Oracle, SFTP, MySQL CDC, SQL Server CDC); the upload gate in CI | Data | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-10` | More load modes: deduplicate on a key, keep history (SCD type 2), refresh or clear one table | Data | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-11` | Connector operations: automatic retries with backoff, a log view per run, column choice | Data | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-12` | Credentials done properly: external secret managers, encryption at rest, OAuth sign-in | Data | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-13` | Table discovery for every source type, not only databases | Data | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-4` | More databases, with live change capture: Oracle tested end to end, MariaDB and Teradata added; live change capture for PostgreSQL, MySQL, SQL Server, Oracle and MongoDB, each gate-tested. Cloud warehouses and enterprise databases excluded (product owner, 2026-10-07) | Data | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-11` | Catalog search that finds columns and tags, with tolerant matching | Data | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-12` | Certified and deprecated marks; governed tags with allowed values | Data | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-13` | Column-level lineage | Data | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-14` | Data quality in depth: a library of ready checks, freshness and volume anomaly detection, an incidents overview | Data | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-15` | Asset page depth: 1,000-row preview, 30-day usage, grant and revoke, restore a dropped table | Data | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-16` | Time travel proven safe: a gate test that past-version queries pass the masking rewrite | Data | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-17` | Upload more formats (Excel, JSON, Parquet, Avro), up to 10 files at once in parallel, up to 2 GB per upload once measured (Databricks' limits) | Data | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-18` | Column types on upload, with names and types editable before loading | Data | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-19` | Uploaded tables can feed pipelines and so reach dashboards | Data | `reference/competitive-comparison.md` (Data matrix) |
| `QS-1` | Query Studio editor basics: column autocomplete, schema browser, several statements, tabs, formatting rules | Query Studio | `reference/competitive-comparison.md` (Query Studio matrix) |
| `QS-2` | Query Studio results: up to 64,000 rows or 10 MB shown, grid filters and column stats, full server downloads up to 5 GB (Databricks' limits), server-side Stop, configurable timeouts | Query Studio | `reference/competitive-comparison.md` (Query Studio matrix) |
| `QS-3` | Saved queries done properly: edit, delete, folders, sharing levels, version history. **Waits for `SEC-20`** | Query Studio | `reference/competitive-comparison.md` (Query Studio matrix) |
| `QS-4` | Query history page with filters, and a query profile | Query Studio | `reference/competitive-comparison.md` (Query Studio matrix) |
| `QS-5` | The bridge to BI: parameters that become dashboard filters, snippets, charts from results, reuse a saved query in SQL, alerts on a query | Query Studio | `reference/competitive-comparison.md` (Query Studio matrix) |
| `AI-16` | Hand-off to the AI team: AI features for the Data module. **First line (AI-written descriptions of tables and columns) built.** Remaining: plain-language table search, AI classification, AI quality suggestions and incident help, AI connector building. The AI team plans these | Ask AI | `docs/core/features/semantic-layer.md` |
| `SEC-6` | Stop older API handlers from returning internal error text | Security | `AGENTS.md` |
| `SEC-16` | Cross-tenant leaks in the Data module: the ingestible-connectors route, one shared upload namespace, a message naming another tenant's upload table, annotation edits without the tenant gate | Security | Data module code audit |
| `SEC-17` | Upload hardening: a column cap, a limit on concurrent uploads, CSV exports that neutralise formula cells | Security | Data module code audit |
| `SEC-18` | No default credentials in compose (`lakehouse`, `rustfsadmin`); must-set secrets use `${X:?}` (rule 5) | Security | `docker-compose.yml` |
| `SEC-19` | The query cost estimate runs raw SQL without the read-only check or table-function block, and returns database error text | Security | `routes/query.rs` |
| `SEC-20` | Queries are not scoped to the tenant: no tenant gate on query runs; saved queries listed for everyone | Security | `routes/query.rs` |
| `SEC-21` | Downloads re-run SQL rewritten for an older policy; whole results buffered before the 2,000-row cut | Security | `routes/query.rs` |
| `SEC-12` | Embed tokens: require an expiry, allow revocation, keep the signing secret out of plain text, set a frame-ancestors policy | Security | `lakehouse-embed` |

## Later

| ID | Item | Area | Source |
| --- | --- | --- | --- |
| `OPS-3` | Run on several servers; high availability | Platform | PRODUCT §1, §2 |
| `SRC-3` | Connectors for business apps: first Databricks' ten generally available SaaS connectors, then its beta list by customer demand. **Waits for `DEC-9`** | Data | PRODUCT §2 |
| `DATA-8` | Instant copies of a table for testing | Data | PRODUCT §2 |
| `BLD-2` | Tables that refresh themselves when their inputs change | Build | PRODUCT §2 |
| `SRC-5` | Real connection test for MongoDB, Kafka, SFTP, Sheets and Oracle sources | Data | `connector_probe.rs` |
| `DOC-2` | Step-by-step user guide checked against a running console | Docs | `reference/user-guide.md` |
| `DATA-3` | Store Gold in open format first; serve from a fast copy | Build | ADR 0010 |
| `DATA-4` | Publish Silver in open format | Build | `features/gold-publish-per-mart.md` |
| `DATA-5` | Measure outside readers of published tables | Build | `routes::gold::consumers` |
| `BI-3` | Import dashboards from Tableau and Power BI | Dashboards | PRODUCT §2 |
| `BI-20` | The rest of the BI matrix, to be considered later: field metadata in the BI layer, certified content, BI-specific AI, admin and platform rows, and Slack, Teams, Office and Google integrations | Dashboards | `reference/competitive-comparison.md` (BI matrix) |
| `QRY-1` | Query other systems in place, without loading | Data | PRODUCT §2 |
| `SRC-2` | Processing over streams | Data | PRODUCT §2 |
| `INT-2` | Search over documents | Ask AI | PRODUCT §2 |
| `GOV-3` | Lineage that reaches outside systems | Governance | PRODUCT §2 |
| `ADM-1` | Several organisations in one install | Administration | PRODUCT §2 |
| `ADM-2` | Workspace settings that can be changed | Administration | `README.md` |
| `OPS-2` | Install with no internet access | Platform | Sales playbook (older; re-check) |
| `OPS-4` | Fail loudly when the application database is unreachable | Platform | `docs/OPERATIONS.md` |
| `OPS-6` | Log and trace viewer | Monitoring | `docs/FEATURE_COVERAGE.md` |
| `BIZ-1` | Enterprise edition | Business | PRODUCT §1 |
| `BIZ-2` | Hosted cloud version | Business | PRODUCT §1 |

## Decide

Competitors have these. Each is large. A "no" is a valid answer.

| ID | Item | Recommendation |
| --- | --- | --- |
| `DEC-1` | Notebooks and Python | Defer |
| `DEC-2` | dbt support | Defer |
| `DEC-3` | AI functions inside SQL | Defer |
| `DEC-4` | Train and serve models | No |
| `DEC-5` | Mobile app | No |
| `DEC-6` | Share data with another organisation | Defer |
| `DEC-9` | How to get SaaS connectors: reuse dlt verified sources (our loader already runs on dlt) or another connector library | Reuse dlt verified sources first |
| `DEC-10` | Data and Query Studio edge cases: entity relationship diagram, a no-code connector builder, business glossary and domains, table favourites, real-time query co-editing, Git for queries | Drop or defer all six |
| `DEC-8` | Which assistant actions need human approval before they run (today: deletions and pauses) | Also: making a dashboard public or creating an embed link, changing who can see a dashboard, scheduling email to addresses outside the company, deleting a dashboard |
| `INT-1` | Digital employees | Parked by the product owner, 2026-10-02 |

## Done and rejected

| ID | Item | Outcome | Date |
| --- | --- | --- | --- |
| `PM-1` | Customer-facing product name | Decided: RantAI Lakehouse | 2026-10-02 |
| `DEC-7` | Write-back from dashboards (buttons and forms that change data) | Rejected: we are a lakehouse, not an ERP | 2026-10-06 |
| `BI-12` | Curated datasets (models) inside BI | Rejected: Gold tables in the lakehouse are the curated datasets | 2026-10-06 |
| `BI-22` | Data preparation inside BI | Rejected: the Build module is our data preparation | 2026-10-06 |
| `BI-23` | Sets (a Tableau power-user feature) | Rejected: groups and hierarchies cover it | 2026-10-06 |
| `BI-24` | PowerPoint export | Rejected: PDF and images cover it | 2026-10-06 |
| `BI-25` | Automatic dashboards from a table (Metabase X-ray) | Rejected: the assistant builds dashboards on request | 2026-10-06 |
| `BI-15` | A very good SQL editor | Moved: Query Studio is a main feature of its own (`QS-1` to `QS-5`) | 2026-10-07 |
| `DATA-7` | Query a table as of an earlier time | Superseded: the picker was built in PR #71; proving it safe is `DATA-16` | 2026-10-07 |
| `BI-21` | Where charts read data | Decided: only data in the lakehouse, for now | 2026-10-06 |
| `SEC-8` | Dependency security checks green on `main` | Done: PR #62. Three TLS-library vulnerabilities cleared by updating the SQL Server client; licence check fixed | 2026-10-02 |
| `SEC-9` | The natural-language query endpoints run the model's SQL without masking or row filters, and need only a sign-in | Done: PR #78. The endpoints were removed; plain-language queries now route through the chat which applies masking and row filters via the policy rewriter | 2026-10-07 |
| `SEC-11` (agent endpoints) | The agent endpoints return raw database error text | Done: PR #78. The endpoints were removed | 2026-10-07 |

## Specs

One file per task in [`specs/`](specs/): why, what users get, target specs
with numbers set against the best competitor, an acceptance checklist and
what is left out. A number marked *(proposed)* is the planner's, not a
competitor's; the product owner confirms it on the feature page. A spec is
not a plan: each task still needs a feature page and a plan before anyone
builds it (`AGENTS.md`).

### Security

| ID | Spec | When | Size |
| --- | --- | --- | --- |
| [`SEC-10`](specs/sec-10.md) | Alert webhooks cannot reach internal addresses | Now | S |
| [`SEC-12`](specs/sec-12.md) | Safer embed tokens | Next | M |
| [`SEC-13`](specs/sec-13.md) | Check the staging file pushed to main | Now | S |
| [`SEC-14`](specs/sec-14.md) | Connector passwords cannot be stolen by re-pointing a connector | Now | M |
| [`SEC-15`](specs/sec-15.md) | Connection tests cannot reach internal addresses | Now | M |
| [`SEC-16`](specs/sec-16.md) | No cross-tenant leaks in the Data module | Next | M |
| [`SEC-17`](specs/sec-17.md) | Upload hardening | Next | S |
| [`SEC-18`](specs/sec-18.md) | No default credentials in compose | Next | S |
| [`SEC-19`](specs/sec-19.md) | The query cost estimate runs only safe SQL | Next | S |
| [`SEC-20`](specs/sec-20.md) | Queries are scoped to the tenant | Next | M |
| [`SEC-21`](specs/sec-21.md) | Downloads and big results are safe | Next | S |

### Dashboards (BI)

| ID | Spec | When | Size |
| --- | --- | --- | --- |
| [`BI-18`](specs/bi-18.md) | Dashboard filters, interactivity and layout | Next | L, in three parts |
| [`BI-9`](specs/bi-9.md) | Group by day, week, month, quarter, year | Next | M |
| [`BI-8`](specs/bi-8.md) | Calculated fields | Next | L |
| [`BI-16`](specs/bi-16.md) | Chart types | Next | L, in three parts |
| [`BI-6`](specs/bi-6.md) | A more capable chart builder | Next | L |
| [`BI-7`](specs/bi-7.md) | Joins without writing SQL | Next | M |
| [`BI-13`](specs/bi-13.md) | Combine two sources on one chart | Next | M |
| [`BI-10`](specs/bi-10.md) | Bins, groups and hierarchies | Next | M |
| [`BI-11`](specs/bi-11.md) | Parameters | Next | M |
| [`BI-2`](specs/bi-2.md) | Reusable metrics | Next | L |
| [`BI-17`](specs/bi-17.md) | Chart formatting, forecasting and clustering | Next | L, in two parts |
| [`RPT-1`](specs/rpt-1.md) | Scheduled reports with attachments | Next | L |
| [`BI-1`](specs/bi-1.md) | Export PDF and Excel | Next | M |
| [`BI-19`](specs/bi-19.md) | Sharing, alerts and permissions | Next | L |
| [`BI-26`](specs/bi-26.md) | Better embedding, not bigger | Next | M |
| [`BI-4`](specs/bi-4.md) | Caching and parallel tiles | Next | M |
| [`BI-5`](specs/bi-5.md) | Timeouts and limits you can set | Next | S |
| [`BI-14`](specs/bi-14.md) | Lineage from a chart to its sources | Next | M |

### Assistant for dashboards

| ID | Spec | When | Size |
| --- | --- | --- | --- |
| [`AI-1`](specs/ai-1.md) | Manage the dashboards that exist today | Next, ready now | M |
| [`AI-2`](specs/ai-2.md) | Dashboard filters | Next, with BI-18 A | S |
| [`AI-3`](specs/ai-3.md) | Time grain | Next, with BI-9 | S |
| [`AI-4`](specs/ai-4.md) | Calculated fields | Next, with BI-8 | M |
| [`AI-5`](specs/ai-5.md) | Every new chart type | Next, with each BI-16 part | S |
| [`AI-6`](specs/ai-6.md) | Richer charts | Next, with each BI phase-2 item | M |
| [`AI-7`](specs/ai-7.md) | Named metrics first | Next, with BI-2 | M |
| [`AI-8`](specs/ai-8.md) | Chart formatting | Next, with BI-17 | S |
| [`AI-9`](specs/ai-9.md) | Tabs and templates | Next, with BI-18 C | S |
| [`AI-10`](specs/ai-10.md) | Reports and exports | Next, with RPT-1 and BI-1 | S |
| [`AI-11`](specs/ai-11.md) | Share and embed, with approval | Next, with BI-19, BI-26 and DEC-8 | S |
| [`AI-12`](specs/ai-12.md) | Help inside the SQL editor | Next, ready now | M |
| [`AI-13`](specs/ai-13.md) | Where does this number come from | Next, with BI-14 | S |
| [`AI-14`](specs/ai-14.md) | Page awareness keeps up | Next, ongoing | S |
| [`AI-15`](specs/ai-15.md) | The standard request set | Next, ready now | M |
| [`AI-16`](specs/ai-16.md) | Hand-off: AI features for the Data module | Next, AI team decides | n/a |

### Data module

| ID | Spec | When | Size |
| --- | --- | --- | --- |
| [`SRC-6`](specs/src-6.md) | Fix the broken connectors | Next, ready now | S |
| [`SRC-7`](specs/src-7.md) | Alerts when a load fails | Next | M |
| [`SRC-8`](specs/src-8.md) | Schema changes at the source | Next | M |
| [`SRC-9`](specs/src-9.md) | Test every advertised connector end to end | Next | M |
| [`SRC-10`](specs/src-10.md) | More load modes | Next | M |
| [`SRC-11`](specs/src-11.md) | Connector operations | Next | M |
| [`SRC-12`](specs/src-12.md) | Credentials done properly | Next | L |
| [`SRC-13`](specs/src-13.md) | Table discovery for every source type | Next | M |
| [`DATA-11`](specs/data-11.md) | Search that finds columns and tags | Next | M |
| [`DATA-12`](specs/data-12.md) | Certification and governed tags | Next | M |
| [`DATA-13`](specs/data-13.md) | Column-level lineage | Next | L |
| [`DATA-14`](specs/data-14.md) | Data quality in depth | Next | L |
| [`DATA-15`](specs/data-15.md) | Asset page depth | Next | M |
| [`DATA-16`](specs/data-16.md) | Time travel proven safe | Next | S |
| [`DATA-17`](specs/data-17.md) | More formats, more files, bigger files | Next | M |
| [`DATA-18`](specs/data-18.md) | Column types on upload | Next | M |
| [`DATA-19`](specs/data-19.md) | Uploaded tables feed pipelines and dashboards | Next | M |
| [`SRC-4`](specs/src-4.md) | More databases, with live change capture | Next | L |
| [`SRC-3`](specs/src-3.md) | Connectors for business apps | Later | L |

### Query Studio

| ID | Spec | When | Size |
| --- | --- | --- | --- |
| [`QS-1`](specs/qs-1.md) | Editor basics | Next | M |
| [`QS-2`](specs/qs-2.md) | Results you can use | Next | M |
| [`QS-3`](specs/qs-3.md) | Saved queries done properly | Next | M |
| [`QS-4`](specs/qs-4.md) | History and query profile | Next | M |
| [`QS-5`](specs/qs-5.md) | Parameters, snippets and charts from results | Next | L |
