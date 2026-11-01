# DATA-21: masking and past versions work on Trino

Backlog `DATA-21` (Task, Data, P2), split from `DATA-16` by the product owner (2026-10-09).
Reassigned to the planner lane on 2026-11-01. Base: `main` at `9131dced`. Every `file:line` below
was read there. Planner: Claude Opus. Developer: another agent (`AGENTS.md`). If the code
disagrees, stop and report it.

## 0. What is wrong (confirmed)

- **The engine is chosen by dialect only** (`rust/crates/lakehouse-api/src/routes/query.rs:130`,
  `:163-196`). Trino parses with `GenericDialect`, and
  `policy_engine::rewrite_sql_for_roles` is the same for both engines.
- **A masked table cannot be read on Trino.** `build_masked_filtered_select`
  (`rust/crates/lakehouse-api/src/sql_rewrite.rs:1497-1531`) always writes the ClickHouse form
  ``replaceRegexpOne(toString(`c`), '(?s)^.*$', '***') AS `c` `` (`:1513`), with backtick
  identifiers (`:1516`, `derived_alias_ident` `:1348`). Generic accepts backticks, so the rewrite
  succeeds, and then Trino rejects both the backticks and the functions.
- **Row-filter functions are ClickHouse only.** The allowlist (`sql_rewrite.rs:~997`) includes
  `toString` and `toDate`.
- **A past version on Trino is always refused.** sqlparser 0.63 parses `VERSION AS OF` only
  without `FOR`, and only when `supports_table_versioning()` is on; Generic leaves it off. Trino's
  `FOR VERSION AS OF n` therefore fails to parse and is mapped to `PAST_VERSION_TRINO_MESSAGE`
  (`query.rs:203-212`, `:837-846`).
- **The version would also be lost.** `substitute_table_factor` (`sql_rewrite.rs:1371-1379`)
  drops a table factor's `version`. On ClickHouse the version is a query-wide `SETTINGS`, which is
  fine; on Trino it is per table.
- **The console offers past versions on ClickHouse only** (`src/lib/snapshot-picker.ts:27`,
  `src/features/queries/time-travel-controls.tsx:35`).

## 1. Decisions (made; do not change)

- **D1. One rewrite, engine-aware.** Add an `Engine` argument (`ClickHouse` or `Trino`) to the
  rewrite path, from `query.rs`'s engine choice down to `build_masked_filtered_select`.
  - The **ClickHouse output stays byte for byte the same**: many tests match its string.
  - On **Trino**:
    - identifiers are double-quoted;
    - a masked column becomes `CASE WHEN "c" IS NULL THEN NULL ELSE '***' END AS "c"`. That is
      the same meaning as ClickHouse's form, where `toString(NULL)` stays NULL;
    - in row filters, `toString(x)` becomes `CAST(x AS varchar)` and `toDate(x)` becomes
      `CAST(x AS date)`;
    - `lower` and `upper` are unchanged;
    - any other ClickHouse-only function in a policy is **refused** on Trino with a fixed message
      naming the function. It is never passed through.
- **D2. A Trino dialect that reads table versions.** Add a `TrinoDialect` newtype over Generic,
  with `supports_table_versioning() == true`.
  - Before parsing, a token pass (reuse the tokenizer from `uses_past_version_clause`, `:846`)
    turns `FOR VERSION AS OF <integer>` into `VERSION AS OF <integer>`.
  - After printing, the reverse pass restores Trino's form.
  - Only an integer snapshot id is accepted. `FOR TIMESTAMP AS OF` stays refused on Trino with
    the existing message, because the ClickHouse side has no timestamp form either.
- **D3.** `substitute_table_factor` keeps the table's `version` on the derived `FROM` for Trino.
  The ClickHouse path is unchanged.
- **D4.** The refusal `PAST_VERSION_TRINO_MESSAGE` is removed for integer versions. The test
  `a_past_version_clause_on_trino_records_blocked` (`tests/query_history.rs:630`) is changed into
  a test that a past version runs on Trino and records `ok`. Say why in the commit (rule 2: the
  test changes because the behaviour it guarded is now supported).
- **D5. The console.** `pinSnapshot` and `supported` (`snapshot-picker.ts`) gain the Trino form,
  `FROM t FOR VERSION AS OF <id>`, and the picker is offered on Trino.
  - Update `docs/core/features/time-travel.md`'s decision 1 and its limits.
- **D6. The policy still holds on a past version** (`DATA-16`'s rule). A masked column read at a
  past version on Trino is masked, and a row filter applies.

## 2. Slices

| Slice | Branch | What |
| --- | --- | --- |
| **S1** | `fix/data-21-trino-masking-time-travel` | D1 to D5, with unit and route tests (Rust and console) |
| **S2** | same branch, after S1's review | Gates. The CI jobs `g8-governance` (`ci.yml:2781`) and `g8-time-travel` (`:2926`) gain a Trino run: start the stack with the `trino` profile; `ops/g8/g8_governance_test.py` and `g8_time_travel_test.py` take an engine argument and run every masking and past-version check on both engines (`g8_time_travel_test.py:311` hardcodes ClickHouse). |

## 3. S1 tasks (one commit each)

- **S1-T1.** The engine argument and the Trino projection and row filter (D1).
  - Unit tests in `sql_rewrite.rs`: Trino twins of the masking tests at `:2153`, `:2248-2285`,
    `:4313-4390`; the ClickHouse strings are unchanged; an unknown ClickHouse function in a Trino
    policy is refused.
- **S1-T2.** `TrinoDialect`, the token passes, and the version kept on the derived `FROM` (D2,
  D3).
  - Unit tests: round-trip `FOR VERSION AS OF 123`; refuse `FOR TIMESTAMP AS OF`; a masked table
    at a past version keeps both the version and the mask (D6).
- **S1-T3.** The route (D4).
  - Change `query_history.rs:630` as D4 says.
  - Add: a masked table on Trino answers with `***` (wiremock Trino, through
    `test_state_with_trino`); a past version on Trino runs.
- **S1-T4.** The console (D5): tests in `snapshot-picker.test.ts` and
  `time-travel-controls.test.tsx`. Update the feature page.
- *Accept, per task:*
  - `cargo fmt --check`;
  - `cargo clippy -p lakehouse-api --all-targets -- -D warnings`;
  - `cargo test -p lakehouse-api sql_rewrite`;
  - `cargo test -p lakehouse-api query`;
  - `cargo test -p lakehouse-api --test query_history`;
  - for the console: `bun run typecheck`, `bun run lint`, and `bun test src/lib src/features/queries`.

## 4. Out of scope

- `FOR TIMESTAMP AS OF` on either engine.
- Masking functions beyond the fixed `***`.
- Other engines.

## 5. Handoff (developer appends below)

### Handoff S1 (developer, 2026-11-01)

**Commits** (one per task, in plan order, each pushed on its own):

| Task | Commit | What |
| --- | --- | --- |
| S1-T1 | `dc9390dd` | `feat(policy): the rewrite is engine-aware, so a masked table reads on Trino (DATA-21 S1-T1)` |
| S1-T2 | `24c5b1ec` | `feat(policy): a Trino dialect that reads a table version, and the past version it keeps (DATA-21 S1-T2)` |
| S1-T3 | `ebfae103` | `feat(query): a past version and a mask work on Trino (DATA-21 S1-T3)` |
| S1-T4 | `abc93146` | `feat(web): the version picker offers Trino's own past version (DATA-21 S1-T4)` |

**Commands run** — all in the foreground, on the final commit `abc93146`, every
cargo command through `bin/cargo-locked`:

| Command | Result |
| --- | --- |
| `cargo fmt --check` | clean (exit 0) |
| `cargo clippy -p lakehouse-api --all-targets -- -D warnings` | clean (exit 0) |
| `cargo test -p lakehouse-api sql_rewrite` | 144 passed, 0 failed (the lib target; the bin target runs the same filter, so 288 across the two) |
| `cargo test -p lakehouse-api query` | 513 passed, 0 failed, 6 ignored (across the crate's test binaries; the lib and the bin targets each run the filter, so the counts are counted twice) |
| `cargo test -p lakehouse-api --test query_history` | 16 passed, 0 failed |
| `bun run typecheck` | clean (exit 0) |
| `bun run lint` | **fails — pre-existing, not this work.** 649 errors, every one of them in `dagster/.venv/lib/python3.12/site-packages/**` (the local Python virtualenv's bundled JavaScript, which `eslint` scans; the config ignores only `.next`, `out`, `build`, `next-env.d.ts`). 0 errors in the repo's own sources: `bun --bun eslint src` → 0 errors, 10 warnings (all pre-existing). The same failure happens with this branch's changes stashed. |
| `bun test src/lib src/features/queries` | 1244 pass, 1 skip, 0 fail (1245 tests across 133 files) |

**Not verified** — and why:

- The full workspace block (`cargo clippy --workspace --all-targets
  --all-features -- -D warnings`, `cargo test --workspace`): **not run.** The
  machine rules for this session say to run only the scoped commands the plan
  lists, and no workspace run. The reviewer re-runs it before the merge.
- `python3 ops/lint/check_intra_package_imports.py`,
  `python3 ops/lint/check_bare_iceberg_count.py` and
  `(cd dagster && python -m pytest dispar_orchestrate -q)`: **not run.** No
  Python or Dagster file changed, and the same machine rule limits this
  session to the plan's scoped commands.
- `docker compose --profile '*' config --quiet`: **not run.** No compose file
  changed.
- The `g8-governance` and `g8-time-travel` gates with a Trino run: slice S2.

**Where the code and the plan differ, and what was done about it** (all in
this slice's favour of the plan's intent; each is a reviewer's call):

1. **D1's row-filter casts print in upper case.** `sqlparser` prints a data
   type in upper case, so the output is `CAST(x AS VARCHAR)` and
   `CAST(x AS DATE)`, not the plan's lower case. Trino reads both spellings
   the same way; the AST nodes are `DataType::Varchar` and `DataType::Date`,
   not a hand-written string.
2. **D2's "the existing message" kept its text.** `PAST_VERSION_TRINO_MESSAGE`
   still reads "Querying a past version of a table is available on the
   ClickHouse engine only.", and it is now used only for a past version by
   TIMESTAMP. Its text names the ClickHouse engine, and the ClickHouse side
   has no timestamp form either (the plan's own reason) — a reviewer may want
   it reworded.
3. **D4's "records `ok`" is the store's `completed`.** The history store's
   vocabulary is `completed | failed | cancelled | blocked`; there is no `ok`
   status, so the changed test asserts `completed`.
4. **The plan's task split needed the route's half of S1-T2 in S1-T3.** S1-T2
   is the module (the dialect, the token passes, the version on the derived
   `FROM`), S1-T3 is the route that uses it. To keep every commit green, the
   route's use of the new pieces is in S1-T3, and the three new public items
   carry a `dead_code` allow in S1-T2 whose reason says the route wires them
   in S1-T3 (the same pattern `sql_rewrite::REFUSED_UNPARSEABLE` uses).
5. **A table with both a version and an alias becomes a derived table.**
   `sqlparser` prints a version AFTER an alias (`FROM t AS x VERSION AS OF
   5`) and Trino reads it before, so such a table is rewritten as
   `(SELECT * FROM t VERSION AS OF 5) AS x`. Without this the statement
   Trino receives would not parse. A governed table needs no such care: its
   own derived table carries the version and the alias separately.
6. **The console's Trino pin matches the table reference textually**, so a
   statement that does not read the table the user picked is not pinned and
   the picker writes nothing. A ClickHouse-style quoted name
   (`icecat_api.`bronze.orders``) is not a table the Trino engine reads, so
   it is not pinned either.
7. **The feature page's acceptance rows `DATA-16-AC5` and `DATA-16-AC6` are
   the owner's rows and were not rewritten**; a note under the table says
   they describe the state before `DATA-21` and that the Trino rows are
   asked again in `DATA-21`'s own acceptance.

### Handoff S2 (developer, 2026-11-01)

**Commits** (one per concern, in plan order, each pushed on its own):

| Commit | What |
| --- | --- |
| `504d562d` | `feat(gates): the G8 gates prove masking and past versions on Trino (DATA-21 S2)` — the two gate scripts |
| `95e32ea7` | `ci(g8): the G8 gates run on Trino as well as ClickHouse (DATA-21 S2)` — the two CI jobs |
| `c9be3d10` | `ci(g8): the Trino leg of the G8 governance gate reads Lakekeeper in-network (DATA-21 S2)` — the gap the local run found |

**Commands run** — all in the foreground, on the final commit `c9be3d10`:

| Command | Result |
| --- | --- |
| `python3 -m py_compile ops/g8/g8_governance_test.py ops/g8/g8_time_travel_test.py` | clean (exit 0) |
| `python3 ops/lint/check_bare_iceberg_count.py` | "R11 guard: no bare count() against a Bronze Iceberg table found." (exit 0) |
| `python3 ops/lint/check_intra_package_imports.py` | "OK: all intra-package dispar_orchestrate imports resolve" (exit 0) |
| `docker compose --profile '*' config --quiet` | clean (exit 0) |
| `actionlint .github/workflows/ci.yml` | clean (exit 0) |
| `python3 -c "import yaml; yaml.safe_load(...)"` over `ci.yml` | parses; both jobs carry the expected matrix |

`actionlint` was **not installed** on this machine, so 1.7.7 was fetched from its
release page into a scratch directory outside the repository. `actionlint` is not
part of this repo's own verification block, so this is a bonus check, not one of
the block's commands.

**Local gate runs** — the real proof, on this machine, on the compose project
`data21g` (the `trino` profile, plus `dagster` for the time-travel stack). The
`lakehouse-api` image was rebuilt from this branch's source first
(`docker compose build lakehouse-api`, `rantai-lakehouse-api:local`, 2026-11-01
12:11; the image on the machine was built before slice S1's commits, so a run
against it would have proved nothing about this branch).

| Gate | Engine | Result |
| --- | --- | --- |
| `g8_governance_test.py` | `trino` | **PASS** — the Gold export published `serving.g8_gate_subjects` as `gold.g8_gate_subjects`, the policy `g8-mask-email-trino` was authored, the analyst's `email` came back `***` with `region` in clear text, the admin's came back in clear text, and both bypass shapes (`SELECT *`, `SELECT *` in a CTE) came back masked |
| `g8_governance_test.py` | `clickhouse` | **PASS** — the leg the gate always was |
| `g8_time_travel_test.py` | `trino` | **PASS** — two loads, older version 2000 rows / current 4010 rows, the analyst 1000 of 2000 at the older version, `customer` masked to `***` with the other columns clear and equal to the admin's, and the pin inside a subquery and inside a CTE both masked |
| `g8_time_travel_test.py` | `clickhouse` | **PASS** — the leg the gate always was |

The two `clickhouse` runs are a bonus: the brief asked for one real run of each
changed gate on Trino, and they are here to show the `clickhouse` leg is not
broken by the change.

The stack was torn down afterwards: `docker compose -p data21g down -v`. No
container, volume or testcontainers image is left behind.

**Not verified** — and why:

- The CI jobs themselves have **not run**. No pull request was opened (the
  developer never opens one), so the matrix legs have not been exercised by
  GitHub Actions. The reviewer runs them.
- The full workspace block (`cargo clippy --workspace --all-targets
  --all-features -- -D warnings`, `cargo test --workspace`): **not run.** No
  Rust file changed in this slice, and the machine rules for this session say
  to run only the checks for what the commit touched.
- `bun run typecheck`, `bun run lint`, `bun run test`: **not run.** No
  TypeScript file changed.
- `python3 -m pytest dispar_orchestrate -q`: **not run.** No Dagster file
  changed.
- The gate scripts have **no unit tests** — they are acceptance gates, run in
  CI — so `py_compile` and the two Python lints are all the static checking
  there is.

**Decisions the plan left open, and what was done** (each is a reviewer's call):

1. **The plan says the gates "take an engine argument and run every masking
   and past-version check on both engines".** Read as: the script takes the
   engine, and CI runs it once per engine. `--engine <name>` beats the
   `G8_ENGINE` environment variable, which beats the `clickhouse` default, so a
   run with no argument at all is byte-for-byte the ClickHouse leg the gates
   always were — the ClickHouse leg's SQL is unchanged, down to `count()` and
   `SETTINGS iceberg_snapshot_id = <id>`.
2. **The CI shape is a matrix** over `engine: [clickhouse, trino]`, not a
   second step in one job. The plan allowed either ("matrix or second step"),
   but a matrix gives each leg its own runner, its own compose project and its
   own fresh volumes — so neither leg's stack, fixtures, loads or snapshots
   touch the other's — and it leaves the `clickhouse` leg's stack exactly as it
   was. The plan's own wording ("start the stack with the `trino` profile") is
   singular, so this is a deliberate reading against that sentence; a second
   step in one job would also have worked, at the price of the time-travel
   gate loading the Bronze table twice in one stack.
3. **The governance gate's Trino leg reads an Iceberg table, and the gate
   publishes one itself.** `Trino` has no ClickHouse connector in this stack
   and its only catalog is Lakekeeper's Iceberg REST catalog, so the gate's own
   `MergeTree` table is invisible to it. The Trino leg publishes the gate's
   own seeded rows through the Gold export
   (`POST /api/gold/export/g8_gate_subjects`, the only write path to that
   catalog), which carries the mart's own `email` and `region` columns across,
   and then governs and reads `iceberg.gold.g8_gate_subjects`. The masked
   column and its unmasked sibling are the same two columns the ClickHouse
   leg proves. The two legs govern two different tables, so they carry two
   different policy names (`g8-mask-email`, `g8-mask-email-trino`) — a
   policy's name is unique.
4. **The time-travel gate needed no policy change at all.** Both engines read
   the same Bronze table (`iceberg.bronze.<table>` through Lakekeeper's
   catalog on Trino, `icecat_api.\`bronze.<table>\`` through the
   `DataLakeCatalog` database on ClickHouse), and the policy engine matches a
   rewritten statement's table by its canonical name — `bronze.<table>`, the
   catalog dropped — so ONE policy governs the table on both engines.
5. **The Trino SQL differs from the ClickHouse SQL in four places**, each
   measured against `trinodb/trino:483` itself rather than guessed: the
   version pin is `FOR VERSION AS OF <id>` on the TABLE, right after the table
   reference (`FROM t WHERE ... FOR VERSION AS OF 1` does not parse); the count
   is `count(*)`; the "every row" predicate is `WHERE true` (`WHERE 1` is not a
   boolean in Trino); and the `ORDER BY` may only name a column the outer
   select list carries, so the Trino shapes select `id` beside the masked
   column.
6. **The `g8-governance` job's `.env` now sets `LAKEKEEPER_BASE_URI` for the
   `trino` leg only.** The local run found it: the Gold export failed with
   "The catalog request failed" because the API's Iceberg catalog client
   dialed `http://localhost:8181/catalog` from inside the compose network.
   `lakekeeper` advertises its own base URI and the REST catalog client
   follows the advertised one, so the advertised address must be the in-network
   name — the same value, for the same reason, as the `gold-export` job's
   `.env` has always set. The `clickhouse` leg does not run Lakekeeper at all,
   so its environment is untouched.
7. **Both job timeouts are raised** (15 -> 25 and 25 -> 35 minutes): the
   `trino` leg's stack carries the Trino coordinator and the Lakekeeper chain
   behind it, which take a few minutes to become healthy before the gate can
   run. A timeout is a ceiling, not a behaviour.
8. **The `ci-required` gate needs no change**: `needs.g8-governance.result` is
   the matrix job's aggregate result.

## 6. Review (planner appends below)

### Review S1 round 1 (planner, 2026-11-01, at `2a374c09`)

- **D1.**
  - The engine argument runs from `query.rs` down to `build_masked_filtered_select`.
  - On Trino a masked column is `CASE WHEN "c" IS NULL THEN NULL ELSE '***' END AS "c"`.
  - Identifiers pass through `lakehouse_core::ident::Ident::new` before they are quoted, so no
    column name can carry a quote into the SQL.
  - In row filters, `toString` and `toDate` become `CAST`, and any other function is refused
    with its name.
  - The ClickHouse output is unchanged, and its existing string tests still pass.
- **D2 and D3.**
  - `TrinoDialect` reads table versions.
  - `FOR VERSION AS OF <integer>` is normalised before the policy gate and restored after the
    rewrite.
  - The version stays on the derived `FROM`. A table that has both a version and an alias becomes
    a derived table, because sqlparser prints the version after the alias. That is a correct
    deviation, and the handoff says why.
- **D4.** The refusal now covers only `FOR TIMESTAMP AS OF`. The history test now records a
  Trino past version as `completed`.
- **D5.** The console pins the Trino form, and the picker is offered on Trino. The feature page is
  updated.
- **D6.** A masked table at a past version keeps both the mask and the version, and the unit
  tests cover it.
- **The seven deviations** are listed in the handoff, and all are accepted.
- **The developer's counts:**
  - `sql_rewrite`: 144;
  - `query`: 513;
  - `query_history`: 16;
  - console: 1244.
- **`bun run lint` failed on bluefin only because of a local `dagster/.venv`** (bundled JS
  files). There are 0 errors in `src/`. That is not a finding, but remove that venv when the
  checkout is next free.

**Open: no BLOCKER, no SHOULD-FIX for S1.** S2 (Trino in the G8 gates) follows on this branch,
after this PR merges.

### Review S2 round 1 (planner, 2026-11-01, at `b5644d7d`)

- **The gates.** `g8-governance` and `g8-time-travel` are now matrices over
  `engine: [clickhouse, trino]`, each leg on its own runner with fresh volumes.
  - The ClickHouse leg's stack and SQL are unchanged.
  - `ci-required` still reads each job's single `result`, which a matrix job reports as the
    aggregate of its legs.
- **The developer's decisions, all accepted:**
  - a matrix rather than a second step;
  - the governance Trino leg governs its own Gold Iceberg table, published by the Gold export,
    because Trino has no ClickHouse connector;
  - `LAKEKEEPER_BASE_URI` is set on the Trino leg, the value the `gold-export` job already uses.
    The local run found that gap before CI would have.
- **The developer's local proof:** both gates pass on both engines, with the Trino legs checking
  the mask, the bypass shapes, a pinned past version, and masking inside a subquery and a CTE.
- **The open question:** no matrix leg has run on GitHub yet. This PR's CI is that proof.

**Open: no BLOCKER, no SHOULD-FIX.** The PR opens once #309 merges and `main` is merged in.

### Review S2 round 2 (planner, 2026-11-01): the PR's CI

- **The first CI run passed all four G8 legs.** That run (on base `2ca345e1`) had
  `Rust · G8 governance masking (trino)` and `Rust · G8 time travel keeps the policy (trino)`
  both green.
- **After updating the branch from `main`, the Trino governance leg failed** (run
  41285813391):
  - the gate printed `[g8] FAILED: Analyst query failed: 422 {"error":"Trino server is still
    initializing …"}`;
  - Trino's own log: `FAILED (SERVER_STARTING_UP) :: elapsed 0ms`.
  - The gate sent its first analyst query before the coordinator had finished starting. The first
    run was lucky. A gate that passes by timing is a flaky gate.

**BLOCKER 1 (S2 round 2): the Trino legs do not wait for Trino to be ready.**
- **Fix:** before its first query on the Trino engine, each Trino leg waits until Trino can answer:
  - `GET /v1/info` reports `"starting": false`, **and**
  - a `SELECT 1` through Trino succeeds,

  with a bounded wait (for example 5 minutes, polling every 2 s) and a clear failure message
  when the time runs out.
- Put it in **one** helper both gate scripts use, or in the workflow step before the gate runs.
  Do not copy it twice (rule 4).
- Do the same for `g8_time_travel_test.py`'s Trino leg: it passed by the same luck.
- Cite `DATA-21 S2 review BLOCKER 1`.
- Prove it locally: run each Trino gate twice on a clean `-p data21g` project.

**Open: BLOCKER 1 (S2 round 2).**

### Fix for review round 2 (developer, 2026-11-01)

**BLOCKER 1 is fixed:** both Trino legs now wait for the coordinator
before their first query on it.

**Commit** — ONE commit on `fix/data-21-s2-trino-gates`:
`fix(gates): the Trino legs wait for Trino to be ready (DATA-21 S2 review BLOCKER 1)`.
It carries this section, so its SHA is the branch's HEAD (`git log -1
--oneline`); the code it carries is listed below, file by file.

**What the fix is.** One new helper, `ops/g8/trino_ready.py`, used by
BOTH gates (`AGENTS.md` rule 4: a duplicated guard is a finding). It
waits until both of the review's conditions hold:

- `GET /v1/info` reports `"starting": false` — the coordinator's own
  readiness flag. The compose `trino` service's healthcheck proves only
  that the endpoint ANSWERS, which it does while the server is still
  starting, so that healthcheck is not a readiness signal and could not
  be the gate's;
- a `SELECT 1` through `Trino` succeeds — posted to `/v1/statement`
  with an UNPRIVILEGED `X-Trino-User` (`g8-readiness`, never
  `trino-maintenance`, the one name the access-control rules grant write
  access to), following the coordinator's own `nextUri` pages.

The wait is **bounded**: five minutes, polling every 2 s, printing a line
every 30 s so a CI log does not look hung. When the budget runs out the
helper raises `TrinoNotReady` and the gate fails with
`Trino never became ready: <which condition was still untrue>`.

The `SELECT 1` goes to `Trino` **directly**, not through
`POST /api/query/run`: the wait is about the ENGINE, and the console's
own path to it (the policy rewrite, the tenant check, the audit trail)
is proved by the gates' own assertions instead. A fault in one of those
would make the wait report the wrong problem.

Each leg waits **before its first query on the Trino engine** — after the
engine-independent work (seeding, the Gold export, the policy, the two
Dagster loads), so that work runs while the coordinator boots. The
ClickHouse legs never call the wait.

**Files changed**

| File | What |
| --- | --- |
| `ops/g8/trino_ready.py` | NEW — the one wait both gates use |
| `ops/g8/g8_governance_test.py` | imports it; waits before the analyst's first query on the Trino leg |
| `ops/g8/g8_time_travel_test.py` | imports it; waits before the first pinned read on the Trino leg; its module docstring's "imports nothing from that file" claim is amended to say exactly what IS shared (the wait, which carries no assertion) |
| `docker-compose.yml` | both G8 runner services pass `G8_TRINO_URL`; the governance runner also sets `PYTHONDONTWRITEBYTECODE` (it now IMPORTS a sibling module, which is what makes Python write bytecode into the bind-mounted checkout) |
| `.env.example` | documents `G8_TRINO_URL` |

`G8_TRINO_URL` is the gate's OWN name for the coordinator's address, and
deliberately not the deployment's `TRINO_URL`: that one may hold a
host-facing address (the `trino-host-port` opt-in publishes Trino on the
host), while a gate inside the compose network must always dial the
in-network name.

**Commands run** — foreground, on this branch, on the final code:

| Command | Result |
| --- | --- |
| `python3 -m py_compile ops/g8/trino_ready.py ops/g8/g8_governance_test.py ops/g8/g8_time_travel_test.py` | clean (exit 0) |
| `python3 ops/lint/check_intra_package_imports.py` | "OK: all intra-package dispar_orchestrate imports resolve" (exit 0) |
| `python3 ops/lint/check_bare_iceberg_count.py` | "R11 guard: no bare count() against a Bronze Iceberg table found." (exit 0) |
| `docker compose --profile '*' config --quiet` | clean (exit 0) |
| `docker compose -p data21g --profile trino up -d --no-build …` | the real `up` (below), not `config` (AGENTS.md §8.8) |

**Gate runs** — the real proof, on this machine, on the compose project
`data21g` (the `trino` profile, plus `dagster` for the time-travel
stack). **Each Trino gate was run TWICE**, as the review asked.

| Gate | Engine | Run | Result |
| --- | --- | --- | --- |
| `g8_governance_test.py` | `trino` | 1 | **PASS** — the Gold export published `serving.g8_gate_subjects` as `gold.g8_gate_subjects`, the policy `g8-mask-email-trino` was authored, the analyst's `email` came back `***` with `region` in clear text, the admin's came back in clear text, and both bypass shapes (`SELECT *`, `SELECT *` in a CTE) came back masked |
| `g8_governance_test.py` | `trino` | 2 | **PASS** — the same assertions, on the same stack, with the policy already authored |
| `g8_time_travel_test.py` | `trino` | 1 | **PASS** — two loads, older version 2000 rows / current 4010 rows, the analyst 1000 of 2000 at the older version, `customer` masked to `***` with the other columns clear and equal to the admin's, and the pin inside a subquery and inside a CTE both masked |
| `g8_time_travel_test.py` | `trino` | 2 | **PASS** — two more loads, older version 6020 rows / current 8040 rows, the analyst 3000 of 6020, the same masking and the same two nested-pin shapes |

The two `clickhouse` legs (the legs this fix must not touch) were run
once each on the same stack and both **PASS**: the governance gate masked
`email` for the analyst on `serving.g8_gate_subjects`, and the
time-travel gate read 5000 of 10060 rows at the older version with
`customer` masked to `***`.

**The wait itself, measured against the real coordinator.** The CI
failure was a query sent while the coordinator was still starting, so
that is the case the fix has to cover:

- `docker compose -p data21g restart trino`, then polling
  `GET /v1/info` once a second: `starting=true` for ~13 s, then
  `starting=false` at t+13 s.
- The helper run against that restarting coordinator (the same module,
  the same function, the real container): it **blocked for 12.9 s over 6
  attempts** and then reported
  `[g8-trino] Trino is ready at http://172.18.0.9:8080 after 6 attempt(s): SELECT 1 answered`.
  That is the window CI failed in, now waited out instead of raced.
- The bounded failure path: `wait_for_trino(url="http://127.0.0.1:9",
  timeout_s=6)` raised
  `TrinoNotReady: Trino at http://127.0.0.1:9 was still not ready after
  6s (3 attempt(s)): GET /v1/info failed: … Connection refused` — bounded,
  and it says what was untrue.

**Tear-down** — `docker compose -p data21g down -v`: no container, no
volume and no image of that project is left behind (`docker ps -a`,
`docker volume ls` and `docker images` are empty of it). No
testcontainers were created by this run, and none are left.

**Not verified** — and why:

- The CI jobs themselves have **not run**. The developer never opens a
  pull request, so the matrix legs have not been exercised by GitHub
  Actions; the reviewer runs them.
- The full workspace block (`cargo clippy --workspace --all-targets
  --all-features -- -D warnings`, `cargo test --workspace`): **not run.**
  No Rust file changed.
- `bun run typecheck`, `bun run lint`, `bun run test`: **not run.** No
  TypeScript file changed.
- `python3 -m pytest dispar_orchestrate -q`: **not run.** No Dagster file
  changed.
- `actionlint`: **not run.** `ci.yml` was not changed by this fix — the
  wait belongs in the gate, not in the workflow, so that every caller of
  the gate (CI and a local run) is protected by it.
