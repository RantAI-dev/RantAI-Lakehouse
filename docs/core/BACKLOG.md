# Backlog

The one list of known work. Look at it weekly; pick what is next.

- **When:** Now, Next, Later, or Decide (needs a yes/no first).
- **Source:** where the item comes from, so it can be checked. "PRODUCT §2"
  is the coverage table in `PRODUCT.md`.
- IDs are never reused. A finished or rejected item moves to the bottom with
  the date and reason; it is not deleted.

## Now

| ID | Item | Area | Source |
| --- | --- | --- | --- |
| `DATA-1` | Gold tables publish to open format automatically, as an option per table. **Built and merged (PRs #60, #63 and the slice C+D PR); waiting for the product owner to run the acceptance checklist** | Build | `features/gold-publish-per-mart.md` |
| `QA-1` | Accept each built feature with a checklist, starting with what a demo shows | All | PRODUCT §4 blocker 1 |
| `SEC-1` | Confirm the leaked key is rotated; decide on rewriting git history | Security | `SECURITY.md` |
| `REL-1` | Protect the main branch | Delivery | `docs/CI.md` |
| `DOC-1` | Archive or rewrite the old product documents that contradict the build | Docs | PRODUCT §6 |
| `GOV-2` | Decide which role may publish a table in open format | Governance | PRODUCT §6 |
| `SEC-9` | The natural-language query endpoints run the model's SQL without masking or row filters, and need only a sign-in. **Shared with the AI team: their code** | Security | `routes/agent.rs`, `policy.rs` |
| `SEC-10` | Alert webhooks can be pointed at internal addresses; use the allowlisted resolver | Security | `lakehouse-notify` |
| `SEC-11` | Dashboard tiles (also on public and embed links) and the agent endpoints return raw database error text. Same rule as `SEC-6`. **Shared with the AI team for the agent endpoints** | Security | `routes/support.rs`, `routes/agent.rs` |
| `SEC-13` | Check `.env.staging`, pushed straight to `main` in `df14b78`, for real credentials or hostnames; rotate anything real | Security | commit `df14b78` |

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
| `BI-15` | A very good SQL editor: column autocomplete, variables, snippets, edit and delete saved queries, version history | Dashboards | `reference/competitive-comparison.md` (BI matrix) |
| `BI-16` | Every chart type Tableau and Metabase have that we lack: progress bar, histogram, Gantt, bullet graph, KPI with comparison and sparkline, raw-row table, pivot table with totals, record detail, world maps and uploaded boundaries, image and web-page cards, spatial analysis, custom chart plugins | Dashboards | `reference/competitive-comparison.md` (BI matrix) |
| `BI-17` | Chart formatting at both competitors' level: number formats, colours, axes, data labels, conditional formatting, goal and reference lines, trend lines, forecasting, clustering, annotations, custom tooltips, themes and fonts | Dashboards | `reference/competitive-comparison.md` (BI matrix) |
| `BI-18` | Dashboards at both competitors' level: tabs; phone and tablet layouts; date-range, relative-date, number, text and linked filters; filter defaults that do not change silently; drill-down for every chart; click to another dashboard or URL; show and hide; saved auto-refresh; stories; templates; favourites; version history; trash; personal space | Dashboards | `reference/competitive-comparison.md` (BI matrix) |
| `AI-1` | **Ready now**. The assistant manages the dashboards that exist today: rename boards, delete them (with approval), arrange and resize tiles, move charts between boards | Ask AI | `AGENTS.md` (assistant parity) |
| `AI-2` | **Waits for `BI-18`**. The assistant adds and changes dashboard filters ("only 2025, only Bali") | Ask AI | `AGENTS.md` (assistant parity) |
| `AI-3` | **Waits for `BI-9`**. The assistant picks the time grain ("monthly") | Ask AI | `AGENTS.md` (assistant parity) |
| `AI-4` | **Waits for `BI-8`**. The assistant writes calculated fields in the product's formula language, not raw SQL | Ask AI | `AGENTS.md` (assistant parity) |
| `AI-5` | **Waits for `BI-16`**, per chart type as it lands. The assistant creates every new chart type: raw and pivot tables, KPI with comparison, histogram, Gantt and the rest | Ask AI | `AGENTS.md` (assistant parity) |
| `AI-6` | **Waits for `BI-6`, `BI-7`, `BI-10`, `BI-11`, `BI-13`**, each part with its BI item. The assistant builds charts with several dimensions and filters, joins tables, uses bins, groups, hierarchies and parameters, and combines two sources | Ask AI | `AGENTS.md` (assistant parity) |
| `AI-7` | **Waits for `BI-2`**. The assistant uses named metrics before it writes SQL, so the same question gives the same number | Ask AI | `AGENTS.md` (assistant parity) |
| `AI-8` | **Waits for `BI-17`**. The assistant changes number formats, colours, axes and labels, and adds trend and forecast lines | Ask AI | `AGENTS.md` (assistant parity) |
| `AI-9` | **Waits for `BI-18`** (layout part). The assistant adds tabs and uses templates | Ask AI | `AGENTS.md` (assistant parity) |
| `AI-10` | **Waits for `RPT-1`, `BI-1`**. The assistant schedules reports ("send this to me every Monday as PDF") and produces a PDF or Excel file on request | Ask AI | `AGENTS.md` (assistant parity) |
| `AI-11` | **Waits for `BI-19`, `BI-26`, `DEC-8`**. The assistant creates share and embed links only with approval, respects dashboard permissions, and never grants permissions | Ask AI | `AGENTS.md` (assistant parity) |
| `AI-12` | **Ready now** for fixing and explaining SQL inside the editor; variables **Waits for `BI-15`** | Ask AI | `AGENTS.md` (assistant parity) |
| `AI-13` | **Waits for `BI-14`**. The assistant answers "where does this number come from" from the chart's lineage (`get_lineage` exists today) | Ask AI | `AGENTS.md` (assistant parity) |
| `AI-14` | Ongoing, with every BI item. What the dashboard page sends to the assistant grows with the features: filters, tabs, metrics, formats | Ask AI | `AGENTS.md` (assistant parity) |
| `AI-15` | **Ready now**. A standard set of about 30 typical dashboard requests, re-run whenever an assistant tool changes, so the assistant does not silently get worse | Ask AI | `AGENTS.md` (assistant parity) |
| `BI-26` | Better embedding, without overdoing it: a single-chart embed that really shows only that chart, light/dark theme and our logo optional, filters passed through the link or token, sizing that fits the host page. Token hardening is `SEC-12`. No embedding SDK and no editing inside embeds | Dashboards | `reference/competitive-comparison.md` (BI matrix) |
| `BI-19` | Sharing and delivery, except Slack, Teams, Office and Google integrations: public link expiry and password; permissions per dashboard and folder; comments; a schedule per digest; alerts with filters that fire once; PNG export; import of exported dashboards. Scheduled reports are `RPT-1`, PDF and Excel export `BI-1`, embedding `BI-26` | Dashboards | `reference/competitive-comparison.md` (BI matrix) |
| `BLD-1` | Joins and aggregations in the pipeline builder | Build | PRODUCT §2 |
| `VER-1` | Verify SFTP, Google Sheets and Oracle sources end to end; test row filters | Data, Governance | PRODUCT §2 |
| `BI-2` | Metrics users can define once and reuse across dashboards (a semantic layer) | Dashboards | PRODUCT §2; `reference/competitive-comparison.md` (BI matrix) |
| `DATA-9` | Upload a file from the console | Data | PRODUCT §2 |
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
| `SEC-6` | Stop older API handlers from returning internal error text | Security | `AGENTS.md` |
| `SEC-12` | Embed tokens: require an expiry, allow revocation, keep the signing secret out of plain text, set a frame-ancestors policy | Security | `lakehouse-embed` |

## Later

| ID | Item | Area | Source |
| --- | --- | --- | --- |
| `OPS-3` | Run on several servers; high availability | Platform | PRODUCT §1, §2 |
| `SRC-3` | Connectors for business apps (pick by customer demand) | Data | PRODUCT §2 |
| `SRC-4` | Live change capture for databases other than PostgreSQL | Data | PRODUCT §2 |
| `DATA-7` | Query a table as of an earlier time | Data | PRODUCT §2 |
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
| `BI-21` | Where charts read data | Decided: only data in the lakehouse, for now | 2026-10-06 |
| `SEC-8` | Dependency security checks green on `main` | Done: PR #62. Three TLS-library vulnerabilities cleared by updating the SQL Server client; licence check fixed | 2026-10-02 |
