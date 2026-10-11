# Backlog

The master list of Lakehouse work. Every item has a stable ID, and the IDs
are never reused. The coding agents read this file, so it is the source of
truth for the work (`DEC-11`).

**This file and the Lark base.** The base mirrors this list. The repo holds
everything about an item: its ID, kind, text, module, priority, status, PR,
source, its dates (the **Dates** section), its spec in [`specs/`](specs/) and
its QA-case results (the acceptance table on its feature page). The base adds
only what a public repo must not hold: the people, Owner and Acceptor, held
through each module's owner (`DEC-12`). When anything here changes, the base
row is updated to match (`AGENTS.md` step 6). Nothing the repo holds is
changed only in the base.

## The columns

- **Kind**: **Feature** (one thing a user can do or rely on; it has an
  acceptance checklist and an Acceptor), **Task** (tracked work that is not
  itself a user capability: a fix, a test, a check), or **Decision** (a choice
  the product owner makes).
- **Module**: from the base's list: Data, Dashboards, Query Studio
  (`DEC-13`), AI Copilot, Build, Governance, Operations,
  Administration & Security. Four names used here are **not yet in the
  base**; the owner adds or remaps them: Platform, Delivery, Docs, Business.
- **Priority**: P0 (blocks production), P1, P2, P3. The old When maps to it:
  Now = P1, Next = P2, Later = P3. P0 today: `SEC-14`, `SEC-15` (`DEC-14`)
  and `SEC-22`.
- **Status**, by kind:
  - Feature: the base's six: **Idea** (no spec yet), **Planned** (has a spec,
    not started), **Building**, **In Acceptance** (merged, waiting for the
    owner's checklist), **Released** (accepted on a real deployment, with
    evidence), **Killed**. Merged is never Released.
  - Task: **Todo**, **In Progress**, **Blocked**, **Done**.
  - Decision: **Open**, **Decided**.
  - Todo and Blocked are the base's words; **In Progress**, **Done**,
    **Open** and **Decided** are the planner's until checked against the
    base's lists, and are renamed here if the base uses other words.
- **PR**: the pull request(s) that delivered it.
- **Source**: where the item comes from, so it can be checked. "PRODUCT §2"
  is the coverage table in `PRODUCT.md`.

**Waits for.** An item that cannot start until another merges says so in its
text (`AGENTS.md`). That is a dependency, not a status: the item stays
**Planned** or **Todo**, never **Blocked**. Blocked means started work that
is stopped, with the reason and who can unblock it. Specs carry the size
estimate (S, M, L), which the base has no field for.

## Now (P0–P1)

| ID | Kind | Item | Module | Priority | Status | PR | Source |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `AI-18` | Feature | Copilot Chat: ask a question or give an instruction in plain language, and the assistant answers or acts with the console's own tools; risky actions wait for approval. Built before this backlog tracked PRs; never accepted | AI Copilot | P1 | In Acceptance |  | `PRODUCT.md` §2 (Home, History) |
| `SEC-22` | Task | Converting an uploaded workbook can exhaust the API's memory and stop the API for every user; the conversion must not be able to end the API process | Administration & Security | P0 | Todo |  | PR #76 review, BLOCKER 1 (merged without the fix) |
| `SEC-23` | Task | Adding a quality, classification or residency rule asked only for a login, while changing one asks for `governance:write`; adding now asks for it too, in the API, the assistant's tools and the console | Administration & Security | P1 | In Progress | #97 | `features/governance-rule-create.md` |
| `DATA-1` | Feature | Gold tables publish to open format automatically, as an option per table | Build | P1 | In Acceptance | #60, #63, #64 | `features/gold-publish-per-mart.md` |
| `QA-1` | Task | Accept each built feature with a checklist, starting with what a demo shows | Delivery | P1 | Todo |  | PRODUCT §4 blocker 1 |
| `SEC-1` | Task | Confirm the leaked key is rotated; decide on rewriting git history | Administration & Security | P1 | Todo |  | `SECURITY.md` |
| `REL-1` | Task | Protect the main branch | Delivery | P1 | Todo |  | `docs/CI.md` |
| `DOC-1` | Task | Archive or rewrite the old product documents that contradict the build | Docs | P1 | Todo |  | PRODUCT §6 |
| `GOV-2` | Decision | Decide which role may publish a table in open format | Governance | P1 | Open |  | PRODUCT §6 |
| `SEC-10` | Task | Alert webhooks can be pointed at internal addresses; use the allowlisted resolver | Administration & Security | P1 | Todo |  | `lakehouse-notify` |
| `SEC-11` | Task | Dashboard tiles and public/embed links return raw database error text. Same rule as `SEC-6` | Administration & Security | P1 | Todo |  | `routes/support.rs` |
| `SEC-14` | Feature | A stored connector password can be stolen by re-pointing the connector: a host change keeps the secret and the next test sends it to the new server | Administration & Security | P0 | Building | #85 | `routes/connectors.rs`, `connector_probe.rs` |
| `SEC-15` | Feature | Connection tests can reach internal addresses: the block is off by default in compose, the REST test follows redirects, no address pinning, CDC delete skips the check | Administration & Security | P0 | In Acceptance | #82 | `docker-compose.yml`, `connector_probe.rs` |
| `SEC-13` | Task | Check `.env.staging`, pushed straight to `main` in `7ec3a81`, for real credentials or hostnames; rotate anything real | Administration & Security | P1 | Todo |  | commit `7ec3a81` |

## Next (P2)

| ID | Kind | Item | Module | Priority | Status | PR | Source |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `RPT-1` | Feature | Scheduled reports with PDF and spreadsheet attachments | Dashboards | P2 | Planned |  | PRODUCT §2 |
| `BI-1` | Feature | Export a dashboard as a server-made PDF and as Excel | Dashboards | P2 | Planned |  | PRODUCT §2 |
| `BI-4` | Feature | Result caching for charts and queries; run a dashboard's tiles in parallel | Dashboards | P2 | Planned |  | `reference/competitive-comparison.md` (BI matrix) |
| `BI-5` | Feature | Configurable query timeouts and row limits; Stop cancels the query on the server | Dashboards | P2 | Planned |  | `reference/competitive-comparison.md` (BI matrix) |
| `BI-6` | Feature | A chart builder as capable as Metabase's and Tableau's: several dimensions, filters, sorting, multi-step questions. No drag and drop; the assistant covers that | Dashboards | P2 | Planned |  | `reference/competitive-comparison.md` (BI matrix) |
| `BI-7` | Feature | Joins in the chart builder without writing SQL | Dashboards | P2 | Planned |  | `reference/competitive-comparison.md` (BI matrix) |
| `BI-8` | Feature | Calculated fields and custom expressions: maths, text, dates, conditions, running totals | Dashboards | P2 | Planned |  | `reference/competitive-comparison.md` (BI matrix) |
| `BI-9` | Feature | Group by day, week, month, quarter, year | Dashboards | P2 | Planned |  | `reference/competitive-comparison.md` (BI matrix) |
| `BI-10` | Feature | Bins, groups and hierarchies | Dashboards | P2 | Planned |  | `reference/competitive-comparison.md` (BI matrix) |
| `BI-11` | Feature | Parameters, usable in charts, SQL and dashboards | Dashboards | P2 | Planned |  | `reference/competitive-comparison.md` (BI matrix) |
| `BI-13` | Feature | Combine two sources on one chart | Dashboards | P2 | Planned |  | `reference/competitive-comparison.md` (BI matrix) |
| `BI-14` | Feature | Lineage from a chart to its sources, reachable from the dashboard | Dashboards | P2 | Planned |  | `reference/competitive-comparison.md` (BI matrix) |
| `BI-16` | Feature | Every chart type Tableau and Metabase have that we lack: progress bar, histogram, Gantt, bullet graph, KPI with comparison and sparkline, raw-row table, pivot table with totals, record detail, world maps and uploaded boundaries, image and web-page cards, spatial analysis, custom chart plugins | Dashboards | P2 | Planned |  | `reference/competitive-comparison.md` (BI matrix) |
| `BI-17` | Feature | Chart formatting at both competitors' level: number formats, colours, axes, data labels, conditional formatting, goal and reference lines, trend lines, forecasting, clustering, annotations, custom tooltips, themes and fonts | Dashboards | P2 | Planned |  | `reference/competitive-comparison.md` (BI matrix) |
| `BI-18` | Feature | Dashboards at both competitors' level: tabs; phone and tablet layouts; date-range, relative-date, number, text and linked filters; filter defaults that do not change silently; drill-down for every chart; click to another dashboard or URL; show and hide; saved auto-refresh; stories; templates; favourites; version history; trash; personal space | Dashboards | P2 | Planned |  | `reference/competitive-comparison.md` (BI matrix) |
| `AI-1` | Feature | The assistant manages the dashboards that exist today: rename boards, delete them (with approval), arrange and resize tiles, move charts between boards | AI Copilot | P2 | Planned |  | `AGENTS.md` (assistant parity) |
| `AI-2` | Feature | **Waits for `BI-18`**. The assistant adds and changes dashboard filters ("only 2025, only Bali") | AI Copilot | P2 | Planned |  | `AGENTS.md` (assistant parity) |
| `AI-3` | Feature | **Waits for `BI-9`**. The assistant picks the time grain ("monthly") | AI Copilot | P2 | Planned |  | `AGENTS.md` (assistant parity) |
| `AI-4` | Feature | **Waits for `BI-8`**. The assistant writes calculated fields in the product's formula language, not raw SQL | AI Copilot | P2 | Planned |  | `AGENTS.md` (assistant parity) |
| `AI-5` | Feature | **Waits for `BI-16`**, per chart type as it lands. The assistant creates every new chart type: raw and pivot tables, KPI with comparison, histogram, Gantt and the rest | AI Copilot | P2 | Planned |  | `AGENTS.md` (assistant parity) |
| `AI-6` | Feature | **Waits for `BI-6`, `BI-7`, `BI-10`, `BI-11`, `BI-13`**, each part with its BI item. The assistant builds charts with several dimensions and filters, joins tables, uses bins, groups, hierarchies and parameters, and combines two sources | AI Copilot | P2 | Planned |  | `AGENTS.md` (assistant parity) |
| `AI-7` | Feature | **Waits for `BI-2`**. The assistant uses named metrics before it writes SQL, so the same question gives the same number | AI Copilot | P2 | Planned |  | `AGENTS.md` (assistant parity) |
| `AI-8` | Feature | **Waits for `BI-17`**. The assistant changes number formats, colours, axes and labels, and adds trend and forecast lines | AI Copilot | P2 | Planned |  | `AGENTS.md` (assistant parity) |
| `AI-9` | Feature | **Waits for `BI-18`** (layout part). The assistant adds tabs and uses templates | AI Copilot | P2 | Planned |  | `AGENTS.md` (assistant parity) |
| `AI-10` | Feature | **Waits for `RPT-1`, `BI-1`**. The assistant schedules reports ("send this to me every Monday as PDF") and produces a PDF or Excel file on request | AI Copilot | P2 | Planned |  | `AGENTS.md` (assistant parity) |
| `AI-11` | Feature | **Waits for `BI-19`, `BI-26`, `DEC-8`**. The assistant creates share and embed links only with approval, respects dashboard permissions, and never grants permissions | AI Copilot | P2 | Planned |  | `AGENTS.md` (assistant parity) |
| `AI-12` | Feature | The assistant fixes and explains SQL inside the editor; variables **Waits for `QS-5`** | AI Copilot | P2 | Planned |  | `AGENTS.md` (assistant parity) |
| `AI-13` | Feature | **Waits for `BI-14`**. The assistant answers "where does this number come from" from the chart's lineage (`get_lineage` exists today) | AI Copilot | P2 | Planned |  | `AGENTS.md` (assistant parity) |
| `AI-14` | Feature | Ongoing, with every BI item. What the dashboard page sends to the assistant grows with the features: filters, tabs, metrics, formats | AI Copilot | P2 | Planned |  | `AGENTS.md` (assistant parity) |
| `AI-15` | Task | A standard set of about 30 typical dashboard requests, re-run whenever an assistant tool changes, so the assistant does not silently get worse | AI Copilot | P2 | Todo |  | `AGENTS.md` (assistant parity) |
| `BI-26` | Feature | Better embedding, without overdoing it: a single-chart embed that really shows only that chart, light/dark theme and our logo optional, filters passed through the link or token, sizing that fits the host page. Token hardening is `SEC-12`. No embedding SDK and no editing inside embeds | Dashboards | P2 | Planned |  | `reference/competitive-comparison.md` (BI matrix) |
| `BI-19` | Feature | Sharing and delivery, except Slack, Teams, Office and Google integrations: public link expiry and password; permissions per dashboard and folder; comments; a schedule per digest; alerts with filters that fire once; PNG export; import of exported dashboards. Scheduled reports are `RPT-1`, PDF and Excel export `BI-1`, embedding `BI-26` | Dashboards | P2 | Planned |  | `reference/competitive-comparison.md` (BI matrix) |
| `BLD-1` | Feature | Joins and aggregations in the pipeline builder | Build | P2 | Idea |  | PRODUCT §2 |
| `VER-1` | Task | Verify SFTP, Google Sheets and Oracle sources end to end; test row filters. The connector half is now `SRC-9`; row filters stay here | Data | P2 | Todo |  | PRODUCT §2 |
| `BI-2` | Feature | Metrics users can define once and reuse across dashboards (a semantic layer) | Dashboards | P2 | Planned |  | PRODUCT §2; `reference/competitive-comparison.md` (BI matrix) |
| `DATA-9` | Feature | Upload a file from the console | Data | P2 | In Acceptance | #71, #73 | `features/upload-file.md` |
| `OPS-8` | Feature | Usage and cost view | Operations | P2 | Idea |  | PRODUCT §2 |
| `SEC-2` | Feature | Login rate limiting | Administration & Security | P2 | In Acceptance | #67 | `features/login-protection-and-session-cleanup.md` |
| `SEC-3` | Task | Test single sign-on against a real identity provider; correct `README.md`, which says it is unbuilt | Administration & Security | P2 | Todo |  | PRODUCT §2 |
| `SEC-7` | Task | A monitored security contact address | Administration & Security | P2 | Todo |  | `SECURITY.md` |
| `REL-2` | Task | Adopt a version and release policy; cut a release from the unreleased work | Delivery | P2 | Todo |  | `reference/release-policy.md` |
| `GOV-1` | Feature | Audit trail covers every console change | Governance | P2 | Idea |  | PRODUCT §3 |
| `OPS-1` | Task | Trim old versions of raw tables | Operations | P2 | Todo |  | `README.md` |
| `DATA-2` | Task | Trim old copies in published Gold tables | Build | P2 | Todo |  | `features/gold-publish-per-mart.md` |
| `DATA-10` | Task | A merge-proof "has this mart changed" signal for publishing, so background merges do not trigger extra copies | Build | P2 | Todo |  | Slice B review, gold-publish plan |
| `DATA-6` | Feature | A "latest copy only" view for outside readers of published tables | Build | P2 | Idea |  | `features/gold-publish-per-mart.md` |
| `SEC-4` | Task | Human security review | Administration & Security | P2 | Todo |  | PRODUCT §4 blocker 3 |
| `SUP-1` | Decision | Decide a support commitment | Business | P2 | Open |  | PRODUCT §4 blocker 5 |
| `OPS-7` | Task | Test and time a restore from backup | Operations | P2 | Todo |  | PRODUCT §2 |
| `SEC-5` | Feature | Clean up old sessions and tokens | Administration & Security | P2 | In Acceptance | #67 | `features/login-protection-and-session-cleanup.md` |
| `SRC-6` | Feature | Fix the broken connectors: object-storage test and key change, Oracle credential change, Sheets shown as unsupported, Sources header text | Data | P2 | Planned |  | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-7` | Feature | Alerts when a connector run or an upload load fails; health from real runs | Data | P2 | Planned |  | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-8` | Feature | Schema changes at the source: detect, notify, and a per-connector choice to apply, ask or pause | Data | P2 | Planned |  | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-9` | Task | Gate tests for every advertised connector (Oracle, SFTP, MySQL CDC, SQL Server CDC); the upload gate in CI | Data | P2 | Todo |  | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-10` | Feature | More load modes: deduplicate on a key, keep history (SCD type 2), refresh or clear one table | Data | P2 | Planned |  | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-11` | Feature | Connector operations: automatic retries with backoff, a log view per run, column choice | Data | P2 | Planned |  | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-12` | Feature | Credentials done properly: external secret managers, encryption at rest, OAuth sign-in | Data | P2 | Planned |  | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-13` | Feature | Table discovery for every source type, not only databases | Data | P2 | Planned |  | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-4` | Feature | More databases, with live change capture: Oracle tested end to end, MariaDB and Teradata added; live change capture for PostgreSQL, MySQL, SQL Server, Oracle and MongoDB, each gate-tested. Cloud warehouses and enterprise databases excluded (product owner, 2026-10-07) | Data | P2 | Planned |  | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-11` | Feature | Catalog search that finds columns and tags, with tolerant matching. The certification filter waits for `DATA-12` | Data | P2 | In Acceptance | #98 | `features/catalog-search.md` |
| `DATA-20` | Feature | Hide single tables from single users in the catalog and in search. Today whoever may open the catalog sees every table in it (found planning `DATA-11`, decision 5) | Data | P2 | Idea |  | `features/catalog-search.md` |
| `DATA-12` | Feature | Certified and deprecated marks; governed tags with allowed values | Data | P2 | Planned |  | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-13` | Feature | Column-level lineage. Also check the lineage gate, which by reading does not accept the `build` link kind the API emits (`specs/data-13.md`) | Data | P2 | Planned |  | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-14` | Feature | Data quality in depth: a fuller library of ready checks (four exist), freshness and volume anomaly detection, incidents and their overview, and alerts on a failed check | Data | P2 | Planned |  | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-15` | Feature | Asset page depth: 1,000-row preview, 30-day usage, grant and revoke, restore a dropped table | Data | P2 | Planned |  | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-16` | Task | Time travel proven safe: a gate test that past-version queries pass the masking rewrite. By reading the code, the Trino form of a past-version query is refused before it reaches Trino, so this is also a fix (`specs/data-16.md`) | Data | P2 | Todo |  | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-17` | Feature | Upload more formats (Excel, JSON, Parquet, Avro), up to 10 files at once in parallel, up to 2 GB per upload once measured (Databricks' limits) | Data | P2 | Building | #76 | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-18` | Feature | Column types on upload, with names and types editable before loading | Data | P2 | Planned |  | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-19` | Feature | Uploaded tables can feed pipelines and so reach dashboards | Data | P2 | Planned |  | `reference/competitive-comparison.md` (Data matrix) |
| `QS-1` | Feature | Query Studio editor basics: column autocomplete, schema browser, several statements, tabs, formatting rules | Query Studio | P2 | Planned |  | `reference/competitive-comparison.md` (Query Studio matrix) |
| `QS-2` | Feature | Query Studio results: up to 64,000 rows or 10 MB shown, grid filters and column stats, full server downloads up to 5 GB (Databricks' limits), server-side Stop, configurable timeouts | Query Studio | P2 | Planned |  | `reference/competitive-comparison.md` (Query Studio matrix) |
| `QS-3` | Feature | Saved queries done properly: edit, delete, folders, sharing levels, version history. **Waits for `SEC-20`** | Query Studio | P2 | Planned |  | `reference/competitive-comparison.md` (Query Studio matrix) |
| `QS-4` | Feature | Query history page with filters, and a query profile | Query Studio | P2 | Planned |  | `reference/competitive-comparison.md` (Query Studio matrix) |
| `QS-5` | Feature | The bridge to BI: parameters that become dashboard filters, snippets, charts from results, reuse a saved query in SQL, alerts on a query | Query Studio | P2 | Planned |  | `reference/competitive-comparison.md` (Query Studio matrix) |
| `AI-16` | Task | Hand-off to the AI team: AI features for the Data module. Remaining: plain-language table search, AI classification, AI quality suggestions and incident help, AI connector building. The AI team plans these | AI Copilot | P2 | In Progress | #79 | `docs/core/features/semantic-layer.md` |
| `AI-17` | Feature | The chat asks once, with options, which reading of an unclear word the person means, remembers the clicked answer per user, and writes in full only the tables a question names | AI Copilot | P2 | In Acceptance | #81 | `features/chat-ask-back.md` |
| `SEC-6` | Task | Stop older API handlers from returning internal error text | Administration & Security | P2 | Todo |  | `AGENTS.md` |
| `SEC-16` | Task | Cross-tenant leaks in the Data module: the ingestible-connectors route, one shared upload namespace, a message naming another tenant's upload table, annotation edits without the tenant gate | Administration & Security | P2 | Todo |  | Data module code audit |
| `SEC-17` | Task | Upload hardening: a column cap, a limit on concurrent uploads, CSV exports that neutralise formula cells | Administration & Security | P2 | Todo |  | Data module code audit |
| `SEC-18` | Task | No default credentials in compose (`lakehouse`, `rustfsadmin`); must-set secrets use `${X:?}` (rule 5) | Administration & Security | P2 | Todo |  | `docker-compose.yml` |
| `SEC-19` | Task | The query cost estimate runs raw SQL without the read-only check or table-function block, and returns database error text | Administration & Security | P2 | Todo |  | `routes/query.rs` |
| `SEC-20` | Task | Queries are not scoped to the tenant: no tenant gate on query runs; saved queries listed for everyone | Administration & Security | P2 | Todo |  | `routes/query.rs` |
| `SEC-21` | Task | Downloads re-run SQL rewritten for an older policy; whole results buffered before the 2,000-row cut | Administration & Security | P2 | Todo |  | `routes/query.rs` |
| `SEC-12` | Feature | Embed tokens: require an expiry, allow revocation, keep the signing secret out of plain text, set a frame-ancestors policy | Administration & Security | P2 | Planned |  | `lakehouse-embed` |

## Later (P3)

| ID | Kind | Item | Module | Priority | Status | PR | Source |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `OPS-3` | Feature | Run on several servers; high availability | Platform | P3 | Idea |  | PRODUCT §1, §2 |
| `SRC-3` | Feature | Connectors for business apps: first Databricks' ten generally available SaaS connectors, then its beta list by customer demand. **Waits for `DEC-9`** | Data | P3 | Planned |  | PRODUCT §2 |
| `DATA-8` | Feature | Instant copies of a table for testing | Data | P3 | Idea |  | PRODUCT §2 |
| `BLD-2` | Feature | Tables that refresh themselves when their inputs change | Build | P3 | Idea |  | PRODUCT §2 |
| `SRC-5` | Feature | Real connection test for MongoDB, Kafka, SFTP, Sheets and Oracle sources | Data | P3 | Idea |  | `connector_probe.rs` |
| `DOC-2` | Task | Step-by-step user guide checked against a running console | Docs | P3 | Todo |  | `reference/user-guide.md` |
| `DATA-3` | Feature | Store Gold in open format first; serve from a fast copy | Build | P3 | Idea |  | ADR 0010 |
| `DATA-4` | Feature | Publish Silver in open format | Build | P3 | Idea |  | `features/gold-publish-per-mart.md` |
| `DATA-5` | Task | Measure outside readers of published tables | Build | P3 | Todo |  | `routes::gold::consumers` |
| `BI-3` | Feature | Import dashboards from Tableau and Power BI | Dashboards | P3 | Idea |  | PRODUCT §2 |
| `BI-20` | Feature | The rest of the BI matrix, to be considered later: field metadata in the BI layer, certified content, BI-specific AI, admin and platform rows, and Slack, Teams, Office and Google integrations | Dashboards | P3 | Idea |  | `reference/competitive-comparison.md` (BI matrix) |
| `QRY-1` | Feature | Query other systems in place, without loading | Data | P3 | Idea |  | PRODUCT §2 |
| `SRC-2` | Feature | Processing over streams | Data | P3 | Idea |  | PRODUCT §2 |
| `INT-2` | Feature | Search over documents | AI Copilot | P3 | Idea |  | PRODUCT §2 |
| `GOV-3` | Feature | Lineage that reaches outside systems | Governance | P3 | Idea |  | PRODUCT §2 |
| `ADM-1` | Feature | Several organisations in one install | Administration & Security | P3 | Idea |  | PRODUCT §2 |
| `ADM-2` | Feature | Workspace settings that can be changed | Administration & Security | P3 | Idea |  | `README.md` |
| `OPS-2` | Feature | Install with no internet access | Platform | P3 | Idea |  | Sales playbook (older; re-check) |
| `OPS-4` | Task | Fail loudly when the application database is unreachable | Platform | P3 | Todo |  | `docs/OPERATIONS.md` |
| `OPS-6` | Feature | Log and trace viewer | Operations | P3 | Idea |  | `docs/FEATURE_COVERAGE.md` |
| `BIZ-1` | Feature | Enterprise edition | Business | P3 | Idea |  | PRODUCT §1 |
| `BIZ-2` | Feature | Hosted cloud version | Business | P3 | Idea |  | PRODUCT §1 |

## Dates

Started is the date of the item's plan; Shipped is the merge date of the PR
that delivered it; both come from the repository's history. **Due** dates are
set by the product owner and are left empty until they are.

| ID | Started | Shipped | Due |
| --- | --- | --- | --- |
| `DATA-1` | 2026-10-02 | 2026-10-03 | |
| `SEC-2` | 2026-10-02 | 2026-10-03 | |
| `SEC-5` | 2026-10-02 | 2026-10-03 | |
| `DATA-9` | 2026-10-02 | 2026-10-07 | |
| `DATA-17` | 2026-10-07 | partly, 2026-10-08 (Excel and Parquet) | |
| `AI-16` | | first line, 2026-10-08 | |
| `AI-17` | | 2026-10-08 | |
| `SEC-14` | 2026-10-08 | first PR, 2026-10-08 | |
| `SEC-15` | 2026-10-08 | 2026-10-08 | |
| `DATA-11` | 2026-10-09 | 2026-10-09 | |
| `SEC-23` | 2026-10-09 | | |

## Decide

Decisions the product owner must make. Each is a base **Decision** record:
the options, the recommendation, and who decides (the product owner through-
out). `GOV-2` and `SUP-1` are also decisions, but carry a priority and sit in
the lists above. A decision's outcome, once made, moves to **Done and
rejected** with the date.

| ID | Decision | Options | Recommendation | Decider |
| --- | --- | --- | --- | --- |
| `DEC-1` | Notebooks and Python | Build · Defer · No | Defer | Product owner |
| `DEC-2` | dbt support | Build · Defer · No | Defer | Product owner |
| `DEC-3` | AI functions inside SQL | Build · Defer · No | Defer | Product owner |
| `DEC-4` | Train and serve models | Build · No | No | Product owner |
| `DEC-5` | Mobile app | Build · No | No | Product owner |
| `DEC-6` | Share data with another organisation | Build · Defer · No | Defer | Product owner |
| `DEC-8` | Which assistant actions need human approval before they run (today: deletions and pauses) | Add: make a dashboard public or embeddable · change who can see a dashboard · email outside the company · delete a dashboard — each on or off | Add all four | Product owner |
| `DEC-9` | How to get SaaS connectors | Reuse dlt verified sources (our loader already runs on dlt) · adopt another connector library · build each by hand | Reuse dlt verified sources first | Product owner |
| `DEC-10` | Data and Query Studio edge cases: entity-relationship diagram, a no-code connector builder, business glossary and domains, table favourites, real-time query co-editing, Git for queries | Drop · Defer · Build, each of the six | Drop or defer all six | Product owner |
| `INT-1` | Digital employees | Build · Park | Parked (product owner, 2026-10-02) | Product owner |

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
| `DEC-11` | Which of the repo and the Lark base is the master | Decided: the repo, for everything it holds (ID, kind, text, module, priority, status, PR, dates, QA results); the base mirrors it and adds the people | 2026-10-09 |
| `DEC-12` | Name staff as Owner and Acceptor in the public repo | Decided: no. The repo names the module; the base holds the person through the module's owner | 2026-10-09 |
| `DEC-13` | Query Studio as a module in the base | Decided: yes, its own module | 2026-10-09 |
| `DEC-14` | Priority of `SEC-14` and `SEC-15` | Decided: P0 | 2026-10-09 |
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
| [`SEC-11`](specs/sec-11.md) | No raw database errors on screen | Now | S |
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
| [`SEC-23`](specs/sec-23.md) | Adding a governance rule needs the permission changing one needs | Now | S |

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
