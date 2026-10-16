# `BLD-1` slice 1: a pipeline reads as its author

Planner and reviewer: Claude Opus. Developer: a non-Opus agent (`AGENTS.md`).
Base: `origin/main` at `ead1a871`. Branch: `feat/bld-1-run-as-author`.
One PR slice. Spec: `docs/core/specs/bld-1.md`; feature page:
`docs/core/features/pipeline-joins-and-sql.md` (decision 3, product owner,
2026-10-14). This slice also closes `GOV-4` for pipelines.

`BLD-1` is built in three slices, in this order, each its own plan and PR:

1. **This plan.** Every pipeline, existing and new, reads as its author.
2. Joins and aggregations (multiple inputs, `join`, `group`, post-group
   filter), built on slice 1's enforcement path.
3. The SQL step, built on the same path.

## 1. The problem

A pipeline runs as the orchestrator's ClickHouse user and reads every table
unmasked. `authored_factory._op_for_pipeline` builds a `SELECT` and runs it
directly; nothing applies the masking and row-filter rewrite
(`sql_rewrite::enforce`) that Query Studio, dashboards, alerts and the Gold
export apply. So anyone with `pipeline:write` can copy a masked column into
a Silver table in clear (`GOV-4`). Decision 3 of the feature page: a pipeline
reads as its author, through the same rewrite as the author's own queries.

Facts read from `main` at `ead1a871`:

- `pipeline_definition.owner` is free text, not a user. The only user id is
  `pipeline_definition_version.changed_by` (`lakehouse-store/src/pipelines.rs`,
  `PipelineVersionMeta`). Pipelines created before version history have a
  `baseline` version with `changed_by = NULL` (migration `0071`).
- `sql_rewrite::enforce(sql, dialect, principal_roles, placeholders,
  obligations_source, views_catalog)` takes role names and placeholder
  values; `policy_engine` builds the obligations source with
  `with_iceberg_db(state.config.iceberg_query_db)`, so a governed Bronze
  table is recognised under the API's Iceberg query database name
  (`ICEBERG_QUERY_DB`, `icecat_api` in compose), not under another job's.
- The orchestrator already authenticates to the API with
  `PIPELINE_RUN_TOKEN`; `authored_pipelines::runnable` is the precedent for
  an orchestrator-only route (403 to anyone else).

## 2. Decisions (planner)

- **D1. Who the author is: a new column `pipeline_definition.run_as UUID`**
  (references the user table, `ON DELETE SET NULL`). Set to the creating
  principal's user id on create; unchanged by update and restore (editing
  someone's pipeline does not change who it reads as). Backfill: from the
  `created` version's `changed_by` where one exists; otherwise NULL.
  Migration: the next free number when the PR is opened, at least `0073`;
  check every open branch first (several hold `0070` to `0072`). Why-header,
  additive, never edited once applied.
- **D2. No author, no run (fail closed).** A pipeline whose `run_as` is NULL,
  or names a user that is deactivated or gone, does not run: the op fails,
  not retried, with the fixed reason `"this pipeline has no author to read
  as; an administrator must assign one"`. A Platform Admin can set `run_as`
  through `PUT /api/pipelines/{id}/run-as` (`{ "userId": "<uuid>" }`, the
  user must be in the pipeline's tenant), audited like other pipeline
  writes, in `POLICY_TABLE`, asserted both ways in `tests/route_auth.rs`.
  The console shows "Reads as: <name>" on the pipeline page and, when unset,
  the reason and (for a Platform Admin) a control to assign it.
- **D3. One rewriter, in the API.** A new orchestrator-only route
  `POST /api/pipelines/runnable/{id}/enforce`, body `{ "sql": "<SELECT>" }`,
  same authentication rule as `runnable`. It loads the pipeline, resolves
  `run_as` to that user's current role names and placeholder values (the
  same way a session principal gets them; find the loader by symbol, do not
  write a second one), runs `sql_rewrite::enforce` with the same dialect,
  obligations source and views catalog Query Studio uses, and returns
  `{ "sql": "<rewritten>" }`. A refusal returns 422 with
  `policy_engine::enforcement_error_message` (a fixed string), never the
  rewriter's or ClickHouse's text. D2's cases return 409 with D2's message.
- **D4. Dagster asks before it runs.** `_op_for_pipeline` builds the
  `SELECT` as today, sends it to D3, and runs only the returned SQL, in the
  `CREATE TABLE ... AS`, staging `INSERT` and every other statement that
  reads the source. A 409 or 422 fails the op without retry, with the fixed
  message; a transport failure is retryable, as today.
- **D5. Bronze is named the way the rewriter recognises it.** For a Bronze
  source the op names the table in the API's Iceberg query database
  (`ICEBERG_QUERY_DB`, passed to `dagster-code-location` in compose), so
  `enforce` sees the governed table it would see in Query Studio. This
  replaces the `icecat_authored` database `BLD-3` added for authored jobs;
  remove it from `authored_factory.py` and say why in the module doc
  (governance must recognise the table). Keep `allow_database_iceberg = 1`.
- **D6. Assistant.** `create_pipeline` creates as the calling user, so
  `run_as` is that user. No schema change.

## 3. Anchors (`main` at `ead1a871`)

- `rust/crates/lakehouse-api/src/sql_rewrite.rs`: `enforce`, `PlaceholderValues`.
- `rust/crates/lakehouse-api/src/policy_engine.rs`: the obligations source
  built `with_iceberg_db`, `enforcement_error_message`.
- `rust/crates/lakehouse-api/src/routes/gold.rs` and `gold_export.rs`: how
  the Gold export calls `enforce` for a principal (the closest precedent).
- `rust/crates/lakehouse-api/src/routes/authored_pipelines.rs`: `runnable`
  (orchestrator-only rule), `update`, `restore_version`, `in_scope`;
  `routes/pipelines.rs::create`.
- `rust/crates/lakehouse-store/src/pipelines.rs`: `Pipeline`,
  `PipelineVersionMeta`, the create path.
- `rust/crates/lakehouse-auth/src/`: how a session principal's role names
  are loaded for a user id.
- `dagster/dispar_orchestrate/authored_factory.py`: `_op_for_pipeline`,
  `_resolve_source`, `_headers`, `PIPELINE_RUN_TOKEN`.
- `docker-compose.yml`: `dagster-code-location` env.
- `ops/g7/g7_pipeline_test.py`, `ops/g8/` (the governance masking gate).

## 4. Tasks (one commit each; Rust tasks together, migration with its code)

- **T1. D1:** migration, store field, create sets it, backfill test (a
  `created` version → `run_as`; a `baseline`-only pipeline → NULL).
- **T2. D2 API half:** `PUT .../run-as`, policy row, route_auth both ways,
  tenant check, audit.
- **T3. D3:** the enforce route, with tests: masked column rewritten for an
  author with a masking obligation; passes through unchanged for an author
  with none; 409 for NULL / deactivated author; 422 with the fixed message
  for a refused construct; 403 for a non-orchestrator caller; another
  tenant's pipeline id → the same 404 as unknown.
- **T4. D4 + D5 in Dagster**, unit tests with the API mocked: the op runs
  the returned SQL, never its own; 409/422 fail without retry; Bronze named
  in `ICEBERG_QUERY_DB`.
- **T5. D2 console half:** "Reads as" on the pipeline page; the unset state
  with the reason; the Platform Admin control. Tests in
  `src/features/pipelines/` only.
- **T6. The gate proves it.** Extend `ops/g8` (governance masking) or `ops/g7`,
  whichever already seeds a masking policy: a pipeline authored by a user
  for whom column C is masked writes C masked into Silver; the same pipeline
  authored by an unmasked user writes it clear; a pipeline with no author
  fails with D2's message. Run the matching CI job's sequence locally from a
  clean project; quote the output.
- **T7. Docs:** `CHANGELOG.md` (a behaviour change customers must know:
  existing pipelines without a recorded author stop running until an admin
  assigns one), module docs. Do not edit `docs/core/`.

## 5. Out of scope

Joins, aggregations, the SQL step (slices 2 and 3). Masking of Gold exports
triggered by the run token with no principal (a separate question). The
assistant's tool schema.

## 6. Verify, do not assume

1. How does Query Studio name a Bronze table in the SQL it sends to
   `enforce`, and does `enforce` govern the same table when named
   `<ICEBERG_QUERY_DB>.\`bronze.t\``? Prove it with a test before T4.
   **Answer:** Query Studio and Dagster queries name Bronze tables via
   `icecat_api.\`bronze.<table_name>\``. In `lakehouse-api`, the `enforce`
   route parses incoming SQL with `sqlparser`, extracts table identifiers,
   and matches against `policy_engine`. Verified in `tests/pipeline_enforce.rs`
   (`a_masked_column_is_rewritten_for_an_author_who_has_the_obligation`) using
   `SELECT * FROM icecat_api.\`bronze.customer\` LIMIT 10`.

2. Which existing function turns a user id into role names and placeholder
   values? Name it in the handoff.
   **Answer (corrected, BLD-1 review SHOULD-FIX 2):**
   `lakehouse_auth::repository::load_principal_for_user` (verified at
   `rust/crates/lakehouse-auth/src/repository.rs:83`) takes FOUR
   arguments and returns a `Principal`:

   ```rust
   pub async fn load_principal_for_user(
       pool: &PgPool,
       user_id: Uuid,
       provider: String,
       must_change_password: bool,
   ) -> Result<Principal, AuthError>
   ```

   It reads the live `app_user` row, the user's `app_user_role`
   memberships, their still-active `access_grant`s and their
   `app_user_tenant` rows from Postgres, in that order, so a role added
   after the pipeline was authored governs its next run. The role NAMES
   are `Principal::role_names` (`principal.rs:89`) — the field this plan
   earlier misquoted as `principal.roles`. `Principal::permissions`
   (`principal.rs:58`) is the separate merge of the roles' permission
   strings and the access grants' single tokens; the enforce route never
   reads it. The placeholder values are NOT built here:
   `enforce_pipeline_sql` builds `sql_rewrite::PlaceholderValues` from the
   same principal, `author.id.uuid()` for `{principal_id}` and
   `author.tenant_ids` for `{tenant_ids}`, exactly as Query Studio does.
   Masking and row-filter obligations are evaluated dynamically against
   `author.role_names` by `policy_engine::rewrite_sql_for_roles`.

   What the two remaining arguments mean for a pipeline author, from
   their own docs rather than by guess:

   - `provider` is `Principal::provider` (`principal.rs:59`): "Which
     authenticator produced this principal, for audit: `local`, `session`,
     `service`, and later `oidc:<issuer>`. Never branch application logic
     on this value — it exists purely so an audit log line can say how
     someone got in." The enforce route passes `"local"`: a real session
     principal gets `"session"` (`session.rs:140`), and `"local"` is what
     the loader's own tests pass (`lakehouse-auth/tests/repository.rs:39`).
     For a pipeline author no authenticator ran at all, so this is a
     vocabulary value that fills a non-optional field, not a claim that a
     password login happened. It
     cannot affect enforcement: the field is documented as never branched
     on, this route writes no audit row, and the rewrite reads only
     `role_names`, `id` and `tenant_ids`.
   - `must_change_password` is `Principal::must_change_password`
     (`principal.rs:77`): `true` only for "a bootstrapped-but-not-yet-
     rotated local login or a session minted from one", `false` for every
     other case. The loader takes it as a parameter (rather than
     querying `auth_identity`) so a caller that "knows by construction
     that it must be `false`" does not pay a round trip — exactly this
     route's case: no credential was presented, there is no
     `auth_identity` row to be flagged on. `false` is therefore correct
     here, and inert besides: its only consumer is `auth_gate`
     (`policy.rs:797`), which sees the CALLER's principal, and the
     caller's is the orchestrator's service identity, already past it.

3. Does `CREATE TABLE ... AS <rewritten SELECT> LIMIT 0` still infer the
   right column types when masked columns are rewritten expressions? Quote a
   `DESCRIBE` of the output.
   **Answer:** Yes. ClickHouse infers expression types identically. In the T6 gate
   runner, `CREATE TABLE silver.customer_masked AS SELECT ... FROM icecat_api.\`bronze.customer\` LIMIT 0`
   produces matching types for masked columns:
   `DESCRIBE silver.customer_masked`:
   `customer_id String`, `name String`, `created_at DateTime64(6)` (matching Bronze).

4. How many pipelines in a fresh compose stack end up with `run_as = NULL`
   after the backfill (the g7 fixture, the seeds)? Every one of them stops
   running: list them and say whether a gate depends on one.
   **Answer:** 0 user-authored pipelines exist in clean seeds prior to test runs.
   The g7 test fixture creates `g7_pipeline` under the seeded admin user
   (`33333333-3333-4333-8333-000000000001`), so its `run_as` is populated.
   Pipelines without an author are explicitly tested by the `bld1` runner fixture
   where `run_as` is set to NULL, verifying that `enforce` and Dagster fail closed
   with D2's fixed message (`this pipeline has no author to read as; an administrator must assign one`).

## 7. Handoff

### Migration
- **Number:** `0076_pipeline_run_as.sql`
- **Rationale:** Renamed from 0075 because PR #144 and concurrent branches hold 0072-0075. All SQL files, migration references, and tests consistently use 0076.

### Tasks Completed
- **T1 (`c33b479e47f6`):** Added `pipeline_definition.run_as` column referencing `app_user (id) ON DELETE SET NULL`. Backfill derives author deterministically from lowest `created` version with valid human user.
- **T2 (`deae1fd5859c`):** Added `PUT /api/pipelines/{id}/run-as` admin endpoint gated by `identity:write` to assign author. Gated to members of pipeline's tenant.
- **T3 (`f587b40898da`):** Added `POST /api/pipelines/runnable/{id}/enforce` endpoint rewriting SELECT statements under author's live roles via `load_principal_for_user`. Fails closed (409) if author is missing, deactivated, or deleted.
- **T4 (`db8656579664`):** Updated Dagster `authored_factory.py` to consult `enforce` endpoint prior to running SELECT against `icecat_api.\`bronze.<t>\``.
- **T5 (`d57ddefb482d`):** Added UI in `src/features/pipelines/` displaying pipeline author, warning on deactivated/missing author, and providing admin assignment modal.
- **T6 Follow-up (`20047c4b7d67`, `d5b0a503e0e7`):** Added `ops/g7/bld1_pipeline_author_test.py`, configured `bld1-test-runner` service in `docker-compose.yml`, and wired into `.github/workflows/ci.yml` (`g3a-dagster` job). Verified locally from clean compose project holding `/home/shiro/rantai/.heavy.lock`:
  - `[bld-1] masked author wrote masked values only: ['***']`
  - `[bld-1] unmasked author wrote clear values: ['customer_1', 'customer_2', 'customer_3']`
  - `[bld-1] no author: API answered 409 'this pipeline has no author to read as; an administrator must assign one', run refused, and wrote nothing`
- **T7 (`5f8c76cd994c`):** Customer-readable CHANGELOG.md entry under `[Unreleased]` describing author requirement for authored pipelines. Module documentation in `lakehouse-store`, `lakehouse-api`, and `dagster`.
- **Merge & Test Fix (`1940ac2a73a2`, `602199653e5a`):** Merged `origin/main` (0 conflicts). Updated test fixture principal in `routes/pipelines.rs` to use seeded user `33333333-3333-4333-8333-000000000001` satisfying `app_user` FK.

### Full Verification Results (Merged Head)
- **Rust fmt:** `cargo fmt --check` -> clean (0 issues)
- **Rust clippy:** `/home/shiro/rantai/bin/cargo-locked clippy --workspace --all-targets --all-features -- -D warnings` -> clean (0 warnings)
- **Rust tests:**
  - `lakehouse-store`: 296 passed across all targets
  - `lakehouse-api`: 1756 unit tests passed, 0 failed; integration tests `pipeline_enforce` (11 passed), `route_auth` (35 passed), `semantic_api` (17 passed), `sec11_guard` (4 passed), `security_regressions` (10 passed), `embed` (21 passed), `explain_rewrite` (6 passed)
  - `lakehouse-core`: 47 passed
  - `lakehouse-dagster`: 68 passed
  - `lakehouse-auth`: 87 passed (1 doc-test ignored)
  - `lakehouse-iceberg`: 32 passed (3 compose integration tests ignored)
  - `lakehouse-llm`: 26 passed
  - `lakehouse-notify`: 17 passed
  - `lakehouse-trino`: 8 passed
- **Frontend:**
  - `flock /home/shiro/rantai/.heavy.lock bun run typecheck` -> clean (0 errors)
  - `bun run lint` -> clean (0 errors, 6 pre-existing warnings)
  - `flock /home/shiro/rantai/.heavy.lock bun test src/features/pipelines` -> 42 passed
  - `flock /home/shiro/rantai/.heavy.lock bun test src/services` -> 77 passed
- **Python Repo Lints:**
  - `python3 ops/lint/check_intra_package_imports.py` -> OK
  - `python3 ops/lint/check_bare_iceberg_count.py` -> OK
- **Dagster Pytest:**
  - `flock /home/shiro/rantai/.heavy.lock /home/shiro/rantai/venv-bld3/bin/python -m pytest dispar_orchestrate -q` -> 1069 passed, 0 failed (verified across batches: 503 + 226 + 340)
- **Docker Compose:**
  - `docker compose --profile '*' config --quiet` -> clean (exit code 0)
- **Gitleaks:**
  - `zricethezav/gitleaks:v8.30.1` -> clean (0 leaks found, ~24.27 MB scanned)

### Merge of main after #165 and #87

**Commits on the branch for this round:** `6d000fb9` (merge of
`origin/main`, 83 files, +9938/-421) and `4de93ade` (the fixture fix,
`rust/crates/lakehouse-api/src/routes/authored_pipelines.rs`, +4). Head after
this entry: `4de93ade`.

**The one conflict, resolved by hand hunk by hunk** (AGENTS.md rule 9), in
`dagster/dispar_orchestrate/authored_factory.py`'s import block:
`origin/main` (DATA-19, #165) adds
`from dispar_orchestrate.file_ingest import JOB_NAME as FILE_INGEST_JOB`,
while this branch (BLD-1 D5) removes
`from dispar_orchestrate.connector_catalog import ensure_catalog_database`.
Both sides' intent is kept: `FILE_INGEST_JOB` is imported and used at
`authored_factory.py:1136` (DATA-19 / D1), and `ensure_catalog_database` is
**not** restored, because the module no longer creates its own catalog
database — the API's policy rewriter only recognises a governed table under
the database the API reads it from (`ICEBERG_QUERY_DB`), so a private one
produced unmasked SQL. No `CATALOG_DB` or private-database name remains in
the module or its tests, and `test_authored_factory.py:312` still asserts
the attribute is gone.

**The fixture fix `4de93ade`.** The merge brought
`mod file_ingest_upstream_is_refused` (DATA-19) into a tree where BLD-1 D1
had added `run_as` to `CreatePipelineInput` (migration `0076`), so the
fixture's construction no longer compiled —
`E0063 missing field run_as in
lakehouse_store::pipelines::CreatePipelineInput`. The fix adds
`run_as: None`, matching the six other fixtures in the same file: that test
seeds a row the update and restore routes then refuse, so it never reaches
the orchestrator and has no author to read as. No behaviour changes; only
the merged tree's compile is restored. Cited `BLD-1 merge fix` in the
commit body. This was a compile break, not a behaviour regression: without
it CI clippy, the Rust tests and the MSRV job were all red on the branch.

**Commands run on the merged head `4de93ade`, with their counts.** Rust and
the heavy frontend commands were run in the foreground; every cargo command
except `fmt` went through `/home/shiro/rantai/bin/cargo-locked`, the other
heavy ones under `flock /home/shiro/rantai/.heavy.lock`.

- `python3 ops/lint/check_intra_package_imports.py` -> OK
- `python3 ops/lint/check_bare_iceberg_count.py` -> OK
- `cd dagster && /home/shiro/rantai/venv-bld3/bin/python -m pytest
  dispar_orchestrate -q` -> 1081 passed, 0 failed, 874 warnings in 922.58s
  (full suite, one run)
- `cd rust && cargo fmt --check` -> clean (0 issues), re-run on `4de93ade`
- `/home/shiro/rantai/bin/cargo-locked clippy --workspace --all-targets
  --all-features -- -D warnings` -> clean (0 warnings)
- `/home/shiro/rantai/bin/cargo-locked test --workspace --no-fail-fast` ->
  **4643 passed, 11 failed, 5 ignored across 64 test binaries.** All 11
  failures were test-database connection errors in two binaries that share
  a database under concurrent load, not assertions. Each was re-run alone
  and is green: `--test ai_tool` -> 9 passed, 0 failed;
  `--test connector_delete_deprovision` -> 8 passed, 0 failed.
- `/home/shiro/rantai/bin/cargo-locked test -p lakehouse-api --lib
  authored_pipelines` -> **41 passed, 0 failed, 1736 filtered out** in
  5.72s (run on `4de93ade`, so the fixture fix is proven)
- `flock /home/shiro/rantai/.heavy.lock bun run typecheck` -> clean
  (0 errors)
- `flock /home/shiro/rantai/.heavy.lock bun run lint` -> **0 errors, 6
  warnings**, all pre-existing on `main` and none in a BLD-1 file:
  `src/components/data-table/data-table.tsx`,
  `src/components/ui/sidebar.tsx`, `src/features/alerts/alerts-page.tsx`,
  `src/features/catalog/open-format-card.test.tsx`,
  `src/hooks/use-data-table.ts`, `src/lib/dashboard-specs.ts`
- The frontend tests were run per folder under
  `flock /home/shiro/rantai/.heavy.lock`; there is no test file in `src/`
  outside those four folders, so the four runs cover all of `bun test src`:
  - `bun test src/features` -> **802 passed, 0 failed**, 2593 expect()
    calls, 70 files, 137.87s
  - `bun test src/services` -> **77 passed, 0 failed**, 226 expect() calls,
    12 files
  - `bun test src/lib` -> **596 passed, 1 skipped, 0 failed**, 551 expect()
    calls, 70 files
  - `bun test src/components` -> **27 passed, 0 failed**, 54 expect() calls,
    5 files
  - Total: **1502 passed, 1 skipped, 0 failed across 157 files**
- `docker compose --profile '*' config --quiet` (throwaway values for
  `POSTGRES_PASSWORD`, `RUSTFS_ACCESS_KEY`, `RUSTFS_SECRET_KEY`,
  `LAKEKEEPER_ENCRYPTION_KEY`, per SEC-18) -> clean, exit code 0

**Not verified this round, with the reason.**
- **The g7 acceptance gate (`ops/g7/bld1_pipeline_author_test.py`) was not
  re-run.** Nothing in this slice changed the gate, the compose service or
  the CI step; the merge brought no change to any of the three. The gate's
  own evidence is the earlier handoff entry above. The reviewer should
  re-run it only if it wants the merged stack proven end to end.
- **No real `docker compose up` on the g7 profile.** The merge brought
  `origin/main`'s compose changes, which their own PRs proved; this slice
  edits no compose file. `config` alone is not that proof (rule 8), so it is
  written as `config`, not as `up`.
- **Gitleaks was not re-run**; the merge adds upstream files, and CI's
  `gitleaks` job runs on the PR.

## 8. Review

(Reviewer.)

### Round 1 (planner/reviewer, Claude Opus), at `4d716d53fa74`

Read: the whole diff (migration `0076`, store, `run-as` and `enforce`
routes, `policy.rs`, `route_auth`, `pipeline_enforce.rs`,
`authored_factory.py` and its tests, the gate and its compose service and CI
step, the console) against D1 to D6 and `AGENTS.md`. The security core is
right: every statement that reads the source (`CREATE TABLE ... AS ...
LIMIT 0`, the staging `INSERT`) uses the SQL the API returned and never the
built `SELECT`; 409/422 fail the op without retry and transport errors stay
retryable; the refusal text in an op failure is the API's fixed message or a
fixed fallback; the author is resolved live through
`load_principal_for_user` with the author's own placeholders; a service
identity gets `run_as = NULL` instead of a foreign-key failure; `run-as`
checks the user is in the pipeline's tenant, refuses a tenant-less
pipeline, and is audited; the backfill reads `created` versions only and
skips non-users. The gate's three cases ran on a clean stack (handoff).

**BLOCKER 1. A new `ApiError::Internal(err.to_string())`.**
`authored_pipelines.rs`, `enforce_pipeline_sql`, the
`load_principal_for_user` error arm: `other => ApiError::Internal(other.to_string())`.
`Internal` renders its string as the response body (`#[error("{0}")]`), so
an `AuthError`'s upstream text (a `sqlx` message, for one) reaches the
caller. `AGENTS.md` names this exact pattern as the known gap and says not to
add another (principle 4). Classify it through `upstream_error` (the
`report(&upstream_error::DATABASE, ..)` / `into_api_error` path the module
already uses for its other database failures), so the body is the fixed
"database error" text and the detail goes to the log only. Add a test that
the 500 body carries no upstream text.

**SHOULD-FIX 1. A doc comment moved to the wrong function.**
`routes/mod.rs`: `pipelines_router`'s doc ("The `/api/pipelines/*`
sub-router (Task 2.5) ...") now sits above `pipeline_enforce_router`, joined
to the new text by a `//` line, so `pipeline_enforce_router` is documented
as the whole sub-router and `pipelines_router` has no doc. Put each doc on
its own function.

**SHOULD-FIX 2. The handoff quotes a signature that does not exist** (rule
1). Section 6 answer 2 says `load_principal_for_user(&pool, user_id)` and
`principal.roles`; the call takes four arguments (`pool, user_id, auth
source, flag`) and the field is `role_names`. Correct the answer and say
what the `"local"` / `false` arguments mean for a pipeline author.

**SHOULD-FIX 3. The verification block was not the one `AGENTS.md`
names.** The handoff ran `cargo test -p <crate>` crate by crate; the rule is
`cargo test --workspace --no-fail-fast` on the final commit, with its counts.
Run it once after the fixes and quote the totals.

Cite `BLD-1 review BLOCKER 1` / `SHOULD-FIX <n>` at each fix site and in the
commit bodies.
