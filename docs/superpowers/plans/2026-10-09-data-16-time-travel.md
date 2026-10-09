# `DATA-16` Time travel proven safe — Implementation Plan

**Status:** ready to build. Decision 1 on the feature page (split Trino off)
was signed by the product owner on 2026-10-09. Decisions 2 and 3 are the
planner's defaults, flagged to the owner.
Written 2026-10-09 by the planner (Claude Opus) for a developer agent, under
the role split in `AGENTS.md`.

**Spec:** `docs/core/specs/data-16.md`. Backlog `DATA-16`.
**Feature page:** `docs/core/features/time-travel.md`. Decision numbers
below (D1–D3) are its rows.

**Why.** Querying an earlier version of a raw table works on ClickHouse with
no proof that masking still applies, and is refused on Trino with a message
about policies. Query Studio's picker writes the form that is refused.

---

## 1. Slice and base

One pull request. Base `origin/main` at `a78ad62`, branch
`fix/data-16-time-travel`, worktree `/home/hv/lakehouse-data16`. Commits stay
local; the planner pushes and opens the pull request.

## 2. What exists today

Anchors verified at `origin/main` `a78ad62`. `A` is
`rust/crates/lakehouse-api/src`. Rows marked *run* were run by the planner on
2026-10-09 against a live API and the dev ClickHouse; the rest is read from
the code.

| # | Code | Verdict |
| --- | --- | --- |
| F1 | *Run.* `POST /api/query/run`, engine `clickhouse`, ``SELECT count() FROM icecat_api.`bronze.northwind_categories` SETTINGS iceberg_snapshot_id = <id>`` returned 14, 28, 42 and 56 for the table's four versions. The same through ``SELECT count() FROM (SELECT category_id FROM icecat_api.`bronze.…`) x SETTINGS …``, the shape `substitute_table_factor` (`A/sql_rewrite.rs:771`) produces, returned 14. | ClickHouse time travel works and survives the rewrite's shape. Not proven with a real policy. |
| F2 | No gate and no Rust test runs a version-pinned query. `ops/g8/g8_governance_test.py` proves masking on a `MergeTree` table at its current state only; its stack has no Iceberg table. | The proof the spec asks for does not exist. |
| F3 | *Run.* Engine `trino`, `SELECT * FROM iceberg.bronze.orders FOR VERSION AS OF 123`: `422 kebijakan tidak dapat dievaluasi: query tidak dapat diproses`. `SELECT 1` on the same engine: `503 trino unavailable`, so the first answer comes before Trino is called. `rewrite_sql_for_roles` (`A/policy_engine.rs:237`) parses with `GenericDialect` (`A/routes/query.rs:124`) and returns `Unparseable`. `sqlparser` 0.62 reads a table version only in dialects with `supports_table_versioning`, which `GenericDialect` is not, and then as `VERSION AS OF`, never `FOR VERSION AS OF`. | Past versions on Trino are refused for every user, with a message about policies. |
| F4 | `build_masked_filtered_select` (`A/sql_rewrite.rs:892`) writes `replaceRegexpOne(toString(…))`, ClickHouse functions, for both engines. | By reading, not run (no Trino here): a table with a masking policy cannot be read on Trino at all. Not this task: `DATA-21`. |
| F5 | `src/lib/snapshot-picker.ts:21` `insertAsOfClause` only ever writes `FOR VERSION AS OF`. `IcebergTimeTravelControls` (`src/features/queries/query-studio-page.tsx:33`) is shown whatever the engine (`:192`), contradicting its own doc comment (`:30`), and lists `{id} · {operation}` with no time (`:111`). The engine is `studio.engine` (`src/features/queries/use-query-studio.ts:36`). | The picker's only output is the form F3 refuses. |
| F6 | `assetSnapshotQueryHref` (`src/lib/asset-query.ts:80`) writes the ClickHouse form for a `bronze.` table, and the Trino form when the target engine is Trino (`:89`). The API always sends `clickhouse` (`A/routes/catalog.rs`, `queryTarget`). | The asset page works. Its Trino branch would produce F3's query. |
| F7 | *Run.* One `SETTINGS iceberg_snapshot_id` applies to every Iceberg table in the query: a query reading two tables at one id fails with `No snapshot found for id` for the second. | A limit, stated to the user (D3). |
| F8 | The Snapshots card (`src/features/catalog/asset-activity.tsx:419`) and the picker say nothing about how far back versions go. | D2. |
| F9 | *Run.* A version id that does not exist is answered `422` with ClickHouse's own error text. Query Studio shows engine errors for every query today. | Not this task: `SEC-11`. Named so it is not mistaken for new. |

## 3. Decisions already made by the planner

- **Nothing in `sql_rewrite.rs` changes.** ClickHouse time travel already
  passes through it (F1). This task proves it; it does not touch the
  enforcement code.
- **Trino is refused with its own message, before the policy message.** In
  `rewrite_sql_for_principal_inner` (`A/routes/query.rs:100`), when the
  engine is Trino and the rewrite answers `Unparseable`, look at the SQL's
  tokens (the `sqlparser` tokenizer, so a string or a comment is not
  mistaken for the clause) for the keyword run `VERSION AS OF` or `TIMESTAMP
  AS OF`. If present, the error is `ApiError::Unprocessable` with the fixed
  text "Querying a past version of a table is available on the ClickHouse
  engine only." Otherwise the error is what it is today. The check runs only
  after a parse failure: a query that parses is never affected. This is a
  message, not a gate: the query was already refused.
- **One function writes the pin, per engine.** `src/lib/snapshot-picker.ts`
  gains `pinSnapshot(sql, engine, snapshotId)`: for `clickhouse` it sets
  `iceberg_snapshot_id` in the query's trailing `SETTINGS` clause (adds the
  clause, adds the setting to an existing clause, or replaces an existing
  `iceberg_snapshot_id`), keeping the id a string as today; for any other
  engine it returns `null`. `insertAsOfClause` and its tests are removed
  with their one caller. `assetSnapshotQueryHref` uses `pinSnapshot` for its
  ClickHouse branch and returns `null` for Trino, with a comment naming
  `DATA-21`.
- **The picker follows the engine.** `IcebergTimeTravelControls` takes the
  engine. On ClickHouse: options read "<date and time> · <operation>" (the
  id as the option's title), the button reads "Query this version", and a
  note says "One version applies to every raw table in the query." On Trino:
  the controls are disabled and the note says "Past versions can be queried
  on ClickHouse only." The table chosen in the picker is no longer written
  into the SQL (the setting is query-wide); it only selects whose versions
  are listed.
- **Retention is what the table shows (D2).** A small pure function gives
  "N versions, the oldest from <date>" from the snapshot list; the Snapshots
  card and the picker both show it. No day count is promised.
- **The gate is a new script on the Bronze-ingest stack.**
  `ops/g8/g8_time_travel_test.py`, run by a new compose service
  `g8-time-travel-test-runner` (profile `dagster`, same shape and
  dependencies as `g3a-test-runner`, mounting `ops/g8`), in a new CI job
  shaped like `g3a-dagster`. It needs a Bronze Iceberg table with two
  versions, which only that stack makes. It does not import from
  `g8_governance_test.py` or `g3a_test.py`: gate scripts stand alone by this
  repo's convention (`ops/g3a/g3a_test.py` module docstring).

## 4. Tasks

One task per commit, in this order. Cite `DATA-16` and the finding at each
fix site and in the commit body (rule 13).

**T1. The Trino message (F3).** As decided. *Check:* unit tests beside the
existing ones in `routes/query.rs`: `FOR VERSION AS OF 1`, `FOR TIMESTAMP AS
OF TIMESTAMP '2026-01-01 00:00:00'` and `VERSION AS OF 1` on Trino give the
fixed text; the same words inside a string literal, on a query that fails to
parse for another reason, give today's text; a query that parses is
untouched; on ClickHouse nothing changes.

**T2. One pin function (F5, F6).** `pinSnapshot` and the retention function
in `src/lib/snapshot-picker.ts`; `assetSnapshotQueryHref` uses it. *Check:*
tests in `snapshot-picker.test.ts` and `asset-query.test.ts`: no `SETTINGS`
→ clause added on its own line; existing `SETTINGS max_threads = 1` → the
setting appended; existing `iceberg_snapshot_id` → replaced, once; a
trailing `;` and trailing whitespace handled; the word `SETTINGS` inside a
string literal is not taken for the clause; a 19-digit id is kept digit for
digit; Trino → `null`; the retention text for 0, 1 and 4 versions.

**T3. The picker and the Snapshots card (F5, F7, F8).** As decided.
*Check:* tests beside the existing ones for Query Studio and
`asset-detail-tabs.test.tsx`: on ClickHouse an option shows the time and the
operation, applying a version changes the SQL through `pinSnapshot`, the
note about one version is shown; on Trino the controls are disabled and the
ClickHouse-only note is shown; the Snapshots card shows the retention text.

**T4. The gate (F1, F2).** `ops/g8/g8_time_travel_test.py`, narrative
docstring saying what it proves and what it cannot. Steps:
1. wait for the stack; sign in as the bootstrap admin;
2. load the Bronze table twice through `bronze_ingest_job`, changing the
   source between the loads so the two versions differ in row count (use
   the source the `g3a` stack already seeds; say in the docstring which
   write mode the job uses and what that makes the two versions);
3. read the table's versions from `GET /api/lakehouse/tables/{ns}/{table}`
   and take the older one;
4. seed an Analyst login and author, through `POST /api/governance/policies`,
   a ready policy for role Analyst on the table's policy key
   (`bronze.<table>`) that masks one column and filters rows on another
   (read `PolicyCondition::parse` for the shape);
5. assert, through `POST /api/query/run` on ClickHouse with the older
   version pinned: the admin gets clear text and the older version's row
   count; the Analyst gets `***` in the masked column, only rows the filter
   allows, and never more rows than the older version holds; the unmasked
   sibling column is clear for both;
6. assert the same for the Analyst with the pin written inside a subquery
   and inside a CTE (masked or refused, never clear text);
7. assert the current version (no pin) differs in row count from the pinned
   one for the admin, so the pin is shown to have done something.
Compose: the runner service, `restart: "no"`, env by `${X:-}` like its
neighbours, no default credential. CI: the new job in
`.github/workflows/ci.yml`, with the same `needs`/`if` scope as
`g3a-dagster`, added to whatever list the required gate checks
(`scripts/ci/required_gate.sh`; a job missing from it fails that script's
own test). *Check:* `docker compose --profile '*' config --quiet`; the two
Python lint lines; `python3 -m py_compile`; `bash
scripts/ci/tests/test_required_gate.sh` and
`scripts/ci/tests/test_detect_change_scope.sh`. The gate's first real run is
CI's: this machine does not bring up the Dagster stack (rule 8 is met by the
CI job, which starts from a clean project; the handoff says *not verified*
locally).

**T5. Docs (planner, same branch).** `CHANGELOG.md`, `docs/core/BACKLOG.md`
(`DATA-16`, new `DATA-21`), `docs/CI.md` if it lists the gates.

## 5. Build limits on this machine

Same as `docs/superpowers/plans/2026-10-08-src-6-broken-connectors.md`
section 5: shared target dir and two jobs
(`CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-catalog-target
CARGO_BUILD_JOBS=2`), `df -h /` before each cargo command, `touch
rust/crates/*/src/lib.rs rust/crates/lakehouse-store/src/*.rs` before a
quoted clippy run, no `cargo test` here (Rust tests are *not verified*; CI
runs them first), `cargo fmt --check` and `cargo clippy -p lakehouse-api
--all-targets -- -D warnings`. TypeScript: `bun install --frozen-lockfile`,
then `bun run typecheck && bun run lint && bun run test` in full. No docker
on this machine for this task.

## 6. Out of scope

- Anything in `sql_rewrite.rs` or `policy_engine.rs`.
- Trino: parsing the version clause, masking in Trino's own functions, a
  gate with Trino (`DATA-21`).
- Two tables at two versions; querying by timestamp; `SEC-11`'s error text.
- The `/api/lakehouse` routes and the maintenance policy.

## Handoff

### Developer, 2026-10-09 (tasks T1 to T4; T5 is the planner's)

**Commits** (branch `fix/data-16-time-travel`, local only, nothing pushed):

- `5973826` fix(query): say that past versions are ClickHouse-only when Trino refuses one (T1)
- `fb9df43` feat(lib): one function writes the ClickHouse snapshot pin, and the retention text
- `e1792f3` feat(frontend): the time-travel picker follows the engine and the Snapshots card shows retention (T3)
- `18c844a` test(ops): a gate proving a past version of a governed Bronze table keeps its policy (T4)

**Commands run and results**

- Rust, from `rust/`, `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-catalog-target CARGO_BUILD_JOBS=2`, `df -h /` 46G free before each: `cargo check -p lakehouse-api` clean. After `touch` of the crate sources: `cargo fmt --check` clean (it flagged only my own hunks first; `cargo fmt` fixed exactly those), `cargo clippy -p lakehouse-api --all-targets -- -D warnings` finished with no warnings (run again after the fmt change).
- `bun install --frozen-lockfile` (772 packages, lockfile untouched). Full, foreground, on the final commit: `bun run typecheck && bun run lint && bun run test` exit 0; tests 922 pass, 1 skip, 0 fail, 923 tests in 101 files; lint 0 errors, 6 warnings, all in files this branch does not touch (`data-table.tsx`, `sidebar.tsx`, `alerts-page.tsx`, `open-format-card.test.tsx`, `use-data-table.ts`, `dashboard-specs.ts`).
- `python3 ops/lint/check_intra_package_imports.py` OK; `python3 ops/lint/check_bare_iceberg_count.py` OK; `python3 -m py_compile ops/g8/g8_time_travel_test.py` OK.
- `bash scripts/ci/tests/test_required_gate.sh`: 27 passed, 0 failed. `bash scripts/ci/tests/test_detect_change_scope.sh`: 32 passed, 0 failed.
- `sg docker -c "docker compose --profile '*' config --quiet"` from the worktree with no `.env`: exit 0.

**Not verified**

- Rust tests (`cargo test` is not allowed here): the two new unit tests in `routes/query.rs` and every existing one are *not verified (first run is CI's)*. The wiring of `uses_past_version_clause` into `rewrite_sql_for_principal_inner` is covered by compile and clippy only; the helper is tested directly, the Trino-and-parse-failure branch is not exercised by a test (it needs `AppState`).
- The gate `ops/g8/g8_time_travel_test.py`, the new compose service and the `g8-time-travel` job have never been run: no container was started. *Not verified*; the first real run is CI's.
- The picker's Select interaction is tested with synthetic mouse events in `bun test`, not in a browser.

**Deviations from the plan, and decisions the plan did not cover**

1. `insertAsOfClause` is removed in T3, not T2 (T2 would not compile, because the picker still called it). T2 keeps it with a note; T3 removes it and its tests with the caller.
2. The controls moved to their own file, `src/features/queries/time-travel-controls.tsx`, so they can be tested; there was no Query Studio page test to sit beside.
3. `pinSnapshot` also returns `null` for an id that is not all digits (it is interpolated into SQL).
4. Retention text: `No versions yet.` / `1 version, from <date>` / `N versions, the oldest from <date>`, the date from `formatDateTime`. The Snapshots card appends it to its description when the list is non-empty.
5. Gate: `bronze_ingest_job` writes with the default `LoadPlan`, mode `append`, so each load adds the whole source as a new snapshot. The source is changed between the loads with `psql` (10 rows with customer `g8tt_late` inserted into `ingest_demo.orders`). Older version = newest snapshot after load 1, current = newest after load 2. The table is `BRONZE_TABLE_NAME` (default `g3a_orders`), not its own table, because the job's target table is fixed by the Dagster service's env.
6. Policy shape: `{"roles":["Analyst"],"table":"bronze.<table>","mask":["customer"],"rowFilter":"id <= 1000"}`, kind `Row filter`, effect `Permit with obligation`.
7. The gate also checks the admin's pinned count equals the snapshot's `summary.totalRecords` when the API reports one, and uses `WHERE`-qualified counts everywhere (R11).
8. The subquery and CTE shapes accept `422` as "refused", like `g8_governance_test.py`; the docstring says this cannot tell the policy refusal from ClickHouse rejecting a `SETTINGS` clause there.
9. Compose service `g8-time-travel-test-runner` also depends on `clickhouse-iceberg-init` (the `icecat_api` database; `lakehouse-api` does not depend on it) and installs `postgresql-client` and `argon2-cffi` like `g8-test-runner`. The CI job names `clickhouse-iceberg-init` in its `up` list, and the job is named `g8-time-travel` with result variable `G8_TIME_TRAVEL_RESULT` in `required_gate.sh`, its test (unset list, every base array, one new failing-job case) and the `ci-required` needs and env.
10. `docs/CI.md` lists the acceptance jobs and still needs `g8-time-travel` (T5, planner).

**Unsure**

- Whether `icecat_api` sees the table the moment the second load ends, and whether the policy engine resolves `real_columns` for `icecat_api.`bronze.g3a_orders``: read from the code, not run.
- Whether ClickHouse accepts `SETTINGS` inside a subquery or CTE through the rewrite; the gate passes either way (masked or 422).

## Review


*(planner)*
