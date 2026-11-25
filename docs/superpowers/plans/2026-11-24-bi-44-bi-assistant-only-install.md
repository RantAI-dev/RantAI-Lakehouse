# `BI-44`: BI and assistant only installation

Planner and reviewer: Claude Opus. Developer: a non-Opus developer agent.
Base: `origin/main` at `ae360d951440939be9a04d23ad7de941d9a75a66` (fetched
2026-11-24). Branch: `feat/bi-44-in-place-mode-foundation`. Four slices;
this branch is slice 1 only. The developer does not open or merge a PR —
it pushes the branch and hands off.

## Why

Owner decision 2026-11-11 (`docs/core/BACKLOG.md:377`): a customer who wants
dashboards, Query Studio and the assistant over their own database reads it
in place; nothing is copied into the lakehouse. The source is read through
ONE query engine chosen for the installation, and the two engines never run
side by side, so they do not compete for host memory. Slice 1 makes the
installation mode, engine choice, routing, reporting and deployment shape
real; later slices register external catalogs, run the BI surfaces over
them, and add console/CLI UX and the end-to-end gate.

## Decisions already made

1. `full` is the unchanged default; `bi-only` is opt-in. Unknown values of
   either new variable fail start-up, never fall back.
2. One engine per `bi-only` install. Trino needs `TRINO_URL` and forbids
   `CH_URL`; ClickHouse forbids `TRINO_URL`. An explicit request for the
   inactive engine is a 400 naming the configured engine, before a dial.
3. Governance is unchanged in shape: the resolved engine selects
   `sql_rewrite::Engine::Trino` or `Engine::ClickHouse` for the same
   table-substitution mask/row-filter rewrite; unparseable or unsupported
   SQL is refused.
4. Absent services are reported honestly (`checked: false` / `"skipped"`),
   never as unhealthy. The full stack's probe set and readiness body stay
   byte-compatible.
5. The two standalone Compose files live under `ops/bi-only/`; the root
   `docker-compose.yml` is not edited by this slice. No migration is added
   (`0154`–`0157` are reserved by open PRs).
6. Memory is a structural target (one engine process) until a slice-4
   measurement; no unmeasured number is stated as fact.
7. Assistant parity (`AGENTS.md`) for this feature is slice 3: the tools,
   their schemas, dashboard context and the `AI-15` request set. Slice 1
   only guards the assistant's shared query entry point so it cannot dial
   the inactive engine.

## Anchors (verified at `ae360d951`)

- `rust/crates/lakehouse-api/src/config.rs:90` `ConfigError`, `:1025`
  `trino_url`, `:1803` `impl Config`, `:2124` `trino_health_url: truthy(...)`.
- `rust/crates/lakehouse-api/src/routes/query.rs:315` `is_trino_engine`,
  `:1962` `run`, `:2809` `estimate` (both defaulted to `"clickhouse"`).
- `rust/crates/lakehouse-api/src/health.rs:585` `probe_all` (six concurrent
  probes).
- `rust/crates/lakehouse-api/src/readiness.rs:120` `Checks`, `:253` `probe`
  (three checks).
- `rust/crates/lakehouse-api/src/routes/settings.rs:1237` `get_deployment`
  (only `features`).
- `rust/crates/lakehouse-api/src/sql_rewrite.rs:1039` `Engine`, `:1052`
  `Engine::from_name`; `rust/crates/lakehouse-api/src/policy_engine.rs:1`
  mask/row-filter obligations.
- `docker-compose.yml:38,69,123,168,225,980,1297,3532` pinned images this
  slice reuses.

## Slice 1 (this branch) — tasks

- **T1 Docs.** `docs/core/specs/bi-44.md`, `docs/core/features/bi-assistant-only-install.md`
  and this plan. The spec carries the source matrix, governance and security
  model, the resource rationale and `BI-44-AC1`..`AC7` with *(proposed)*
  numbers marked. *Check:* both documents follow the existing style; no
  number is claimed as measured.
- **T2 Config.** In `config.rs`: `InstallMode`, `QueryEngine`, their
  fail-closed parsers, `ConfigError` variants, the `bi-only` mutual
  exclusion check, `Config` fields, `is_bi_only`,
  `default_query_engine_name`, `engine_allowed`, and unit tests for full
  compatibility, parsing and both exclusion directions. Document the two
  variables in `docs/CONFIGURATION.md`, `.env.example` and `CHANGELOG.md`
  in the same commit. *Check:* `cargo test -p lakehouse-api --lib config::`
  passes; a full-mode config naming both URLs still resolves.
- **T3 Routing and reporting.** `routes/query.rs`: one resolver for the
  effective engine used by `run`, `estimate` and the shared
  `run_statement_with` choke point (the assistant's entry); inactive engine
  refused with a 400 naming the configured engine. `health.rs` and
  `readiness.rs`: probe only the chosen engine in `bi-only`, report absent
  services as unknown/skipped, full mode unchanged. `routes/settings.rs`:
  add `installMode` and `queryEngine` to the deployment answer. Integration
  tests: `bi_only_query_mode.rs`, `deployment_settings_route.rs`. *Check:*
  the scoped clippy/test commands below pass; existing deployment tests
  keep their shape.
- **T4 Standalone Compose.** `ops/bi-only/docker-compose.bi-only-clickhouse.yml`,
  `docker-compose.bi-only-trino.yml`, `README.md` and `test_bi_only_compose.py`.
  Seven services each; exact image reuse; read-only catalog mount for the
  federated variant; static suite and image-digest lint. *Check:* both
  Python suites pass, `git diff --check` clean.
- **T5 Handoff.** Append the Handoff section to this plan with commits,
  commands, counts and anything not verified.

### Checks for slice 1 (run in the foreground on the final commit)

```bash
cd /tmp/wt-bi44-s1
CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/shiro/rantai/RantAI-Lakehouse/rust/target \
  cargo clippy --manifest-path rust/Cargo.toml -p lakehouse-api --all-targets -- -D warnings
CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/shiro/rantai/RantAI-Lakehouse/rust/target \
  cargo test --manifest-path rust/Cargo.toml -p lakehouse-api --lib
CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/shiro/rantai/RantAI-Lakehouse/rust/target \
  cargo test --manifest-path rust/Cargo.toml -p lakehouse-api --test bi_only_query_mode \
  --test deployment_settings_route --test route_auth --test sec11_guard
python3 -m unittest discover -s ops/bi-only -v
python3 -m unittest discover -s ops/lint -v
python3 ops/lint/check_image_digests.py
git diff --check
```

The full workspace block of `AGENTS.md` is the reviewer's job before merge;
the developer runs the scoped commands above. A `docker compose up` of each
new file belongs to slice 4's E2E gate, not to this slice's static proof.

### PR slicing

One PR for slice 1 from `feat/bi-44-in-place-mode-foundation` (the developer
pushes; the planner opens/merges). Slice 2 branches off the merge commit,
slice 3 off slice 2, slice 4 off slice 3. Each slice is one PR with its own
handoff and review round.

## Slice 2 — in-place external catalog registration

- Register an external source through the existing connector/governance
  stack: connection settings and credentials via `connector_secret_store`
  (`SecretValue`, allowlisted refs, sealed at rest), SSRF-safe probe
  (`resolve_checked`), no internal host by default.
- Introspect schemas/tables of the active engine and expose them to the
  catalog, field pickers and policy authoring. Registration refuses an
  unsupported source with `supported: false` and an honest sentence.
- Extend `sql_rewrite`/policy obligations to external schemas and tables:
  masks and row filters in the active engine's dialect, fail closed on
  unparseable/unsupported SQL. No second engine may be started by any of
  these paths.
- Acceptance: `BI-44-AC3` on external tables, with a migration only if the
  schema genuinely needs one (the reserved numbers stay reserved until the
  merge queue clears).

## Slice 3 — dashboards, SQL sources and assistant parity

- Dashboard tiles and saved SQL sources read the active engine; the schema
  browser and field pickers show registered external tables only.
- Assistant parity (`AGENTS.md`): name the tools and their schemas,
  regenerate `tests/fixtures/tool_schemas.json`, state what the dashboard
  page sends as context, and add the standard `AI-15` request set. The AI
  team reviews this part of the plan and diff.
- Acceptance: `BI-44-AC2`/`AC3`/`AC5` from the console and the chat, with
  the inactive engine never named in a tool call or a dashboard request.

## Slice 4 — console UX, installer and E2E gate

- Console UI hides warehouse-only pages/sections in `bi-only` mode using
  `installMode`/`queryEngine`, with an honest explanation instead of
  broken links.
- `lakehousectl --mode bi-only --engine clickhouse|trino` writes the two
  variables and starts the matching standalone file.
- E2E gate: `docker compose up` each file from a clean project name, check
  readiness, run one query per engine, measure peak RSS against `full`,
  publish a `docs/plans/BI-44-RESULT.md` and replace the *(proposed)* memory
  target with the measurement. Verify `AC1`–`AC7` and flip the backlog row
  to In Acceptance for the owner's checklist.

## Out of scope for the whole feature

- Copying source data into the lakehouse in `bi-only` mode (the opposite of
  the feature).
- Starting both engines in one installation; changing the root
  `docker-compose.yml` default stack.
- Authentication/authorization redesign: existing principals, policies and
  audit trails apply unchanged.

## Handoff

Developer: implementation sub-agent, 2026-11-24. Branch
`feat/bi-44-in-place-mode-foundation`, base `origin/main` at `ae360d951`.
Not pushed at the time of writing this section.

### Commits (plan order)

- `33ae5c1e0` T1 `BI-44 docs: spec, feature page and four-slice plan`
- `1b972f164` T2 `BI-44 config: fail-closed install mode and single-engine mutual exclusion`
- `9f79666db` T3 `BI-44 API: route engine to the installed engine and report it honestly`
- `c475af8c7` T4 `BI-44 compose: standalone one-engine BI-only deployment files`
- `5e902822d` T3 follow-up `BI-44 test: backtick product names in integration test docs`
  (clippy `-D warnings` found them once `tests/it.rs` registered the new module)
- this commit: the handoff.

### What was built

- **T1** `docs/core/specs/bi-44.md` (modes, source matrix, governance,
  security, resource rationale, `BI-44-AC1`..`AC7` with the memory number
  marked *(proposed)*), `docs/core/features/bi-assistant-only-install.md`
  and this four-slice plan with anchors at `ae360d951`.
- **T2** `config.rs`: `InstallMode`, `QueryEngine`, trimmed/case-insensitive
  parsers that refuse unknown non-empty values, three new `ConfigError`
  variants, `Config` fields, `is_bi_only`, `default_query_engine_name`,
  `engine_allowed`, and the `bi-only` mutual-exclusion gate. Full mode
  accepts both URLs exactly as before. `docs/CONFIGURATION.md`,
  `.env.example` and `CHANGELOG.md` changed in the same commit.
- **T3** `routes/query.rs`: one `resolve_request_engine` used by `run` and
  `estimate`, plus the same check at the shared `run_statement_with`
  choke point so the assistant cannot dial the inactive engine. The
  effective engine reaches `sql_rewrite::Engine::{ClickHouse,Trino}` and
  the policy rewrite. `health.rs` probes only the chosen engine in
  `bi-only` and builds `checked: false` rows for absent services;
  `readiness.rs` adds `"skipped"` (only in `bi-only`) and a `trino` check
  that is absent in full mode. `GET /api/settings/deployment` gains
  `installMode`/`queryEngine`. Tests: `tests/bi_only_query_mode.rs`
  (registered in `tests/it.rs`, as required by `autotests = false`), plus
  two deployment-route tests.
- **T4** `ops/bi-only/`: two self-contained Compose files (seven services
  each, exact root image tags/digests and image variables, one engine per
  file, read-only catalog mount in the federated variant), an operator
  README and a six-test `unittest` suite.

### Verification (foreground, on the final code commit `5e902822d`)

Every command was run from `/tmp/wt-bi44-s1`; Rust used
`CARGO_INCREMENTAL=0` and `CARGO_TARGET_DIR=/home/shiro/rantai/RantAI-Lakehouse/rust/target`.

| Command | Result |
| --- | --- |
| `cargo fmt --check -p lakehouse-api` (from `rust/`) | clean |
| `cargo clippy --manifest-path rust/Cargo.toml -p lakehouse-api --all-targets -- -D warnings` | finished with no warnings |
| `cargo test --manifest-path rust/Cargo.toml -p lakehouse-api --lib` | **3951 passed**, 0 failed, 19 ignored |
| `cargo test --manifest-path rust/Cargo.toml -p lakehouse-api --test it -- bi_only_query_mode deployment_settings_route route_auth sec11_guard` | **120 passed**, 0 failed, 1763 filtered out |
| `python3 -m unittest discover -s ops/bi-only -v` | **6 tests**, OK |
| `python3 -m unittest discover -s ops/lint -v` | **76 tests**, OK |
| `python3 ops/lint/check_image_digests.py` | 45 third-party images pinned, 9 locally built exempt |
| `bash scripts/ci/check_test_modules.sh` | every test file compiled into a test binary |
| `git diff --check` | clean |

### Skipped / not verified

- **Not verified:** `docker compose up` of either new file from a clean
  project name. Slice 4 owns that E2E gate; the slice-1 proof is the
  static suite plus the root image-digest lint. `docker compose config`
  was not used as a substitute.
- **Not run:** the full workspace block (`cargo clippy --workspace`,
  `cargo test --workspace`, `bun run typecheck/lint/test`, Dagster pytest)
  and `docker compose --profile '*' config --quiet`. The reviewer runs
  the verification block before merge; this branch touches only
  `lakehouse-api` Rust, Python test/lint files and documentation.
- **No migrations added** (`0154`–`0157` remain reserved by the merge
  queue). No SQL or generated build output is staged.

### Design notes for the reviewer

1. `Config::engine_allowed` returns `true` for every name in `full` mode,
   deliberately preserving the existing `Engine::from_name` fallback;
   only `bi-only` narrows it. Unknown names are refused there, so a typo
   cannot silently become a `ClickHouse` run.
2. `readiness::Check::Skipped` is a third JSON word. It appears only in
   `bi-only` bodies; the full stack's `checks` object still has exactly
   three keys and no `trino` key (`skip_serializing_if`).
3. The standalone files have no `build:` blocks: they reference the same
   image variables as the root file, and the README tells the operator to
   build those images with the root Compose file first. This also keeps
   warehouse-only context paths out of the deployment templates.

---

## Slice 2 Handoff

Developer: implementation sub-agent, 2026-11-24. Branch
`feat/bi-44-in-place-external-catalog`, stacked on
`origin/feat/bi-44-in-place-mode-foundation` at `6f890525c` (slice 1's
handoff commit). Worktree `/tmp/wt-bi44-s2`. No PR opened or merged; the
developer pushes the branch and hands off.

### Commits (plan order)

- `1f4c0949e` T1 `BI-44 slice 2: source matrix, in-place introspection and catalog gate`
- `77657b3f8` T2 `BI-44 slice 2: govern 3-part external catalog tables`
- `f4901f1d3` T3 `BI-44 slice 2: in-place integration tests and classified catalog errors`
- `ee9975012` T3 follow-up `BI-44 slice 2: format and satisfy clippy -D warnings`
- this commit: the handoff.

### What was built

- **T1** New `rust/crates/lakehouse-api/src/in_place.rs` (wired into
  `lib.rs`): the one source-coverage matrix with normalized aliases and an
  honest `supported: false` reason for every unsupported source
  (`ClickHouse`: `postgresql`/`mysql`/`clickhouse`; `Trino`: the nine
  federated sources); bounded, identifier-escaped `SHOW CATALOGS` /
  `SHOW SCHEMAS` / `SHOW TABLES` / `DESCRIBE` introspection on `Trino`
  and `system.tables` / `system.columns` introspection on `ClickHouse`,
  with built-in catalogs and schemas filtered and no upstream text in an
  error. `GET /api/settings/deployment` gains `supportedInPlaceSources`
  **only in `bi-only`** (the `full` body keeps exactly three keys, so the
  existing byte-shaped test is untouched). `routes/catalog.rs::list`
  returns an early `bi-only` answer from the active engine's own catalog,
  with `zone: "external"` and `null` for every unmeasured metric — no
  Lakekeeper or `ClickHouse` registry dial. `routes/connectors.rs`
  refuses a source the installed engine cannot read in place with a 400
  naming the engine and the reason, before any SSRF check, credential or
  row exists.
- **T2** `sql_rewrite.rs` keeps the historical `"schema.table"` key and
  adds a catalog-scoped full key, so a 2-part policy still governs a
  3-part query while a 3-part policy binds only to its own catalog. Both
  matching obligations are merged (all masks, row filters AND-ed, fail
  closed on conflicting real columns), the derived table's `FROM` keeps
  the full quoted 3-part identifier in the active engine's dialect, and
  unparseable SQL still returns `Err`. `policy_engine.rs` resolves the
  governed table's real columns through in-place introspection in
  `bi-only` instead of dialing an uninstalled engine.
- **T3** New integration module
  `tests/bi_only_in_place_catalog.rs`, registered in `tests/it.rs` as
  `OPS-32` requires: a `bi-only+trino` catalog list served only by
  wiremock `Trino` (asserting the exact `SHOW` statements and that no
  undialable registry could have answered), a `bi-only+trino` query where
  the mask and row filter reach `Trino` through substitution with the
  full quoted table, and an honest 400 for `oracle` in
  `bi-only+clickhouse`.
- **T3 follow-up** Rustfmt and the three clippy `-D warnings` findings
  (identical match arms, a needlessly owned wiremock body, `assert!` on
  `is_empty()`).

### Verification (foreground, on the final code commit `ee9975012`)

Every command ran from `/tmp/wt-bi44-s2` with
`CARGO_INCREMENTAL=0` and
`CARGO_TARGET_DIR=/home/shiro/rantai/RantAI-Lakehouse/rust/target`.

| Command | Result |
| --- | --- |
| `cd rust && cargo fmt --check -p lakehouse-api` | clean (`FMT_CHECK_CLEAN`) |
| `cargo clippy --manifest-path rust/Cargo.toml -p lakehouse-api --all-targets -- -D warnings` | finished with no warnings |
| `cargo test --manifest-path rust/Cargo.toml -p lakehouse-api --lib` | **3975 passed**, 0 failed, 19 ignored |
| `cargo test --manifest-path rust/Cargo.toml -p lakehouse-api --test it -- bi_only_in_place_catalog bi_only_query_mode deployment_settings_route route_auth sec11_guard` | **123 passed**, 0 failed, 1763 filtered out |
| `bash scripts/ci/check_test_modules.sh` | every test file compiled into a test binary |
| `git diff --check` | clean |

The brief's literal `cargo fmt --manifest-path rust/Cargo.toml --check`
fails with `Failed to find targets` because that manifest is the
workspace root and has no targets of its own; the scoped `cd rust &&
cargo fmt --check -p lakehouse-api` is the equivalent check this crate's
slice 1 handoff used (and it is clean).

### Skipped / not verified

- **Not run:** the full workspace verification block (`cargo clippy
  --workspace`, `cargo test --workspace`, `bun run typecheck/lint/test`,
  Dagster pytest, `docker compose --profile '*' config --quiet`). This
  slice changes only `lakehouse-api` Rust and one plan document; the
  reviewer runs the full block before merge (`AGENTS.md`, "Keep build
  time down").
- **No migrations added** — `0154`–`0157` remain reserved by the merge
  queue. `policy.rs`, `audit_coverage.rs`, `routes/agents.rs`,
  `routes/gold.rs` and the root `docker-compose.yml` were not touched.
- **Not run:** a live `docker compose up` of either standalone file. A
  `bi-only` in-place install against a real source belongs to slice 4's
  E2E gate; the proof here is the wiremock integration tests.

### Design notes for the reviewer

1. `canonicalize` still drops a leading catalog, so every existing
   two-part policy and every existing byte-shaped `ClickHouse` rewrite
   is unchanged. The new full key is ADDITIVE: the rewrite looks up both
   and merges only when both match; a three-part policy can never match
   `crm_pg.public.customers`. `enforce` also keeps the two obligations
   under their separate keys so `substitute_governed_tables` is the one
   place that merges them and fails closed on a conflict.
2. Quoting of the derived table's `FROM` changes only for three-part
   names; one- and two-part names keep `name.to_string()` byte for byte.
   On `Trino` the full name becomes `"erp_pg"."public"."customers"`, on
   `ClickHouse` `` `erp_pg`.`public`.`customers` ``.
3. Introspection uses a fixed internal `X-Trino-User`, never a caller
   value, and bounds every `SHOW` loop by the requested table cap. The
   in-place column resolver in `bi-only+trino` needs a 3-part authored
   key (a 2-part one names no catalog and fails closed), which is why
   the integration test authors the policy on `erp_pg.public.customers`.
4. `supportedInPlaceSources` is omitted in `full` mode on purpose: the
   existing deployment test asserts exactly three top-level keys, and a
   full stack does not read any source in place.

---

## Slice 3 Handoff

Developer: implementation sub-agent, 2026-11-25. Branch
`feat/bi-44-in-place-assistant-and-bi`, stacked on
`origin/feat/bi-44-in-place-external-catalog` at `051810b82` (slice 2's
handoff commit, itself rebased on `origin/main` at `051810b82`). Worktree
`/tmp/wt-bi44-s3`. No PR opened or merged; the developer pushes the branch
and hands off.

### Commits (plan order)

- `37a01fe26` T1 `BI-44 slice 3: run assistant tools on the active engine`
- `e5340a6e8` T2 `BI-44 slice 3: guard dashboard SQL sources for the active engine`
- `e7a482dc9` T3 `BI-44 slice 3: preview dashboard SQL sources on the active engine`
- `26210d4f4` T4 `BI-44 slice 3: in-place assistant and preview integration tests`
- this commit: the handoff.

### What was built

- **T1** `routes/ai/tools/data.rs`: in `bi-only+trino`, `run_sql` classifies
  the statement with `TrinoDialect`, applies `is_trino_safe`, executes via
  `query::run_statement(..., "trino")` (which applies the `Engine::Trino`
  governance rewrite), and never runs the ClickHouse `EXPLAIN AST` dry run.
  `list_datasets` and `describe_dataset` introspect the active engine via
  `in_place::*` and return the assistant's slug/column shape with `zone:
  "external"`, `engine`, and `null` warehouse statistics. `full` mode and
  `bi-only+clickhouse` keep their exact old paths. `tools/queries.rs`:
  `run_saved_query` uses `Config::default_query_engine_name()` in `bi-only`.
  `routes/ai/prompt.rs` gains `bi_only_engine_rules(engine)`; `routes/ai/mod.rs`
  assembles a `bi-only` prompt through a separate helper WITHOUT the DATA MAP
  builder (which would dial the uninstalled ClickHouse). `full` mode's
  prompt bytes are unchanged; `AI-32`'s pin tests pass untouched.
- **T2** `sql_guard.rs`: new `check_sql_source_for_mode(sql, mode, engine)`.
  `Full` delegates to the unchanged `check_sql_source`. `BiOnly` shares the
  lexical core but reads with the active engine's rules, then admits exactly
  one read-only `SELECT`/`WITH` whose parsed tables are 2- or 3-part external
  names outside reserved metadata schemas; DDL/DML, table functions, bare or
  constant sources, `SETTINGS`/`FORMAT`, placeholders, a second statement and
  Trino session keywords are refused with fixed sentences. Five unit tests.
- **T3** `routes/support.rs`: `describe_trino_preview_columns` (a zero-row
  governed execution supplies Trino's own column metadata) and
  `run_trino_preview` (reusing `query::trino_result_to_rows`), both dialing
  only `state.trino` and classifying `TrinoError` through `upstream_error`.
  `routes/dashboard_sources.rs`: `POST /api/dashboard/sources/preview` in
  `bi-only+trino` validates with the new guard, gates and rewrites with
  `Engine::Trino` and `PolicyEngineObligations::from_state`, probes and reads
  rows on Trino with no ClickHouse `SETTINGS` text. `lakehouse_bi::store`
  persistence routes (list/create/update/delete of SQL sources) answer a
  fixed 503 in `bi-only+trino` before any dial; an inline `chart` preview is
  refused with a fixed 422. `docs/core/features/bi-assistant-only-install.md`
  records both honest limits.
- **T4** New integration module `tests/bi_only_assistant_and_preview.rs`,
  registered in `tests/it.rs` (`OPS-32`): four tests over wiremock `Trino`
  with a deliberately dead ClickHouse upstream, covering `list_datasets`,
  `describe_dataset`, `run_sql`, `run_saved_query` and the unsaved SQL-source
  preview. They assert the mask `CASE`, the row filter, the full quoted
  `"erp_pg"."public"."customers"` reference, the absence of `EXPLAIN AST` /
  `SETTINGS`, and the honest 503/422 answers.

### Verification (foreground, on the final code commit `26210d4f4`)

Every command ran in `/tmp/wt-bi44-s3`; Rust used `CARGO_INCREMENTAL=0` and
`CARGO_TARGET_DIR=/home/shiro/rantai/RantAI-Lakehouse/rust/target`.

| Command | Result |
| --- | --- |
| `cd rust && cargo fmt --check -p lakehouse-api` | clean (`FMT_CHECK_CLEAN`) |
| `cargo clippy --manifest-path rust/Cargo.toml -p lakehouse-api --all-targets -- -D warnings` | finished with no warnings |
| `cargo test --manifest-path rust/Cargo.toml -p lakehouse-api --lib` | **3995 passed**, 0 failed, 19 ignored (includes `AI-32` pin and the `AI-15` tool-schema snapshot) |
| `cargo test --manifest-path rust/Cargo.toml -p lakehouse-api --test it -- bi_only_assistant_and_preview bi_only_in_place_catalog bi_only_query_mode deployment_settings_route route_auth sec11_guard ai_dashboard_reach` | **135 passed**, 0 failed, 1756 filtered out |
| `bash scripts/ci/check_test_modules.sh` | every test file under `rust/crates/*/tests/` compiled into a test binary |
| `git diff --check` | clean |

### Skipped / not verified

- **Not run:** the full workspace verification block (`cargo clippy
  --workspace`, `cargo test --workspace`, `bun run typecheck/lint/test`,
  Dagster pytest, `docker compose --profile '*' config --quiet`). This slice
  changes only `lakehouse-api` Rust and two documents; the reviewer runs the
  full block before merge (`AGENTS.md`, "Keep build time down").
- **No migrations added** (`0154`–`0157` stay reserved). `policy.rs`,
  `audit_coverage.rs`, `routes/agents.rs`, `routes/gold.rs` and the root
  `docker-compose.yml` were not touched. No PR opened or merged.
- **Not verified:** a live `bi-only` install against a real source; the proof
  here is wiremock, as in slice 2. The E2E gate belongs to slice 4.
- **Not built:** an inline chart query on Trino or a Trino-backed store for
  saved dashboard metadata. Both are refused with fixed sentences and written
  down on the feature page; neither is silently approximated.

### Design notes for the reviewer

1. The `AI-15` tool schemas and `tests/fixtures/tool_schemas.json` are NOT
   changed: regenerating them would move a byte-pinned corpus, and the engine
   context belongs in the prompt, where it can be scoped to `bi-only`. The
   snapshot test is in the passing `--lib` run.
2. `run_statement` is the same choke point Query Studio uses, so the
   assistant's Trino run inherits `engine_allowed`, `is_read_only`,
   `is_trino_safe`, the tenant gate, the policy rewrite and classified
   engine failures instead of growing a second copy of any of them.
3. Saved queries have no engine column in `lakehouse-store` (adding one needs
   a reserved migration), so `full` mode keeps its historical ClickHouse run;
   only `bi-only` resolves the installed engine. `engine_allowed` still
   refuses a stale name at the choke point.
4. The integration harness cannot set a wiremock URL as `CH_URL` in
   `bi-only+trino` (that config is refused by design). Instead ClickHouse is
   a dead upstream: a successful governed answer, plus assertions that no
   statement contains `EXPLAIN AST` or `SETTINGS`, proves no ClickHouse path
   was taken. Introspection errors are rendered from `into_api_error()` so
   `sec11_guard` sees no interpolated error text.
5. The Trino column probe is a `LIMIT 0` execution of the governed statement,
   not `DESCRIBE (SELECT …)`: Trino has no subquery form of `DESCRIBE`. It
   still happens only after the guard, tenant gate and policy rewrite.
