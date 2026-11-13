# OPS-25: a pull request runs only the acceptance gates its files can affect

Backlog `OPS-25` (free in the repository and the task base on 2026-11-13; the planner files
the row with the next bookkeeping). Planner: Claude Opus, 2026-11-13. Developer: a different
agent. Owner's decision, 2026-11-13, approved explicitly: run on a pull request only the
acceptance gates the changed files can affect; every gate still runs in the nightly full
run and whenever asked; `main` may occasionally be red until the nightly run catches a gate
a pull request did not run. Anchors verified at `main` `0b6650421` (merge of #506, `OPS-24`).
Branch `ci/gates-by-path` off `origin/main`. Only `.github/workflows/ci.yml`, `scripts/ci/`,
`ops/lint/` and `docs/CI.md` change. No Rust, TypeScript, Python product code, compose file
or migration is touched.

Starting point (`docs/superpowers/plans/2026-11-13-ci-faster-runs.md` §0.1, measured): the 25
gate jobs (27 runs with the two `g8` matrices) and `image-smoke` are 156.5 of the 253
job-minutes of a code pull request, and a run waits longer for runners than it runs. Today
every gate runs whenever the scope is `rust`, `dagster`, `stack` or `all`
(`ci.yml:815` and the 25 identical `if:` lines listed in T3). `OPS-24` §4 named this as the
largest lever left.

No test, lint, gate, allowlist or threshold is removed or loosened (AGENTS.md rule 2): a
gate that does not run on a pull request still runs every night and on demand, and
`ci-required` still turns red on any gate that was selected and did not succeed.

## 1. Decisions

**D1. Nine gate groups, declared once.** Each of the 25 gate jobs belongs to exactly one
group, read from what the gate brings up and runs:

| Group | Jobs | Count | What they exercise |
|---|---|---|---|
| `storage` | `g1-rustfs`, `g2-seaweedfs` | 2 | `lakehouse-iceberg`'s `g1_lakekeeper` test (`docker-compose.yml:1725-1780`); tenant provisioning on SeaweedFS (`ops/g2/g2_tenant_test.py`) |
| `pipelines` | `g3a-dagster`, `g3-maintenance`, `gold-export` | 3 | Dagster + dlt Bronze ingest, G7 and BLD-1 authored pipelines (run inside `g3a-dagster`, `ci.yml:1176-1207`), maintenance, Gold export |
| `cdc` | `g4-cdc`, `g4-mysql`, `g4-mongo`, `g4-oracle`, `g4-mssql` | 5 | change capture through Kafka Connect (`ops/g4/`) |
| `ingest` | `g6-ingest`, `g6-ingest-other`, `g6-oracle`, `g6-ops`, `g6-discovery-scale` | 5 | batch connectors, connector operations, discovery at scale (`ops/g6/`) |
| `saas` | `g6-saas-a`, `g6-saas-b` | 2 | business-app connectors against the vendor stubs |
| `secrets` | `g6-credentials`, `g6-secret-managers`, `g6-oauth`, `g10-sso` | 4 | credentials at rest, Vault/AWS, OAuth sign-in, Keycloak SSO |
| `alerts` | `g6-alerts` | 1 | alert delivery (`ops/g6/src17_alert_delivery_test.py`) |
| `upload` | `g9-upload` | 1 | file upload (`ops/g9/upload_test.py`) |
| `governance` | `g8-governance`, `g8-time-travel` | 2 (4 runs) | masking and time travel on Trino and ClickHouse |

`images` and `image-smoke` are not in a group: they keep today's `if:` exactly
(`ci.yml:943`, `:3921`), so a change that selects no group but touches the API or Dagster
still builds both images and proves the API starts (`/health`).

**D2. The path map is one data file, `scripts/ci/gate_paths.txt`.** Two kinds of line:
`group <name> <job> <job>…` (D1, the only place membership is written) and
`path <pattern> <groups>`, where `<groups>` is a comma list of group names, `all` or `none`.
Patterns are bash `[[ $file == $pattern ]]` globs, where `*` also matches `/`. **The first
matching `path` line wins**, so specific lines come before the catch-alls. A changed file
that matches no `path` line selects every group (unknown means everything) and is logged as
`Unmapped file: <path> (selecting every gate group)`. Documents-only files never reach the
map (the existing rule at `detect_change_scope.sh:144-149` `continue`s first).

**D3. What is shared, and why `lakehouse-api/src/` is shared by default.** One binary
(`rust/Dockerfile:93`, `cargo build -p lakehouse-api`) serves every gate, and 21 of the 25
gates create connectors, run pipelines and read back through `routes/connectors.rs`,
`routes/pipelines.rs`, `routes/query.rs` and `routes/governance.rs` (read from the URLs in
`ops/g*/`). So `rust/crates/lakehouse-api/src/*` ends in `all`; only modules whose callers are
one feature are mapped to a group (the `crate::<module>` users were listed at `0b6650421`).
Shared (`all`), at least: `docker-compose.yml`, `.env.example`, `.github/*`, `scripts/*`
(including `scripts/ci/*`), `rust/migrations/*`, `rust/Cargo.toml`, `rust/Cargo.lock`,
every crate's `Cargo.toml` and `src/` not listed below, `rust/Dockerfile`,
`rust/entrypoint.api.sh`, `rust/rust-toolchain.toml`, `dagster/Dockerfile`,
`dagster/pyproject.toml`, `dagster/dagster.yaml`, `dagster/dispar_orchestrate/definitions.py`
and every Dagster module not listed, `ops/fixtures/*`, `ops/gates/*` (the registry and
`api_log_for_references.sh` every gate calls), `ops/oidc-mock/*` (built by most gates),
`ops/smtp-ca/*` (mounted into the API, `docker-compose.yml:829`), `ops/backups/*` (mounted
into ClickHouse, `docker-compose.yml:143`), `ops/init-env.sh`, and any other `ops/*` or
`rust/*` path. In the API that means `lib.rs`, `main.rs`, `state.rs`, `config.rs`,
`policy.rs` (`POLICY_TABLE`), `policy_engine.rs`, `sql_rewrite.rs`, `routes/mod.rs` (the
router), `routes/support.rs`, `routes/auth.rs`, `routes/identity.rs`, `routes/connectors.rs`,
`routes/pipelines.rs`, `routes/query.rs`, `routes/governance.rs`, `upstream_error.rs`, and
all of `lakehouse-core`, `lakehouse-store/src/` (except the modules in T1), `lakehouse-auth`,
`lakehouse-iceberg/src/`, `lakehouse-clickhouse`, `lakehouse-dagster`.

**D4. `none` is explicit and narrow.** A path selects no group only when no gate can load
it: integration tests and benches (`rust/crates/*/tests/*`, except G1's own test, which is
mapped to `storage` first), `rust/tests/*` (the parity corpus; `Repo lints` checks it),
`lakehouse-test-support` (a `[dev-dependencies]` entry in every crate that names it:
`lakehouse-api/Cargo.toml:206,214`, `lakehouse-store/Cargo.toml:19,27`,
`lakehouse-auth/Cargo.toml:27,32`), `lakehousectl` (not in any image), the assistant
(`routes/ai/*`, `lakehouse-llm`, the store's `chat_term.rs`, `assistant_settings.rs`,
`knowledge.rs`, `routes/knowledge.rs`), Dagster unit tests (`test_*`, `conftest.py`),
`ops/lint`, `ops/release`, `ops/ai_eval`, `ops/bench`, `ops/geo`, `ops/demo`,
`ops/forecast` (none is mounted or run by a gate: `grep -c` in `docker-compose.yml` and
`ci.yml` is 0 for each), and the console (`src/*`, `public/*`, the frontend config files,
`Dockerfile.frontend`). The console image is never built or started by a gate: `images`
builds the API and Dagster only, and every gate's `up` names its services
(`ci.yml:1168-1170`, `:1403-1408`). **This is a deviation from the brief** ("every
Dockerfile is shared"): `Dockerfile.frontend` is `none` for that reason, and the three gate
Dockerfiles go to their gate's group (`ops/g6/Dockerfile.saas_stub` to `saas`,
`ops/g6/Dockerfile.rest_stub` to the `ops/g6` set, `ops/g7/Dockerfile` to `pipelines`).
Two paths the console owns are compiled into `lakehouse-bi` and go to `governance,alerts`:
`src/lib/dashboard-specs.ts` and `public/geo/*` (`lakehouse-bi/src/formula/compile.rs:2535`).

**D5. Fail closed.** Every group is selected when: the existing `all` is `true` (nightly,
`workflow_dispatch`, an unknown event, an unreadable base, an empty or failed diff,
`CI_PUSH_FULL=true` on a push, `.github/*`, `ops/fixtures/*`, `scripts/ci/*`, an
unclassified file); a file is unmapped (D2); the label of D6 is present; or
`gate_paths.txt` is missing or unreadable (`::warning::` and every group). A `path` line that
names an unknown group, or a `group` line with an unknown job, is an error: the script exits 1
and `changes` is red.

**D6. Label `ci:all-gates`.** On a `pull_request` event, the label selects every group and
sets `stack=true` (so `images` runs even on a console-only pull request; it does not start
the Rust tests). The workflow passes only a boolean:
`ALL_GATES_LABEL: ${{ github.event_name == 'pull_request' && contains(github.event.pull_request.labels.*.name, 'ci:all-gates') }}`;
the script compares it to exactly `true`. No label text reaches a script. Labels are in the
event payload, so no new permission is needed (`changes` keeps `contents`, `checks`,
`pull-requests: read`, `ci.yml:63-66`). A fork's pull request can carry the label (only
someone with triage rights on this repository can add it) and it only makes CI run more.
No `labeled` trigger type is added: adding any label would then start a run and cancel the
running one (`cancel-in-progress` for pull requests, `ci.yml:38`). The label takes effect
on the next push to the pull request, or a close and reopen. Whether "Re-run jobs" sees a
label added after the original event is *not verified*; `docs/CI.md` says to push. The
planner creates the label in the repository; the developer does not.

**D7. `ci-required` (extends `OPS-24` D6).** `changes` gains the output
`gate_groups`: `none`, or the selected groups between commas in D1's order
(`,cdc,ingest,`). `required_gate.sh` reads the job-to-group lines from `gate_paths.txt`
and `SELECTED_GATE_GROUPS`. For each gate job (failure or cancelled is red first, unchanged,
`required_gate.sh:83-87`):

| `req_gates` (acceptance scope, not a proven push) | `req_all` (`all`, nightly, dispatch) | the job's group | Required |
|---|---|---|---|
| false | any | any | no |
| true | true | any | yes |
| true | false | in `SELECTED_GATE_GROUPS` | yes |
| true | false | not in it, and the value is `none` or well-formed | no |
| true | false | value unset, empty or malformed | **yes** (fail closed) |
| true | false | the job has no `group` line, or the file is unreadable | **yes** (fail closed) |

Well-formed is `none` or `^,([a-z]+,)+$`. `images` stays `req_acceptance` and `image-smoke`
stays `req_gates` (`required_gate.sh:224-225`, unchanged).

**D8. Pushes (keeps `OPS-24` D4-D5).** A proven push skips every gate as today (the
`proven != 'true'` term stays first in each `if:`, and `req_gates` is false). An unproven
push runs the gates of the groups its own diff selects. The nightly run is the backstop.

**D9. Each gate's `if:` keeps today's condition and adds its group.** The new form is
today's line with `&& needs.changes.outputs.gates_<group> == 'true'` appended inside the
outer expression. Keeping the scope term means a console-only change still runs no gate,
whatever the map says.

**D10. A consistency lint, `ops/lint/check_gate_groups.py`, in `Repo lints`.** Model:
`ops/lint/check_rust_embedded_paths.py` (it asks the real script). Checks, each a printed
reason on failure: (a) every job in `ci.yml` except the fixed set `changes`, `repo-lints`,
`verify`, `fmt`, `clippy`, `build`, `test`, `msrv`, `dagster-unit-tests`,
`dagster-loader-tests`, `images`, `image-smoke`, `ci-required` is in exactly one group, and
every job a `group` line names exists; (b) each gate job's `if:` contains
`needs.changes.outputs.gates_<its group> == 'true'` and no other `gates_` output; (c)
`changes.outputs` has `gates_<g>` for every group and `gate_groups`, and the `ci-required`
step that runs `required_gate.sh` has `SELECTED_GATE_GROUPS`; (d) every group has at least
one `path` line; (e) every tracked file (`git ls-files`) under `ops/g2`, `ops/g3`,
`ops/g3a`, `ops/g4`, `ops/g6`, `ops/g7`, `ops/g8`, `ops/g9`, `ops/g10`, `ops/gold_export`,
`ops/sso`, `ops/seaweedfs` classifies, through `detect_change_scope.sh --classify`, to a
group list (not `all`, not `none`, not unmapped). A `path` line matching no tracked file is
printed as a warning, not a failure.

## 2. Tasks

Five tasks, one commit each, in order. Every commit message cites `OPS-25`. Each new block
of code carries a short `OPS-25 Dn` comment saying why (rule 13 style).

### T1. The path map and the scope script

Files: new `scripts/ci/gate_paths.txt`; `scripts/ci/detect_change_scope.sh`;
`scripts/ci/tests/test_detect_change_scope.sh`.

1. Write `gate_paths.txt` with a header comment (what it is, first match wins, `*` matches
   `/`, `all`/`none`, unmapped means every group, the nightly run is the backstop,
   `OPS-25`). Then the nine `group` lines of D1, then these `path` lines **in this order**
   (one pattern per line; `a,b` means the comma list):

   - `none` first: `rust/crates/lakehouse-iceberg/tests/g1_*` → `storage` (before the next
     line); `rust/crates/*/tests/*`, `rust/crates/*/benches/*`, `rust/tests/*`,
     `rust/crates/lakehouse-test-support/*`, `rust/crates/lakehousectl/*`,
     `rust/crates/lakehouse-llm/*`, `rust/crates/lakehouse-api/src/routes/ai/*`,
     `rust/crates/lakehouse-api/src/routes/knowledge.rs`,
     `rust/crates/lakehouse-store/src/chat_term.rs`,
     `rust/crates/lakehouse-store/src/assistant_settings.rs`,
     `rust/crates/lakehouse-store/src/knowledge.rs`, `dagster/dispar_orchestrate/test_*`,
     `dagster/dispar_orchestrate/conftest.py`, `ops/lint/*`, `ops/release/*`,
     `ops/ai_eval/*`, `ops/bench/*`, `ops/geo/*`, `ops/demo/*`, `ops/forecast/*` → `none`.
   - Console: `src/lib/dashboard-specs.ts` and `public/geo/*` → `governance,alerts`; then
     `src/*`, `public/*`, `package.json`, `bun.lock`, `bunfig.toml`, `tsconfig.json`,
     `next.config.ts`, `eslint.config.mjs`, `postcss.config.mjs`, `components.json`,
     `Dockerfile.frontend` → `none` (the same list as `detect_change_scope.sh:171-178`).
   - `storage`: `ops/g2/*`, `ops/seaweedfs/*`.
   - `cdc`: `ops/g4/*`, `rust/crates/lakehouse-api/src/cdc_*`,
     `…/lakehouse-api/src/kafka_connect.rs`, `…/lakehouse-api/src/capture_lag.rs`,
     `…/lakehouse-api/src/routes/capture_lag.rs`, `…/lakehouse-store/src/cdc.rs`,
     `…/lakehouse-store/src/capture_lag.rs`, `dagster/dispar_orchestrate/cdc_apply.py`.
   - `saas`: `ops/g6/docker-compose.g6-saas.override.yml`, `ops/g6/saas_stub*`,
     `ops/g6/Dockerfile.saas_stub`, `ops/g6/g6_saas_test.py`, `ops/g6/test_saas_gate_*`,
     `…/lakehouse-store/src/saas_vendors.rs`, `dagster/dispar_orchestrate/adapters/saas.py`,
     `dagster/dispar_orchestrate/saas/*`; and `…/lakehouse-api/src/connector_probe/saas.rs`,
     `…/lakehouse-api/src/connector_oauth.rs`, `…/lakehouse-api/src/routes/connector_oauth.rs`,
     `…/lakehouse-store/src/connector_oauth.rs` → `secrets,saas`.
   - `secrets`: `ops/g6/docker-compose.src12*`, `ops/g6/src12_*`, `ops/g10/*`, `ops/sso/*`,
     `…/lakehouse-api/src/secret_manager/aws.rs`, `…/azure.rs`, `…/gcp.rs`, `…/vault.rs`,
     `dagster/dispar_orchestrate/secret_manager_*` (the vendor files only; `mod.rs`,
     `refs.rs`, `secret_manager.py`, `secret_resolver.py` resolve every connector's
     `secretRef` and stay shared).
   - `alerts`: `rust/crates/lakehouse-notify/*`, `…/lakehouse-api/src/webhook_guard.rs`,
     `…/lakehouse-store/src/alert_delivery.rs` → `alerts`; `ops/g6/src17_*` →
     `alerts,ingest,secrets` (imported by `src11_*` and `src12_oauth_test.py`);
     `rust/crates/lakehouse-alerts/*`, `…/lakehouse-api/src/routes/alerts.rs`,
     `…/routes/load_alerts.rs`, `…/routes/chart_alerts.rs`, `…/routes/notifications.rs`,
     `…/lakehouse-store/src/alert_state.rs`, `…/lakehouse-store/src/email_settings.rs`,
     `dagster/dispar_orchestrate/alerts_run.py` → `alerts,cdc,ingest,pipelines`
     (`/api/alerts/run`, `routes/mod.rs:1394`, is called by `ops/g4/g4_test.py:325` and
     `ops/gold_export/gold_export_test.py`).
   - `upload`: `ops/g9/*` → `upload`; `…/lakehouse-api/src/upload_*`,
     `…/routes/uploads.rs`, `…/routes/uploads/*`, `…/lakehouse-store/src/uploads.rs`,
     `…/lakehouse-store/src/upload_folders.rs`, `dagster/dispar_orchestrate/file_ingest.py`,
     `…/upload_types.py`, `…/parquet_ingest.py` → `upload,ingest` (`g6-ingest-other` loads
     files).
   - `pipelines`: `ops/g3/*`, `ops/g3a/*`, `ops/g7/*`, `ops/gold_export/*`,
     `…/lakehouse-api/src/pipeline_compile.rs`, `…/transform_grammar.rs`,
     `…/routes/authored_pipelines.rs`, `…/routes/pipeline_preview.rs`,
     `…/lakehouse-store/src/maintenance_policy.rs`, `dagster/dispar_orchestrate/maintenance.py`,
     `…/authored_*` → `pipelines`; `…/lakehouse-api/src/gold_*`, `…/routes/gold.rs`,
     `…/lakehouse-store/src/gold_publication.rs`, `dagster/dispar_orchestrate/gold_export.py`,
     `…/routes/catalog.rs`, `…/routes/catalog_search.rs`, `…/routes/catalog_profile.rs`,
     `…/routes/catalog_grants.rs`, `…/routes/catalog_restore.rs`,
     `…/routes/catalog_source.rs`, `…/lakehouse-api/src/bronze_stats_cache.rs`,
     `…/catalog_search_cache.rs` → `pipelines,governance` (`g8` exports Gold; `g3a` and
     `gold-export` read the catalog). `routes/catalog_governance.rs` and
     `routes/catalog_query.rs` are not listed: they stay shared.
   - `governance`: `ops/g8/*`, `ops/trino/*`, `rust/crates/lakehouse-trino/*` →
     `governance`; the BI side → `governance,alerts` (`lakehouse-alerts` depends on
     `lakehouse-bi`): `rust/crates/lakehouse-bi/*`, `rust/crates/lakehouse-embed/*`,
     API `routes/dashboard*`, `embed.rs`, `layout_tabs.rs`, `metrics.rs`,
     `metrics_rewrite.rs`, `calc_fields.rs`, `hierarchies.rs`, `drill.rs`, `maps.rs`,
     `params.rs`, `param_rewrite.rs`, `layers.rs`, `layers_rewrite.rs`, `bin_rewrite.rs`,
     `chart_shapes_rewrite.rs`, `type_b_rewrite.rs`, `join_rewrite.rs`,
     `organise_paths.rs`, `render.rs`, `pdf_export.rs`, `report_runner.rs`, `reports.rs`,
     `home.rs`, `overview.rs`, `public_links.rs`, `saved_queries.rs` (all under
     `routes/`), and `src/tile_pool.rs`, `src/result_cache.rs`, `src/result_cache/*`,
     `src/pdf.rs`, `src/export.rs`, `src/csv_export.rs`, `src/csv_safe.rs`; store
     `dashboard_*`, `home_layout.rs`, `geo_map.rs`, `report_*`, `reporting_settings.rs`,
     `public_links.rs`, `semantic.rs`, `saved_query_access.rs`, `overview.rs`.
   - The `ops/g6` remainder, after every `ops/g6/…` line above: `ops/g6/src11_*` →
     `ingest,secrets`; `ops/g6/src13_*` and `ops/g6/g6_oracle_test.py` → `ingest`;
     `ops/g6/docker-compose.g6-oracle.override.yml`, `ops/g6/oracle-tls-proxy.conf` →
     `cdc,ingest`; `ops/g6/*` → `cdc,ingest,saas,secrets,alerts` (the matrix module and
     `docker-compose.g6.override.yml` are used by all five, `ci.yml:1559`, `:2243`, `:2773`,
     `:3141`, `:3501`).
   - Shared, last: `docker-compose.yml`, `.env.example`, `.github/*`, `scripts/*`,
     `rust/migrations/*`, `dagster/*`, `ops/*`, `rust/*` → `all`. (The more specific shared
     paths of D3 all fall under these four catch-alls; write D3's list as a comment above
     them so a reader sees it.)

   `…` is `rust/crates/lakehouse-api/src`, `rust/crates/lakehouse-store/src` or
   `dagster/dispar_orchestrate` as the line before it; write every pattern out in full.

2. In `detect_change_scope.sh`:
   - Replace the five copies of the "force all" output block (`:50-60`, `:71-81`,
     `:88-98`, `:104-114`, `:120-130`) with one function `emit_all_outputs` (rule 4), which
     also writes `gates_<g>=true` for every group and `gate_groups=,storage,…,governance,`.
   - Load the map from `${GATE_PATHS_FILE:-$(dirname "$0")/gate_paths.txt}` once, before
     the loop (missing → D5). Add `classify_gate_groups <file>` that returns the first
     matching line's groups, or `unmapped`.
   - In the loop (`:141-230`), for every file that is not documents-only, add its groups to
     a selected set; `all` or `unmapped` selects every group (log the unmapped line).
   - After the loop: if `all=true`, or `ALL_GATES_LABEL` is exactly `true` (then also
     `stack=true`, log `Label ci:all-gates: selecting every gate group`), select every
     group. Print and write `gates_<g>` for each group and `gate_groups` (D7 format).
   - New mode, before any git call: `detect_change_scope.sh --classify <path>…` prints
     `<path><TAB><groups|none|all|unmapped|docs>` per path and exits 0. Used by T4.
   - Update the header (`:2-17`) with the new outputs, `ALL_GATES_LABEL` and
     `GATE_PATHS_FILE`.
3. Self-tests, a new helper `assert_groups "<sentence>" "<expected gate_groups>" <files…>`
   in `test_detect_change_scope.sh` (it copies the real `gate_paths.txt`; no network).
   Cases, each named as a full sentence:
   - one path per group selecting only that group: `ops/g2/g2_tenant_test.py` → `,storage,`;
     `ops/g3a/g3a_test.py` → `,pipelines,`; `ops/g4/g4_test.py` → `,cdc,`;
     `ops/g6/src13_discovery_scale_test.py` → `,ingest,`; `ops/g6/saas_stub_hubspot.py` →
     `,saas,`; `ops/sso/realm.template.json` → `,secrets,`;
     `rust/crates/lakehouse-notify/src/lib.rs` → `,alerts,`; `ops/g9/upload_test.py` →
     `,upload,`; `ops/g8/g8_governance_test.py` → `,governance,`;
   - a union: `ops/g2/x.py` + `ops/g9/y.py` → `,storage,upload,`;
   - shared paths each select all nine: `docker-compose.yml`, `.env.example`,
     `rust/migrations/0999_x.sql`, `rust/Cargo.lock`, `rust/crates/lakehouse-store/src/lib.rs`,
     `rust/crates/lakehouse-api/src/policy.rs`, `rust/crates/lakehouse-api/src/routes/mod.rs`,
     `dagster/Dockerfile`, `ops/gates/gate_summary.py`, `scripts/compose.sh`;
   - an unmapped path (`newdir/file.txt`) selects all nine and logs `Unmapped file`;
   - documents-only (`docs/test.md`) → `none`; console-only (`src/components/button.tsx`)
     → `none`; `rust/crates/lakehouse-api/tests/route_auth.rs` → `none`;
   - `ALL_GATES_LABEL=true` with `src/components/button.tsx` → all nine and `stack=true`;
     `ALL_GATES_LABEL=TRUE` → `none`;
   - `EVENT_NAME=schedule` → all nine; `CI_PUSH_FULL`-style `FORCE_ALL=true` on a push →
     all nine; an unreadable base → all nine;
   - `GATE_PATHS_FILE=/nonexistent` → all nine and a warning; a map with
     `path ops/g2/* nosuchgroup` → exit 1;
   - `--classify ops/g4/g4_test.py docs/a.md` prints `cdc` and `docs`.

Check: `bash scripts/ci/tests/test_detect_change_scope.sh; echo "exit $?"`, Results line
quoted, every old case still passes. Break it once: delete the `ops/g4/*` line and quote the
failing case, restore it.

### T2. `required_gate.sh` requires a gate only when its group was selected

Files: `scripts/ci/required_gate.sh`, `scripts/ci/tests/test_required_gate.sh`.

1. Header (`:9-62`): add `SELECTED_GATE_GROUPS` and `GATE_PATHS_FILE`, citing `OPS-25` D7.
2. In `run_gate` after `req_gates` (`:169-172`): load `group` lines from
   `${GATE_PATHS_FILE:-$SCRIPT_DIR/gate_paths.txt}` (compute `SCRIPT_DIR` at the top, as
   the self-test branch at `:235` already does), validate `SELECTED_GATE_GROUPS` (D7), and
   add `req_gate_for <job>` implementing the D7 table; print one line per job:
   `Job [g4-cdc]: group=cdc, selected=false`.
3. Replace `"$req_gates"` with `"$(req_gate_for <job>)"` in the 25 `check_job` lines
   `:189-213`. Leave `:224-225` as they are.
4. Self-tests: add `SELECTED_GATE_GROUPS GATE_PATHS_FILE` to the `unset` list in `expect`
   (`:38-45`). New cases, full sentences: a pull request selecting `,cdc,` passes with the
   five `g4-*` successful and the other 20 skipped; the same fails when `g4-mysql` is
   skipped; it fails when `g6-ingest` (not selected) failed; `none` passes with all 25
   skipped; an unset, empty and malformed (`cdc`) value each fail with the 20 skipped
   (fail closed); `SCOPE_ALL=true` with `,cdc,` fails when a non-cdc gate is skipped;
   `EVENT_NAME=schedule` with `,cdc,` fails when a non-cdc gate is skipped; a proven push
   with `,cdc,` passes with all gates skipped; a `GATE_PATHS_FILE` without `g4-cdc`'s group
   line makes `g4-cdc` required. Existing cases that set an acceptance scope without
   `SELECTED_GATE_GROUPS` keep their meaning, because unset means every group (say so in a
   comment).

Check: `bash scripts/ci/tests/test_required_gate.sh; echo "exit $?"`. Break it once: make
`req_gate_for` return `false` for a selected group and quote the failing case.

### T3. Wire the workflow

File: `.github/workflows/ci.yml`.

1. `changes.outputs` (after `:92`): `gates_storage` … `gates_governance` (nine) and
   `gate_groups`, each `${{ steps.scope.outputs.<name> }}`, with an `OPS-25` comment.
2. "Detect change scope" step `env:` (after `:113`): the `ALL_GATES_LABEL` line of D6, with
   a comment that only a boolean crosses into the script.
3. Append ` && needs.changes.outputs.gates_<group> == 'true'` at the end of each of the 25
   gate `if:` lines, after the closing parenthesis (D9): `:815` g1, `:861` g2 (`storage`); `:1036` g3a, `:1226`
   g3, `:3697` gold-export (`pipelines`); `:1314`, `:1479`, `:1629`, `:1780`, `:1924`
   (`cdc`); `:2169`, `:2330`, `:2495`, `:2852`, `:3377` (`ingest`); `:3492`, `:3590`
   (`saas`); `:2087` g10, `:2983`, `:3133`, `:3267` (`secrets`); `:2705` (`alerts`); `:2605`
   (`upload`); `:3811`, `:4002` (`governance`). Do not touch `:943` (`images`) or `:3921`
   (`image-smoke`). Example, `g4-cdc`:
   `if: needs.changes.outputs.proven != 'true' && (needs.changes.outputs.rust == 'true' || needs.changes.outputs.dagster == 'true' || needs.changes.outputs.stack == 'true' || needs.changes.outputs.all == 'true') && needs.changes.outputs.gates_cdc == 'true'`
4. `ci-required` "Enforce required gate" `env:` (after `:4182`):
   `SELECTED_GATE_GROUPS: ${{ needs.changes.outputs.gate_groups }}`.
5. Update the comment above `changes` (`:47-52`) to say gates also follow their group.

Check: the YAML parse line of §3; `bash scripts/ci/tests/test_required_gate.sh` (its
consistency test reads `ci.yml`); `grep -c "needs.changes.outputs.gates_" .github/workflows/ci.yml`
is 25 in `if:` lines plus the outputs.

### T4. The consistency lint

Files: new `ops/lint/check_gate_groups.py`, new `ops/lint/test_check_gate_groups.py`,
`.github/workflows/ci.yml` (`repo-lints`).

1. The lint implements D10 (a)-(e): narrative module docstring (why, what, what it does not
   cover), `from __future__ import annotations`, standard library plus `yaml`, reads
   `ci.yml`, `gate_paths.txt` and `git ls-files`, and calls `detect_change_scope.sh
   --classify` once with every file of check (e). Exit 1 with one line per finding.
2. Unit tests (no network), full-sentence names, each on a temporary copy: a job in two
   groups fails; a gate job in no group fails; a group with no `path` line fails; an `if:`
   naming another group's output fails; an unmapped file under `ops/g9/` fails; the real
   repository passes.
3. `repo-lints`: a step after "Unit-test the release tooling (REL-2 S2)" (`:261-265`, which
   installs `pyyaml`): `- name: Check every gate job has one group and every gate file a
   path rule (OPS-25)` / `run: python3 ops/lint/check_gate_groups.py`, with a comment. The
   unit tests already run through `python3 -m unittest discover -s ops/lint` (`:218-220`).

Check: `python3 ops/lint/check_gate_groups.py; echo "exit $?"` and
`python3 -m unittest discover -s ops/lint -p 'test_*.py'`. Break it once for (a), (d) and
(e) on the real files and quote the three messages.

### T5. `docs/CI.md` and the dry run

1. `docs/CI.md`, in "Change Scopes and the Single Gate" (`:29-52`): a subsection "Gates by
   path (`OPS-25`)": the nine groups with their jobs (D1 table), the map's location and
   format (first match wins, unmapped means every group), the shared rule (D3, one
   paragraph), what is `none` (D4), the label and how it takes effect (D6), the
   `ci-required` table (D7), pushes (D8), and this sentence: "The owner accepted on
   2026-11-13 that `main` may occasionally be red until the nightly run catches a gate that
   a pull request did not run." Add `gate_groups` to the scope table's text.
2. The dry run: for each pull request of §3b, list its files with
   `git diff --name-only <merge>^1 <merge>` (find the commit with
   `git log --first-parent --oneline origin/main | grep -E '#<n>\b'`), run
   `scripts/ci/detect_change_scope.sh --classify` on them, and fill the table in the
   handoff. A row that differs from §3b is reported with the line of the map that decided
   it; the developer does not change the map to match the planner's table.

Check: the doc renders (no broken table); the dry-run table is in the handoff.

## 3. Verification

On the final commit, in the foreground, quoting each Results line and exit code:

```bash
for t in scripts/ci/tests/*.sh; do bash "$t"; echo "$t exit $?"; done
bash -n scripts/ci/*.sh scripts/ci/tests/*.sh
python3 ops/lint/check_gate_groups.py
python3 ops/lint/check_rust_embedded_paths.py
python3 ops/lint/check_bare_iceberg_count.py && python3 ops/lint/check_intra_package_imports.py
python3 -m unittest discover -s ops/lint -p 'test_*.py'
python3 -c "import yaml,sys; [yaml.safe_load(open(f)) for f in sys.argv[1:]]" .github/workflows/*.yml
git status
```

`actionlint` and `shellcheck` are not installed: write *not verified* for both.

### 3a. Change of machine

The developer works on the ARM64 build machine in the checkout
`/home/rismana/rantai/lakehouse-agents-2`, only under that user's work directory. No cargo,
bun or Docker command is needed or run. The commands are those of §3. The developer pushes
each commit to `ci/gates-by-path`, never opens or merges a pull request, never adds a
label, and never changes a repository setting or variable.

### 3b. Measured after the pull request opens

1. This pull request's own run: it touches `.github/` and `scripts/ci/`, so `all=true` and
   every group is selected; all 25 gates (27 runs) and `image-smoke` run. The planner quotes
   the run id, the `changes` log lines `gate_groups=,storage,pipelines,cdc,ingest,saas,secrets,alerts,upload,governance,`
   and `ci-required`'s per-job lines. It does not prove a narrow selection; the first
   narrow pull request after the merge does, and the planner checks its run the same way.
2. Dry run of the map against the last ten merged pull requests (planner's reading of the
   map above; T5 reproduces it with the real script). "Today" is what runs at `0b6650421`.

| Pull request | Files (gate-relevant) | Groups selected | Gate jobs run (today) |
|---|---|---|---|
| #507 | documents | none (documents-only) | 0 (0) |
| #506 | `.github/`, `scripts/ci/`, `ops/lint/` | all nine | 25 (25) |
| #505 | `CHANGELOG.md`, documents | none (documents-only) | 0 (0) |
| #502 | `rust/crates/lakehouse-test-support/src/lib.rs` | none; `images` and `image-smoke` run | **0 (25)** |
| #500 | `policy.rs`, `routes/mod.rs`, `routes/support.rs`, pipelines | all nine (shared) | 25 (25) |
| #499 | console | none, no acceptance scope | 0 (0) |
| #498 | documents | none | 0 (0) |
| #497 | console, documents | none | 0 (0) |
| #495 | documents | none | 0 (0) |
| #494 | documents | none | 0 (0) |

Older pull requests that touched Rust, Dagster or the stack (since #457), for the same
reading: #457 (`routes/ai/*`, tests) → none, **0 (25)**; #468 (`routes/catalog.rs`, tests) →
`pipelines,governance`, **5 jobs, 7 runs (25)**; #466, #459, #481, #487 (`policy.rs` or
`lib.rs`), #482 (`policy_engine.rs`), #483 (`routes/query.rs`), #484 (`upstream_error.rs`),
#486 (`routes/mod.rs`), #492 and #493 (`lakehouse-store`), #485, #490, #467 (compose or
`.env.example`), #489 (`ops/fixtures/`), #473 (`.env.staging`, unclassified) → all nine,
25 (25).

**Honest reading.** The map saves gates on 3 of the 20 pull requests above that run gates
today (#502, #457, #468: 70 of 500 gate jobs, 14%), because most Rust pull requests touch a shared file:
`policy.rs` in 5, `routes/mod.rs` in 5, `lakehouse-store/src/` in 6, compose in 3. The
per-gate job-minutes are not measured; `OPS-24` §0.1 gives 156.5 job-minutes for 28 jobs,
an average of 5.6, which is an average, not a per-gate number. The planner does not quote a
saving in minutes until a narrow pull request's run is read (step 1).

## 4. Out of scope

- **The lever this plan exposes**: a new route adds a `POLICY_TABLE` row
  (`policy.rs`) and a router line (`routes/mod.rs`), so nearly every feature pull request is
  "shared". Proposal: split the router and the policy table per feature module so a new
  route touches only its module (a Rust change of its own, with `tests/route_auth.rs`
  unchanged as the guard), then map those modules. Likewise `lakehouse-store/src/lib.rs`.
- Mapping `routes/connectors.rs`, `routes/pipelines.rs`, `routes/query.rs` and
  `routes/governance.rs` to groups: 21 gates call them; they stay shared.
- Splitting gates within a job (per connector of the ingest matrix), or per-gate timing.
- A `labeled` trigger (D6); a merge queue; more concurrent runners.
- The Rust test partitions (`OPS-24` S2), Coverage, `security.yml`, `release.yml`,
  `deploy-staging.yml`: not touched.
- `docs/core/BACKLOG.md`: the planner files and updates the `OPS-25` row.

## Open questions for the owner

- **Q1.** The honest saving is about one gate-running pull request in six until the
  router and policy table are split (§4, first item). Recommendation: merge this today as
  the mechanism, and file the split as its own backlog item.

## 5. Handoff

## 6. Review
