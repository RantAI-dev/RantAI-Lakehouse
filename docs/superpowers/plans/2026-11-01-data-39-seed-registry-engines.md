# DATA-39 (data part): the seed declares the registry tables the way their owner does

Backlog `DATA-39` (Task, Data, P3), found building `DATA-24` T2 (#308). Base: `main` at
`2c110ebc`. Planner: Claude Opus. Developer: another agent (`AGENTS.md`).

## 0. What is wrong

- `scripts/seed-clickhouse.sql` declares `lake.`bronze_meta.dataset_catalog`` (`:143`, engine at
  `:152`), `bronze_meta.dataset_sync` (`:155`, `:167`) and their `bronze_meta_sec` twins
  (`:191`/`:200`, `:203`/`:215`) as plain `MergeTree`.
- Their owner, `dagster/dispar_orchestrate/bronze_catalog.py` (`:305`), creates them as
  `ReplacingMergeTree ORDER BY slug`. A dev or CI stack seeded from the script therefore keeps
  duplicate rows that a real stack folds away.
- #308 already fixed `dataset_column`.

## 1. Decisions

- **D1.** Each of the four tables uses exactly the engine and `ORDER BY` its owner declares. Read
  `bronze_catalog.py` for the exact text, including any version column passed to
  `ReplacingMergeTree(...)`.
- **D2.** Do not change any Rust and do not touch the assistant's code: the `ai/data_map.rs` and
  `ai/tools/data.rs` reads without `FINAL` belong to the AI team.
- **D3. Investigate only, no code.** The third point of `DATA-39`: the catalog's per-table column
  count "takes the first row it sees". Read `routes/catalog.rs` around `list_body`, now that
  #308 added `FINAL`, and write in the handoff whether a wrong count is still possible, with the
  `file:line`. Change nothing.

## 2. Task

- **T1.** D1, in one commit citing `DATA-39`.
- *Accept:*
  - apply the whole seed script to a throwaway `clickhouse/clickhouse-server:26.8` container
    twice; it must exit 0 both times;
  - `SHOW CREATE TABLE` on each of the four tables matches the owner;
  - remove the container.

## 3. Handoff (developer appends below)
