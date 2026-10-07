# Backlog

The one list of known work. Look at it weekly; pick what is next.

- **When:** Now, Next, Later, or Decide (needs a yes/no first).
- **Source:** where the item comes from, so it can be checked. "PRODUCT §2"
  is the coverage table in `PRODUCT.md`.
- IDs are never reused. A finished or rejected item moves to the bottom with
  the date and reason; it is not deleted.
- **Specs:** every BI, assistant-for-dashboards, Data and Query Studio item
  has its spec under "Specs" at the end of this file: what users get, what to
  build against what exists today, the competitor reference and a size
  estimate. A spec is not a plan: each item still needs a feature page and a
  plan before anyone builds it (`AGENTS.md`).

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
| `SEC-14` | A stored connector password can be stolen by re-pointing the connector: a host change keeps the secret and the next test sends it to the new server | Security | `routes/connectors.rs`, `connector_probe.rs` |
| `SEC-15` | Connection tests can reach internal addresses: the block is off by default in compose, the REST test follows redirects, no address pinning, CDC delete skips the check | Security | `docker-compose.yml`, `connector_probe.rs` |
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
| `DATA-11` | Catalog search that finds columns and tags, with tolerant matching | Data | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-12` | Certified and deprecated marks; governed tags with allowed values | Data | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-13` | Column-level lineage | Data | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-14` | Data quality in depth: a library of ready checks, freshness and volume anomaly detection, an incidents overview | Data | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-15` | Asset page depth: 1,000-row preview, 30-day usage, grant and revoke, restore a dropped table | Data | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-16` | Time travel proven safe: a gate test that past-version queries pass the masking rewrite | Data | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-17` | Upload more formats (Excel, JSON, Parquet), several files at once, a larger measured size limit | Data | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-18` | Column types on upload, with names and types editable before loading | Data | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-19` | Uploaded tables can feed pipelines and so reach dashboards | Data | `reference/competitive-comparison.md` (Data matrix) |
| `QS-1` | Query Studio editor basics: column autocomplete, schema browser, several statements, tabs, formatting rules | Query Studio | `reference/competitive-comparison.md` (Query Studio matrix) |
| `QS-2` | Query Studio results: larger configurable results, grid filters and column stats, server downloads, server-side Stop, configurable timeouts | Query Studio | `reference/competitive-comparison.md` (Query Studio matrix) |
| `QS-3` | Saved queries done properly: edit, delete, folders, sharing levels, version history. **Waits for `SEC-20`** | Query Studio | `reference/competitive-comparison.md` (Query Studio matrix) |
| `QS-4` | Query history page with filters, and a query profile | Query Studio | `reference/competitive-comparison.md` (Query Studio matrix) |
| `QS-5` | The bridge to BI: parameters that become dashboard filters, snippets, charts from results, reuse a saved query in SQL, alerts on a query | Query Studio | `reference/competitive-comparison.md` (Query Studio matrix) |
| `AI-16` | Hand-off to the AI team: AI features for the Data module (AI-written descriptions, plain-language table search, AI classification, AI quality suggestions and incident help, AI connector building). The AI team plans these | Ask AI | `reference/competitive-comparison.md` (Data matrix) (AI table) |
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
| `SRC-3` | Connectors for business apps (pick by customer demand). **Waits for `DEC-9`** | Data | PRODUCT §2 |
| `SRC-4` | Live change capture for databases other than PostgreSQL | Data | PRODUCT §2 |
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

## Specs

Generated from the roadmap pages' data, so the pages and this list agree.
"Today" describes `main` at `47a826d` for BI and at `f3a3196` for Data and
Query Studio, read from the code, not tested. Sizes: S = days, M = one to two
weeks, L = several weeks; estimates, not commitments. An item is done when
every row of its Build table works in a running console and the product
owner accepts it.

### Dashboards (BI)

#### Phase 0: Security fixes first

Found by the BI code audit. They are small and come before any feature work, because two of them expose data a user should not see.

##### `SEC-9` Natural-language queries respect masking

- **Size:** S (planner's estimate)
- **What users get:** A user who may not see a column cannot get it by asking in plain language.
- **Reference:** Both competitors run AI queries inside the user's permissions.

| Build | Today |
| --- | --- |
| Run the model's SQL through the same masking and row-filter rewrite as Query Studio | Runs straight against ClickHouse |
| Require query permission, not just a sign-in | Any signed-in user |

##### `SEC-10` Alert webhooks cannot reach internal addresses

- **Size:** S (planner's estimate)
- **What users get:** An alert cannot be used to call services inside the customer's network.
- **Reference:** Required by our own rule for outbound calls in AGENTS.md.

| Build | Today |
| --- | --- |
| Send webhooks through the allowlisted resolver | Only checks that the URL starts with http(s) |

##### `SEC-11` No raw database errors on screen

- **Size:** S (planner's estimate)
- **What users get:** Public and embed viewers never see internal error text.
- **Reference:** Our principle 4: upstream error text never reaches a response.

| Build | Today |
| --- | --- |
| Classify tile errors, including on public and embed links | Raw ClickHouse message shown |
| Classify the agent endpoints' errors | Raw message returned |

##### `SEC-12` Safer embed tokens

- **Size:** M (planner's estimate)
- **What users get:** An embed link can expire and can be withdrawn.
- **Reference:** Tableau connected apps and Metabase guest embeds both expire tokens.

| Build | Today |
| --- | --- |
| Require an expiry on every token | Expiry optional; tokens can live forever |
| Revoke a token or all tokens of a dashboard | No revocation |
| Keep the signing secret out of plain text | Stored in plain text when not set by environment |
| Set a frame-ancestors policy | None |

##### `SEC-13` Check the staging file pushed to main

- **Size:** S (planner's estimate)
- **What users get:** Confidence that no real password or server address is in the repository.
- **Reference:** Commit df14b78 added .env.staging directly to main.

| Build | Today |
| --- | --- |
| Confirm .env.staging holds no real credentials or hostnames; rotate anything real | Not checked; the reviewer could not read it |

#### Phase 1: Dashboard basics

What every viewer and author touches daily. The biggest gap against Metabase's free edition.

##### `BI-18` (part A) Filters that work like other BI tools

- **Size:** L (planner's estimate)
- **What users get:** Filter by any date range, number or text; filters that narrow each other; defaults that stay put.
- **Reference:** Metabase: all plans. Tableau: all editions.

| Build | Today |
| --- | --- |
| Date-range and relative-date filters (last 30 days, this quarter) | Only a year filter, hard-coded to a column named tahun, not saved |
| Number and text filters | None |
| Linked filters (choosing a province narrows the city list) | None |
| Search inside a long value list | Capped at 200 values, no search |
| Defaults change only when someone saves them | An editor's ad-hoc filter silently becomes the shared default, also for public and embed viewers |
| Filter values from SQL-source charts | Values come only from Gold tables |

##### `BI-18` (part B) Click-through and drill-down everywhere

- **Size:** M (planner's estimate)
- **What users get:** Click any chart to see the rows behind it or jump to a related dashboard.
- **Reference:** Metabase: custom destinations, all plans. Tableau: go-to-sheet and URL actions.

| Build | Today |
| --- | --- |
| Drill-down for every chart type and for SQL-source charts | Not for nine chart types or SQL-source charts; 100-row cap |
| Click to another dashboard or a URL, passing the clicked value | None |
| Saved auto-refresh and real full-screen | Browser-only refresh, not saved; full-screen is a page overlay |

##### `BI-9` Group by day, week, month, quarter, year

- **Size:** M (planner's estimate)
- **What users get:** Time charts at the right grain without writing SQL.
- **Reference:** Both competitors, all tiers.

| Build | Today |
| --- | --- |
| Time grain on any date column in the builder | None |
| Grain switch on the dashboard | None |

##### `BI-8` Calculated fields

- **Size:** L (planner's estimate)
- **What users get:** New columns from formulas: profit = revenue − cost, growth %, text clean-up, date parts.
- **Reference:** Metabase: about 100 functions. Tableau: row, aggregate and LOD expressions.

| Build | Today |
| --- | --- |
| Formula editor with maths, text, dates, conditions | Only plain column names |
| Running totals and period comparisons | None |
| Validation with clear errors before saving | n/a |

##### `BI-16` (part A) Tables and KPI cards

- **Size:** M (planner's estimate)
- **What users get:** The two most used dashboard elements, done properly.
- **Reference:** Both competitors.

| Build | Today |
| --- | --- |
| Table of raw rows with column formats | Tables are always grouped, at most 100 rows |
| Pivot table with totals and subtotals | None |
| KPI with comparison to last period and a sparkline | Plain big number |
| Record detail view | Drill-down list only |

#### Phase 2: Authoring depth

Lets analysts build what they need in the console. The assistant stays the fast path; the builder must be able to express the same things.

##### `BI-6` A more capable chart builder

- **Size:** L (planner's estimate)
- **What users get:** Several dimensions, filters, sorting and multi-step questions in the builder. No drag and drop.
- **Reference:** Metabase notebook editor; Tableau shelves.

| Build | Today |
| --- | --- |
| Several dimensions and breakdowns | One dimension, one optional breakdown |
| Filters inside the chart | None |
| Sorting and limits on any column | Order and limit only |
| Multi-step questions (summarise, then filter the result) | None |

##### `BI-7` Joins without writing SQL

- **Size:** M (planner's estimate)
- **What users get:** Combine two tables in the builder by picking the matching columns.
- **Reference:** Metabase: all plans. Tableau: relationships and joins.

| Build | Today |
| --- | --- |
| Pick a second table and the columns to match | Only by writing SQL |
| Inner, left, right and full joins | None |

##### `BI-13` Combine two sources on one chart

- **Size:** M (planner's estimate)
- **What users get:** Plot two questions together when they share a dimension.
- **Reference:** Metabase Visualizer; Tableau data blending.

| Build | Today |
| --- | --- |
| Overlay a second question on a chart | Each chart reads one table or one SQL source |

##### `BI-10` Bins, groups and hierarchies

- **Size:** M (planner's estimate)
- **What users get:** Age bands, custom region groups, drill from province to city.
- **Reference:** Tableau: all. Metabase: binning.

| Build | Today |
| --- | --- |
| Bins for numbers | None |
| Custom groups of values | None |
| Hierarchies to drill through | None |

##### `BI-11` Parameters

- **Size:** M (planner's estimate)
- **What users get:** One control that changes a value used across charts and SQL, such as a target or a top-N.
- **Reference:** Tableau parameters and parameter actions; Metabase SQL variables.

| Build | Today |
| --- | --- |
| Parameters in charts and dashboards | None |
| Variables in SQL | None |

##### `BI-2` Reusable metrics

- **Size:** L (planner's estimate)
- **What users get:** Define 'revenue' or 'active users' once; every chart uses the same definition.
- **Reference:** Metabase metrics and segments, all plans; Tableau Pulse metrics.

| Build | Today |
| --- | --- |
| Users define, name and describe metrics | Built-in cards defined once in a deployment file |
| Charts and the assistant pick metrics by name | None |

#### Phase 3: Look and layout

Makes dashboards presentable to management and customers.

##### `BI-17` (part A) Chart formatting

- **Size:** L (planner's estimate)
- **What users get:** Control over how every chart looks.
- **Reference:** Both competitors.

| Build | Today |
| --- | --- |
| Number formats: decimals, currency, percent, compact | The saved format is ignored; numbers round to whole numbers |
| Choose colours per series and palettes | One fixed palette |
| Axis titles, ranges and log scale | None |
| Data labels on points | None |
| Conditional formatting on tables and KPIs | None |
| Goal and reference lines, trend lines | None |
| Annotations and custom tooltips | None |
| Themes and fonts | Follows app dark mode only |

##### `BI-18` (part C) Layout and organisation

- **Size:** L (planner's estimate)
- **What users get:** Bigger dashboards that stay tidy, and safety when things go wrong.
- **Reference:** Metabase: all plans. Tableau: all editions.

| Build | Today |
| --- | --- |
| Tabs | None |
| Phone and tablet layouts | Tiles scale but do not stack |
| Show and hide sections | None |
| Stories / written reports with charts | None |
| Templates | None |
| Favourites and better search | Last visited only; search on the browse page only |
| Version history with revert, and trash with restore | None |
| Personal space for drafts | None |
| Duplicate copies filters and works on the built-in board | Filters dropped; fails on the built-in board |

##### `BI-16` (part B) Simple chart types we lack

- **Size:** M (planner's estimate)
- **What users get:** Common charts every competitor offers.
- **Reference:** Both competitors.

| Build | Today |
| --- | --- |
| Progress bar | None |
| Histogram | None |
| Bullet graph | None |
| Image card and web-page card | None |

#### Phase 4: Delivery and sharing

Gets dashboards to people who never open the console.

##### `RPT-1` Scheduled reports with attachments

- **Size:** L (planner's estimate)
- **What users get:** A dashboard arrives by email every Monday as PDF and Excel.
- **Reference:** Metabase: CSV, Excel and PDF, all plans. Tableau: PNG and PDF.

| Build | Today |
| --- | --- |
| Schedules per report (daily, weekly, monthly) | None |
| PDF and Excel attachments | Text digests only |
| Skip when empty | None |

##### `BI-1` Export PDF and Excel

- **Size:** M (planner's estimate)
- **What users get:** One click to a clean PDF or a spreadsheet.
- **Reference:** Both competitors.

| Build | Today |
| --- | --- |
| Server-made dashboard PDF | Browser print |
| Excel export | CSV only |
| Download button for the existing server CSV export | Endpoint exists, no button |

##### `BI-19` Sharing, alerts and permissions

- **Size:** L (planner's estimate)
- **What users get:** Share safely, control who sees what, and get alerts that make sense.
- **Reference:** Metabase: collection permissions on all plans. Tableau: project permissions.

| Build | Today |
| --- | --- |
| Permissions per dashboard and folder | Global per role only; Analysts cannot view dashboards |
| Public links with expiry and password | No expiry, no password |
| A schedule per digest | Every digest goes out every 15 minutes |
| Alerts with filters that fire once per change | One aggregate over a whole table; re-fires every check |
| PNG export of a chart | None |
| Comments on dashboards | None |
| Import an exported dashboard | YAML export only |

##### `BI-26` Better embedding, not bigger

- **Size:** M (planner's estimate)
- **What users get:** Embeds that look like part of the host site and show only what they should.
- **Reference:** Metabase guest embeds; Tableau Embedding API (we stop short of an SDK).

| Build | Today |
| --- | --- |
| A single-chart embed that shows only that chart | Removing a URL parameter shows the whole board |
| Light/dark theme and optional logo | 'Rantai Lake' hard-coded |
| Filters passed through the link or token | Locked filters in signed tokens only |
| Sizing that fits the host page | Fixed |

#### Phase 5: Speed

Mostly backend work, so it can run alongside phases 1 to 4. The SQL editor moved to Query Studio, a main feature of its own (QS-1 to QS-5).

##### `BI-4` Caching and parallel tiles

- **Size:** M (planner's estimate)
- **What users get:** Dashboards open fast and do not hit the engine for the same query twice.
- **Reference:** Metabase: adaptive caching on all plans. Tableau: extracts and query cache.

| Build | Today |
| --- | --- |
| Result cache with a freshness rule | Every request runs fresh |
| Tiles load in parallel | One after another |

##### `BI-5` Timeouts and limits you can set

- **Size:** S (planner's estimate)
- **What users get:** Admins choose how long and how big a query may be.
- **Reference:** Both competitors.

| Build | Today |
| --- | --- |
| Configurable timeouts and row limits | Fixed: 2,000 rows, 30 s or 60 s |
| Stop cancels the query on the server | Only the browser request is cancelled |

#### Phase 6: Advanced visuals and analytics

Tableau-level depth. Valuable, but after the basics are solid.

##### `BI-16` (part C) Advanced chart types and maps

- **Size:** L (planner's estimate)
- **What users get:** Gantt charts, any country's map, and spatial analysis.
- **Reference:** Tableau: all. Metabase: custom visualizations (Pro+).

| Build | Today |
| --- | --- |
| Gantt chart | None |
| World map and uploaded boundaries | Jakarta and Indonesia maps only |
| Spatial analysis (distance, areas, spatial joins) | None |
| Custom chart plugins | None |

##### `BI-17` (part B) Forecasting and clustering

- **Size:** M (planner's estimate)
- **What users get:** A forecast line on a trend; automatic grouping of similar points.
- **Reference:** Tableau: exponential-smoothing forecast and k-means clustering.

| Build | Today |
| --- | --- |
| Forecast on time series | None |
| Clustering on scatter charts | None |

##### `BI-14` Lineage from a chart to its sources

- **Size:** M (planner's estimate)
- **What users get:** From any chart, see which tables and pipelines feed it.
- **Reference:** Tableau Catalog (Enterprise); Metabase dependency graph (Pro+). A natural strength for a lakehouse.

| Build | Today |
| --- | --- |
| Lineage link on each chart | Lineage page exists, not linked from dashboards |

### Assistant for dashboards

The assistant's side of the BI roadmap (`AGENTS.md`, assistant parity).

##### `AI-1` Manage the dashboards that exist today

- **Status:** Ready now
- **A user says:** "Rename this dashboard to 'Q3 Tourism' and move the Bali chart to the top."
- **The assistant needs:** Tools to rename a dashboard, delete one (with approval), arrange and resize tiles, and move a chart to another dashboard.

##### `AI-12` Help inside the SQL editor

- **Status:** Ready now. Variables wait for QS-5 (Query Studio).
- **A user says:** "Why does this query fail? Fix it."
- **The assistant needs:** The assistant opened from the editor with the current SQL and error, proposing a fix the user accepts or rejects.

##### `AI-15` The standard request set

- **Status:** Ready now
- **A user says:** "about 30 typical requests, run automatically"
- **The assistant needs:** Requests with expected results, run whenever an assistant tool changes, so the assistant does not silently get worse.

##### `AI-14` Page awareness keeps up

- **Status:** Ready now. Ongoing, alongside every BI feature.
- **A user says:** "Which region dropped the most on this tab?"
- **The assistant needs:** What the dashboard page sends to the assistant grows with each feature: filters, tabs, metrics, formats.

##### `AI-2` Dashboard filters

- **Status:** Waits for `BI-18` (BI phase 1)
- **A user says:** "Show only 2025 and only Bali."
- **The assistant needs:** A tool to add, change and clear dashboard filters (date range, number, text), following the rule that defaults change only when saved.

##### `AI-3` Time grain

- **Status:** Waits for `BI-9` (BI phase 1)
- **A user says:** "Make this monthly."
- **The assistant needs:** A time-grain field in the chart tools.

##### `AI-4` Calculated fields

- **Status:** Waits for `BI-8` (BI phase 1)
- **A user says:** "Add profit as revenue minus cost."
- **The assistant needs:** Formulas written in the product's formula language, not raw SQL, validated before saving.

##### `AI-5` Every new chart type

- **Status:** Waits for `BI-16` (BI phase 1, 3, 6)
- **A user says:** "Show this as a pivot table with totals."
- **The assistant needs:** Each new chart type added to the chart tools' schema in the same change that adds it.

##### `AI-6` Richer charts

- **Status:** Waits for `BI-6` (BI phase 2), `BI-7` (BI phase 2), `BI-10` (BI phase 2), `BI-11` (BI phase 2), `BI-13` (BI phase 2)
- **A user says:** "Visitors by province and month, domestic only, next to hotel occupancy."
- **The assistant needs:** Several dimensions, filters, joins, bins, groups, hierarchies, parameters and two sources on one chart, each with its BI item.

##### `AI-7` Named metrics first

- **Status:** Waits for `BI-2` (BI phase 2)
- **A user says:** "What was revenue last quarter?"
- **The assistant needs:** Tools to list and use metrics; the assistant picks a named metric before writing SQL, so one question gives one number.

##### `AI-8` Chart formatting

- **Status:** Waits for `BI-17` (BI phase 3, 6)
- **A user says:** "Show values in rupiah, make the bars green, add a target line at 1,000."
- **The assistant needs:** Format, colour, axis, label, trend-line and forecast fields in the chart tools.

##### `AI-9` Tabs and templates

- **Status:** Waits for `BI-18` (BI phase 3)
- **A user says:** "Put the map charts in a separate tab."
- **The assistant needs:** Tabs and templates in the dashboard tools.

##### `AI-10` Reports and exports

- **Status:** Waits for `RPT-1` (BI phase 4), `BI-1` (BI phase 4)
- **A user says:** "Send this dashboard to me every Monday as a PDF."
- **The assistant needs:** A tool to schedule a report and one to produce a PDF or Excel file.

##### `AI-11` Share and embed, with approval

- **Status:** Waits for `BI-19` (BI phase 4), `BI-26` (BI phase 4), `DEC-8` (decision)
- **A user says:** "Make a public link for this dashboard."
- **The assistant needs:** Share and embed tools that go through approval, respect dashboard permissions and never grant them.

##### `AI-13` Where does this number come from

- **Status:** Waits for `BI-14` (BI phase 6)
- **A user says:** "Where does this number come from?"
- **The assistant needs:** The chart's lineage from BI-14, through the existing get_lineage tool.

### Data module

#### Phase 0: Security fixes first

Found by the Data module code audit. Two are high severity; all come before feature work.

##### `SEC-14` Connector passwords cannot be stolen by re-pointing a connector

- **Size:** M (planner's estimate)
- **What users get:** Changing where a connector points can never send its stored password to a new server.
- **Reference:** Re-checked in code: saving the ingest spec keeps the secret while the host changes.

| Build | Today |
| --- | --- |
| Changing a connector's host or port clears its stored credentials, or requires them again | Host changes keep the stored password |
| Connection tests require TLS and never send a password in clear text | PostgreSQL test uses a mode the server can decline |
| The credential-change dial override follows the same rule | Same gap |

##### `SEC-15` Connection tests cannot reach internal addresses

- **Size:** M (planner's estimate)
- **What users get:** A connection test cannot be used to map or attack services inside the customer's network.
- **Reference:** Re-checked: CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS defaults to true; the REST test follows redirects.

| Build | Today |
| --- | --- |
| The internal-address block is on by default in compose | Off by default |
| The REST test does not follow redirects, or re-checks every hop | Follows redirects after checking only the first host |
| Every API-side test pins the resolved address | No pinning; DNS can change between check and dial |
| Deleting a CDC connector uses the same check | Dials without it |
| Error messages do not reveal internal IP addresses | They do |

##### `SEC-16` No cross-tenant leaks in the Data module

- **Size:** M (planner's estimate)
- **What users get:** One organisation never sees another's connectors, uploads or table names.
- **Reference:** From the audit; not re-checked.

| Build | Today |
| --- | --- |
| The ingestible-connectors route is filtered by tenant | Lists every tenant's connectors with hosts and usernames |
| Uploads land in a namespace per tenant | All tenants share one Bronze namespace |
| Messages never name another tenant's upload table | One connector message does |
| Catalog annotation edits pass the tenant gate | They skip it |

##### `SEC-17` Upload hardening

- **Size:** S (planner's estimate)
- **What users get:** An upload cannot exhaust the ingestion process, and exported files cannot run formulas.
- **Reference:** From the audit; not re-checked.

| Build | Today |
| --- | --- |
| A cap on the number of columns | None: one long header line can create millions of columns |
| A limit on concurrent uploads | None |
| CSV exports neutralise cells starting with =, +, - or @ | Cells exported as they are |

##### `SEC-18` No default credentials in compose

- **Size:** S (planner's estimate)
- **What users get:** A fresh install never runs with a known password.
- **Reference:** Re-checked: CONNECTOR_PG_PASSWORD falls back to 'lakehouse'; AGENTS.md rule 5.

| Build | Today |
| --- | --- |
| Must-set secrets use ${X:?} so compose refuses to start without them | Fall back to 'lakehouse' and 'rustfsadmin' |

#### Phase 1: Connector reliability

Make every advertised connector work and say when it does not, before adding more.

##### `SRC-6` Fix the broken connectors

- **Size:** S (planner's estimate)
- **What users get:** Every connector type shown as supported can be tested and edited.
- **Reference:** Bugs found by reading the code.

| Build | Today |
| --- | --- |
| Object-storage connectors made in the wizard can be tested and have their keys changed | The wizard and the test disagree on the host format |
| Oracle credentials can be changed and the test reads correctly | Refused with 422; every test reads as failed |
| Google Sheets is shown as unsupported until it works | Listed as supported, works nowhere |
| The Sources page header describes what exists | Promises SaaS and federation |

##### `SRC-7` Alerts when a load fails

- **Size:** M (planner's estimate)
- **What users get:** People hear about a failed connector run or upload without opening the console.
- **Reference:** Airbyte: email and webhook on failure, success and schema change. Databricks and Snowflake: alerts.

| Build | Today |
| --- | --- |
| An alert on every failed connector run and upload load | Failures raise nothing |
| Connector health updates from real runs, not only manual tests | Changes only when someone runs a test |
| Email and webhook channels, reusing the alert channels that exist | n/a |

##### `SRC-8` Schema changes at the source

- **Size:** M (planner's estimate)
- **What users get:** When a source table changes shape, someone is told, and the choice of what happens is theirs.
- **Reference:** Airbyte: propagate, approve or pause; breaking changes pause. Databricks: new columns added, removed ones marked inactive.

| Build | Today |
| --- | --- |
| Detect added, removed and changed columns on each run | Default evolution, silent |
| A per-connector setting: apply automatically, ask first, or pause | None |
| A notice in the connector page and an alert | None |

##### `SRC-9` Test every advertised connector end to end

- **Size:** M (planner's estimate)
- **What users get:** Every connector type we list has a gate test in CI.
- **Reference:** Includes the connector half of VER-1.

| Build | Today |
| --- | --- |
| Gate tests for Oracle, SFTP, MySQL CDC and SQL Server CDC | Never gate-tested |
| The file-upload gate (g9) runs in CI | Exists but not in CI |

##### `SRC-10` More load modes

- **Size:** M (planner's estimate)
- **What users get:** Loads that keep one current row per key, or keep history.
- **Reference:** Airbyte: five sync modes including deduplication. Databricks: SCD type 1 and 2.

| Build | Today |
| --- | --- |
| Deduplicate on a primary key (merge) | None |
| Keep history of changes (SCD type 2) | None |
| Refresh one table from scratch, or clear it | None |

##### `SRC-11` Connector operations

- **Size:** M (planner's estimate)
- **What users get:** Runs that recover by themselves and leave a readable trail.
- **Reference:** Airbyte: retries with backoff, logs per attempt, column selection.

| Build | Today |
| --- | --- |
| Automatic retries with backoff | Not checked for ingest jobs |
| A log view per run | Per-table results only |
| Choose columns, not only tables | Column choice not checked |

##### `SRC-12` Credentials done properly

- **Size:** L (planner's estimate)
- **What users get:** Credentials kept in a secret manager or encrypted, and one-click sign-in for sources that support it.
- **Reference:** Airbyte (Core): AWS, GCP, Azure, Vault. Databricks and Snowflake: secret objects and external secret managers.

| Build | Today |
| --- | --- |
| External secret managers: Vault, AWS, GCP, Azure | None |
| Encrypted at rest when no secret manager is used | Plain files at rest |
| OAuth sign-in for sources that offer it | REST OAuth2 answers unsupported |

##### `SRC-13` Table discovery for every source type

- **Size:** M (planner's estimate)
- **What users get:** Pick tables from a list for any source, not only databases.
- **Reference:** All three competitors discover schemas for every connector.

| Build | Today |
| --- | --- |
| Discovery for files, REST, MongoDB, Kafka, SFTP and Oracle | SQL and CDC sources only |

#### Phase 2: A smarter catalog

The catalog is close to the competitors; these close the gap where it shows most.

##### `DATA-11` Search that finds columns and tags

- **Size:** M (planner's estimate)
- **What users get:** Find a table by a column name or a tag, from anywhere.
- **Reference:** Databricks: names, comments, column names, tags, search syntax. Snowflake: fuzzy metadata search.

| Build | Today |
| --- | --- |
| Column names and descriptions are searchable | Searchable nowhere |
| Tags are searchable and filterable in Data Explorer | Neither |
| Tolerant matching (word order, typos) | Substring match only |

##### `DATA-12` Certification and governed tags

- **Size:** M (planner's estimate)
- **What users get:** Trusted tables are marked, and tags follow agreed values.
- **Reference:** Databricks: certified and deprecated marks, governed tags. Snowflake: certification tag, tag propagation.

| Build | Today |
| --- | --- |
| Certified and deprecated marks, shown in search and on the asset | None |
| Tags with allowed values set by an admin | Free-text tags |

##### `DATA-13` Column-level lineage

- **Size:** L (planner's estimate)
- **What users get:** See which source columns feed a Gold column.
- **Reference:** Databricks and Snowflake: automatic column-level lineage. Lineage from outside systems stays GOV-3.

| Build | Today |
| --- | --- |
| Column mappings recorded for pipelines and builds | Always empty |
| Column lineage on the asset's Lineage tab | Hidden because empty |

##### `DATA-14` Data quality in depth

- **Size:** L (planner's estimate)
- **What users get:** Quality checked automatically, with a clear list of what is wrong.
- **Reference:** Snowflake: system metric library, expectations, schedules, anomaly detection, incidents dashboard. Databricks: anomaly detection and profiling.

| Build | Today |
| --- | --- |
| A library of ready checks: nulls, duplicates, row count, freshness, accepted values | Hand-written rules |
| Automatic freshness and volume anomaly detection | Freshness against a set target only |
| A quality overview with open incidents | None |

##### `DATA-15` Asset page depth

- **Size:** M (planner's estimate)
- **What users get:** The asset page matches Databricks' table page.
- **Reference:** Databricks Catalog Explorer.

| Build | Today |
| --- | --- |
| Data preview up to 1,000 rows | 25–100 rows |
| Usage over 30 days, frequent users, column popularity | 7 days of counts |
| Grant and revoke access from the asset | Not available |
| Restore a dropped table | Not available |

##### `DATA-16` Time travel proven safe

- **Size:** S (planner's estimate)
- **What users get:** Querying a past version of a table is trusted to apply masking.
- **Reference:** Supersedes DATA-7: the picker is built.

| Build | Today |
| --- | --- |
| A gate test proving past-version queries pass the masking rewrite | No test |
| The picker works for both engines | Built, unproven |

#### Phase 3: File upload in depth

Upload is built and safe (DATA-9, waiting for acceptance); these make it as useful as the competitors'.

##### `DATA-17` More formats, more files, bigger files

- **Size:** M (planner's estimate)
- **What users get:** Upload Excel, JSON and Parquet, several files at once, and larger files.
- **Reference:** Databricks: CSV, TSV, JSON, Avro, Parquet, 10 files up to 2 GB. Snowflake: 250 files of 250 MB.

| Build | Today |
| --- | --- |
| Excel, JSON and Parquet | Delimited text only |
| Several files in one upload | One file |
| A larger size limit, measured against the shared ingestion process | 50 MB and 2,000,000 rows |

##### `DATA-18` Column types on upload

- **Size:** M (planner's estimate)
- **What users get:** Numbers and dates load as numbers and dates.
- **Reference:** Both competitors infer types and let you edit names and types before loading.

| Build | Today |
| --- | --- |
| Types detected from the file | Every column stored as text |
| Edit column names and types before loading | Names cleaned automatically |

##### `DATA-19` Uploaded tables feed pipelines and dashboards

- **Size:** M (planner's estimate)
- **What users get:** An uploaded table can become a Silver or Gold table and appear on a dashboard.
- **Reference:** Both competitors treat an uploaded table like any other. Coordinate with the pipelines stream.

| Build | Today |
| --- | --- |
| An upload can be the source of a pipeline | Not possible |

#### Phase 4: Connector breadth

Breadth by reuse, not by hand.

##### `SRC-3` Connectors for business apps

- **Size:** L (planner's estimate) · **Waits for** `DEC-9`
- **What users get:** CRM, ads, support and finance sources without building each one.
- **Reference:** Airbyte: 602 sources. Databricks: about 60 managed. Snowflake: about 25.

| Build | Today |
| --- | --- |
| Pick a reuse route: dlt verified sources (our loader already runs on dlt) or another library | Decision DEC-9 |
| The first ten SaaS sources by customer demand | None |

### Query Studio

#### Phase 0: Security fixes first

Query Studio's findings from the Data module audit.

- `SEC-9` Plain-language queries respect masking: specified above.

##### `SEC-19` The cost estimate runs only safe SQL

- **Size:** S (planner's estimate)
- **What users get:** Estimating a query cannot make the engine fetch outside URLs.
- **Reference:** From the audit; not re-checked.

| Build | Today |
| --- | --- |
| Read-only check and table-function block before EXPLAIN | Raw SQL |
| No database error text in the response | First 240 characters returned |

##### `SEC-20` Queries are scoped to the tenant

- **Size:** M (planner's estimate)
- **What users get:** A user in one organisation cannot query another's tables or see its saved queries.
- **Reference:** From the audit; not re-checked.

| Build | Today |
| --- | --- |
| Tenant gate on query runs | None |
| Saved queries listed per tenant | Listed for everyone |

##### `SEC-21` Downloads and big results are safe

- **Size:** S (planner's estimate)
- **What users get:** A download always applies today's masks, and a huge result cannot exhaust the API.
- **Reference:** From the audit; not re-checked.

| Build | Today |
| --- | --- |
| Re-downloads apply the current policy | Use the SQL rewritten at run time |
| Results are capped in the engine, not after buffering | Whole result buffered |

#### Phase 1: Everyday basics

What every SQL user expects on day one.

##### `QS-1` Editor basics

- **Size:** M (planner's estimate)
- **What users get:** An editor that knows your columns and handles real work.
- **Reference:** Databricks SQL editor; Snowflake Workspaces; Metabase SQL editor.

| Build | Today |
| --- | --- |
| Column autocomplete | Table names only |
| A schema browser beside the editor | Not checked |
| Several statements and run selection | Not checked |
| Query tabs | Not checked |
| Formatting rules | A small formatter |

##### `QS-2` Results you can use

- **Size:** M (planner's estimate)
- **What users get:** See, filter and download full results; stop a query that runs too long.
- **Reference:** Databricks: 64,000 rows, filters, column profiling, CSV/TSV/Excel to about 5 GB.

| Build | Today |
| --- | --- |
| Larger results, configurable | 2,000 rows, still computed in full |
| Filters, column statistics and a cell inspector in the grid | Sort only |
| Download CSV, Excel and Parquet through the server | Browser CSV of 2,000 rows; server route has no button |
| Stop cancels the query on the server | Browser only |
| Configurable timeouts | Fixed 60 s |

#### Phase 2: Saved queries and history

Work that lasts and can be shared.

##### `QS-3` Saved queries done properly

- **Size:** M (planner's estimate) · **Waits for** `SEC-20`
- **What users get:** Organise, share and recover saved queries.
- **Reference:** Databricks: folders and sharing levels. Metabase: collections with permissions, 15 versions.

| Build | Today |
| --- | --- |
| Edit, rename and delete | Create and list only |
| Folders and sharing levels | None |
| Version history with revert | None |

##### `QS-4` History and query profile

- **Size:** M (planner's estimate)
- **What users get:** Find any past query and see why a slow one is slow.
- **Reference:** Databricks and Snowflake: history pages with filters and operator-level profiles.

| Build | Today |
| --- | --- |
| A history page with filters | Last 200 in a side panel |
| A query profile from the engine's plan | Always 'Not measured' |

#### Phase 3: The bridge to BI

Query Studio is shared by BI and the lakehouse; this is the BI half.

##### `QS-5` Parameters, snippets and charts from results

- **Size:** L (planner's estimate)
- **What users get:** SQL that becomes dashboard content directly.
- **Reference:** Metabase: variables that become dashboard filters, snippets, any chart from a SQL result, reuse a question as a CTE.

| Build | Today |
| --- | --- |
| Parameters that become dashboard filters | None |
| Snippets | None |
| A chart straight from a result | Through 'save as SQL source' only |
| Reuse a saved query inside SQL | None |
| Alerts on a query result | Alerts only on dashboard metrics |
