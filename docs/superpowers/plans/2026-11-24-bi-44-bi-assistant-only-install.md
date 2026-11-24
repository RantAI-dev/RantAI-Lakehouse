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

(pending slice 1; the developer appends commits, commands with counts, and
anything skipped or not verified before pushing.)
