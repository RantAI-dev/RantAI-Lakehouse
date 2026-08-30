# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project intends to adhere to [Semantic Versioning](https://semver.org/)
once a first release is tagged.

## [Unreleased]

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
