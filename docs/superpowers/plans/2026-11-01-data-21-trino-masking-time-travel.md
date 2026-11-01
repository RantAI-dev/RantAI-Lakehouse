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

## 6. Review (planner appends below)
