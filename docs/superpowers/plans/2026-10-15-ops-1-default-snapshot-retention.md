# `OPS-1` Default snapshot retention: plan

Planner: Claude Opus (Security & Delivery lane). Developer: a non-Opus agent.
Base: `origin/main` at `41d7677a`. Branch: `feat/ops-1-default-snapshot-retention`,
worktree `/home/shiro/rantai/wt/ops-1`. Spec: `docs/core/specs/ops-1.md`
(D1–D3 signed by the product owner 2026-10-15). A Task: no feature page.

## 1. Anchors (verified at `41d7677a`)

- `dagster/dispar_orchestrate/maintenance.py`
  - module docstring, `snapshotsToKeep` paragraph near line 130: Trino 483's
    `expire_snapshots` takes only a `retention_threshold` age; Trino's
    `iceberg.expire-snapshots.min-retention` floor is 7 days by default.
  - `_run_maintenance_for_table` (line 544): collects `skipped_verbs`,
    calls `_expire_snapshots_result` (line 842) and `_optimize_result`.
  - `_expire_snapshots_result`: with no policy or no `snapshotsToKeep`
    (line 862) it returns the ClickHouse probe's refusal as the skip and
    makes no Trino call. With `snapshotsToKeep` it reads
    `"<t>$snapshots"`, computes a threshold (`_compute_retention_threshold`,
    line 812), runs `ALTER TABLE … EXECUTE expire_snapshots(retention_threshold => '<n>s')`,
    and maps `TrinoUnavailableError` to `_TRINO_UNREACHABLE_SKIP` and
    `TrinoQueryError` to `refused`.
  - `_cadence_allows` applies a policy's `schedule`.
- `dagster/dispar_orchestrate/test_maintenance.py`: the unit tests for the
  above (mocked Trino, no network).
- `bronze_maintenance_schedule` runs daily at 03:00, `RUNNING` by default.
- `README.md` lines 439-447 ("Bronze snapshot history is currently
  unbounded"); `docs/adr/0004-bronze-naming-partitioning-retention.md`
  lines 85-131 (no retention); `docs/OPERATIONS.md` maintenance section.

## 2. Decisions

- E1. One constant in `maintenance.py`, `DEFAULT_SNAPSHOT_RETENTION = "7d"`,
  with a comment citing D1 and Trino's floor. No environment variable
  (the spec excludes a setting).
- E2. `_expire_snapshots_result`: when `keep is None` (no policy, or no
  `snapshotsToKeep`), run `ALTER TABLE iceberg.<ns>."<t>" EXECUTE
  expire_snapshots(retention_threshold => '7d')` through the same
  `_trino_execute`, with the same error mapping; the verb-run record says
  `detail = "default retention 7d (no snapshotsToKeep policy)"` on success.
  No cadence check for the default (the job is daily). The ClickHouse
  probe's refusal is no longer the skip reason when Trino ran; keep the
  probe itself (it is the measurement that ClickHouse still refuses) and
  record its outcome only if the module already does so elsewhere —
  do not drop information the governance view shows today without saying
  so in the commit body.
- E3. Identifiers in the SQL stay server-controlled (namespace and table
  come from the catalog listing, as today); no user input enters it.
- E4. Nothing changes for a policy with `snapshotsToKeep` (D2).

## 3. Tasks

**T1. Default retention in `maintenance.py` (E1–E4) with tests.**
Tests in `test_maintenance.py`, full-sentence names, mocked Trino:
- `test_table_without_policy_expires_snapshots_older_than_seven_days`:
  the SQL sent is exactly the `'7d'` statement; outcome `applied`.
- `test_table_without_policy_records_trino_unreachable_as_skipped`.
- `test_table_without_policy_records_a_trino_refusal_as_refused`.
- `test_policy_with_snapshots_to_keep_still_uses_its_own_threshold`
  (existing behaviour, keep or extend the existing test).
Update the module docstring paragraph that says no policy means no
expiry. *Check:* `cd dagster && ~/.cache/rantai-dagster-venv/bin/python -m pytest dispar_orchestrate/test_maintenance.py -q`
(under the lock) and then the whole package; quote both totals. Mutation:
change `'7d'` to `'8d'` and show the first test fail, then restore.

**T2. Measure it (`docs/plans/OPS-1-RESULT.md`).** On a local stack with
the `dagster` and `trino` profiles, a unique project name (`-p ops1check`),
`.env` from `sh ops/init-env.sh` if `SEC-18` (#83) has merged, else a
throwaway `.env`:
1. Create or ingest one Bronze table and write to it enough times to have
   at least 10 snapshots (any existing gate seed that writes repeatedly is
   fine; say which).
2. Run the maintenance job once with no policy; record the verb run
   (expected: `applied`, nothing older than 7 days, snapshot count
   unchanged) — this is AC1.
3. Stop `trino`, run again: record `skipped` / "trino unreachable" (AC2).
4. To show snapshots are actually reclaimed, give the table a policy with
   `snapshotsToKeep: 2` and, for this measurement only, start Trino with
   `iceberg.expire-snapshots.min-retention=0s` through a file under
   `ops/` (an override used only for the measurement, never by compose's
   default start; say exactly how). Record snapshot count and the size of
   the table's metadata directory in RustFS before and after.
Follow the shape of `docs/plans/G3-RESULT.md`: environment and image
versions, commands, a results table, what this decides, and "Not
measured". `down -v` afterwards. *Check:* the file exists and every number
in it came from a command quoted in it.

**T3. Docs.** `README.md` bullet at 439-447 rewritten (expiry by default,
7 days, needs the `trino` profile, see OPS-1-RESULT); `docs/adr/0004`
gets an `## Addendum (2026-10-15, OPS-1)` with Context/Decision/
Consequences/Verification; `docs/OPERATIONS.md` maintenance section;
`CHANGELOG.md` `[Unreleased]` one bullet.

## 4. Pull request

One slice. The planner merges #171 (`DOC-1`, touches `README.md`) first;
merge `origin/main` into this branch before the handoff if it moved.

## 5. Out of scope

Compose changes (the measurement override lives under `ops/` and is not
wired into `docker-compose.yml`); Lakekeeper's task queue; compaction;
any Rust or console change.

## 6. Verify, do not assume

- Does `probe_expire_snapshots_skip` feed anything the API or console
  shows (grep `skipped_verbs` in `rust/` and `src/`)? Keep what they read.
- Is `"7d"` accepted by Trino 483 as a `retention_threshold` literal, or
  does it need `'7.00d'`? Check Trino's docs or the existing tests'
  format, and use the measured form.

## 7. Handoff

- **Branch**: `feat/ops-1-default-snapshot-retention`
- **Base**: Merged `origin/main` (commit `0a97abfe`) via merge commit `146b1396`.
- **Commits**:
  - `2632cecb`: `feat(dagster): default snapshot retention to 7d (OPS-1)`
- **T1 Implementation**:
  - `dagster/dispar_orchestrate/maintenance.py`: Added `DEFAULT_SNAPSHOT_RETENTION = "7d"`. Updated `_expire_snapshots_result` so tables without a governance policy or with `snapshotsToKeep: None` call Trino:
    `ALTER TABLE iceberg.{namespace}."{name}" EXECUTE expire_snapshots(retention_threshold => '7d')`
  - Handled Trino error classifications: `TrinoUnavailableError` -> skip with logged warning; `TrinoQueryError` -> refusal; 5xx/unreachable policy fetch -> skip cleanly without attempting default expiry; 401/403 -> fail whole run.
  - Tests in `test_maintenance.py`: Added 4 tests verifying default 7-day retention expiry, refusal recording, Trino unreachable skip, and explicit `snapshotsToKeep` retention.
- **Verification Totals**:
  - `dispar_orchestrate/test_maintenance.py`: **43 passed in 1.28s**.
  - Mutation check: Mutated `DEFAULT_SNAPSHOT_RETENTION = "7d"` to `"8d"`. `test_table_without_policy_expires_snapshots_older_than_seven_days` failed with `AssertionError: 'default retention 8d ...' != 'default retention 7d ...'`. Restored to `"7d"`; test passed.
  - Full suite `dispar_orchestrate -q`: **1054 passed, 13 failed**. All 13 failures are pre-existing on `main` in `test_history_write_measurement.py` due to Python 3.14 / SQLAlchemy / `pyiceberg.catalog.sql.SqlCatalog` typing compatibility.
- **T2 Measurement Status**:
  - Halted per instruction: *"If T2 cannot be done as written (no seed gives 10 snapshots, or the Trino override would need an edit to docker-compose.yml), commit and push T1, then stop and report exactly what blocked T2."*
  - **Blocker 1 (No 10-snapshot seed)**: No existing seed in the repository produces 10 snapshots against a Bronze table. Existing seeds (`g3a-source-init`, `g4-source-init`, `g6-source-init`) write once and yield 1 snapshot.
  - **Blocker 2 (Trino min-retention override requires compose change)**: `docker-compose.yml:trino` writes `/etc/trino/catalog/iceberg.properties` via an inline shell heredoc in `command:`. Injecting `iceberg.expire-snapshots.min-retention=0s` cannot be done via environment variables and would require modifying `docker-compose.yml` (forbidden per Section 5: *"Compose changes (the measurement override lives under ops/ and is not wired into docker-compose.yml)"*).
- **T3 Documentation**:
  - `README.md` lines 459-470 rewritten to reflect default 7-day snapshot retention floor via Trino.
  - `docs/adr/0004-bronze-naming-partitioning-retention.md` updated with `## Addendum (2026-10-15, OPS-1)`.
  - `docs/OPERATIONS.md` updated in the maintenance section.
  - `CHANGELOG.md` updated under `## [Unreleased]` -> `### Added`.
- **Pre-commit / Hygiene Checks**:
  - `check_intra_package_imports.py`: OK.
  - `check_bare_iceberg_count.py`: OK.
  - Gitleaks scan: Clean (0 leaks found).

### Handoff (2026-10-16, round 2 — T2, the measurement)

1. **Commits** (`git log --oneline origin/main..HEAD`):
   - `76cd01c9` `chore(trino): measurement-only override for the OPS-1 snapshot floor`
   - `0d9b73e0` `docs(plans): OPS-1 result, the measured default snapshot retention`

2. **Verification actually run** (every command below ran in this worktree, in the
   foreground, on the pushed commit; logs under `/home/shiro/rantai/review-logs/`):
   - `sh ops/init-env.sh` → wrote a local, gitignored `.env` (SEC-18: no default credentials).
   - `docker compose --profile '*' config --quiet` → OK, unchanged by the new file
     (run again after the override was added).
   - `docker compose -f docker-compose.yml -f ops/trino/docker-compose.ops1-measure.override.yml --profile trino config` → OK; the rendered `services.trino.command` is byte-identical to the base file's apart from one added line once shell comments are stripped, verified by diffing the two configs.
   - `docker compose -p ops1check --profile dagster build dagster-code-location` → `Image rantai-lakehouse-dagster:local Built`.
   - `docker compose -p ops1check --profile dagster --profile trino up -d lakehouse-api dagster-code-location dagster-webserver dagster-daemon trino trino-maintenance-cron` → started; `docker compose -p ops1check ps` reports `trino running healthy`, `dagster-webserver healthy`.
   - Ten `docker compose -p ops1check exec -T trino trino --server http://trino:8080 --user trino-maintenance --execute "INSERT INTO iceberg.bronze.ops1_measure VALUES (current_timestamp, N)"` → `INSERT: 1 row` for each, then `count(*)` on `"bronze.ops1_measure$snapshots"` → `10`.
   - Four runs of the maintenance job, each `curl -X POST /api/pipelines/bronze_maintenance_job/trigger`, each polled to `completed` (`GET /api/pipelines/bronze_maintenance_job/runs`): `033c8b95…`, `90e38e5a…`, `6f44f86c…`, `64276604…`.
   - `docker compose -f docker-compose.yml -f ops/trino/docker-compose.ops1-measure.override.yml -p ops1check --profile trino up -d trino` → `grep min-retention /etc/trino/catalog/iceberg.properties` → `23:iceberg.expire-snapshots.min-retention=0s` (the override proven by a real `up`, not by `config`).
   - `docker compose -p ops1check --profile dagster --profile trino down -v` → all 21 containers, 6 volumes and the network removed; `docker ps -a | grep ops1check`, `docker volume ls | grep ops1check`, `docker network ls | grep ops1check` each print nothing. The unrelated `ops7drill-*` stack and its testcontainers were left untouched.
   - gitleaks (`zricethezav/gitleaks:v8.30.1 dir …`) before each push: 9 findings, all in the untracked, gitignored local `.env` (the random values `init-env.sh` generated); zero in the committed files.
   - *Not verified*: the Rust, TypeScript, Dagster-pytest and console blocks of the project's verification list. This round adds only a YAML file under `ops/` and a Markdown result document — no `rust/`, `src/`, `dagster/` or `docker-compose.yml` change — so the scoped checks were the compose ones quoted above. T3's docs were verified in round 1 and are unchanged since.

3. **Premises that turned out wrong, and what was skipped**
   - Round-1 BLOCKER 1's "create the measurement table through Trino
     (`CREATE TABLE iceberg.<bronze namespace>.ops1_measure (id bigint)`)" is not
     possible in this build: Lakekeeper's `trino` principal holds
     `select modify` and never `create` (`lakekeeper-authz-init`'s
     `grant trino select modify`), and `docker-compose.yml`'s `trino` service
     records the reproduced `ForbiddenException` for both `trino-maintenance`
     and `console-reader`. The table was created instead with the `dlt`
     principal (the one `dagster-code-location`'s bronze ingest writes as)
     through pyiceberg, with the same Bronze shape (day-partitioned
     `_ingested_at`, format-version 2). Recorded in `OPS-1-RESULT.md`.
   - T2 step 2 expects "snapshot count unchanged". Measured **10 → 11**:
     `trino-maintenance-cron` (ADR 0009's 6-hour loop, which starts with the
     `trino` profile) rewrote the measurement table at
     `2026-10-15T22:43:05Z` (`rewritten_data_files_count=10,
     added_data_files_count=1`) — one `replace` snapshot, from its own log, not
     from the maintenance job. The cron was then stopped and every later
     measurement quotes it.
   - Two stack facts, both measured and recorded in `OPS-1-RESULT.md`:
     Lakekeeper's warehouse has no `default-prefix`, so a new table lives at
     `s3://lakehouse-warehouse/<uuid>/` rather than `bronze/<name>/`; and the
     whole `dagster` profile could not be started profile-wide because this
     host cannot pull `debezium-server`'s digest-pinned image, so the stack was
     brought up by service name.
   - **Found, not fixed** (report item 5):
     `GET /api/lakehouse/tables/{ns}/{table}/maintenance` answers
     `lastRun: null` / `lastVerbRuns: []` for a Bronze table, because it filters
     `bronze_meta.*` on the bare table name from the URL while the job records
     the qualified `bronze.<name>`; the same query with the qualified name
     returns the row. Pre-existing on `origin/main` (this branch touches no
     `rust/` file — `git diff --stat origin/main...HEAD` lists none) and the
     aggregate `GET /api/governance/maintenance` shows the run correctly.
     Location: `rust/crates/lakehouse-api/src/routes/lakehouse.rs`
     (`maintenance_verb_run_query`) and
     `rust/crates/lakehouse-api/src/routes/governance.rs`
     (`maintenance_run_query`), against
     `dagster/dispar_orchestrate/maintenance.py`'s
     `discover_bronze_tables`-sourced `table_name`.

4. **Plan-file state**

```text
$ git status --short
 M docs/superpowers/plans/2026-10-15-ops-1-default-snapshot-retention.md

$ git log --oneline origin/main..HEAD | head
0d9b73e0 docs(plans): OPS-1 result, the measured default snapshot retention
76cd01c9 chore(trino): measurement-only override for the OPS-1 snapshot floor
2dae5fbf docs(plan): OPS-1 review round 1: T2 is required, with the way through
0e666b77 docs: default 7d snapshot retention (OPS-1)
2632cecb feat(dagster): default snapshot retention to 7d (OPS-1)
146b1396 Merge remote-tracking branch 'origin/main' into 'feat/ops-1-default-snapshot-retention'
81484aca docs(plan): OPS-1 spec and plan, default snapshot retention

$ git ls-remote --heads origin feat/ops-1-default-snapshot-retention
0d9b73e03f53fd6c300219f84535de8ee4e26de0	refs/heads/feat/ops-1-default-snapshot-retention
```

## 8. Review

Round 1, reviewed 2026-10-15 at `0e666b77`.

T1 is right: no policy, or no `snapshotsToKeep`, runs
`expire_snapshots(retention_threshold => '7d')` through Trino with the
existing error mapping; a policy-fetch failure skips expiry instead of
applying the default (correct: a policy might keep older snapshots, so
expiring blind could delete what it protects). The ClickHouse probe's
refusal is no longer the recorded reason when Trino runs; acceptable,
since the verb run now records what happened. Mutation evidence quoted.

The 13 failures in `test_history_write_measurement.py` reproduce on `main`
(`8e2f6a19`, planner's run: 13 failed) with the local venv's Python
3.14.6; CI uses 3.11. Not this PR's; noted for the environment.

**BLOCKER 1. T2 (the measurement) is not done.** Both reported blockers
have a way through that the plan allows:

- 10 snapshots: create a measurement-only table through Trino
  (`CREATE TABLE iceberg.<bronze namespace>.ops1_measure (id bigint)`) and
  run 10 single-row `INSERT`s; each commit is one snapshot. Drop it at the
  end. No seed change needed.
- The 0s floor: an override file under `ops/` (for example
  `ops/trino/docker-compose.ops1-measure.override.yml`) that redefines the
  `trino` service's `command:` with the same heredoc plus
  `iceberg.expire-snapshots.min-retention=0s`, used only as
  `docker compose -f docker-compose.yml -f <that file> …` during the
  measurement. That is a file under `ops/`, not an edit to
  `docker-compose.yml`, which is what section 5 means. Its header says it is
  for this measurement only and must never be used by a deployment.

Then do T2 steps 1-4 as written (step 4 measures with that override;
steps 2 and 3 with the plain file) and write `docs/plans/OPS-1-RESULT.md`.

Round 2, reviewed 2026-10-16 at `9fca593f` (developer: Stepfun), then
`origin/main` merged in (`CHANGELOG.md`: both sides kept). BLOCKER 1 is
fixed: `ops/trino/docker-compose.ops1-measure.override.yml` (used only with
`-f`; its header says it must never reach a deployment) and
`docs/plans/OPS-1-RESULT.md`. Every number in the result was checked
against the raw log `review-logs/ops1-measure.log` (before 10 snapshots,
41 objects; AC2 unchanged 11 / 56 / 237,422 B; 0s floor after 2 snapshots,
39 objects, 191,622 B). No open finding.

Re-run by the planner on the merged head: `check_intra_package_imports.py`
OK, `check_bare_iceberg_count.py` OK, `check_compose_init_readiness.py`
exit 0; `docker compose --profile '*' config --quiet` and the same with
`-f ops/trino/docker-compose.ops1-measure.override.yml --profile trino`
both exit 0; Dagster tests on Python 3.11 as CI runs them
(`pytest dispar_orchestrate -m "not loader"`): 805 passed, 258 deselected,
0 failed. (The local 3.14 venv fails 223 tests with a `SQLAlchemy` typing
error on `main` too; it is not a usable gate.) gitleaks `dir`: 9 findings,
all in the gitignored, uncommitted `.env` the drill generated.

The result found a bug outside this item: the per-table maintenance route
filters on the bare table name while the job records the qualified one, so
`lastRun`/`lastVerbRuns` are always empty for Bronze tables. Filed as
`OPS-9`.
