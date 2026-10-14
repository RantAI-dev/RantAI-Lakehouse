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
| `SEC-22` | Task | Converting an uploaded workbook can exhaust the API's memory and stop the API for every user; the conversion must not be able to end the API process. Also: the Parquet reader trusts sizes from the file's footer the same way (by reading, not run); no limit on conversions running at once; no byte cap on workbook text; each preview converts the whole file again; the limits doc in `upload_workbook.rs` says the opposite of the code | Administration & Security | P0 | Todo |  | PR #76 review, BLOCKER 1 (merged without the fix); `superpowers/plans/2026-10-12-delivery-review.md` |
| `SEC-23` | Task | Adding a quality, classification or residency rule asked only for a login, while changing one asks for `governance:write`; adding now asks for it too, in the API, the assistant's tools and the console | Administration & Security | P1 | Done | #97 | `features/governance-rule-create.md` |
| `DATA-1` | Feature | Gold tables publish to open format automatically, as an option per table | Build | P1 | In Acceptance | #60, #63, #64 | `features/gold-publish-per-mart.md` |
| `QA-1` | Task | Accept each built feature with a checklist, starting with what a demo shows | Delivery | P1 | Todo |  | PRODUCT §4 blocker 1 |
| `SEC-1` | Task | Confirm the leaked key is rotated; decide on rewriting git history | Administration & Security | P1 | Todo |  | `SECURITY.md` |
| `REL-1` | Task | Protect the main branch | Delivery | P1 | Todo |  | `docs/CI.md` |
| `DOC-1` | Task | Archive or rewrite the old product documents that contradict the build | Docs | P1 | Todo |  | PRODUCT §6 |
| `GOV-2` | Decision | Decide which role may publish a table in open format | Governance | P1 | Open |  | PRODUCT §6 |
| `SEC-10` | Task | Alert webhooks can be pointed at internal addresses; use the allowlisted resolver. The `SRC-17` gate (`ops/g6/src17_alert_delivery_test.py`) receives webhooks on its own network: this item gives that gate its documented opt-in | Administration & Security | P1 | Todo |  | `lakehouse-notify` |
| `SEC-11` | Task | Dashboard tiles and public/embed links return raw database error text. Same rule as `SEC-6` | Administration & Security | P1 | Todo |  | `routes/support.rs` |
| `SEC-14` | Feature | A stored connector password can be stolen by re-pointing the connector: a host change keeps the secret and the next test sends it to the new server | Administration & Security | P0 | Building | #85 | `routes/connectors.rs`, `connector_probe.rs` |
| `SEC-15` | Feature | Connection tests can reach internal addresses: the block is off by default in compose, the REST test follows redirects, no address pinning, CDC delete skips the check | Administration & Security | P0 | In Acceptance | #82 | `docker-compose.yml`, `connector_probe.rs` |
| `SEC-13` | Task | Check `.env.staging`, pushed straight to `main` in `7ec3a81`, for real credentials or hostnames; rotate anything real | Administration & Security | P1 | Todo |  | commit `7ec3a81` |
| `SEC-24` | Task | The assistant's system prompt gives any signed-in user the DATA MAP (tables, row counts, sample values) without `catalog:read`; a Dashboard Viewer sees samples Query Studio would refuse it. Give the map only with `catalog:read`, and sample values only with `query:read` | Administration & Security | P1 | Todo |  | `superpowers/plans/2026-10-12-delivery-review.md` (`SEC-9` gap) |
| `BLD-3` | Task | A pipeline built in the console cannot read a Bronze table, so Bronze to Silver does not run: the job reads its source as the ClickHouse table `<sourceZone>.<sourceTable>`, and ClickHouse has no `bronze` database; Bronze is Iceberg, readable only through a `DataLakeCatalog` database (`icecat_api`, tables named `bronze.<table>`). The console and the assistant still offer Bronze as a source. Read the source through the job's own `DataLakeCatalog` database, as `maintenance.py` does, with masking and governance still applied; prove it on a running stack, which needs `SRC-20`. Found by reading the code after a user report, not yet reproduced | Build | P1 | Todo |  | `authored_factory.py` (`_op_for_pipeline`), `scripts/seed-clickhouse.sql` |
| `SEC-28` | Task | A pipeline built in the console can be read, edited, deleted, restored, run and paused by its ID from another organisation: the per-ID pipeline routes and the run routes do not check the caller's tenant. Also: run settings checked by top-level key only; a restored version is not validated; orchestrator error text in some responses. Fixed on the branch of PR #80, written before this backlog tracked it | Administration & Security | P1 | In Progress | #80 | PR #59 review (F2); PR #80 review |
| `SRC-16` | Task | A connector loaded after the fact (files, REST, MongoDB, Kafka, SFTP) on the "pause" policy stays paused for good after its first change: nothing waits for approval, so nothing can lift it. Also a pending change that Approve refuses keeps the connector paused after every other change is approved | Data | P1 | Done | #120 | `superpowers/plans/2026-10-12-delivery-review.md` (`SRC-8` BLOCKER) |

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
| `AI-12` | Feature | The assistant fixes and explains SQL inside the editor; variables **Waits for `QS-5`** | AI Copilot | P2 | Building | #127 | `AGENTS.md` (assistant parity) |
| `AI-13` | Feature | **Waits for `BI-14`**. The assistant answers "where does this number come from" from the chart's lineage (`get_lineage` exists today) | AI Copilot | P2 | Planned |  | `AGENTS.md` (assistant parity) |
| `AI-14` | Feature | Ongoing, with every BI item. What the dashboard page sends to the assistant grows with the features: filters, tabs, metrics, formats | AI Copilot | P2 | Planned |  | `AGENTS.md` (assistant parity) |
| `AI-15` | Task | A standard set of about 30 typical dashboard requests, re-run whenever an assistant tool changes, so the assistant does not silently get worse | AI Copilot | P2 | Todo |  | `AGENTS.md` (assistant parity) |
| `BI-26` | Feature | Better embedding, without overdoing it: a single-chart embed that really shows only that chart, light/dark theme and our logo optional, filters passed through the link or token, sizing that fits the host page. Token hardening is `SEC-12`. No embedding SDK and no editing inside embeds | Dashboards | P2 | Planned |  | `reference/competitive-comparison.md` (BI matrix) |
| `BI-19` | Feature | Sharing and delivery, except Slack, Teams, Office and Google integrations: public link expiry and password; permissions per dashboard and folder; comments; a schedule per digest; alerts with filters that fire once; PNG export; import of exported dashboards. Scheduled reports are `RPT-1`, PDF and Excel export `BI-1`, embedding `BI-26` | Dashboards | P2 | Planned |  | `reference/competitive-comparison.md` (BI matrix) |
| `BLD-1` | Feature | Joins, aggregations and a SQL step in the pipeline builder, and the same through the assistant. **Waits for `BLD-3`** | Build | P2 | Planned |  | PRODUCT §2; [spec](specs/bld-1.md) |
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
| `DATA-2` | Task | Keep the last copies of a published Gold table, remove the rest. **Waits for `DATA-10`** | Build | P2 | Todo |  | `features/gold-publish-per-mart.md`; [spec](specs/data-2.md) |
| `DATA-10` | Task | A merge-proof "has this mart changed" signal for publishing, so background merges do not trigger extra copies | Build | P2 | Todo |  | Slice B review, gold-publish plan |
| `DATA-6` | Feature | A "latest copy only" table for outside readers of published tables. **Waits for `DATA-2`** | Build | P2 | Planned |  | `features/gold-publish-per-mart.md`; [spec](specs/data-6.md) |
| `SEC-4` | Task | Human security review | Administration & Security | P2 | Todo |  | PRODUCT §4 blocker 3 |
| `SUP-1` | Decision | Decide a support commitment | Business | P2 | Open |  | PRODUCT §4 blocker 5 |
| `OPS-7` | Task | Test and time a restore from backup | Operations | P2 | Todo |  | PRODUCT §2 |
| `SEC-5` | Feature | Clean up old sessions and tokens | Administration & Security | P2 | In Acceptance | #67 | `features/login-protection-and-session-cleanup.md` |
| `SRC-6` | Feature | Fix the broken connectors: object-storage test and key change, Oracle credential change, Sheets shown as unsupported, Sources header text | Data | P2 | In Acceptance | #101 | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-7` | Feature | Alerts when a connector run or an upload load fails; health from real runs | Data | P2 | In Acceptance | #101 | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-8` | Feature | Schema changes at the source: detect, notify, and a per-connector choice to apply, ask or pause | Data | P2 | In Acceptance | #101 | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-9` | Task | Gate tests for every advertised connector (Oracle, SFTP, MySQL CDC, SQL Server CDC); the upload gate in CI | Data | P2 | Done | #114 | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-10` | Feature | More load modes: deduplicate on a key, keep history (SCD type 2), refresh or clear one table | Data | P2 | In Acceptance | #115 | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-11` | Feature | Connector operations: automatic retries with backoff, a log view per run, column choice | Data | P2 | Planned |  | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-12` | Feature | Credentials done properly: external secret managers, encryption at rest, OAuth sign-in | Data | P2 | Planned |  | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-13` | Feature | Table discovery for every source type, not only databases | Data | P2 | Planned |  | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-15` | Feature | Load a SQL column whose type changed to one its Bronze column cannot hold (integer to text, text to a number, a changed decimal) into a second column when the change is approved. Today such a change cannot be approved and the table waits (`SRC-8`, decision 10). Starts after the g6 gate has shown the loader's behaviour on a real database. No spec yet | Data | P3 | Idea |  | `features/source-schema-changes.md` |
| `SRC-4` | Feature | More databases, with live change capture: Oracle tested end to end, MariaDB and Teradata added; live change capture for PostgreSQL, MySQL, SQL Server, Oracle and MongoDB, each gate-tested. Cloud warehouses and enterprise databases excluded (product owner, 2026-10-07) | Data | P2 | Planned |  | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-11` | Feature | Catalog search that finds columns and tags, with tolerant matching. The certification filter waits for `DATA-12` | Data | P2 | In Acceptance | #98 | `features/catalog-search.md` |
| `DATA-20` | Feature | Hide single tables from single users in the catalog and in search. Today whoever may open the catalog sees every table in it (found planning `DATA-11`, decision 5) | Data | P2 | Idea |  | `features/catalog-search.md` |
| `DATA-12` | Feature | Certified and deprecated marks; governed tags with allowed values | Data | P2 | Planned |  | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-13` | Feature | Column-level lineage. Also check the lineage gate, which by reading does not accept the `build` link kind the API emits (`specs/data-13.md`) | Data | P2 | Planned |  | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-14` | Feature | Data quality in depth: a fuller library of ready checks (four exist), freshness and volume anomaly detection, incidents and their overview, and alerts on a failed check | Data | P2 | Planned |  | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-15` | Feature | Asset page depth: 1,000-row preview, 30-day usage, grant and revoke, restore a dropped table | Data | P2 | Planned |  | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-16` | Task | Time travel proven safe on ClickHouse: a gate test that a past-version query keeps masking and row filters, a fix so a role named by a policy can read the policy's raw table at all, a version picker that follows the engine, and a plain refusal on Trino (`specs/data-16.md`) | Data | P2 | Done | #100 | `features/time-travel.md` |
| `DATA-21` | Task | Past versions and masking on Trino: the masking rewrite writes ClickHouse functions for both engines and cannot read Trino's version clause, so on Trino a past version is refused and, by reading, a masked table cannot be read at all. Needs the rewrite made engine-aware and a gate with Trino. Split from `DATA-16` by the product owner (2026-10-09) | Data | P2 | Todo |  | `features/time-travel.md` |
| `DATA-25` | Task | Time travel follow-ups from the `DATA-16` review: a past version that does not exist reaches the Query Studio response as ClickHouse's own text (principle 4); the Trino refusal (422) for a past-version query has no route-level test | Data | P2 | Todo |  | PR #100 review |
| `DATA-17` | Feature | Upload more formats (Excel, JSON, Parquet, Avro), up to 10 files at once in parallel, up to 2 GB per upload once measured (Databricks' limits) | Data | P2 | Building | #76 | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-18` | Feature | Column types on upload, with names and types editable before loading | Data | P2 | Planned |  | `reference/competitive-comparison.md` (Data matrix) |
| `DATA-19` | Feature | Uploaded tables can feed pipelines and so reach dashboards. **Waits for `BLD-3`** | Data | P2 | Planned |  | `reference/competitive-comparison.md` (Data matrix) |
| `QS-1` | Feature | Query Studio editor basics: column autocomplete, schema browser, several statements, tabs, formatting rules | Query Studio | P2 | Planned |  | `reference/competitive-comparison.md` (Query Studio matrix) |
| `QS-2` | Feature | Query Studio results: up to 64,000 rows or 10 MB shown, grid filters and column stats, full server downloads up to 5 GB (Databricks' limits), server-side Stop, configurable timeouts | Query Studio | P2 | Planned |  | `reference/competitive-comparison.md` (Query Studio matrix) |
| `QS-3` | Feature | Saved queries done properly: edit, delete, folders, sharing levels, version history. **Waits for `SEC-20`** | Query Studio | P2 | Planned |  | `reference/competitive-comparison.md` (Query Studio matrix) |
| `QS-4` | Feature | Query history page with filters, and a query profile | Query Studio | P2 | Planned |  | `reference/competitive-comparison.md` (Query Studio matrix) |
| `QS-5` | Feature | The bridge to BI: parameters that become dashboard filters, snippets, charts from results, reuse a saved query in SQL, alerts on a query | Query Studio | P2 | Planned |  | `reference/competitive-comparison.md` (Query Studio matrix) |
| `AI-16` | Task | Hand-off to the AI team: AI features for the Data module. Remaining: plain-language table search, AI classification, AI quality suggestions and incident help, AI connector building. The AI team plans these | AI Copilot | P2 | In Progress | #79, #109, #110, #111, #112 | `docs/core/features/semantic-layer.md` |
| `AI-17` | Feature | The chat asks once, with options, which reading of an unclear word the person means, remembers the clicked answer per user, and writes in full only the tables a question names | AI Copilot | P2 | In Acceptance | #81 | `features/chat-ask-back.md` |
| `WEB-1` | Feature | Landing page at `/landing`: a 3D night lake with the logo's creature, and a scroll story through sources, the Bronze/Silver/Gold layers, the assistant, dashboards, governance and open source. Copy limited to what is built | Business | P2 | In Acceptance | #107 | `superpowers/plans/2026-10-11-landing-page.md` |
| `REL-3` | Task | `package-lock.json` is stale: it lacks packages `package.json` lists (`@dnd-kit/core` and others). Delete it (bun is the package manager; CI and the Dockerfile read `bun.lock`) or regenerate it, in a PR of its own | Delivery | P3 | Done | TBD | WEB-1 review round 1 |
| `REL-4` | Task | Move the console to TypeScript 7 (the new compiler); dependabot #136 fails typecheck and build. Dependabot ignores TypeScript majors until this is planned | Delivery | P3 | Todo |  | `superpowers/plans/2026-10-14-deps-triage.md` |
| `REL-5` | Task | Move to eslint 10 and its flat-config changes; dependabot #143 fails lint. Dependabot ignores eslint majors until this is planned | Delivery | P3 | Todo |  | `superpowers/plans/2026-10-14-deps-triage.md` |
| `REL-6` | Task | Move the data grids to TanStack Table 9; dependabot #133 fails typecheck. Dependabot ignores its majors until this is planned | Delivery | P3 | Todo |  | `superpowers/plans/2026-10-14-deps-triage.md` |
| `REL-7` | Task | arrow and parquet 60 with a newer `iceberg` crate: `iceberg = "=0.10.1"` still uses arrow 58, so #139, #140 and #142 fail to compile. **Waits for** an `iceberg` release on arrow 60; dependabot now groups the three | Delivery | P3 | Todo |  | `superpowers/plans/2026-10-14-deps-triage.md` |
| `SEC-6` | Task | Stop older API handlers from returning internal error text | Administration & Security | P2 | Todo |  | `AGENTS.md` |
| `SEC-16` | Task | Cross-tenant leaks in the Data module: the ingestible-connectors route, one shared upload namespace, a message naming another tenant's upload table, annotation edits without the tenant gate | Administration & Security | P2 | Todo |  | Data module code audit |
| `SEC-17` | Task | Upload hardening: a column cap, a limit on concurrent uploads, CSV exports that neutralise formula cells | Administration & Security | P2 | Todo |  | Data module code audit |
| `SEC-18` | Task | No default credentials in compose (`lakehouse`, `rustfsadmin`); must-set secrets use `${X:?}` (rule 5) | Administration & Security | P2 | Todo |  | `docker-compose.yml` |
| `SEC-19` | Task | The query cost estimate runs raw SQL without the read-only check or table-function block, and returns database error text | Administration & Security | P2 | Todo |  | `routes/query.rs` |
| `SEC-20` | Task | Queries are not scoped to the tenant: no tenant gate on query runs; saved queries listed for everyone | Administration & Security | P2 | Todo |  | `routes/query.rs` |
| `SEC-21` | Task | Downloads re-run SQL rewritten for an older policy; whole results buffered before the 2,000-row cut | Administration & Security | P2 | Todo |  | `routes/query.rs` |
| `SEC-12` | Feature | Embed tokens: require an expiry, allow revocation, keep the signing secret out of plain text, set a frame-ancestors policy | Administration & Security | P2 | Planned |  | `lakehouse-embed` |
| `SEC-25` | Task | Connector security follow-ups: a connector that signs in with no credential (Kafka, auth `none`) can never be re-pointed (409 without a credential, 400 with one); the PATCH-`host` rule and its doc for `files` connectors are stale since `SRC-6`; the `SEC-15` refusal message has runs of spaces | Administration & Security | P2 | Todo |  | `superpowers/plans/2026-10-12-delivery-review.md` |
| `SEC-26` | Task | About thirty assistant tool results carry raw upstream error text (`err.to_string()`) into the chat response's `toolTrace`. Same rule as `SEC-6` | Administration & Security | P2 | Todo |  | `superpowers/plans/2026-10-12-delivery-review.md` |
| `SEC-27` | Task | The Security workflow is red on `main`: `cargo audit` reports advisories for `quinn-proto` 0.11.17 (`RUSTSEC-2026-0338`, `-0340`, `-0343`), and both `gitleaks` jobs fail. Update or waive the dependency with a reason; triage the `gitleaks` findings as `docs/CI.md` describes. Until then a pull request that touches dependencies cannot merge (`AGENTS.md`, step 6) | Administration & Security | P1 | In Progress | #121 | CI on `main` at `c083125` (2026-10-13); named in the `SRC-16` review |
| `SRC-17` | Task | Load-failure alert delivery: a failed delivery is recorded as if sent (feature-page row 5 cannot pass); a rule-store failure loses the alert because the dedupe row is written first; the Alerts page still says nothing raises `connector_schema_change`; a gate step that receives a webhook, and `SRC-7-RESULT.md` with measured seconds | Data | P2 | Done | #124 | `superpowers/plans/2026-10-12-delivery-review.md` (`SRC-7`) |
| `SRC-18` | Task | A paused connector still loads when a run was already queued or is launched from Dagster: only the due list and the run route check the pause. The schema-observation routes use `ingest:read` instead of their own permission | Data | P2 | Todo |  | `superpowers/plans/2026-10-12-delivery-review.md` (`SRC-8`) |
| `SRC-19` | Task | A wrong password for a PostgreSQL source is retried as if it were a passing fault: the driver gives no error code to tell it apart, so the run fails after the retries instead of at once (MySQL, SQL Server and Oracle fail at once since `SRC-7`'s gate) | Data | P3 | Todo |  | `superpowers/plans/2026-10-10-src-9-connector-gates.md` |
| `SEC-29` | Task | 25 responses in `routes/pipelines.rs` put the orchestrator's error text in the body (`js_error(err)`; `DagsterError::Server` is Dagster's own GraphQL text). Same rule as `SEC-6`: a fixed body, the detail to `tracing` | Administration & Security | P2 | Todo |  | PR #80 review |
| `GOV-4` | Task | A masking policy on a column does not follow its rows through a pipeline: the pipeline copies unmasked rows into its Silver or Gold table, and the policy on the source column does not apply to the copy. Decide whether policies carry to outputs or outputs must be classified again | Governance | P2 | Todo |  | `BLD-3` plan, D5 |
| `SRC-20` | Task | The pipeline gate `ops/g7` is run by no CI job | Data | P3 | Todo |  | `plans/SRC-9-RESULT.md` |
| `SRC-21` | Task | Load-mode follow-ups from `SRC-10`: the assistant's `set_ingest_spec` schema does not know `loadMode` or `primaryKey`, so a spec it saves drops each table's mode and key (the job then replaces); no test covers a pending removal of a key column next to an approvable change when the pause is lifted | Data | P2 | Done | #145 | PR #115 review; `superpowers/plans/2026-10-14-src-21-load-modes-followups.md` |
| `DATA-24` | Task | Catalog search follow-ups: measure the 500 ms target on 10,000 tables (`DATA-11-RESULT.md`); keep the Owner and Tags filters reachable on a laptop-width window; read column rows with `FINAL`; run acceptance rows 10, 11 and 13 | Data | P2 | Todo |  | `superpowers/plans/2026-10-12-delivery-review.md` (`DATA-11`) |
| `AI-19` | Task | The AI team's plans for `AI-16` and `AI-17` in `docs/superpowers/plans/` (`AI-16-AC1`, `AGENTS.md` step 1); the feature pages say plans are not kept in the repository | AI Copilot | P2 | Todo |  | `superpowers/plans/2026-10-12-delivery-review.md` |

## Later (P3)

| ID | Kind | Item | Module | Priority | Status | PR | Source |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `OPS-3` | Feature | Run on several servers; high availability | Platform | P3 | Idea |  | PRODUCT §1, §2 |
| `SRC-3` | Feature | Connectors for business apps: first Databricks' ten generally available SaaS connectors, then its beta list by customer demand. **Waits for `DEC-9`** | Data | P3 | Planned |  | PRODUCT §2 |
| `DATA-8` | Feature | Instant copies of a table for testing | Data | P3 | Idea |  | PRODUCT §2 |
| `BLD-2` | Feature | Tables that refresh themselves within a target lag set per pipeline, and the same through the assistant. **Waits for `BLD-3`, `DATA-10`** | Build | P3 | Planned |  | PRODUCT §2; [spec](specs/bld-2.md) |
| `SRC-5` | Feature | Real connection test for MongoDB, Kafka, SFTP, Sheets and Oracle sources | Data | P3 | Idea |  | `connector_probe.rs` |
| `DOC-2` | Task | Step-by-step user guide checked against a running console | Docs | P3 | Todo |  | `reference/user-guide.md` |
| `DATA-3` | Feature | Store Gold in open format first; serve from a fast copy. **Waits for `DATA-4`** and a new ADR | Build | P3 | Planned |  | ADR 0010; [spec](specs/data-3.md) |
| `DATA-4` | Feature | Publish Silver in open format. **Waits for `DATA-10`, `DATA-2`** | Build | P3 | Planned |  | `features/gold-publish-per-mart.md`; [spec](specs/data-4.md) |
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
| `DATA-16` | 2026-10-09 | 2026-10-12 | |
| `SEC-23` | 2026-10-09 | 2026-10-12 | |
| `WEB-1` | 2026-10-11 | 2026-10-11 | |
| `SRC-6` | 2026-10-08 | 2026-10-12 | |
| `SRC-7` | 2026-10-08 | 2026-10-12 | |
| `SRC-8` | 2026-10-09 | 2026-10-12 | |
| `SRC-9` | 2026-10-10 | 2026-10-13 | |
| `SRC-10` | 2026-10-12 | 2026-10-14 | |
| `SRC-16` | 2026-10-13 | 2026-10-13 | |
| `SRC-17` | 2026-10-13 | 2026-10-14 | |
| `AI-12` | 2026-10-13 | | |
| `SRC-10` | 2026-10-12 | 2026-10-14 | |
| `SRC-21` | 2026-10-14 | 2026-10-14 | |

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

### Pipelines (Build)

| ID | Spec | When | Size |
| --- | --- | --- | --- |
| [`BLD-1`](specs/bld-1.md) | Joins, aggregations and a SQL step | Next | L, in two parts |
| [`BLD-2`](specs/bld-2.md) | Tables that refresh themselves within a target lag | Later | L |
| [`DATA-2`](specs/data-2.md) | Keep the last copies of a published Gold table | Next | M |
| [`DATA-6`](specs/data-6.md) | A latest-copy-only table for outside readers | Next | S |
| [`DATA-4`](specs/data-4.md) | Publish Silver in open format | Later | M |
| [`DATA-3`](specs/data-3.md) | Store Gold in open format first | Later | L |

### Query Studio

| ID | Spec | When | Size |
| --- | --- | --- | --- |
| [`QS-1`](specs/qs-1.md) | Editor basics | Next | M |
| [`QS-2`](specs/qs-2.md) | Results you can use | Next | M |
| [`QS-3`](specs/qs-3.md) | Saved queries done properly | Next | M |
| [`QS-4`](specs/qs-4.md) | History and query profile | Next | M |
| [`QS-5`](specs/qs-5.md) | Parameters, snippets and charts from results | Next | L |
