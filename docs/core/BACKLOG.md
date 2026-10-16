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

**Who does what** is held in the base (Owner for a Feature, Assignee for a
Task), never here (public repo, `DEC-12`). Every open item has one. Work is
spread by weight, not by count: a spec's size S = 1, M = 3, L = 6; an item
with no spec counts 1 (Task) or 3 (Feature). The planner balanced the
Lakehouse team to within about 10 of each other on 2026-10-15 (product
owner's request) and keeps new items in line when it adds them.

**Blocked by** (was "Waits for"). An item that cannot start until another
merges says so in its text: **Blocked by `BI-18`**, or for one part of it,
**Blocked by `QS-5` (variables part)**, so the rest can start. Only open
blockers are written; a blocker that ships is removed from the text. The
reverse direction (**Blocks**) is listed under Dependencies below, derived
from these markers and never written by hand. A blocker is a dependency, not
a status: the item stays **Planned** or **Todo**, never **Blocked**. Blocked
means started work that is stopped, with the reason and who can unblock it.
The base mirrors both directions as two-way links. Specs carry the size
estimate (S, M, L), which the base has no field for.

## Now (P0–P1)

| ID | Kind | Item | Module | Priority | Status | PR | Source |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `AI-18` | Feature | Copilot Chat: ask a question or give an instruction in plain language, and the assistant answers or acts with the console's own tools; risky actions wait for approval. Built before this backlog tracked PRs; never accepted | AI Copilot | P1 | In Acceptance | #169 | `PRODUCT.md` §2 (Home, History) |
| `SEC-22` | Task | Converting an uploaded workbook can exhaust the API's memory and stop the API for every user; the conversion must not be able to end the API process. Also: the Parquet reader trusts sizes from the file's footer the same way (by reading, not run); no limit on conversions running at once; no byte cap on workbook text; each preview converts the whole file again; the limits doc in `upload_workbook.rs` says the opposite of the code | Administration & Security | P0 | Todo |  | PR #76 review, BLOCKER 1 (merged without the fix); `superpowers/plans/2026-10-12-delivery-review.md` |
| `SEC-23` | Task | Adding a quality, classification or residency rule asked only for a login, while changing one asks for `governance:write`; adding now asks for it too, in the API, the assistant's tools and the console | Administration & Security | P1 | Done | #97 | `features/governance-rule-create.md` |
| `DATA-1` | Feature | Gold tables publish to open format automatically, as an option per table | Build | P1 | In Acceptance | #60, #63, #64 | `features/gold-publish-per-mart.md` |
| `QA-1` | Task | Accept each built feature with a checklist, starting with what a demo shows | Delivery | P1 | Todo |  | PRODUCT §4 blocker 1 |
| `SEC-1` | Task | Confirm the leaked key is rotated; decide on rewriting git history. Decided 2026-10-14: no rewrite; the key and the staging values are rotated (product owner) and their history findings are baselined by exact fingerprint in `.gitleaksignore` | Administration & Security | P1 | Done | #153 | `SECURITY.md` |
| `REL-1` | Task | Protect the main branch | Delivery | P1 | Todo |  | `docs/CI.md` |
| `DOC-1` | Task | Archive or rewrite the old product documents that contradict the build | Docs | P1 | Done | #171 | PRODUCT §6 |
| `GOV-2` | Decision | Decide which role may publish a table in open format | Governance | P1 | Open |  | PRODUCT §6 |
| `SEC-10` | Task | Alert webhooks can be pointed at internal addresses; use the allowlisted resolver. The `SRC-17` gate (`ops/g6/src17_alert_delivery_test.py`) receives webhooks on its own network: this item gives that gate its documented opt-in | Administration & Security | P1 | Done | #105 | `lakehouse-notify` |
| `SEC-11` | Task | Dashboard tiles and public/embed links return raw database error text. Same rule as `SEC-6` | Administration & Security | P1 | Done | #105 | `routes/support.rs` |
| `SEC-14` | Feature | A stored connector password can be stolen by re-pointing the connector: a host change keeps the secret and the next test sends it to the new server | Administration & Security | P0 | Building | #85 | `routes/connectors.rs`, `connector_probe.rs` |
| `SEC-15` | Feature | Connection tests can reach internal addresses: the block is off by default in compose, the REST test follows redirects, no address pinning, CDC delete skips the check | Administration & Security | P0 | In Acceptance | #82 | `docker-compose.yml`, `connector_probe.rs` |
| `SEC-13` | Task | Check `.env.staging`, pushed straight to `main` in `7ec3a81`, for real credentials or hostnames; rotate anything real | Administration & Security | P1 | Done | #130 | commit `7ec3a81` |
| `SEC-24` | Task | The assistant's system prompt gives any signed-in user the DATA MAP (tables, row counts, sample values) without `catalog:read`; a Dashboard Viewer sees samples Query Studio would refuse it. Give the map only with `catalog:read`, and sample values only with `query:read` | Administration & Security | P1 | Todo |  | `superpowers/plans/2026-10-12-delivery-review.md` (`SEC-9` gap) |
| `BLD-3` | Task | A pipeline built in the console cannot read a Bronze table, so Bronze to Silver does not run: the job reads its source as the ClickHouse table `<sourceZone>.<sourceTable>`, and ClickHouse has no `bronze` database; Bronze is Iceberg, readable only through a `DataLakeCatalog` database (`icecat_api`, tables named `bronze.<table>`). The console and the assistant still offer Bronze as a source. Read the source through the job's own `DataLakeCatalog` database, as `maintenance.py` does, with masking and governance still applied; prove it on a running stack, which needs `SRC-20`. Found by reading the code after a user report, not yet reproduced | Build | P1 | Done | #155 | `authored_factory.py` (`_op_for_pipeline`), `scripts/seed-clickhouse.sql` |
| `SEC-28` | Task | A pipeline built in the console can be read, edited, deleted, restored, run and paused by its ID from another organisation: the per-ID pipeline routes and the run routes do not check the caller's tenant. Also: run settings checked by top-level key only; a restored version is not validated; orchestrator error text in some responses. Fixed on the branch of PR #80, written before this backlog tracked it | Administration & Security | P1 | Done | #80 | PR #59 review (F2); PR #80 review |
| `SEC-30` | Task | The Security workflow is red on `main`: `rustls` 0.23.45 has four advisories published 2026-10-14 (`RUSTSEC-2026-0346` `Debug` exposes TLS secrets, `-0347` buffered data sent after an ECH rejection, `-0348` certificate decompression can allocate 1 GiB before authentication, `-0349` an unauthenticated ServerHello can downgrade later connections). Update to `rustls` >= 0.23.46. Until then a pull request that touches dependencies cannot merge (`AGENTS.md`, step 6) | Administration & Security | P1 | Done | #159 | CI on `main` at `c9a8d56f`, re-run 2026-10-14; named in the PR #155 review |
| `SEC-35` | Task | The lineage route shows every tenant's connectors and pipelines to a caller who belongs to no tenant: `tenant_scope::resolve` returns `None` for such a caller, and `lineage::add_tenant_owned` passes it on as "no filter", against `tenant_scope`'s own rule (no tenant means an empty list). Being fixed with `DATA-19` (review BLOCKER 1), which also covers the upload edges it adds; check the other callers of `ConnectorFilter { tenant_id: None }` for the same mistake. Fixed in #165 (`DATA-19` review BLOCKER 1). | Administration & Security | P1 | Done | #165 | `DATA-19` review, round 1 |
| `SEC-36` | Task | The full-history `gitleaks` scan runs `git fetch --all`, so it scans every branch, not only `main` and the PR: test values pushed on an open branch turn it red on `main` and on every PR. Since 2026-10-15 it is red with 20 `generic-api-key` findings, all on `feat/phase-1-rest` (#144, `SRC-12`/`SRC-13`): `CONNECTOR_SECRETS_KEY=` in `ci.yml` and the AES vector fixture `connector_secret_vector.json`. #144 must either use values the rule does not match or add exact fingerprints with proof (`SEC-1`) before it merges; then decide whether the scan should cover only `main` plus the PR head. On #144 since 2026-10-15: no key-shaped literal is committed any more (CI generates its keys at run time, the test vector describes its key), and the 22 historical findings are baselined by exact fingerprint with proof in `.gitleaksignore`; the full-history scan of the branch reports none. The scope question was settled by #172 (the history scan checks `main` and the PR, a daily scan checks every branch); the baseline stays needed for #144's own history | Administration & Security | P1 | Done | #172, #144 | `DATA-2` review (PR #164) |
| `SRC-16` | Task | A connector loaded after the fact (files, REST, MongoDB, Kafka, SFTP) on the "pause" policy stays paused for good after its first change: nothing waits for approval, so nothing can lift it. Also a pending change that Approve refuses keeps the connector paused after every other change is approved | Data | P1 | Done | #120 | `superpowers/plans/2026-10-12-delivery-review.md` (`SRC-8` BLOCKER) |

## Next (P2)

| ID | Kind | Item | Module | Priority | Status | PR | Source |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `RPT-1` | Feature | Scheduled reports with PDF and spreadsheet attachments. **Blocked by `BI-1` (PDF and Excel attachments)** | Dashboards | P2 | Planned |  | PRODUCT §2 |
| `BI-1` | Feature | Export a dashboard as a server-made PDF and as Excel | Dashboards | P2 | Planned |  | PRODUCT §2 |
| `BI-4` | Feature | Result caching for charts and queries; run a dashboard's tiles in parallel | Dashboards | P2 | Planned |  | `reference/competitive-comparison.md` (BI matrix) |
| `BI-5` | Feature | Configurable query timeouts and row limits; Stop cancels the query on the server | Dashboards | P2 | Planned |  | `reference/competitive-comparison.md` (BI matrix) |
| `BI-6` | Feature | A chart builder as capable as Metabase's and Tableau's: several dimensions, filters, sorting, multi-step questions. No drag and drop; the assistant covers that | Dashboards | P2 | In Acceptance | [#160](https://github.com/RantAI-dev/RantAI-Lakehouse/pull/160) | `reference/competitive-comparison.md` (BI matrix) |
| `BI-7` | Feature | Joins in the chart builder without writing SQL | Dashboards | P2 | In Acceptance | [#160](https://github.com/RantAI-dev/RantAI-Lakehouse/pull/160) | `reference/competitive-comparison.md` (BI matrix) |
| `BI-8` | Feature | Calculated fields and custom expressions: maths, text, dates, conditions, running totals | Dashboards | P2 | In Acceptance | #104 | `reference/competitive-comparison.md` (BI matrix) |
| `BI-9` | Feature | Group by day, week, month, quarter, year | Dashboards | P2 | In Acceptance | #104 | `reference/competitive-comparison.md` (BI matrix) |
| `BI-10` | Feature | Bins, groups and hierarchies | Dashboards | P2 | In Acceptance | [#160](https://github.com/RantAI-dev/RantAI-Lakehouse/pull/160) | `reference/competitive-comparison.md` (BI matrix) |
| `BI-11` | Feature | Parameters, usable in charts, SQL and dashboards. **Blocked by `QS-5` (parameters in SQL)** | Dashboards | P2 | In Acceptance | [#160](https://github.com/RantAI-dev/RantAI-Lakehouse/pull/160) | `reference/competitive-comparison.md` (BI matrix) |
| `BI-13` | Feature | Combine two sources on one chart | Dashboards | P2 | In Acceptance | [#160](https://github.com/RantAI-dev/RantAI-Lakehouse/pull/160) | `reference/competitive-comparison.md` (BI matrix) |
| `BI-14` | Feature | Lineage from a chart to its sources, reachable from the dashboard. **Blocked by `DATA-13` (column part)** | Dashboards | P2 | Planned |  | `reference/competitive-comparison.md` (BI matrix) |
| `BI-16` | Feature | Every chart type Tableau and Metabase have that we lack: progress bar, histogram, Gantt, bullet graph, KPI with comparison and sparkline, raw-row table, pivot table with totals, record detail, world maps and uploaded boundaries, image and web-page cards, spatial analysis, custom chart plugins. Part A (tables and KPI) in #104; parts B and C left. **Blocked by `BI-10` (histogram bins)** | Dashboards | P2 | Building | #104 | `reference/competitive-comparison.md` (BI matrix) |
| `BI-17` | Feature | Chart formatting at both competitors' level: number formats, colours, axes, data labels, conditional formatting, goal and reference lines, trend lines, forecasting, clustering, annotations, custom tooltips, themes and fonts | Dashboards | P2 | Planned |  | `reference/competitive-comparison.md` (BI matrix) |
| `BI-18` | Feature | Dashboards at both competitors' level: tabs; phone and tablet layouts; date-range, relative-date, number, text and linked filters; filter defaults that do not change silently; drill-down for every chart; click to another dashboard or URL; show and hide; saved auto-refresh; stories; templates; favourites; version history; trash; personal space. Parts A (filters) and B (click-through) in #104; part C (tabs, phone layout, sections, stories, templates, favourites, versions, trash, personal space, duplicate) left | Dashboards | P2 | Building | #104 | `reference/competitive-comparison.md` (BI matrix) |
| `AI-1` | Feature | The assistant manages the dashboards that exist today: rename boards, delete them (with approval), arrange and resize tiles, move charts between boards | AI Copilot | P2 | Planned |  | `AGENTS.md` (assistant parity) |
| `AI-2` | Feature | The assistant adds and changes dashboard filters ("only 2025, only Bali"). Deferred out of `BI-18` part A by the product owner on 2026-10-15: phase 1 ships without the filter tool, the AI team builds it next (`dashboard-filters.md` decision 8). **Blocked by `BI-18`** | AI Copilot | P2 | Planned |  | `AGENTS.md` (assistant parity) |
| `AI-3` | Feature | The assistant picks the time grain ("monthly") | AI Copilot | P2 | In Acceptance | #104 | `AGENTS.md` (assistant parity) |
| `AI-4` | Feature | The assistant writes calculated fields in the product's formula language, not raw SQL | AI Copilot | P2 | In Acceptance | #104 | `AGENTS.md` (assistant parity) |
| `AI-5` | Feature | The assistant creates every new chart type: raw and pivot tables, KPI with comparison, histogram, Gantt and the rest. The chart types of `BI-16` part A in #104; `AI-5-AC1` not checked yet. **Blocked by `BI-16` (per chart type as it lands)** | AI Copilot | P2 | Building | #104 | `AGENTS.md` (assistant parity) |
| `AI-6` | Feature | The assistant builds charts with several dimensions and filters, joins tables, uses bins, groups, hierarchies and parameters, and combines two sources. **Blocked by `BI-6`, `BI-7`, `BI-10`, `BI-11`, `BI-13`** | AI Copilot | P2 | In Acceptance | [#160](https://github.com/RantAI-dev/RantAI-Lakehouse/pull/160) | `AGENTS.md` (assistant parity) |
| `AI-7` | Feature | The assistant uses named metrics before it writes SQL, so the same question gives the same number. **Blocked by `BI-2`** **Owner QA, 2026-10-15:** the assistant ran SQL before looking for a metric (fails `AI-7-AC1` on `reusable-metrics.md`), and the chart draft card showed `sum(<column>)` instead of the metric's label. Both stay open under this item. | AI Copilot | P2 | In Acceptance | [#160](https://github.com/RantAI-dev/RantAI-Lakehouse/pull/160) | `AGENTS.md` (assistant parity) |
| `AI-8` | Feature | The assistant changes number formats, colours, axes and labels, and adds trend and forecast lines. **Blocked by `BI-17`** | AI Copilot | P2 | Planned |  | `AGENTS.md` (assistant parity) |
| `AI-9` | Feature | The assistant adds tabs and uses templates. **Blocked by `BI-18` (layout part)** | AI Copilot | P2 | Planned |  | `AGENTS.md` (assistant parity) |
| `AI-10` | Feature | The assistant schedules reports ("send this to me every Monday as PDF") and produces a PDF or Excel file on request. **Blocked by `RPT-1`, `BI-1`** | AI Copilot | P2 | Planned |  | `AGENTS.md` (assistant parity) |
| `AI-11` | Feature | The assistant creates share and embed links only with approval, respects dashboard permissions, and never grants permissions. **Blocked by `BI-19`, `BI-26`, `DEC-8`** | AI Copilot | P2 | Planned |  | `AGENTS.md` (assistant parity) |
| `AI-12` | Feature | The assistant fixes and explains SQL inside the editor; variables. **Blocked by `QS-5` (variables part)** | AI Copilot | P2 | Building | #127, #128 | `AGENTS.md` (assistant parity) |
| `AI-13` | Feature | The assistant answers "where does this number come from" from the chart's lineage (`get_lineage` exists today). **Blocked by `BI-14`** | AI Copilot | P2 | Planned |  | `AGENTS.md` (assistant parity) |
| `AI-14` | Feature | Ongoing, with every BI item. What the dashboard page sends to the assistant grows with the features: filters, tabs, metrics, formats | AI Copilot | P2 | Planned |  | `AGENTS.md` (assistant parity) |
| `AI-15` | Task | A standard set of about 30 typical dashboard requests, re-run whenever an assistant tool changes, so the assistant does not silently get worse | AI Copilot | P2 | Done | #175 | `AGENTS.md` (assistant parity) |
| `BI-26` | Feature | Better embedding, without overdoing it: a single-chart embed that really shows only that chart, light/dark theme and our logo optional, filters passed through the link or token, sizing that fits the host page. Token hardening is `SEC-12`. No embedding SDK and no editing inside embeds. **Blocked by `BI-18` (locking a dashboard filter in an embed)** | Dashboards | P2 | Planned |  | `reference/competitive-comparison.md` (BI matrix) |
| `BI-19` | Feature | Sharing and delivery, except Slack, Teams, Office and Google integrations: public link expiry and password; permissions per dashboard and folder; comments; a schedule per digest; alerts with filters that fire once; PNG export; import of exported dashboards. Scheduled reports are `RPT-1`, PDF and Excel export `BI-1`, embedding `BI-26`. **Blocked by `RPT-1` (digest schedules)** | Dashboards | P2 | Planned |  | `reference/competitive-comparison.md` (BI matrix) |
| `BLD-1` | Feature | Joins, aggregations and a SQL step in the pipeline builder, and the same through the assistant. | Build | P2 | Building | [#173](https://github.com/RantAI-dev/RantAI-Lakehouse/pull/173) (slice 1: a pipeline reads as its author) | PRODUCT §2; [spec](specs/bld-1.md) |
| `VER-1` | Task | Verify SFTP, Google Sheets and Oracle sources end to end; test row filters. The connector half is now `SRC-9`; row filters stay here | Data | P2 | Todo |  | PRODUCT §2 |
| `BI-2` | Feature | Metrics users can define once and reuse across dashboards (a semantic layer). **Blocked by `DATA-12` (certified metrics part)** | Dashboards | P2 | In Acceptance | [#160](https://github.com/RantAI-dev/RantAI-Lakehouse/pull/160) | PRODUCT §2; `reference/competitive-comparison.md` (BI matrix) |
| `DATA-9` | Feature | Upload a file from the console | Data | P2 | In Acceptance | #71, #73 | `features/upload-file.md` |
| `OPS-8` | Feature | Usage and cost view | Operations | P2 | Idea |  | PRODUCT §2 |
| `SEC-2` | Feature | Login rate limiting | Administration & Security | P2 | In Acceptance | #67 | `features/login-protection-and-session-cleanup.md` |
| `SEC-3` | Task | Test single sign-on against a real identity provider; correct `README.md`, which says it is unbuilt | Administration & Security | P2 | Done | #181 | PRODUCT §2 |
| `SEC-40` | Task | Sign in once through Microsoft Entra ID and once through Google with a test app registration and test user each, following `docs/OPERATIONS.md` (single sign-on), and record the date, settings (no secrets) and outcome in `docs/plans/SEC-3-RESULT.md` (`SEC-3-AC2`, `SEC-3-AC3`). Product owner's test, later | Administration & Security | P3 | Todo |  | `SEC-3` (#181) |
| `SEC-38` | Task | Single sign-on reads the signing keys' location (`jwks_uri`) from the provider's discovery document, so `OIDC_JWKS_URL` no longer has to be set by hand: today the API assumes `{OIDC_ISSUER}/.well-known/jwks.json`, which Entra ID and Google do not serve | Administration & Security | P3 | Todo |  | `SEC-3` review, `docs/plans/SEC-3-RESULT.md` |
| `SEC-39` | Task | An Entra ID user in more groups than the token can carry gets a group overage marker instead of `groups`; the console then applies no mapped role, silently. At least log it and say so on the user's sign-in; following the Microsoft Graph link is the full fix | Administration & Security | P3 | Todo |  | `SEC-3` review, `docs/OPERATIONS.md` |
| `SEC-7` | Task | A monitored security contact address | Administration & Security | P2 | Todo |  | `SECURITY.md` |
| `REL-2` | Task | Adopt a version and release policy; cut a release from the unreleased work | Delivery | P2 | Todo |  | `reference/release-policy.md` |
| `GOV-1` | Feature | Audit trail covers every console change | Governance | P2 | Idea |  | PRODUCT §3 |
| `OPS-1` | Task | Trim old versions of raw tables | Operations | P2 | Done | #178 | `README.md` |
| `OPS-9` | Task | The per-table maintenance route (`GET /api/lakehouse/tables/{ns}/{table}/maintenance`) never shows a Bronze table's last run: the job records the qualified name (`bronze.<table>`) while `routes/lakehouse.rs` `maintenance_verb_run_query` and `routes/governance.rs` `maintenance_run_query` filter on the bare name from the URL. The aggregate Governance view is right | Operations | P2 | Todo |  | `docs/plans/OPS-1-RESULT.md` |
| `DATA-2` | Task | Keep the last copies of a published Gold table, remove the rest. | Build | P2 | Done | #164 | `features/gold-publish-per-mart.md`; [spec](specs/data-2.md) |
| `DATA-10` | Task | A merge-proof "has this mart changed" signal for publishing, so background merges do not trigger extra copies | Build | P2 | Done | #156 | Slice B review, gold-publish plan |
| `DATA-6` | Feature | A "latest copy only" table for outside readers of published tables. | Build | P2 | Planned |  | `features/gold-publish-per-mart.md`; [spec](specs/data-6.md) |
| `SEC-4` | Task | Human security review | Administration & Security | P2 | Todo |  | PRODUCT §4 blocker 3 |
| `SUP-1` | Decision | Decide a support commitment | Business | P2 | Open |  | PRODUCT §4 blocker 5 |
| `OPS-7` | Task | Test and time a restore from backup | Operations | P2 | Done | #185 | PRODUCT §2 |
| `SEC-5` | Feature | Clean up old sessions and tokens | Administration & Security | P2 | In Acceptance | #67 | `features/login-protection-and-session-cleanup.md` |
| `SRC-6` | Feature | Fix the broken connectors: object-storage test and key change, Oracle credential change, Sheets shown as unsupported, Sources header text | Data | P2 | In Acceptance | #101 | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-7` | Feature | Alerts when a connector run or an upload load fails; health from real runs | Data | P2 | In Acceptance | #101 | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-8` | Feature | Schema changes at the source: detect, notify, and a per-connector choice to apply, ask or pause | Data | P2 | In Acceptance | #101 | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-9` | Task | Gate tests for every advertised connector (Oracle, SFTP, MySQL CDC, SQL Server CDC); the upload gate in CI | Data | P2 | Done | #114 | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-10` | Feature | More load modes: deduplicate on a key, keep history (SCD type 2), refresh or clear one table | Data | P2 | In Acceptance | #115 | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-11` | Feature | Connector operations: automatic retries with backoff, a log view per run, column choice. | Data | P2 | In Acceptance | [#144](https://github.com/RantAI-dev/RantAI-Lakehouse/pull/144) | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-12` | Feature | Credentials done properly: external secret managers, encryption at rest, OAuth sign-in. | Data | P2 | In Acceptance | [#144](https://github.com/RantAI-dev/RantAI-Lakehouse/pull/144) | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-13` | Feature | Table discovery for every source type, not only databases. | Data | P2 | In Acceptance | [#144](https://github.com/RantAI-dev/RantAI-Lakehouse/pull/144) | `reference/competitive-comparison.md` (Data matrix) |
| `SRC-15` | Feature | Load a SQL column whose type changed to one its Bronze column cannot hold (integer to text, text to a number, a changed decimal) into a second column when the change is approved. Today such a change cannot be approved and the table waits (`SRC-8`, decision 10). Starts after the g6 gate has shown the loader's behaviour on a real database. No spec yet | Data | P3 | Idea |  | `features/source-schema-changes.md` |
| `SRC-4` | Feature | More databases, with live change capture: Oracle tested end to end, MariaDB and Teradata added; live change capture for PostgreSQL, MySQL, SQL Server, Oracle and MongoDB, each gate-tested. Cloud warehouses and enterprise databases excluded (product owner, 2026-10-07). **Blocked by `SRC-11` (column choice), `SEC-14` (credentials)** | Data | P2 | Planned |  | `reference/competitive-comparison.md` (Data matrix) |
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
| `DATA-19` | Feature | Uploaded tables can feed pipelines and so reach dashboards. | Data | P2 | In Acceptance | #165 | `reference/competitive-comparison.md` (Data matrix) |
| `QS-1` | Feature | Query Studio editor basics: column autocomplete, schema browser, several statements, tabs, formatting rules | Query Studio | P2 | Planned |  | `reference/competitive-comparison.md` (Query Studio matrix) |
| `QS-2` | Feature | Query Studio results: up to 64,000 rows or 10 MB shown, grid filters and column stats, full server downloads up to 5 GB (Databricks' limits), server-side Stop, configurable timeouts. **Blocked by `SEC-21` (full-result download)** | Query Studio | P2 | Planned |  | `reference/competitive-comparison.md` (Query Studio matrix) |
| `QS-3` | Feature | Saved queries done properly: edit, delete, folders, sharing levels, version history. **Blocked by `SEC-20`** | Query Studio | P2 | Planned |  | `reference/competitive-comparison.md` (Query Studio matrix) |
| `QS-4` | Feature | Query history page with filters, and a query profile | Query Studio | P2 | Planned |  | `reference/competitive-comparison.md` (Query Studio matrix) |
| `QS-5` | Feature | The bridge to BI: parameters that become dashboard filters, snippets, charts from results, reuse a saved query in SQL, alerts on a query. **Blocked by `BI-18` (the dashboard date filter)** | Query Studio | P2 | Planned |  | `reference/competitive-comparison.md` (Query Studio matrix) |
| `AI-16` | Task | Hand-off to the AI team: AI features for the Data module. Remaining: plain-language table search, AI classification, AI quality suggestions and incident help, AI connector building. The AI team plans these | AI Copilot | P2 | In Progress | #79, #95, #96, #109, #110, #111, #112, #118 | `docs/core/features/semantic-layer.md` |
| `AI-17` | Feature | The chat asks once, with options, which reading of an unclear word the person means, remembers the clicked answer per user, and writes in full only the tables a question names | AI Copilot | P2 | In Acceptance | #81, #84, #125 | `features/chat-ask-back.md` |
| `WEB-1` | Feature | Landing page at `/landing`: a 3D night lake with the logo's creature, and a scroll story through sources, the Bronze/Silver/Gold layers, the assistant, dashboards, governance and open source. Copy limited to what is built | Business | P2 | In Acceptance | #107 | `superpowers/plans/2026-10-11-landing-page.md` |
| `REL-3` | Task | `package-lock.json` is stale: it lacks packages `package.json` lists (`@dnd-kit/core` and others). Delete it (bun is the package manager; CI and the Dockerfile read `bun.lock`) or regenerate it, in a PR of its own | Delivery | P3 | Done | TBD | WEB-1 review round 1 |
| `REL-4` | Task | Move the console to TypeScript 7 (the new compiler); dependabot #136 fails typecheck and build. Dependabot ignores TypeScript majors until this is planned. **Blocked by** an outside release: TypeScript 7.0 ships no JavaScript API, so typescript-eslint and `next build` cannot run on it; typescript-eslint waits for 7.1's new API (checked 2026-10-16) | Delivery | P3 | Todo |  | `superpowers/plans/2026-10-14-deps-triage.md` |
| `REL-5` | Task | Move to eslint 10 and its flat-config changes; dependabot #143 fails lint. Dependabot ignores eslint majors until this is planned | Delivery | P3 | Done | #179 | `superpowers/plans/2026-10-14-deps-triage.md` |
| `REL-6` | Task | Move the data grids to TanStack Table 9; dependabot #133 fails typecheck. Dependabot ignores its majors until this is planned. **Blocked by `BLD-1`, `BLD-2`**: v9 changes the column type generics, so every column file changes, `src/features/pipelines/pipeline-columns.tsx` among them, which the pipelines lane holds until those merge | Delivery | P3 | Todo |  | `superpowers/plans/2026-10-14-deps-triage.md` |
| `REL-7` | Task | arrow and parquet 60 with a newer `iceberg` crate: `iceberg = "=0.10.1"` still uses arrow 58, so #139, #140 and #142 fail to compile. **Blocked by** an `iceberg` release on arrow 60 (outside the repo); dependabot now groups the three | Delivery | P3 | Todo |  | `superpowers/plans/2026-10-14-deps-triage.md` |
| `REL-8` | Task | `next` and `eslint-config-next` 16.1.6 → 16.4.0: CI's `bun run build` fails with "Failed to load external module next/dist/compiled/next-server/app-page-turbo.runtime.prod.js: TypeError: Expected CommonJS module to have a function wrapper ... this is a bug in Bun"; the same lockfile builds locally on Bun 1.3.12 and 1.3.14. Try a newer Bun in CI, or report it to Bun | Delivery | P3 | Done | #170 | dependabot #131; `superpowers/plans/2026-10-14-deps-triage.md` |
| `SEC-6` | Task | Stop older API handlers from returning internal error text. Closed by `SEC-11`'s change: no handler builds a response from an error's text, and `tests/sec11_guard.rs` fails the build if one does | Administration & Security | P2 | Done | #105 | `AGENTS.md` |
| `SEC-16` | Task | Cross-tenant leaks in the Data module: the ingestible-connectors route, one shared upload namespace, a message naming another tenant's upload table, annotation edits without the tenant gate | Administration & Security | P2 | Done | #87 | Data module code audit |
| `SEC-17` | Task | Upload hardening: a column cap, a limit on concurrent uploads, CSV exports that neutralise formula cells | Administration & Security | P2 | Done | #86 | Data module code audit |
| `SEC-18` | Task | No default credentials in compose (`lakehouse`, `rustfsadmin`); must-set secrets use `${X:?}` (rule 5) | Administration & Security | P2 | Done | #83 | `docker-compose.yml` |
| `SEC-37` | Task | Default credentials left in code after `SEC-18`: `rustfsadmin` fallbacks in `ops/g3/g3_loadgen.py`, `ops/g6/g6_ingest_matrix_test.py` and `dagster/dispar_orchestrate/dlt_pipeline.py`, and `postgres://lakehouse:lakehouse@localhost` as `DATABASE_URL`'s default in `lakehouse-api` `config.rs`. Compose always sets them now, so none is reached in a compose start | Administration & Security | P2 | Todo |  | PR #83 review |
| `SEC-19` | Task | The query cost estimate runs raw SQL without the read-only check or table-function block, and returns database error text | Administration & Security | P2 | Todo |  | `routes/query.rs` |
| `SEC-20` | Task | Queries are not scoped to the tenant: no tenant gate on query runs; saved queries listed for everyone | Administration & Security | P2 | Todo |  | `routes/query.rs` |
| `SEC-21` | Task | Downloads re-run SQL rewritten for an older policy; whole results buffered before the 2,000-row cut | Administration & Security | P2 | Todo |  | `routes/query.rs` |
| `SEC-12` | Feature | Embed tokens: require an expiry, allow revocation, keep the signing secret out of plain text, set a frame-ancestors policy | Administration & Security | P2 | In Acceptance | #105 | `lakehouse-embed` |
| `SEC-25` | Task | Connector security follow-ups: a connector that signs in with no credential (Kafka, auth `none`) can never be re-pointed (409 without a credential, 400 with one); the PATCH-`host` rule and its doc for `files` connectors are stale since `SRC-6`; the `SEC-15` refusal message has runs of spaces | Administration & Security | P2 | Todo |  | `superpowers/plans/2026-10-12-delivery-review.md` |
| `SEC-26` | Task | About thirty assistant tool results carry raw upstream error text (`err.to_string()`) into the chat response's `toolTrace`. Same rule as `SEC-6` | Administration & Security | P2 | Todo |  | `superpowers/plans/2026-10-12-delivery-review.md` |
| `SEC-27` | Task | The Security workflow is red on `main`: `cargo audit` reports advisories for `quinn-proto` 0.11.17 (`RUSTSEC-2026-0338`, `-0340`, `-0343`), and both `gitleaks` jobs fail. Update or waive the dependency with a reason; triage the `gitleaks` findings as `docs/CI.md` describes. Until then a pull request that touches dependencies cannot merge (`AGENTS.md`, step 6) | Administration & Security | P1 | Done | #121, #130, #153 | CI on `main` at `c083125` (2026-10-13); named in the `SRC-16` review |
| `SRC-17` | Task | Load-failure alert delivery: a failed delivery is recorded as if sent (feature-page row 5 cannot pass); a rule-store failure loses the alert because the dedupe row is written first; the Alerts page still says nothing raises `connector_schema_change`; a gate step that receives a webhook, and `SRC-7-RESULT.md` with measured seconds | Data | P2 | Done | #124 | `superpowers/plans/2026-10-12-delivery-review.md` (`SRC-7`) |
| `SRC-18` | Task | A paused connector still loads when a run was already queued or is launched from Dagster: only the due list and the run route check the pause. The schema-observation routes use `ingest:read` instead of their own permission. First half closed by `SRC-11` (#144) The first half (a paused connector's queued run) is closed by `SRC-11` decision 13 in #144. | Data | P2 | Todo | [#144](https://github.com/RantAI-dev/RantAI-Lakehouse/pull/144) (first half) | `superpowers/plans/2026-10-12-delivery-review.md` (`SRC-8`) |
| `SRC-19` | Task | A wrong password for a PostgreSQL source is retried as if it were a passing fault: the driver gives no error code to tell it apart, so the run fails after the retries instead of at once (MySQL, SQL Server and Oracle fail at once since `SRC-7`'s gate) Closed by `SRC-11` decision 2. | Data | P3 | Done | [#144](https://github.com/RantAI-dev/RantAI-Lakehouse/pull/144) | `superpowers/plans/2026-10-10-src-9-connector-gates.md` |
| `SEC-29` | Task | 25 responses in `routes/pipelines.rs` put the orchestrator's error text in the body (`js_error(err)`; `DagsterError::Server` is Dagster's own GraphQL text). Same rule as `SEC-6`: a fixed body, the detail to `tracing`. Closed by `SEC-11`'s change: no `js_error` is left; orchestrator failures answer a fixed message and a reference id | Administration & Security | P2 | Done | #105 | PR #80 review |
| `GOV-4` | Task | A masking policy on a column does not follow its rows through a pipeline: the pipeline copies unmasked rows into its Silver or Gold table, and the policy on the source column does not apply to the copy. Decide whether policies carry to outputs or outputs must be classified again. **Decided by the product owner, 2026-10-14:** a pipeline reads as its author, through the same masking and row-filter rewrite as the author's own queries; built in `BLD-1`. Closed by `BLD-1` slice 1 | Governance | P2 | Done | [#173](https://github.com/RantAI-dev/RantAI-Lakehouse/pull/173) | `BLD-3` plan, D5 |
| `SRC-20` | Task | The pipeline gate `ops/g7` is run by no CI job | Data | P3 | Done | #155 | `plans/SRC-9-RESULT.md` |
| `SRC-21` | Task | Load-mode follow-ups from `SRC-10`: the assistant's `set_ingest_spec` schema does not know `loadMode` or `primaryKey`, so a spec it saves drops each table's mode and key (the job then replaces); no test covers a pending removal of a key column next to an approvable change when the pause is lifted | Data | P2 | Done | #145 | PR #115 review; `superpowers/plans/2026-10-14-src-21-load-modes-followups.md` |
| `SRC-22` | Task | The assistant's `discover_source` tool still calls the unmounted `/discover` handler (`routes/connectors.rs` `discover`, `connector_discover.rs`), which lists SQL sources only. Move it to `SRC-13`'s discovery routes (every type, names first, previews), regenerate `tests/fixtures/tool_schemas.json`, then delete the old module. Assistant work outside dashboards: the AI team plans it | Data | P3 | Todo |  | `superpowers/plans/2026-10-15-src-13-table-discovery.md` (review) |
| `DATA-24` | Task | Catalog search follow-ups: measure the 500 ms target on 10,000 tables (`DATA-11-RESULT.md`); keep the Owner and Tags filters reachable on a laptop-width window; read column rows with `FINAL`; run acceptance rows 10, 11 and 13 | Data | P2 | Todo |  | `superpowers/plans/2026-10-12-delivery-review.md` (`DATA-11`) |
| `AI-19` | Task | The AI team's plans for `AI-16` and `AI-17` in `docs/superpowers/plans/` (`AI-16-AC1`, `AGENTS.md` step 1); the feature pages say plans are not kept in the repository | AI Copilot | P2 | Todo |  | `superpowers/plans/2026-10-12-delivery-review.md` |
| `SEC-31` | Task | The policy rewrite wrote a string literal back with `sqlparser`'s `Display`, which does not re-encode backslash escapes for `ClickHouse`, so a literal could reach the database with another value than the caller wrote. Fixed in `f8104c8`; ticketed so it is reviewed on its own | Administration & Security | P1 | Done | #104 | PR #104 review |
| `SEC-32` | Task | Admin-written mask and row-filter expressions are not re-encoded by the `SEC-31` fix; check whether they need it | Administration & Security | P2 | Todo |  | `superpowers/plans/2026-10-07-dashboard-filters.md` follow-up, PR #104 review |
| `SEC-33` | Task | A Query Studio download rewrites the history's statement, which was stored already rewritten, so an old mask or row filter stays inside and the current one applies on top. Fails closed, but the doc says today's policy governs the file; store the original statement or test the stored shape and correct the doc (`query.rs`, `tests/query_download.rs` seeds raw SQL) | Administration & Security | P2 | Todo |  | PR #104 review SHOULD-FIX 1 |
| `SEC-34` | Task | Calculated-field names read back from storage enter the SQL as a plain alias, not re-validated (`fields.rs` `row_to_field`, `builder.rs`). Not reachable through the API; wrap the name in `Ident` on read | Administration & Security | P2 | Todo |  | PR #104 review SHOULD-FIX 2 |
| `BI-27` | Task | Whole-result export holds up to three copies of 100,001 rows and mart exports have no time or byte limit (`dashboard.rs`). Take the rows instead of cloning, add a byte cap and a timeout, or stream | Dashboards | P2 | Todo |  | PR #104 review SHOULD-FIX 3 |
| `BI-28` | Task | Export honesty: negative `Int64` values are written `'-5` (decide number columns from `result.meta`); a file can include rows a tile skipped with nothing saying so (`RecordsSql.skipped` dropped); a raw table with a calculated column cannot be exported (400) | Dashboards | P2 | Todo |  | PR #104 review SHOULD-FIX 4 |
| `BI-29` | Task | Dashboard feature code imports `@/services/clients/*` directly in 25 files (was 11): `records.ts`, `table-export.ts`, `filters/use-filter-values.ts` (hand-rolled effect, not `useService`), `click-setting.tsx`. Route them through `@/services` | Dashboards | P2 | Todo |  | PR #104 review SHOULD-FIX 6 |
| `BI-30` | Task | One CSV rule: `csv_export.rs` repeats `upload_workbook.rs`'s quoting and BOM, its `neutralise` differs from `csv_safe::neutralize_csv` (#86), and `src/lib/csv.ts` does not neutralise. Pick one owner | Dashboards | P2 | Todo |  | PR #104 review SHOULD-FIX 7 |
| `AI-20` | Task | The dashboard's page context sends the filters but not the time-grain override (`dashboard-page.tsx`), so the assistant cannot see it | AI Copilot | P2 | Todo |  | PR #104 review SHOULD-FIX 11 |

## Later (P3)

| ID | Kind | Item | Module | Priority | Status | PR | Source |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `OPS-3` | Feature | Run on several servers; high availability | Platform | P3 | Idea |  | PRODUCT §1, §2 |
| `SRC-3` | Feature | Connectors for business apps: first Databricks' ten generally available SaaS connectors, then its beta list by customer demand. **Blocked by `DEC-9`, `SRC-12` (OAuth sign-in)** | Data | P3 | Planned |  | PRODUCT §2 |
| `DATA-8` | Feature | Instant copies of a table for testing | Data | P3 | Idea |  | PRODUCT §2 |
| `BLD-4` | Task | A pipeline that names `ingest_job` in its upstreams runs after every connector's load, not only after the load of its own source table: its sensor counts any `ingest_job` success. Filter by the run's connector and target table, as `DATA-19` does for `file_ingest_job`. | Build | P3 | Todo |  | `DATA-19` plan, section 5 |
| `BLD-2` | Feature | Tables that refresh themselves within a target lag set per pipeline, and the same through the assistant. | Build | P3 | Building |  | PRODUCT §2; [spec](specs/bld-2.md) |
| `SRC-5` | Feature | Real connection test for MongoDB, Kafka, SFTP, Sheets and Oracle sources | Data | P3 | Idea |  | `connector_probe.rs` |
| `DOC-2` | Task | Step-by-step user guide checked against a running console | Docs | P3 | Todo |  | `reference/user-guide.md` |
| `DATA-3` | Feature | Store Gold in open format first; serve from a fast copy. **Blocked by `DATA-4`** and a new ADR that replaces ADR 0010's direction | Build | P3 | Planned |  | ADR 0010; [spec](specs/data-3.md) |
| `DATA-4` | Feature | Publish Silver in open format. | Build | P3 | Planned |  | `features/gold-publish-per-mart.md`; [spec](specs/data-4.md) |
| `DATA-5` | Task | Measure outside readers of published tables | Build | P3 | Todo |  | `routes::gold::consumers` |
| `BI-3` | Feature | Import dashboards from Tableau and Power BI | Dashboards | P3 | Idea |  | PRODUCT §2 |
| `BI-20` | Feature | The rest of the BI matrix, to be considered later: field metadata in the BI layer, certified content, BI-specific AI, admin and platform rows, and Slack, Teams, Office and Google integrations | Dashboards | P3 | Idea |  | `reference/competitive-comparison.md` (BI matrix) |
| `QRY-1` | Feature | Query other systems in place, without loading | Data | P3 | Idea |  | PRODUCT §2 |
| `SRC-2` | Feature | Processing over streams | Data | P3 | Idea |  | PRODUCT §2 |
| `INT-2` | Feature | Search over documents | AI Copilot | P3 | Idea |  | PRODUCT §2 |
| `GOV-3` | Feature | Lineage that reaches outside systems | Governance | P3 | Idea |  | PRODUCT §2 |
| `GOV-5` | Task | The audit page lists a chat draft waiting for Confirm, and a request waiting for approval, as Denied with policy `deny`: `outcome_of` in `routes/governance.rs` maps every gate outcome that is not a success to `denied`, on purpose, so a decision that is still open cannot read as a success. Decide whether the two should read Pending | Governance | P3 | Todo |  | `AI-18` checklist row 11, 2026-10-14 |
| `ADM-1` | Feature | Several organisations in one install | Administration & Security | P3 | Idea |  | PRODUCT §2 |
| `ADM-2` | Feature | Workspace settings that can be changed | Administration & Security | P3 | Idea |  | `README.md` |
| `OPS-2` | Feature | Install with no internet access | Platform | P3 | Idea |  | Sales playbook (older; re-check) |
| `SRC-23` | Task | `route_auth.rs` `run_history_log_and_resume_need_connector_manage_...` passes without proving its claim: the `connector:manage` principal is in no tenant, so the tenant gate answers 404 before the permission gate and `assert_ne!(.., FORBIDDEN)` holds either way. Join both principals to the seeded tenant, as the discovery and OAuth tests do (the defect `SRC-13` review 3 BLOCKER 2 fixed there). Found in the #144 review | Sources | P3 | Todo |  | #144 review |
| `SRC-24` | Task | `SRC-11` plan task 9 asks `docs/plans/SRC-11-RESULT.md` for the time to auto-disable and a run log's size; the RESULT has neither and does not list them under "Not measured". Measure them or list them (principle 2). Found in the #144 review | Sources | P3 | Todo |  | #144 review |
| `OPS-10` | Task | The shared test Postgres runs out of shared memory under the full workspace run: `53100 could not resize shared memory segment ... No space left on device` and `UnexpectedEof` on connect, in whichever binaries run many per-test databases at once (seen in `embed_tokens`, `ai_tool`, `connector_delete_deprovision`, `upload_routes`). Each passes alone. Give the test-support container a larger `/dev/shm` (`--shm-size`) or cap its connections, so a full run is green without re-runs. Also seen once in CI: the `g6-ops` auto-disable step timed out after 150 s on #160 and passed on re-run | Platform | P2 | Todo |  | review runs 2026-10-16 |
| `BI-31` | Task | Close out `BI` phase 2 (#160) for acceptance: (1) the owner signs the decision rows still marked "Owner to confirm at QA" on `chart-builder-questions.md`, `joins-without-sql.md`, `bins-groups-hierarchies.md`, `parameters.md`, `combined-charts.md` and `reusable-metrics.md`, with the date; (2) answer the two open questions in the `BI-2` plan (does a seeded role hold `metric:write`; are metric and segment definitions readable by everyone with `dashboard:read`); (3) one check on a running stack with a masked and row-filtered role across a join, a drill-down and an embed. Found in the #160 review | Dashboards | P2 | Todo |  | #160 review |
| `OPS-4` | Task | Fail loudly when the application database is unreachable | Platform | P3 | Todo |  | `docs/OPERATIONS.md` |
| `OPS-6` | Feature | Log and trace viewer | Operations | P3 | Idea |  | `docs/archive/FEATURE_COVERAGE.md` |
| `BIZ-1` | Feature | Enterprise edition | Business | P3 | Idea |  | PRODUCT §1 |
| `BIZ-2` | Feature | Hosted cloud version | Business | P3 | Idea |  | PRODUCT §1 |

## Dependencies

Derived from the **Blocked by** markers above (planner, 2026-10-15, from every
spec's text as well as its header). Sorted by how much each item blocks, so
the first rows are the ones worth assigning first. Two loops were cut:
`BI-18` part A no longer waits for `QS-5` or `BI-26`; filtering a SQL-source
chart through parameters is delivered by `QS-5`, and locking a filter in an
embed by `BI-26`, both after `BI-18`'s filters exist. Release coupling is not
a blocker: each `AI-x` ships in the same release as its `BI-x` (assistant
parity), built after it.

| Item | Status | Blocked by | Blocks |
| --- | --- | --- | --- |
| `BI-18` | Building | — | `AI-2`, `AI-9` (layout part), `BI-26` (locking a dashboard filter in an embed), `QS-5` (the dashboard date filter) |
| `BI-1` | Planned | — | `AI-10`, `RPT-1` (PDF and Excel attachments) |
| `RPT-1` | Planned | `BI-1` (PDF and Excel attachments) | `AI-10`, `BI-19` (digest schedules) |
| `BI-14` | Planned | `DATA-13` (column part) | `AI-13` |
| `BI-16` | Building | — | `AI-5` (per chart type as it lands) |
| `BI-17` | Planned | — | `AI-8` |
| `BI-19` | Planned | `RPT-1` (digest schedules) | `AI-11` |
| `BI-26` | Planned | `BI-18` (locking a dashboard filter in an embed) | `AI-11` |
| `BLD-1` | Building | — | `REL-6` |
| `BLD-2` | Building | — | `REL-6` |
| `DATA-13` | Planned | — | `BI-14` (column part) |
| `DATA-4` | Planned | — | `DATA-3` |
| `DEC-8` | Open (decision) | — | `AI-11` |
| `DEC-9` | Open (decision) | — | `SRC-3` |
| `QS-5` | Planned | `BI-18` (the dashboard date filter) | `AI-12` (variables part) |
| `SEC-14` | Building | — | `SRC-4` (credentials) |
| `SEC-20` | Todo | — | `QS-3` |
| `SEC-21` | Todo | — | `QS-2` (full-result download) |
| `AI-10` | Planned | `RPT-1`, `BI-1` | — |
| `AI-11` | Planned | `BI-19`, `BI-26`, `DEC-8` | — |
| `AI-12` | Building | `QS-5` (variables part) | — |
| `AI-13` | Planned | `BI-14` | — |
| `AI-2` | Planned | `BI-18` | — |
| `AI-5` | Building | `BI-16` (per chart type as it lands) | — |
| `AI-8` | Planned | `BI-17` | — |
| `AI-9` | Planned | `BI-18` (layout part) | — |
| `DATA-3` | Planned | `DATA-4` | — |
| `QS-2` | Planned | `SEC-21` (full-result download) | — |
| `QS-3` | Planned | `SEC-20` | — |
| `REL-6` | Todo | `BLD-1`, `BLD-2` | — |
| `SRC-3` | Planned | `DEC-9` | — |
| `SRC-4` | Planned | `SEC-14` (credentials) | — |

**Ready to start** (Todo, Planned or Idea, nothing open blocks it; Idea items still need a spec):

`ADM-1`, `ADM-2`, `AI-1`, `AI-14`, `AI-19`, `AI-20`, `BI-1`, `BI-17`, `BI-20`, `BI-27`, `BI-28`, `BI-29`, `BI-3`, `BI-30`, `BI-31`, `BI-4`, `BI-5`, `BIZ-1`, `BIZ-2`, `BLD-4`, `DATA-12`, `DATA-13`, `DATA-14`, `DATA-15`, `DATA-18`, `DATA-20`, `DATA-21`, `DATA-24`, `DATA-25`, `DATA-4`, `DATA-5`, `DATA-6`, `DATA-8`, `DOC-2`, `GOV-1`, `GOV-3`, `GOV-5`, `INT-2`, `OPS-10`, `OPS-2`, `OPS-3`, `OPS-4`, `OPS-6`, `OPS-8`, `OPS-9`, `QA-1`, `QRY-1`, `QS-1`, `QS-4`, `REL-1`, `REL-2`, `REL-4`, `REL-7`, `SEC-19`, `SEC-20`, `SEC-21`, `SEC-22`, `SEC-24`, `SEC-25`, `SEC-26`, `SEC-32`, `SEC-33`, `SEC-34`, `SEC-37`, `SEC-38`, `SEC-39`, `SEC-4`, `SEC-40`, `SEC-7`, `SRC-15`, `SRC-18`, `SRC-2`, `SRC-22`, `SRC-23`, `SRC-24`, `SRC-5`, `VER-1`

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
| `AI-15` | | 2026-10-15 | |
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
| `SEC-28` | 2026-10-07 | 2026-10-14 | |
| `BLD-3` | 2026-10-14 | 2026-10-14 | |
| `SRC-20` | 2026-10-14 | 2026-10-14 | |
| `DATA-10` | 2026-10-14 | 2026-10-14 | |
| `SEC-30` | 2026-10-14 | 2026-10-15 | |
| `OPS-7` | 2026-10-15 | 2026-10-16 | |
| `SEC-3` | 2026-10-16 | 2026-10-16 | |
| `REL-5` | 2026-10-16 | 2026-10-16 | |
| `OPS-1` | 2026-10-15 | 2026-10-16 | |
| `SEC-16` | 2026-10-08 | 2026-10-15 | |
| `REL-8` | 2026-10-15 | 2026-10-15 | |
| `SEC-17` | 2026-10-08 | 2026-10-16 | |
| `SEC-18` | 2026-10-08 | 2026-10-15 | |
| `SEC-27` | 2026-10-13 | 2026-10-13 | |
| `SEC-36` | 2026-10-15 | 2026-10-15 | |
| `DOC-1` | 2026-10-15 | 2026-10-15 | |
| `SEC-13` | 2026-10-13 | 2026-10-14 | |
| `SEC-1` | 2026-10-14 | 2026-10-14 | |
| `SEC-10` | 2026-10-10 | 2026-10-15 | |
| `SEC-11` | 2026-10-10 | 2026-10-15 | |
| `SEC-12` | 2026-10-10 | 2026-10-15 | |
| `SEC-6` | | 2026-10-15 | |
| `SEC-29` | | 2026-10-15 | |
| `DATA-2` | 2026-10-14 | 2026-10-15 | |
| `BI-8` | | 2026-10-15 | |
| `BI-9` | | 2026-10-15 | |
| `AI-3` | | 2026-10-15 | |
| `AI-4` | | 2026-10-15 | |
| `SEC-31` | | 2026-10-15 | |
| `DATA-19` | 2026-10-14 | 2026-10-15 | |
| `BLD-1` | 2026-10-15 | | |
| `BLD-2` | 2026-10-15 | | |
| `SEC-35` | 2026-10-14 | 2026-10-15 | |
| `GOV-4` | 2026-10-15 | 2026-10-16 | |
| `SRC-11` | | 2026-10-16 | |
| `SRC-12` | | 2026-10-16 | |
| `SRC-13` | | 2026-10-16 | |
| `SRC-19` | | 2026-10-16 | |
| `BI-6` | | 2026-10-16 | |
| `BI-7` | | 2026-10-16 | |
| `BI-10` | | 2026-10-16 | |
| `BI-11` | | 2026-10-16 | |
| `BI-13` | | 2026-10-16 | |
| `BI-2` | | 2026-10-16 | |
| `AI-6` | | 2026-10-16 | |
| `AI-7` | | 2026-10-16 | |

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
| `SEC-9` | The natural-language query endpoints run the model's SQL without masking or row filters, and need only a sign-in | Done: PR #78. The endpoints were removed; plain-language queries now route through the chat which applies masking and row filters via the policy rewriter. Hardening after Done, in PR #105 (`8b67ee2`): the assistant's `run_sql` refuses a statement the policy forbids before sending anything to `ClickHouse` (it used to send `EXPLAIN AST` first) | 2026-10-07 |
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
