# `OPS-38`: lighter CI runs so the queue recovers

Planner and reviewer: Claude Opus. Developer: Gemini 4 Hydrogen (owner
2026-11-21: "minta antigravity kerjain", "gw butuh ci cepet"). Base: `main`.
Branch: `ci/ops-38-lighter-runs`. A CI-workflow PR: merged after its fast
checks (scope self-tests, repo lints, gitleaks), per the owner's rule.

## Why (measured 2026-11-20/21)

- A full run alone takes 24–27 min (the nightly schedule runs), so OPS-24 and
  OPS-25 still work.
- But at most 20–21 hosted jobs run at once (the org's cap), and on 11-20 there
  were 178 pull-request runs (48 on 11-19). Jobs waited 30–93 min for a runner
  and ran 10–28 min.
- Two causes inside the workflow:
  1. **Draft PRs run the full workflow.** Stacked and unfinished PRs (e.g. six
     stacked dashboard branches pushed together) take ~40 jobs each.
  2. **The gate map falls back to every gate.** Any `rust/crates/lakehouse-api`
     file without a `path` rule in `scripts/ci/gate_paths.txt` selects all nine
     groups (fail closed, OPS-25 D5). A change to `src/sql_lineage.rs` ran 39 jobs.

## Decisions (planner, owner-directed 2026-11-21)

- **D1. A draft PR runs only the fast checks.** On `pull_request` events whose
  PR is a draft: `Detect Change Scope`, `Repo lints`, `Rust · fmt`, the
  frontend job, gitleaks/security (separate workflow, unchanged) and
  `ci-required` run; every other job (Rust build, clippy, MSRV, tests, gates,
  images, Dagster) is skipped. `ci-required` passes on a draft when the fast
  checks pass and says in its log that the PR is a draft. Add
  `types: [opened, synchronize, reopened, ready_for_review]` to the
  `pull_request` trigger so marking a PR ready starts the full run. The
  `changes` job exports `draft` (`github.event.pull_request.draft`, compared to
  exactly `true`, fail closed: unknown → not a draft → full run).
- **D2. Narrow the gate map with evidence only.** Add `path` rules to
  `scripts/ci/gate_paths.txt` for `rust/crates/lakehouse-api/src/**` (and
  `lakehouse-store/src/**`) files ONLY where it is proven which gates can reach
  the code: a file maps to the groups whose gate scripts (`ops/g*`, the gate
  jobs' commands in `ci.yml`) call a route defined in that file, or to `none`
  when no gate calls any route of it and no gate-exercised module imports it.
  Record the evidence for each new rule as a comment line above it (the route
  paths grepped and the result). A file whose reach is unclear keeps the
  default (all groups). Start with the files PRs touched most (export,
  csv_export, csv_safe, sql_lineage, query, query_history, reports, dashboard,
  render, ai/*, alerts, pipelines compile/forms), and stop there.
- **D3. Nothing else changes**: no check dropped from a ready PR, no change to
  the push proof, the nightly full run or the kill switch `CI_PUSH_FULL`.

## Tasks

- **T1 (D1):** the trigger types, the `draft` output, the `if:` on every heavy
  job (one helper expression, not 30 copies, if the workflow allows: e.g. an
  output `heavy = draft != 'true'` from `changes`), and `ci-required` reading
  it (`scripts/ci/required_gate.sh` and its self-test
  `scripts/ci/tests/test_required_gate.sh`: a draft with skipped heavy jobs
  passes; a ready PR with a skipped heavy job still fails unless its group was
  not selected). Self-tests must fail on the old script.
- **T2 (D2):** the new `path` rules with evidence comments; add cases to
  `scripts/ci/tests/test_detect_change_scope.sh` for two of them (a file mapped
  to `none` selects no gate; a mapped file selects only its groups); run
  `python3 ops/lint/check_gate_groups.py` and its test.
- **T3:** `docs/CI.md` gains a short "Draft PRs" paragraph and the rule for
  adding a `path` line with evidence. `CHANGELOG.md` one line. Backlog row
  `OPS-38` (Done, this PR) and a Dates row.

## Verification

```bash
bash scripts/ci/tests/test_detect_change_scope.sh
bash scripts/ci/tests/test_required_gate.sh
bash scripts/ci/tests/test_detect_push_proof.sh
python3 ops/lint/check_gate_groups.py && python3 -m pytest ops/lint/test_check_gate_groups.py -q
python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/ci.yml'))"
git status
```

No cargo, bun or docker is needed.

## Out of scope

Self-hosted runners, the merge queue, the flaky tests (`REL-27`, the login
selection test: a separate PR).

## Handoff

### Commits (branch `ci/ops-38-lighter-runs`, base `65bd35c02`)

1. **`1ea687204`** — `ci(ops-38): run only fast checks on draft PRs (D1)` (T1)
   - `.github/workflows/ci.yml`: added `types: [opened, synchronize, reopened, ready_for_review]` on `pull_request`; passed `PR_DRAFT: ${{ github.event.pull_request.draft }}` to `detect_change_scope.sh` and `required_gate.sh`; exported `draft` and `heavy` (`steps.scope.outputs.draft != 'true' && 'true' || 'false'`) from `changes`; gated all 33 heavy jobs and the two `REL-20` nextest steps inside `ci-required` on `needs.changes.outputs.heavy == 'true'`.
   - `scripts/ci/detect_change_scope.sh`: emits `draft=true` iff `EVENT_NAME=pull_request` and `PR_DRAFT=true`; otherwise fails closed to `draft=false` (logging `::warning::` on unexpected/non-PR values).
   - `scripts/ci/required_gate.sh`: on `EVENT_NAME=pull_request` with `PR_DRAFT=true`, logs the draft PR state and allows heavy jobs to be `skipped`, while still requiring fast checks (`changes`, `repo-lints`, `verify` when in scope, `fmt` when in scope) and failing on any failed/cancelled job.
   - `scripts/ci/tests/test_detect_change_scope.sh` (+6 D1 assertions) and `scripts/ci/tests/test_required_gate.sh` (+16 D1 assertions).
2. **`21165fdb9`** — `ci(ops-38): narrow gate_paths.txt with proven reach evidence (D2)` (T2)
   - `scripts/ci/gate_paths.txt`: added 13 evidence-backed `path` rules with `# OPS-38 D2:` comments immediately above each rule.
   - `scripts/ci/tests/test_detect_change_scope.sh` (+13 D2 assertions covering all 13 new rules).
3. **T3 commit** (`HEAD`, after this handoff) — `docs(ops-38): document draft PRs, gate-map evidence rule, backlog, and handoff`
   - `docs/CI.md` (`### Draft PRs (OPS-38)` and `**Adding a path line requires evidence (OPS-38 D2).**`), `CHANGELOG.md`, `docs/core/BACKLOG.md` (`OPS-38` in `Now` and `Dates`), and this `## Handoff`.

### Verification (foreground)

Ran on `21165fdb9` + T3 doc updates (before appending this Handoff):
- `bash scripts/ci/tests/test_detect_change_scope.sh`: **104 passed, 0 failed** (exit 0).
- `bash scripts/ci/tests/test_required_gate.sh`: **106 passed, 0 failed** (exit 0).
- `bash scripts/ci/tests/test_detect_push_proof.sh`: **38 passed, 0 failed** (exit 0).
- `python3 ops/lint/check_gate_groups.py`: passed (`gate groups are consistent: every gate job has one group, its if: names it, and every gate file maps to a group`; exit 0).
- `python3 -m pytest ops/lint/test_check_gate_groups.py -q`: `No module named pytest` in system `python3`; fallback `python3 -m unittest ops/lint/test_check_gate_groups.py -v`: **Ran 6 tests, OK** (exit 0).
- `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/ci.yml'))"`: valid YAML (exit 0).
- `git status`: only `CHANGELOG.md`, `docs/CI.md`, `docs/core/BACKLOG.md` modified prior to appending this Handoff.

### Old-script failure checks (how checked)

- **T1 (`test_required_gate.sh` against `65bd35c02`)**: ran the updated `scripts/ci/tests/test_required_gate.sh` in an isolated temp copy containing `65bd35c02:scripts/ci/required_gate.sh`; reported **102 passed, 4 failed** (the 3 draft pass cases failed exit 0 expected / got 1, and `draft PR did not log draft state` failed `rc=1`).
- **T1 (`test_detect_change_scope.sh` against `65bd35c02`)**: ran the updated `scripts/ci/tests/test_detect_change_scope.sh` in the same isolated temp copy containing `65bd35c02:scripts/ci/detect_change_scope.sh` and `65bd35c02:scripts/ci/gate_paths.txt`; exited non-zero (`set -e` aborted on the first D1 test `draft=true on pull_request with PR_DRAFT=true` because the old script did not write `draft=` to `GITHUB_OUTPUT`).
- **T2 (`test_detect_change_scope.sh` against `1ea687204:scripts/ci/gate_paths.txt`)**: ran the updated `scripts/ci/tests/test_detect_change_scope.sh` with `GATE_MAP` pointing at `1ea687204:scripts/ci/gate_paths.txt`; reported **91 passed, 13 failed** (all 13 new D2 assertions returned `all` groups instead of the narrowed groups/`none`).

### D2 evidence summary

Each of the 13 added rules in `scripts/ci/gate_paths.txt` carries an inline `# OPS-38 D2:` comment above it recording the exact routes grepped across `ops/`, `dagster/`, and `.github/workflows/ci.yml` plus recursive Rust callers/importers:

- **7 rules mapped to `none`** (0 gate hits across `ops/`, `dagster/`, `.github/workflows/ci.yml`; no gate-exercised Rust importer):
  1. `rust/crates/lakehouse-api/src/routes/audit_events.rs` — `/api/governance/audit/events`, `/events/{id}`, `/actors`, `/export.csv` (`g3a` hits `/api/governance/audit` in `routes/governance.rs`); only `routes/mod.rs` imports it.
  2. `rust/crates/lakehouse-api/src/routes/query_cancel.rs` — `POST /api/query/cancel`; only `routes/mod.rs` and `routes/ai/tools/limits.rs` (`none`) import it.
  3. `rust/crates/lakehouse-api/src/routes/query_limits.rs` — `/api/settings/limits`; only `routes/mod.rs` imports `routes::query_limits` (`src/query_limits.rs` middleware stays on `all`).
  4. `rust/crates/lakehouse-api/src/routes/lineage/used_by.rs` — `GET /api/governance/lineage/used-by`; only `routes/mod.rs` imports it.
  5. `rust/crates/lakehouse-api/src/routes/lineage/charts.rs` — `GET /api/dashboard/charts/{id}/lineage`; only `routes/mod.rs` and `routes/lineage/used_by.rs` (`none`) import it.
  6. `rust/crates/lakehouse-api/src/routes/storage.rs` — `/api/storage`, `/api/storage/policies`, `/operations`, `/restore`; only `routes/mod.rs` imports it.
  7. `rust/crates/lakehouse-store/src/storage.rs` — imported only by `routes/storage.rs` (`none`).

- **6 rules mapped to narrowed gate groups**:
  8. `rust/crates/lakehouse-api/src/pipeline_forms.rs` → `pipelines` (no routes; imported only by `pipeline_compile.rs` [`pipelines`]).
  9. `rust/crates/lakehouse-api/src/dashboard_visibility.rs` → `governance,alerts` (private `mod`; imported only by `routes/dashboard_access.rs` and `routes/embed.rs` [`governance,alerts`]).
  10. `rust/crates/lakehouse-api/src/routes/iceberg_door.rs` → `pipelines,governance` (`/api/iceberg` hit only by `ops/gold_export/gold_export_test.py` [`pipelines`]; imported only by `gold_export.rs`, `routes/gold.rs` [`pipelines,governance`], and `routes/mod.rs`).
  11. `rust/crates/lakehouse-store/src/outside_reads.rs` → `pipelines,governance` (imported only by `routes/gold.rs` and `routes/iceberg_door.rs` [`pipelines,governance`]).
  12. `rust/crates/lakehouse-api/src/routes/catalog_drop.rs` → `upload,ingest,pipelines,governance` (`GET`/`POST /api/catalog/{id}/drop` and `GET /api/catalog/dropped/tables` hit only by `ops/g9/upload_test.py` [`upload`]; imported only by `routes/uploads.rs` [`upload,ingest`], `routes/catalog_restore.rs` [`pipelines,governance`], and `routes/mod.rs`).
  13. `rust/crates/lakehouse-api/src/sql_lineage.rs` → `pipelines,governance,alerts,cdc,ingest` (no routes; imported only by `routes/lineage/columns.rs`, where `sql_lineage::{map_select,statement_target}` is called by `record_sql_source` [`routes/dashboard_sources.rs`: `governance,alerts`], `record_pipeline_run` [`put_pending` written only by `routes/authored_pipelines.rs`: `pipelines`], and `run_builds_pass` [`routes/alerts.rs`: `alerts,cdc,ingest,pipelines`]).

*(Note on D2 named examples already covered by existing rules before `OPS-38`: `gold_*` → `pipelines,governance`, `routes/dashboard*` / `routes/embed.rs` → `governance,alerts`, `routes/ai/*` → `none`, `routes/alerts.rs` → `alerts,cdc,ingest,pipelines`, `pipeline_compile.rs` → `pipelines`; `routes/query.rs` and `store/src/queries.rs` stay on the `all` default because gates across groups hit `/api/query/*`.)*

### Left on the `all` default

Verified via `bash scripts/ci/detect_change_scope.sh --classify` across all 267 `.rs` files under `rust/crates/lakehouse-api/src/` and `rust/crates/lakehouse-store/src/` (**97 files** remain on `all`; 170 files map to narrowed groups or `none`):

- **`rust/crates/lakehouse-api/src/` (71 files)**: `audit_coverage.rs`, `auth.rs`, `bounded.rs`, `config.rs`, `connector_deprovision.rs`, `connector_discover.rs`, `connector_listing.rs`, `connector_probe.rs`, `connector_secret_store.rs`, `crypto_provider.rs`, `db_roles.rs`, `error.rs`, `health.rs`, `internal_hosts.rs`, `json.rs`, `lakehouse_catalog.rs`, `lakekeeper_token.rs`, `lib.rs`, `main.rs`, `next_run.rs`, `pg_probe.rs`, `pipeline_lag.rs`, `pipeline_source.rs`, `policy.rs`, `policy_engine.rs`, `query_limits.rs`, `query_params.rs`, `readiness.rs`, `routes/agents.rs`, `routes/audit_scope.rs`, `routes/auth.rs`, `routes/bronze_registry_fixture.rs`, `routes/catalog_governance.rs`, `routes/catalog_query.rs`, `routes/connector_discovery.rs`, `routes/connector_runs.rs`, `routes/connectors.rs`, `routes/governance.rs`, `routes/identity.rs`, `routes/lakehouse.rs`, `routes/lineage.rs`, `routes/lineage/builds.rs`, `routes/lineage/columns.rs`, `routes/mod.rs`, `routes/ops.rs`, `routes/pipelines.rs`, `routes/quality.rs`, `routes/quality_anomaly.rs`, `routes/quality_incidents.rs`, `routes/quality_load.rs`, `routes/quality_schedules.rs`, `routes/query.rs`, `routes/schema_changes.rs`, `routes/schema_versions.rs`, `routes/settings.rs`, `routes/support.rs`, `routes/usage.rs`, `runtime_settings.rs`, `rustfs_client.rs`, `secret_crypto.rs`, `secret_manager/mod.rs`, `secret_manager/refs.rs`, `sql_guard.rs`, `sql_lex.rs`, `sql_rewrite.rs`, `state.rs`, `table_access.rs`, `tenant.rs`, `tenant_scope.rs`, `tls_proxy.rs`, `upstream_error.rs`.
- **`rust/crates/lakehouse-store/src/` (26 files)**: `agents.rs`, `annotation.rs`, `audit.rs`, `column_lineage.rs`, `connector_discovery.rs`, `connector_probe_result.rs`, `connector_run.rs`, `connector_type.rs`, `connectors.rs`, `error.rs`, `governance.rs`, `governed_tag.rs`, `identity.rs`, `ingest_spec.rs`, `lib.rs`, `pipelines.rs`, `quality.rs`, `queries.rs`, `query_limits.rs`, `schema_change.rs`, `schema_diff.rs`, `sessions.rs`, `sign_in_settings.rs`, `sso_settings.rs`, `usage.rs`, `workspace_settings.rs`.

## Review
