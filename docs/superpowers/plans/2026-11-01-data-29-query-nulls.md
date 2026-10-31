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

## Review (planner appends below)
