# DATA-29: the query route answers a NULL cell as null, not as an empty string

Backlog `DATA-29` (Task, Data, P3). Reassigned to the planner lane on 2026-11-01. Base: `main` at
`a33e3d81`. Planner: Claude Opus. Developer: another agent (`AGENTS.md`). If the code disagrees, stop
and report it.

## What is wrong (confirmed)

- **The server flattens NULL.** `stringify_cell` (`rust/crates/lakehouse-api/src/routes/query.rs:1530-1538`)
  maps `None` and `Null` to `""`, and every other value to its text. It is used for ClickHouse at
  `:1024` and for Trino at `:1551`. So a NULL and an empty text cannot be told apart.
- **The console already expects null.** `QueryCell = string | number | boolean | null`
  (`src/services/contracts/queries.ts:22`), and the grid renders null as a muted "NULL"
  (`query-results-section.tsx:52`). The TSV copy, filters and statistics handle null too
  (`query-grid-tools.ts:34,71,86`).

## Decisions

- **D1.** The function becomes `cell_value(...) -> serde_json::Value`.
  - `None` and `Null` become `Value::Null`.
  - Every other value stays the same string it is today, so non-null cells do not change.
  - Rename it, because it no longer stringifies.
- **D2.** The byte cap counts a null as 4 bytes, the length of `null`. Check the existing
  `cell_byte_cost` (`:1592`) and its test (`:3364`). If they already treat null as 5 (with the
  comma), keep what they do and say so.
- **D3. The assistant.** Its tools (`routes/ai/tools/data.rs:118-167`, `routes/ai/tools/queries.rs`)
  get null for NULL. `routes/ai/citations.rs:216-243` already skips non-string cells. Check every
  place in `routes/ai` that reads a cell as `as_str()` and make it handle null. The tool schemas
  do not change.
- **D4. Downloads.** CSV and XLSX are built elsewhere and keep their own NULL handling. Do not
  touch them.
- **D5. The upload gate.** `ops/g9/upload_test.py:1025-1032` has a comment about the old
  behaviour. Update the comment, and the check at `:514` if it would now print `None`.

## Task (one commit)

- **T1.** Make D1 to D5. Cite `DATA-29` at the sites.
- Tests:
  - a ClickHouse run and a Trino run with a NULL and an empty string in the same row answer `null`
    and `""`, extending the run tests in `query.rs` and `tests/query_history.rs`;
  - the byte cap test;
  - one assistant tool test with a NULL cell.
- *Accept:* `cargo fmt --check`; `cargo clippy -p lakehouse-api --all-targets -- -D warnings`;
  `cargo test -p lakehouse-api query`; `cargo test -p lakehouse-api ai::`;
  `bun test src/features/queries`.

## Verification before handoff

The scoped commands above, then `git status`. CI runs the full workspace (owner's rule,
2026-10-30).

## Handoff (developer appends below)

### Handoff (developer, 2026-10-31)

T1 is done as decided (D1–D5). Commit: `7c0b06e6` — "feat(query): a NULL cell is null, not an
empty string (DATA-29)", on `fix/data-29-query-nulls`.

**What changed**

- **D1.** `stringify_cell` is now `cell_value(...) -> serde_json::Value`
  (`rust/crates/lakehouse-api/src/routes/query.rs:1539`). `None` and `Some(Value::Null)` answer
  `Value::Null`; every other value keeps the text it had before, so a non-null cell does not
  change. Both engine branches use it (`query.rs:1024` for ClickHouse, `query.rs:1560` for
  Trino).
- **D2.** The existing rule already charged a null cell five bytes — `cell_byte_cost`
  (`query.rs:1607`) counts any non-string value's compact JSON text (`null` is four) plus the
  one separator byte — and its test already said so (`query.rs:3398`). Kept as it was, and the
  doc comment now says why: a NULL cell is sent as `null` and no longer as the empty string, so
  the cap counts the bytes the answer really carries. What the change really moves is the cell
  that reaches the rule: a NULL cell was the empty string (one byte) and is now `null` (five).
- **D3.** `run_sql` (`routes/ai/tools/data.rs:42`) and `run_saved_query`
  (`routes/ai/tools/queries.rs:118`) hand the model's answer on as the route built it, so a
  NULL reaches the model as `null`; both carry a doc note saying so. `routes/ai/citations.rs`
  reads cells for the evidence: `derived_values` (`citations.rs:222`) skips a `null` cell
  exactly as it skipped the empty string before (no number, no text), so the evidence a
  conversation carries does not change; `numeric_leaves` skips it as well. Every other
  `as_str()` in `routes/ai` reads a tool argument or a semantic name, not a result cell — the
  tool schemas do not change.
- **D4.** The downloads are written by the engine (`download_engine_file` /
  `write_download_rows`), never through the run route's cells: not touched.
- **D5.** `ops/g9/upload_test.py`: the comment that described the old behaviour
  (`upload_test.py:1030`) now says that the route answers a NULL cell as `null` and that the
  gate still asks ClickHouse `isNull` beside each value, so the gate's own answer does not
  depend on the route's shape — nothing is loosened. `step_rows_and_types`
  (`upload_test.py:511`) reads a null cell as the empty value in its text comparison instead of
  printing Python's `None` as if it were the cell's text.

**Tests added**

- `routes::query::tests::a_null_cell_is_null_and_every_other_cell_keeps_its_text` and
  `...::a_null_cell_and_an_empty_cell_are_told_apart` (`query.rs:3373`).
- The byte cap: `...::a_null_cell_costs_more_than_an_empty_one` (`query.rs:3412`) and the null
  row in `...::a_row_costs_the_sum_of_its_cells`; the existing
  `...::a_cell_costs_its_text_plus_one_byte_for_the_separator` keeps its five bytes and says
  why.
- A Trino run: `routes::query::tests::trino_engine::a_null_cell_is_null_and_an_empty_cell_is_an_empty_string`
  (`query.rs:3959`) — the engine answers one column as `null` and the other as `""`, and the
  response carries `null` and `""`.
- A ClickHouse run: `recording::a_null_cell_is_null_and_an_empty_cell_is_an_empty_string`
  (`rust/crates/lakehouse-api/tests/query_history.rs:816`) — the same, through the router.
- An assistant tool: `routes::ai::tools::data::run_sql_delegation::run_sql_hands_the_model_a_null_cell_as_null`
  (`routes/ai/tools/data.rs:1170`) — the model's answer carries `null` and `""`.

**Commands run** (every cargo command through `/home/shiro/rantai/bin/cargo-locked`, from the
worktree's `rust/`; the plan named `/home/shiro/rantai/RantAI-Lakehouse/bin/cargo-locked`, which
does not exist on this machine — the script is at `/home/shiro/rantai/bin/cargo-locked` and was
read before use)

- `cargo fmt --check` — exit 0, no diff (one formatting fix in my own new test before the
  clean run).
- `cargo clippy -p lakehouse-api --all-targets -- -D warnings` — exit 0, no warnings
  ("Finished `dev` profile in 4m 38s").
- `cargo test -p lakehouse-api query` — exit 0: 90 test binaries, 517 passed, 0 failed,
  6 ignored (lib 244 passed / 3 ignored, bin 246 passed / 3 ignored).
- `cargo test -p lakehouse-api ai::` — exit 0: 90 test binaries, 880 passed, 0 failed,
  2 ignored (lib 440 passed / 1 ignored, bin 440 passed / 1 ignored).
- `cargo test -p lakehouse-api --test query_history a_null_cell_is_null_and_an_empty_cell_is_an_empty_string`
  — exit 0: 1 passed, 0 failed, 15 filtered out. **Run on purpose:** the plan's filter
  (`cargo test -p lakehouse-api query`) matches test names, and this test's name does not
  contain "query", so the filter alone never runs the new ClickHouse test.
- `bun install --frozen-lockfile` — 739 packages installed (`node_modules` was missing).
- `bun test src/features/queries` — 138 pass, 0 fail, 397 expect() calls, 16 files.
- `python3 -m py_compile ops/g9/upload_test.py` — ok (a syntax check; the gate itself needs the
  whole stack).
- `git status` — only the six source files of this change, nothing generated.

**Not verified**

- The full workspace run (`cargo clippy --workspace`, `cargo test --workspace`): CI runs it
  (owner's rule, 2026-10-30). Not run here.
- `cargo test -p lakehouse-api` without a filter (every test of the crate): not run; the
  plan's two filters plus the one new integration test were.
- The g9 upload gate end to end: not run (it needs Postgres, ClickHouse, MinIO, Dagster and a
  running API). The gate's Python file was syntax-checked only.
- `bun test` for the whole console and the `ops/lint/*.py` checks: not run; the plan scopes
  the console to `src/features/queries` and the lint scripts cover the Dagster package, which
  this change does not touch.
- The console needed no change: `QueryCell` already allows `null`
  (`src/services/contracts/queries.ts:22`), the grid renders it as a muted "NULL"
  (`query-results-section.tsx:53`) and the grid tools handle it
  (`query-grid-tools.ts:34`).

## Review (planner appends below)
