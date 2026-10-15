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

(developer)

## 8. Review

(planner)
