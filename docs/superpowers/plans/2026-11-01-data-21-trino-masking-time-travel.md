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

## 6. Review (planner appends below)
