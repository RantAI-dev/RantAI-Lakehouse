# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project intends to adhere to [Semantic Versioning](https://semver.org/)
once a first release is tagged.

## [Unreleased]

### Added

- Console-authored pipelines now run: `authored_factory.py` builds one `authored__<id>` job, and a cron schedule when the pipeline has one, per `ready` or `paused` pipeline from the new service-only `GET /api/pipelines/runnable`. The API asks the orchestrator to reload after every change, which is why the code location now runs `dagster code-server start`.
- Edit (`PUT /api/pipelines/{id}`, `/pipelines/{id}/edit`) and delete (`DELETE /api/pipelines/{id}`) for authored pipelines, with audit events.
- `GET /api/pipelines/{id}/schedule-ticks` and a Schedule history on the pipeline page: each time a schedule was due and whether it launched, skipped or failed. The orchestrator's error text is not sent.
- Rows per run: gold export and ingest report the rows they wrote, shown as "Rows written" in the run inspector.
- `scripts/compose.sh`: `docker compose` with `GIT_SHA` set from the checkout, which the pipeline page's source view needs.
- Dashboard SQL sources: a saved read-only `SELECT` (e.g. a join across
  several `serving` marts) that a chart can use instead of one mart.
  Authored from Query Studio ("Save as SQL source") with the new
  `dashboard:sql` permission (only `*:*` holds it today); restricted to
  one `SELECT`/`WITH` over `serving.*` tables, run through the policy
  rewrite, capped at 2,000 rows and 30 s. Charts on a source follow its
  current SQL, dashboard filters apply, and the Copilot can list sources
  and build charts on them (`list_sql_sources`, `sqlSource`).
- Dashboard folders (stage 1): nested up to four levels, holding
  dashboards and SQL sources; only empty folders can be deleted. Managed
  from the dashboard title menu; "Move to folder…" in the ⋯ menu.
- Page-aware Copilot: the dashboard's tiles with their first rows and the
  active filters, or Query Studio's SQL and last result, are sent as page
  context (bounded; the API cap is now 6,000 characters).
- Chart builder: chart types in a sidebar next to the form and live
  preview, replacing the separate gallery step.
- Four chart kinds: sankey and sunburst (flow / two-level hierarchy over
  a dimension and a required breakdown), box plot (min, quartiles, max per
  category via `quantilesExact`), and calendar heatmap (daily values, up
  to the last year of data). Available in the builder, to the Copilot, and
  on SQL sources.

- Gold Exports console page: per-mart last export (`snapshotId`/
  `exportedAt`, read straight off the Iceberg table's own snapshot), an
  "Export now" action, export history from a new `console.gold_export_run`
  table, and a `GET /api/gold/export/{mart}/consumers` route that reports
  an honest `supported: false` until Trino query-history correlation is
  written (the `lakehouse-trino` client crate exists but this route does
  not yet call it) (WS6).
- `POST`/`GET /api/gold/export/{mart}`'s `check_export_token` now also
  accepts a session holding the `gold:export` permission, in addition to
  the pre-existing shared-token/service-identity paths — a deployment
  that sets `GOLD_EXPORT_RUN_TOKEN` for the Dagster schedule no longer
  locks the console's "Export now" button out for every human operator
  (WS6).
- The scheduled `gold_export_schedule` (daily 04:00) is restored. Known
  gap: unlike `agent_run_schedule`/`alerts_run_schedule`, no service
  identity is provisioned for it yet, so its nightly run currently gets
  `401` at `Policy::RequiresAuth` before `check_export_token` ever runs —
  shipped anyway per AGENTS.md rule 2 (a schedule that visibly fails is
  more honest than one withheld to hide the gap) (WS6).
- Per-tenant built-in "Main" dashboard tile catalog loaded from a JSON
  file via `BUILTIN_DASHBOARD_SPEC`, replacing the removed
  `BUILTIN_DASHBOARD_ENABLED` boolean flag (WS6).
- Dashboard list page at `/dashboards/browse`, as a gallery of cards or a
  table with search, filters and sorting. Each dashboard shows its tile
  count, owner, last-updated time and whether it is shared publicly or
  embeddable; rows offer rename, duplicate and delete. Reachable from the
  board switcher ("Browse all dashboards"), the command palette, and the
  navbar. Dashboards can now carry a one-line description, set when they
  are created or renamed and shown in the list.
- `GET /api/dashboard/boards` now returns `chartCount`, `builtin`,
  `description`, `createdBy` and `updatedAt` per board. `chartCount` is
  derived from `console.bi_chart` on each request rather than stored, so it
  cannot go stale when a chart moves between boards. `updatedAt` exposes the
  same column as the existing `createdAt` under an honest name: the table is
  a `ReplacingMergeTree(created_at)`, so that column is the version column
  and every save rewrites it.
- `POST /api/dashboard/boards` accepts `description`, and records the
  signed-in caller's display name in `created_by`.
  `PUT /api/dashboard/boards` accepts `description`.

### Changed

- `/dashboards` no longer renders a page. It resolves: to the dashboard you
  last had open, or — when you have not created one yet — to the built-in
  "Main" board, and only otherwise to the list at `/dashboards/browse`.
  Returning to Dashboards from another section therefore reopens the
  dashboard you were working on instead of making you pick it again. The
  single-dashboard canvas moved to `/dashboards/[id]`. The last-opened
  dashboard is remembered per browser and validated against the server on
  every resolve, so a deleted board never leaves you on an empty canvas.
- **Relicensed the project from Apache-2.0 to AGPL-3.0-or-later.**
  `v0.1.0` was released and remains distributed under Apache-2.0 — that
  historical release is unaffected. All source as of this change is
  licensed AGPL-3.0-or-later: `LICENSE` now carries the full AGPLv3 text
  (copyright RantAI), `rust/Cargo.toml`'s `[workspace.package] license`
  (inherited by all 12 `lakehouse-*` crates) is `AGPL-3.0-or-later`,
  `package.json` now declares `"license": "AGPL-3.0-or-later"`,
  `rust/deny.toml` carries per-crate exceptions so `cargo deny check
  licenses` accepts the first-party AGPL declaration without loosening the
  permissive-only allowlist for third-party dependencies, and `NOTICE` /
  `README.md` reflect the new license. A dependency re-audit
  (`cargo deny check licenses`) found no dependency license incompatible
  with distributing the combined work under AGPL-3.0.
- **Console honesty pass.** Every value the API could not actually measure
  now reports as not measured instead of a plausible-looking number: overview
  KPI tiles, service health, catalog asset size/freshness/usage, pipeline run
  counters and cost, digital-employee metrics, query transparency fields
  (cache hit, pushdowns, query plan), operations health/version/latency, and
  workload start times are all `null`/"unknown" until something real
  computes them, instead of hardcoded zeros or constants.
- Audit-trail deep links (query runs, approvals, connectors) now carry a real
  event id only when a real audit event exists, instead of a synthesized id
  that named no event and 404ed on click.
- Governance lineage is reported as unsupported rather than rendering a
  fixed source→bronze→silver template for every dataset; data-quality rule
  results report as not evaluated rather than defaulting to "warning".
- The connector creation wizard now runs the real connection test against
  the newly created connector and shows the actual outcome, instead of a
  local "Test passed" step that contacted nothing; connector health reports
  unknown, with no last-test time, until a probe actually runs.
- Identity records no longer serve activity timestamps that were never
  written; rotation status now matches what authentication itself enforces
  (expired once the credential's expiry has passed).
- The app shell now shows every real page unconditionally instead of hiding
  them behind a "preview" flag, and drops the always-on notification and
  presence indicators that nothing measured.
- User-facing messages, error text, and code comments in the console and API
  were translated from Indonesian to English; copilot prompt text sent to
  the model is intentionally left unchanged.
- The alerts table now distinguishes an API failure from having no alerts,
  instead of showing an empty state for both.

### Fixed

- A pipeline created in the console now belongs to its creator's tenant and appears on the Pipelines list; it used to be stored without a tenant and was invisible to every list.
- `PIPELINE_RUN_TOKEN` is now passed to `lakehouse-api` and `dagster-code-location`; it was passed to neither, so no authored pipeline could become a job.
- The authored-pipeline schedule field offers cron presets and a validated custom cron; its old free-text default ("Every hour") never fired.
- The Pipelines list shows "Never run" instead of an empty badge for a job that has never run, and schedules in words.
- Dashboard drill-down, filter values (`/api/dashboard/values`), alert
  values and digests now go through the policy rewrite like dashboard
  tiles, so masking and row filters apply there too (alerts and digests as
  the least-privileged "Dashboard Viewer" role, like embeds).
- LLM failures no longer show the provider's raw response text in the
  Copilot or the text-to-SQL agent.
- `enforce` passes no permissions to the statement classifier (it passed
  role names); sensitive `system.*` tables stay unreadable from governed
  SQL surfaces, now stated explicitly.
- A new dashboard no longer shows the previous board's charts (a stale
  load response won the race); closed selects in the chart builder show
  labels instead of raw ids; Copilot chart drafts on a SQL source preview
  correctly.
- Tests: `lakehouse-test-support` reuses one labelled Postgres container
  instead of leaking one per test binary; the connector secret allowlist
  test no longer depends on a developer `.env` (loaded by `sqlx::test`).

### Removed

- Pages and dialogs with no backend behind them: storage tiering (including
  the "Restore to Hot" dialog, which hardcoded the asset it restored),
  knowledge sources/vector jobs/semantic search, agent workflows/tools
  registry/agentic builder, governance residency/workspace settings, the
  policy-impact preview, and the query-collaboration surface. Their API
  routes remain registered for later workstreams that will rebuild them on
  real data.
- The pipeline detail view no longer synthesizes an op graph, description,
  or config summary on the client; it shows the pipeline's real run history
  with an honestly empty graph tab.
- The query-transparency estimate panel, which rendered plan stages as
  "completed" before they had run.
- Seeded pipeline and alert rows that were indistinguishable from real
  activity are pruned by migration.

### Fixed

- Sorting a table column now actually reorders the rows. `useDataTable`
  built its column whitelist from `column.id`, but TanStack derives that id
  from `accessorKey` inside the table rather than on the definition object,
  so almost every column was missing from the whitelist and the URL parser
  discarded the sort it had just written. Restoring a filter from table
  memory failed the same way. Affects every table page, not only the
  dashboard list.

## [0.1.0] - 2026-08-30

First tagged release. Everything below reflects the commit history on
`main..feat/rust-backend` (84 commits, merged via #1) — a full backend port
from the original TypeScript/Next.js API routes to a Rust/axum service, plus
the CI/security work done to prepare the repository for its first release.

### Added

- Rust workspace scaffold (`rust/`, 11 `lakehouse-*` crates) alongside the
  existing Next.js frontend, as the target of a full backend port from
  TypeScript to Rust/axum.
- `lakehouse-core`: shared error type (`ApiError`), SQL-injection-safe
  identifier newtypes (`Ident`, `SqlLiteral`), and status enums.
- `lakehouse-clickhouse`: HTTP client for ClickHouse's plain HTTP interface,
  porting `src/services/clients/clickhouse.ts`.
- axum HTTP chassis for `lakehouse-api`: config resolution, error bridge,
  shared state, health check, per-route request timeouts matching each
  original route's `maxDuration`.
- Route ports to axum: catalog, storage, overview, ops, governance
  (including lineage), query (`run`/`estimate`), pipelines (list, runs,
  trigger), dashboard (all 8 sub-routes), embed (`/api/embed/data`,
  `/api/public/dashboard/{token}`), agent (`ask`/`query`/`text-to-sql`), AI
  chat/sessions/build-status, alerts (`/api/alerts`, `/api/alerts/run`).
- New crates added during the port: `lakehouse-dagster` (Dagster GraphQL
  client), `lakehouse-embed` (signed embedding, HS256 JWT), `lakehouse-llm`
  (OpenAI-compatible chat completions client), `lakehouse-notify` (webhook +
  SMTP delivery), `lakehouse-bi` (dashboard specs + SQL builders +
  ClickHouse-backed board/chart store), `lakehouse-alerts` (threshold
  alerts + scheduled digests).
- `lakehouse-store`: Postgres-backed OLTP foundation for Phase 2
  (`console`-schema mutation state that ClickHouse is a poor fit for), with
  lazy, non-fatal connection handling.
- Phase 2 domains backed by real Postgres storage and routes: identity
  (`/api/identity/*`), governance policies/authored rules, saved
  queries/history/collaboration, pipelines (authored definitions + real
  Dagster mutations), storage/ops/overview (real `KILL QUERY`, alert
  instances), connectors, knowledge (sources + vector jobs), and digital
  employees (agents, tools, workflows, runs, approvals).
- `lakehouse-auth`: the authentication core — `Principal`, the
  `Authenticator` trait/seam, and local-password, session, and
  service-token authenticators, all reading/writing a single
  `auth_identity` table designed to hold any future identity provider as
  rows, not schema changes.
- Authentication wired into the axum router (`crate::auth`,
  `crate::policy`'s deny-by-default `POLICY_TABLE`), plus a route-policy
  completeness test that hard-fails on any route missing a policy entry.
- OIDC identity-provider support (Task 3.5): `OidcAuthenticator` as a
  resource server verifying bearer `id_token`s against a configured
  provider's JWKS, with JIT provisioning and configurable IdP-group-to-
  local-role mapping (union, not IdP-authoritative — see
  `rust/crates/lakehouse-auth/README.md`).
- Frontend: login flow, session-aware app shell, and centralized 401
  handling routed through `apiFetch`.
- Full cutover: TypeScript Next.js API routes deleted; the Rust
  `lakehouse-api` service is now the sole backend, reached via the
  `next.config.ts` `/api/*` rewrite.
- Corpus parity harness + TS/Rust spec drift guard, used throughout the
  port to verify each new Rust route's responses against golden output
  captured from the original TypeScript backend.
- Dashboards: click-to-cross-filter and drill-down records (Metabase-style),
  PDF export via print (no added dependency), a self-contained Jakarta
  choropleth geomap (no external tile dependency).
- Threshold alerts and scheduled digests (webhook + email delivery).
- Self-contained `docker compose` backend stack (Postgres, ClickHouse,
  `lakehouse-api`) with migrations run at container boot via the entrypoint,
  plus operations docs.
- `lakehouse-test-support` crate; Postgres integration tests de-ignored;
  named regression tests for four specific security properties; HTTP-level
  authorization contract tests exercising the real router end to end.
- CI restructured into fast-feedback and heavy workflows: `cargo audit`,
  `cargo deny check`, `gitleaks` (working tree and full-history scan),
  `cargo llvm-cov` coverage + CycloneDX SBOM generation, and a Docker
  build/smoke-test job — see `docs/CI.md`.
- Release-prep hygiene for open-sourcing: license, docs, and CI templates.

### Changed

- BI chart aggregate typed as an enum, closing a raw-string SQL path.
- `ChartInput` made lenient so text/kpi charts are reachable via AI chat.
- `ensure_bi_table`'s DDL bootstrap cached once per process instead of
  re-issued per request.
- Six ad-hoc SQL escapers replaced with `SqlLiteral` instead of
  quote-stripping.
- XML tool-call argument parsing made order-preserving; `buildRunId`
  omission and `<think>` tag stripping made case-insensitive in the AI
  chat path.
- Governance `GET /api/governance/{kind}` now unions authored rules into
  the response.
- Infra endpoint URLs and credentials made env-only — no internal defaults
  baked into source.
- Remaining admin routes gated on the new permissions model; ad-hoc
  `/api/*` fetch calls in the frontend routed through the central
  `apiFetch` (and its 401 handling) instead of calling `fetch` directly.

### Fixed

- Every JSON response now emits `application/json;charset=utf-8`
  consistently.
- ClickHouse and Dagster clients no longer leak internal endpoint URLs on
  transport failure.
- Invalid `SMTP_PORT` degrades gracefully (falls back to `587`) instead of
  failing boot; `SMTP_SECURE`'s effective value now folds in the
  `port === 465` rule from the original TypeScript, not just the raw env
  var.
- Request-timeout responses render as a proper JSON error envelope instead
  of a bare timeout.
- Stale `#[allow(dead_code)]` on `ApiRejection`/`ErrorBody` removed once no
  longer needed.
- BI: stopped dropping every live stored chart on the new envelope shape.
- Stale `pub mod tenant;` dropped from `lib.rs` (the tenant module is not
  wired into any route — see the Security section of the release notes for
  what this does and does not mean for tenant isolation).
- `docker compose`: `lakehouse-api`'s runtime base image matched to the
  builder's OS; built from a pinned Rust 1.96.1 `Dockerfile`.
- Nested `if let` chains collapsed for clippy on current stable.
- `must_change_password` now enforced server-side, not just as a UI hint.
- `gitleaks`'s full-history job fixed to actually detect the key it was
  missing, then to stop flagging its own scanner config and docs as new
  matches.

### Security

- **Unauthenticated API surface** — before the `lakehouse-auth` core was
  wired into the router, every route (including writes to Postgres-backed
  storage) was open. Fixed by introducing `Principal`/`Authenticator` and
  gating the router on `crate::policy`'s deny-by-default `POLICY_TABLE`.
- **`/api/identity/*` privilege escalation** — identity routes were
  auth-gated but not permission-gated, allowing any authenticated caller to
  reach admin-only identity operations. Fixed by permission-gating those
  routes (D1).
- **Embed signing secret returned over HTTP** — a dashboard-embed response
  was returning the HMAC signing secret used to sign embed tokens. Fixed by
  no longer including it in the response (D2).
- **`ai/chat` executing write tools in read-only mode** — the write-tool
  block was previously enforced only at advertisement time (tools were
  hidden from the model) but not at dispatch time, so a crafted tool call
  could still execute a write. Fixed by enforcing the block at dispatch
  (D3).
- `/api/alerts/run` now fails closed (401) when `ALERTS_RUN_TOKEN` is unset,
  instead of allowing unauthenticated calls (D4).
- `Config`'s `Debug` implementation hand-written (not derived) so secret
  fields (`ch_password`, `llm_key`, `embed_secret`, `alerts_run_token`,
  `smtp_pass`, `database_url`) can never leak into a `{:?}`-formatted log
  line; enforced further by a `check-no-secrets.sh` CI script after a
  secret was leaked once during development.

[Unreleased]: https://github.com/RantAI-dev/RantAI-Lakehouse/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/RantAI-dev/RantAI-Lakehouse/releases/tag/v0.1.0
