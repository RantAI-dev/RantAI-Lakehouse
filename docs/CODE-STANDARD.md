# Code standard

How code is written in this repository. It applies to humans and to AI agents
alike; the agent-specific section at the end exists because agents fail in
predictable ways that a human reviewer would not, and this repository has
paid for each of those failures at least once.

This document was derived from what the code already does well, not from a
generic style guide. Every rule below is followed by the code it was lifted
from. Where code and this document disagree, one of them is wrong — fix it,
do not quietly follow the other.

`CONTRIBUTING.md` covers the verification gate, commit format, and PR
expectations. This document is about the code itself.

---

## 1. The five principles

Everything else in this document is a consequence of these.

### 1.1 Say *why*, at the code

A module, migration, compose service, or config field opens with prose that
explains why it exists, which invariant it protects, and what it deliberately
does not do. A reader should never need `git blame` to learn why something is
shaped the way it is.

```rust
//! Resolving a `secretRef` string to a credential value — never the other
//! way around.
//! ... This module is the resolver ADR 0002
//! (`docs/adr/0002-secretref-resolution.md`) designs ...
```
— `rust/crates/lakehouse-core/src/secret.rs`

Comments inside function bodies justify a decision or point at the source of
truth. They do not restate the next line.

Comments, doc comments, docstrings, and commit messages are written in
English. The console's historical Indonesian status notes
(`src/services/index.ts`, `src/services/clients/*.ts`) are the exception
to migrate, not the precedent to extend.

### 1.2 Never fabricate

Code that cannot do something says so. It does not return a plausible-looking
result.

- An unsupported connector type returns `supported: false` with an honest
  message — "never a fabricated latency or success"
  (`rust/crates/lakehouse-api/src/connector_probe.rs`).
- A skipped maintenance verb is recorded in `skipped_verbs` with the reason —
  "not a silent no-op" (`dagster/dispar_orchestrate/maintenance.py`).
- A mock that has no real backend is retired together with its consumers,
  not left emitting invented metrics ("Removing a field meant removing its
  consumers, not optional-ing it" — the streaming mock retirement).
- A gap is written down where the reader will look: "This is a documented
  gap, not a silent one." (`docs/adr/0004-…`)

**Bypassing a choke point is allowed only with the reason at the site.**
`apiFetch`, `POLICY_TABLE`, `_assert_or_create_schema`, `ApiJson`, the
allowlisted resolver — each exists so one place enforces a rule. Code that
goes around one (a public share view calling `fetch` directly; the YAML
export bypassing `ApiJson`) says so in a comment naming why the rule does not
apply there.

### 1.3 Fail closed

Absence of configuration denies; it never silently allows.

- A route with no `POLICY_TABLE` entry is a hard 500 `route_policy_unclassified`,
  not public (`rust/crates/lakehouse-api/src/policy.rs`).
- An unset run token (`ALERTS_RUN_TOKEN`, `GOLD_EXPORT_RUN_TOKEN`) means 401,
  the "D4 shape" (`CHANGELOG.md`, Security).
- A required secret has no compose default: `${LAKEKEEPER_ENCRYPTION_KEY:?set in .env}`.
- A `secretRef` that is not on the allowlist is `NotAllowed`, distinct from
  `NotFound`, never a fall-through to a broader resolver.

### 1.4 Upstream text never reaches a response

Errors from Postgres, ClickHouse, object stores, or a dialed host are
classified into a fixed set of generic reasons before they cross an HTTP
boundary. The raw `Display` text is a data leak and, for caller-controlled
hosts, an SSRF response reader.

```rust
fn internal_message_never_leaks_source_error_text() { ... }
```
— `rust/crates/lakehouse-store/src/error.rs` is the regression guard.

Known deviation (audit, 2026-09-08): `ApiError::Internal` renders its
payload verbatim (`#[error("{0}")]`), and fourteen Phase-1 handlers wrap a
`ClickHouse` error with `ApiError::Internal(err.to_string())`, so an upstream
message can reach a 500 body. The store layer already does this right
(`StoreError::Database` → fixed `"database error"`); the `ChError` path has
not been brought up to it because the TypeScript parity corpus pins those
bodies. It is tracked as a deliberate, bounded gap — do not add a fifteenth
site.

### 1.5 Measured, not assumed

A numeric budget, a version-specific claim, or a "works/doesn't work"
statement traces to a dated, version-pinned measurement in `docs/plans/*-RESULT.md`
or `*-REMEASUREMENT.md`. When a measurement falsifies an earlier claim, the
new document says so by name ("This corrects `CLICKHOUSE-MAINTENANCE-FINDINGS.md`'s
claim that …"). Code comments cite the measurement ("measured basis:
docs/plans/P5-RESULT.md" in `ops/g4/g4_test.py`).

---

## 2. Vocabulary

These short identifiers appear in code comments, migrations, docs, and commit
messages. Use them; do not invent parallel ones. Each must resolve to a real
definition.

| Tag | Meaning | Defined in |
|---|---|---|
| `P0`–`P6` | Build phases of the lakehouse foundation | `docs/plans/LAKEHOUSE-FOUNDATION-PLAN.md` §3 |
| `G1`–`G4`, `G3a` | Acceptance gates, one per phase, each a runnable test under `ops/g*/` | same, plus `docs/plans/G*-RESULT.md` |
| `R1`–`R11` | Risk-register entries | `docs/plans/LAKEHOUSE-FOUNDATION-PLAN.md` §5 |
| `D1`–`D4` | Four named security fixes (identity gating, embed secret, write-tool block, fail-closed run token) | `CHANGELOG.md` Security section |
| `ADR NNNN` | Architecture decision record | `docs/adr/NNNN-*.md` |
| `P5 review fix (BLOCKER/SHOULD-FIX)`, `PR #35 review, blocker 2` | Provenance of a change made to satisfy a review finding | the PR review itself; cite the PR number or phase |
| `CREDENTIAL HYGIENE (review finding)` | A credential-handling defect found in review and fixed in place | inline at the fix |

When a fix answers a review finding, the comment at the fix site names the
finding, so the "why" is co-located with the code.

---

## 3. Rust

Workspace lints are the floor: `clippy::all` denied, `pedantic` warned and
promoted to errors by `-D warnings` in CI, `unwrap_used`/`expect_used`
denied, `missing_docs` warned (`rust/Cargo.toml` `[workspace.lints]`).

### 3.1 Documentation

- Every non-trivial module opens with a `//!` block: what it is for, the
  invariant it protects, its security posture, and which ADR designed it.
  Subsections (`# Why X`, `# D4: fail closed when …`) are normal.
- Every `pub fn` returning `Result` has an `# Errors` section naming which
  variant means what. `# Panics` is rare because panicking paths are avoided.
- Product names are backticked in doc comments (`` `ClickHouse` ``,
  `` `PostgreSQL` ``, `` `RustFS` ``). Do not `#[allow(clippy::doc_markdown)]`
  — there are zero such allows in the workspace; keep it that way.

### 3.2 Errors and control flow

- Outside tests: `?`, `let Some(x) = … else { return Err(…) }`, `ok_or_else`.
  Never `unwrap`/`expect`.
- Inside tests only: `#![allow(clippy::unwrap_used, clippy::expect_used)]` at
  the top of the `#[cfg(test)]` module or the integration-test file. Never a
  crate-level allow.
- Every handler's `?` converges on `ApiError` (`lakehouse-core::error`), whose
  `.status()` table is the single mapping to HTTP codes. Add a variant there
  rather than constructing status codes in a handler.
- Classify before you surface: `StoreError::Database` renders a fixed
  `"database error"`; `connector_dial::classify_sqlx_error` yields
  `"authentication failed"` / `"timed out"` / … — never `err.to_string()` into
  a body, a log line at `info`, or an `Outcome::message`.
- `#[allow(lint, reason = "…")]` always carries a *checkable* reason: a bound
  that makes a cast safe, or why a struct of bools is not a state machine.

### 3.3 Secrets

- A credential value lives in `lakehouse_core::secret::SecretValue`: hand-written
  `Debug` printing `<redacted>`, **no `Serialize` impl** (so serializing it
  fails to compile), access only through `.expose_secret()` so every use is
  visible.
- Types that carry dial information (`ConnectorDialInfo`) have no `Debug`
  impl at all. `Config`'s `Debug` skips its secret fields.
- A `secretRef` is a *reference* (`env:NAME`), never a value; the store
  rejects raw-looking secrets (`looks_like_raw_secret`).
- Any resolver handed to code that dials a caller-controlled host is an
  `AllowlistedSecretResolver` scoped to `CONNECTOR_ALLOWED_SECRET_REFS`. The
  allowlist names connector-dedicated variables (`CONNECTOR_PG_*`,
  `CONNECTOR_S3_*`), never the API's own secrets. Widening it is a code
  change with a test.
- A caller may not name an allowlisted ref in a request
  (`reject_allowlisted_secret_ref`), including in the `<user>@` segment of a
  Postgres dial string.

### 3.4 HTTP routes (axum)

- Handlers return `ApiResult<ApiJson<T>>`. `ApiJson`, not `axum::Json` — it
  pins the content-type bytes and float formatting the TypeScript parity
  corpus records.
- Every route has a `POLICY_TABLE` entry. Adding a route without one fails at
  request time (deny-by-default) and in `routes::mod::route_policy_tests`.
- `tests/route_auth.rs` sweeps `POLICY_TABLE` itself; a new entry is covered
  automatically, but a permission-gated route must be asserted in *both*
  directions: wrong principal → 403, right principal → never 401/403.
- Caller-controlled network targets go through `connector_dial::resolve_checked`
  (DNS-resolved, internal ranges blocked unless
  `CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS=true`) and a bounded
  `tokio::time::timeout`.

### 3.5 Store layer (sqlx) and migrations

- Values are bound (`$1`, `.bind(..)`). `format!` may splice **constant**
  identifier fragments (`{CONNECTOR_COLUMNS}`), never a caller value.
- `rust/migrations/NNNN_description.sql`, strictly increasing numbers,
  unique per stack. Each opens with a `--` header explaining why, citing the
  phase/finding and the code that depends on it.
- Never edit an applied migration; `sqlx::migrate!` checksums exist to refuse
  that. Add a new one.
- Mutating seeded rows uses a targeted `UPDATE … WHERE id = … AND col = '<old value>'`
  so an operator's hand-edited value survives. Seeds use
  `INSERT … ON CONFLICT DO NOTHING` with fixed ids.
- Store tests are `#[sqlx::test(migrations = "../../migrations")]`; a fresh
  database per test on a testcontainers Postgres started once per binary.
  Migrations are embedded at **compile** time — after editing one, force a
  rebuild before trusting a green run.

### 3.6 Config

- `Config::from_map(&HashMap)` is the testable core; `from_env` wraps it.
- Booleans are `env.get("X").is_some_and(|v| v == "true")` — exact string,
  not truthiness.
- Defaults are the literal dev-stack values and are visible in
  `.env.example`; a secret that must not default uses `:?` in compose.

### 3.7 Tests

- Unit tests in-module under `#[cfg(test)]`; integration tests in the crate's
  `tests/` with a shared `tests/common/mod.rs`.
- Names are assertions in `snake_case`:
  `out_of_scope_env_ref_is_rejected_not_silently_resolved`,
  `tenant_membership_is_not_used_to_scope_any_domain_query`.
- A regression guard is a standalone test named after the property that
  broke, with a doc comment saying what regressed and why the test fails if
  it happens again (`tests/security_regressions.rs`). A guard proves only the
  property it names; say so if the broader behaviour is still a gap.
- Mock third-party HTTP with `wiremock`; never hit a real service in a unit
  test.

---

## 4. TypeScript (Next.js console)

### 4.1 Layering — one direction only

```
src/services/contracts/*   types only, no imports from clients
       ↓
src/services/clients/*     fetch via apiFetch; implements the contract interface
       ↓
src/services/index.ts      binds one implementation per domain
       ↓
src/features/**            imports from "@/services" only — never clients/ or mock/ directly
       ↓
src/app/**/page.tsx        thin default export rendering the feature component
```

Adding an API response type: type in `contracts/`, method on the
`XxxService` interface, implementation in `clients/` (`get<T>(…)`), consumer
via `useService` in `features/`.

### 4.2 Types

- `type` for data shapes; `interface` only for the `XxxService` contracts.
- PascalCase types, camelCase fields mirroring the Rust API's serde
  `camelCase` output exactly (`walRetainedBytes`, `snapshotGrowthMeasured`).
- Optional fields use `?`. Small literal unions inline; widen with `| string`
  when the backend may emit values not yet enumerated
  (`status: "ok" | "warning" | "critical" | string`).
- Shared enums live in `src/lib/status.ts`; import, do not redeclare.
- Contract fields that mirror a backend column carry a doc comment naming the
  producing module and the finding/ADR (`contracts/governance.ts`
  `MaintenanceRun`).

### 4.3 Fetching, errors, state

- All requests go through `apiFetch` (`src/services/http.ts`) — the single
  401 → `/login?next=` choke point. Do not call `fetch` from a feature.
- Non-OK responses map to `ServiceError` codes (`not_found`,
  `invalid_request`, `permission_denied`, `unavailable`).
- Loads use `useService(fetcher, deps)`; mutations use `useServiceAction`.
  Pages branch on `state.status` and render `LoadingSkeleton` / `ErrorState`
  from `src/components/patterns/*`. No hand-rolled `setTimeout`, no ad-hoc
  loading booleans.

### 4.4 Components

- Files kebab-case; feature components are named exports; route files are
  `export default function Page()` and nothing else.
- `"use client"` is the first line of every feature component.
- shadcn/ui primitives in `src/components/ui/*`; repo patterns
  (`PageHeader`, `DataTable`, `FilterToolbar`, `Pill`) in
  `src/components/patterns/*`; tokens in `design-system/`. Tailwind classes
  inline; no CSS modules.
- The three `react-hooks/exhaustive-deps` disables are the known ceiling.
  Adding one needs a comment saying which dependency is intentionally
  omitted and why.

### 4.5 Mocks

Mocks are being retired, not extended. Do not add a mock for a backend that
does not exist; the remaining ones (`mock/knowledge.ts` search) are documented
in `services/index.ts` with the reason. When a field loses its backing,
remove its consumers.

### 4.6 Tests

`bun test src/lib` — colocated `*.test.ts` using `node:test` + `node:assert/strict`,
one `test("name", …)` per function. Pure logic in `src/lib` is testable;
keep it there rather than inside components.

---

## 5. Python (Dagster code location, `ops/`)

- Every module opens with a narrative docstring (10–50 lines): why it
  exists, which finding/ADR/measurement it answers, what it deliberately does
  not do. Then `from __future__ import annotations`. Built-in generics
  (`list[dict]`, `tuple[str, ...]`), not `typing.List`.
- Configuration is `@dataclass(frozen=True)` with `from_env()` reading through
  the shared `_env(name, default)` helper (stripped, empty means unset).
- HTTP is `requests` with an explicit `timeout=` and `raise_for_status()`,
  every time.
- Two failure idioms, chosen by class:
  - configuration/auth failure → `dagster.Failure` with an actionable message
    naming the exact `.env` value to set;
  - degraded-but-continuable failure → caught, logged, recorded in
    `skipped_verbs` with the reason, and the loop continues. Work already
    done (e.g. orphan files removed) is still recorded.
- ClickHouse SQL is built through `_sql_string_literal` and only ever
  interpolates fixed literals or server-controlled identifiers; never end-user
  input. Say so in the docstring when you add a statement.
- `bronze_catalog.EXPECTED_SCHEMAS` is the single owner of the registry
  schema (R10). Creating a table elsewhere, or altering one outside
  `_assert_or_create_schema`, is drift. Additive columns go there and get an
  `ALTER TABLE … ADD COLUMN` path; anything else raises `SchemaDriftError`.
- Schedules carry `default_status=DefaultScheduleStatus.RUNNING` with the
  reason inline ("a schedule that ships STOPPED never runs until someone flips
  it in the UI"). A job that cannot authenticate yet is registered without a
  schedule and says why.
- Intra-package imports must resolve (`ops/lint/check_intra_package_imports.py`,
  AST-based, enforced in CI): a commented-out symbol left imported elsewhere
  takes the whole code location down.
- Tests: `unittest.TestCase`, `unittest.mock` / `monkeypatch`, no network.
  Method names are full sentences:
  `test_original_nine_column_table_gets_missing_columns_added`,
  `test_retyped_existing_column_still_raises`.
- Gate tests (`ops/g*/g*_test.py`): readiness polling via `_wait_for`, one
  `[gN] …` line per step, budgets that cite their measured basis, `PASS` /
  `FAILED: <reason>` and a non-zero exit. Never a count without a `WHERE`
  against a Bronze Iceberg table (R11, `ops/lint/check_bare_iceberg_count.py`).

---

## 6. Compose, shell, SQL

- Service blocks open with `# ── … ──` comments explaining why the service
  exists and its failure mode.
- `${X:-default}` for dev-stack defaults; `${X:?message}` for anything that
  must not default (encryption keys). A shared token is **never** given a
  default in the compose file — the file doubles as the deployment template,
  so a default is a public credential.
- YAML mapping keys are unique per service; a duplicated key is a parse
  error that `docker compose config` will refuse.
- `$$` for a literal `$` inside `command:`. JSON payloads are built with
  `jq -n --arg`; secrets are never spliced into a string by the shell.
- One-shot init containers: `restart: "no"`, idempotent, and they **wait for
  the thing they need**. `depends_on: condition: service_healthy` is not
  enough for Postgres — the official image starts a throwaway server for init
  scripts, `pg_isready` passes against it, then it restarts. Poll a real
  `SELECT 1` (see `openfga-db-init`), or retry the operation
  (`rustfs-bucket-init`).
- Optional service groups sit behind `profiles:`. A service that must publish
  a host port on a shared network namespace gets its own proxy container
  (`trino-host-port`), never `network_mode: service:x` plus `ports:` — Compose
  accepts that; the daemon rejects it at runtime.
- Migrations: see §3.5. ClickHouse DDL lives in one place (`bronze_catalog.py`
  `TableSchema.create_ddl`) and the demo fixture is kept identical to it.

---

## 7. Documentation that travels with code

Code that changes behaviour changes these in the same commit:

- `README.md` config table: `| Variable | Purpose | Default | Required? |`,
  purpose cell cites the ADR/finding.
- `.env.example`: a comment block per variable — what it configures, which
  phase introduced it, cross-refs. Placeholder secrets are the sentinel
  `CHANGE-ME-BEFORE-ANY-NON-THROWAWAY-USE` with a note on generating a real
  one; a value that must fail closed is left empty with the reason.
- `CHANGELOG.md` `[Unreleased]`, with `### Security` bullets leading with a
  bold phrase.
- An ADR gains an `## Addendum (…)` with its own Context/Decision/
  Consequences/Verification when a later phase revisits it; a new number only
  for a new decision. `## Verification` states what was actually run.
- Any suppression — gitleaks allowlist entry, `cargo audit --ignore`,
  `allow-ghsas`, `#[allow]` — carries an inline, falsifiable justification
  naming the specific finding, and (for gitleaks) the call sites that prove
  the value is synthetic. Never allowlist a live, un-rotated secret; never
  remove an allowlist entry without proving the literal is gone from history.

---

## 8. Especially for AI agents

Each rule here corresponds to a mistake an agent actually made in this
repository. Treat them as hard constraints.

1. **Verify a claim against the code before writing it down.** A doc comment
   said a function "takes `&ConnectorSlug`, never a `&str`" while its
   signature took both. Read the signature; then write the sentence.
2. **Never weaken a guard to get green.** Not a test assertion, not a lint,
   not a `:?`, not an allowlist, not a threshold. If a test is wrong, fix the
   test and say in the commit message why it was wrong.
3. **Never delete a security exception on the claim that it is unneeded**
   without proving it (`git log -S`, gitleaks over history). One was removed
   with "nothing left to need an exception for" while the key was still in
   commit history.
4. **Reuse before you write.** Grep for the helper first (`connector_dial`,
   `classify_sqlx_error`, `_sql_string_literal`, `mockCall`, `useService`).
   Two byte-identical copies of an SSRF guard in two modules is a review
   finding, not a solution.
5. **No default credentials in a template.** A `${TOKEN:-dev-token}` in
   compose silently opens the endpoint the token protects. Leave it empty and
   make the consumer fail loudly with the reason.
6. **Tests run against this branch's code.** In a stacked branch, a test that
   constructs a config with a field a later branch adds is broken here, even
   if CI does not run it here. Check the branch's own signatures.
7. **A green run is one you actually ran, from a fresh build, in the
   foreground.** Report the exact command and counts. "Should pass",
   "clippy-only", or a test binary compiled before a migration changed are
   not verification. If you could not run something, write *not verified* and
   why.
8. **Compose changes are verified by `up`, not by `config`.** `config`
   accepts shapes the daemon rejects, and it does not see a Postgres restart
   race. Run the affected profile from a clean project name and tear it down.
9. **Resolve conflicts hunk by hunk.** `git checkout --theirs` on a file both
   sides changed throws away one side's work (it dropped 443 lines of a
   DELETE handler once). After any resolution: recompile, re-run the affected
   tests.
10. **Fixes live in the branch that owns the code.** In the stacked chain
    (p0 → p1 → … → compose-fixes) a fix for #31 lands on `lakehouse/p5-cdc`,
    then the branches above are rebased. A fix parked on the top branch
    leaves the lower PR merging a broken state.
11. **Never commit build output.** `__pycache__/`, `*.pyc`, `dagster/build/`,
    `*.egg-info/`, `rust/target/`. Run `git status` before every commit and
    read it.
12. **No secrets, hostnames, IPs, ports of real deployments, or client names
    in code, docs, or commit messages.** The compose file is the deployment
    template and the repository is public.
13. **Cite the finding.** When a change answers a review item, name it at the
    fix site (`P5 review fix (BLOCKER)`, `PR #35 review, blocker 2`, `R10`,
    `D4`) and in the commit body.
14. **Prefer "unsupported, honestly" over "works, approximately."** A
    connector type this build cannot dial returns `supported: false`. A verb
    ClickHouse refuses is recorded as skipped with the refusal. Never
    synthesize a latency, a row count, or a success.
15. **Foreground over background for anything you must report on.** Yielding
    to wait on a background build and then reporting "waiting" is not a
    result. Run it, read it, report it.

---

## 9. Before you commit code

Not the PR checklist — the code-writing one. Run from the branch you are on:

```bash
# Rust
cd rust && cargo fmt --check \
  && cargo clippy --workspace --all-targets --all-features -- -D warnings \
  && cargo test --workspace
# Frontend
bun run typecheck && bun run lint && bun run test
# Python
python3 ops/lint/check_intra_package_imports.py
python3 ops/lint/check_bare_iceberg_count.py
(cd dagster && python -m pytest dispar_orchestrate -q)
# Compose (config is necessary, not sufficient — see §8.8)
docker compose --profile '*' config --quiet
# Hygiene
git status                                   # nothing generated staged
gitleaks git --log-opts="<base>..HEAD" --config .gitleaks.toml --redact
```

Then read your own diff once as the reviewer will: every new comment true,
every new error path classified, every new route in `POLICY_TABLE`, every new
env var in README and `.env.example`, every number traceable.
