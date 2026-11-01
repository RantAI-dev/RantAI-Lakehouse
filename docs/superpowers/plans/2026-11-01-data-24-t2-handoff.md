# DATA-24 T2 — handoff (developer, 2026-11-01)

Plan: `docs/superpowers/plans/2026-11-01-data-24-catalog-search-follow-ups.md` (task T2, decision
D2). Branch `fix/data-24-t2-final`, off `origin/fix/data-24-t2-final` at `9bfcdb15`.

## The commit

`179adda5` — "fix(catalog): read the Bronze column registry with FINAL (DATA-24 T2)".

Three files:

- `rust/crates/lakehouse-api/src/routes/catalog.rs`
- `scripts/seed-clickhouse.sql`
- `.github/workflows/ci.yml`

## What it does

- **`search_column_index` reads with `FINAL`** (`routes/catalog.rs:757-768`). Both registry reads
  (`bronze_meta.dataset_column`, `bronze_meta_sec.dataset_column`) now carry `FINAL`, the way the
  asset-detail query already did (`routes/catalog.rs:2352-2353`, which says why). Without it a
  column a second connector run registered was in the search copy twice, so every search that
  matched its name matched it twice.
- **The per-slug column count reads with `FINAL`** (`routes/catalog.rs:526-536`, inside
  `list_body`). `columnCount` said twice the columns the table has, for the same reason.
- **`scripts/seed-clickhouse.sql` declares both `dataset_column` tables as
  `ReplacingMergeTree ORDER BY (slug, key_asli)`**, matching the one owner of that DDL
  (`dagster/dispar_orchestrate/bronze_catalog.py`'s `_registry_schemas`). A plain `MergeTree` has
  no merged copy for the reads to ask for.
- **`ci.yml` runs the bench's own tests** (`python3 -m unittest discover -s ops/bench -p
  'test_*.py'`). This is the wiring the plan's review assigned to this slice ("Wiring the bench's
  tests into `ci.yml` (the developer's note) goes in with T2"); it is not part of the plan's T2
  task bullets, so it is called out here for the reviewer. On this branch `ops/bench` has no
  `test_*.py` yet — the bench's tests come with T3, which the plan says merges before this slice;
  the discovery step exits 0 with 0 tests until then.

## The test the plan asks for

Two tests in `routes/catalog.rs` (`mod tests::registry_final`), which stand the engine up as a
`wiremock` fake:

- `the_search_column_index_counts_a_re_registered_column_once` — the search copy's column index
  holds the column a second run registered once.
- `the_column_count_of_a_slug_is_not_inflated_by_a_second_run` — the list's `columnCount` for that
  slug is the number of columns the table has.

The fake is the engine for those two reads: it holds the registry as two runs of a connector left
it (`orders` with the column `total` registered twice, `returns` in the `_sec` registry) and
answers the way `ClickHouse` answers — every copy when the read does not ask for the merged one,
one row per `ORDER BY` key (the last write winning) when it does. So a read that dropped `FINAL`
sees the copies and the assertions fail.

**They bite.** With `FINAL` removed from the two queries (mutation checked on a copy of the
file), both fail:

```
the_search_column_index_counts_a_re_registered_column_once ... FAILED
  left: 2  right: 1
  the column a second run registered is one column: [("total", "Order total"), ("buyer", "The buyer"), ("total", "Order total")]
the_column_count_of_a_slug_is_not_inflated_by_a_second_run ... FAILED
  left: Number(3)  right: Number(2)
```

## Checks run in the foreground on the final commit (`179adda5`)

There is no `bin/cargo-locked` in this checkout, so plain cargo with `-j 1`.

| Command | Result |
| --- | --- |
| `cargo fmt --check` | clean, no output |
| `cargo clippy -p lakehouse-api --all-targets -- -D warnings` | clean, no finding (3m 44s) |
| `cargo test -p lakehouse-api catalog` | 550 passed, 0 failed, 0 ignored (90 test binaries: 265 lib unittests, 267 bin unittests, the rest the integration binaries whose names match the filter) |

## Also checked, outside the acceptance block

- **The SQL against the real engine.** The `26.8` `ClickHouse` image the compose pins
  (`clickhouse/clickhouse-server:26.8@sha256:d426d2f6…`) was started as a throwaway container and
  the whole `scripts/seed-clickhouse.sql` was applied to it with `clickhouse-client --multiquery`:
  exit 0, and both `dataset_column` tables are `ReplacingMergeTree` with `sorting_key` `slug,
  key_asli`. The seed was then applied a second time on the same server: exit 0 (the seed stays
  idempotent).
- **The `FINAL` semantics against the real engine.** With `SYSTEM STOP MERGES` on the registry
  (the situation the comment describes — the copies are not merged yet) and the same column
  inserted twice:

  | Read | Answer |
  | --- | --- |
  | rows the engine holds | 3 |
  | `count()` **without** `FINAL` | 3 |
  | `count()` **with** `FINAL` | 1 |
  | the per-slug query the code now sends (`… FINAL GROUP BY slug`) | 1 |
  | the index query the code now sends (`… FINAL` in the subquery) | one row per column, the last write winning |

  The engine keeps the **last** row written for a key, which is what the fake models.
- **`python3 -m unittest discover -s ops/bench -p 'test_*.py'`** — 0 tests, exit 0 (the bench's
  test file is on T3's branch, not on this one).
- **`.github/workflows/ci.yml` parses** (`yaml.safe_load`, 34 jobs). `actionlint` is **not**
  installed on this machine, so the workflow was not linted; the change is one `run:` line inside
  an existing step.

## Not verified, and why

- **The full workspace verification block was not run.** Per AGENTS.md, a commit runs the checks
  for the languages it touched; this slice touches Rust, SQL and one CI workflow line. The
  console (TypeScript), the Dagster code location (Python) and compose were not touched and were
  not run. The one Python-ish thing this commit adds is the `ci.yml` discovery line, which was
  run by hand as quoted above.
- **`actionlint` was not run** on `ci.yml` — not installed here.
- **The change was not exercised against a running stack.** The `ClickHouse` check above was a
  throwaway container with the seed applied by hand, not `docker compose up` of the whole stack
  with the API reading the seeded registry.
- **A stack seeded before this change keeps its old engine.** `CREATE TABLE IF NOT EXISTS` never
  alters a table that exists, so a `dataset_column` table that is already a plain `MergeTree`
  stays one until it is dropped and created again. The seed file says so in a note next to the
  DDL; the change does not drop any table, because the seed is meant to be safe to re-run against
  a live stack.
- **The Rust tests run the fake, not the engine.** The `ClickHouse` container above is the proof
  that the SQL and the `FINAL` semantics are right; the tests in the repository are hermetic (no
  Docker, no network) and prove that the code asks for the merged copy.

## Findings for the planner (not changed here — outside T2's scope)

1. **Two more reads of the same registry do not use `FINAL`**:
   `rust/crates/lakehouse-api/src/routes/ai/data_map.rs:603-604` and
   `rust/crates/lakehouse-api/src/routes/ai/tools/data.rs:480-481`. Both read
   `lake.\`bronze_meta.dataset_column\`` and its `_sec` twin the way the two reads this commit
   fixed did. They are in the assistant's area, which the AI team plans itself, so they were left
   alone.
2. **The seed's `dataset_catalog` and `dataset_sync` are also `MergeTree`** while
   `bronze_catalog.py` declares them `ReplacingMergeTree ORDER BY (slug)`. The plan's D2 named
   only the two `dataset_column` tables, so the other two were left as they are. The same
   "a second run writes it again" argument applies to them, and `list_body`'s catalog read has no
   `FINAL` either — that is a separate decision, not a T2 detail.
3. **`list_body`'s per-slug count takes the first row it finds for a slug.** With `UNION ALL` over
   the two registries, a slug present in both is counted by the first branch only. That is
   pre-existing behaviour and was not changed here; the fake puts `orders` in one registry and
   `returns` in the other, so the tests do not pin that behaviour either way.

## Review (planner, 2026-11-01, at `0697469c`)

- **D2.**
  - Both `dataset_column` reads in `search_column_index`, and the per-slug column count, read
    with `FINAL`, as the asset-detail query does.
  - The seed declares both tables `ReplacingMergeTree ORDER BY (slug, key_asli)`, matching
    `bronze_catalog.py`.
- **The two `wiremock` tests** model the engine's answer with and without `FINAL`, and both fail
  with it removed. The developer also checked the seed and the `FINAL` behaviour on a real
  ClickHouse 26.8.
- **The bench's tests** are wired into `ci.yml`, as the review of T1 and T3 asked.
- **The developer's counts:** fmt and clippy are clean, and `cargo test -p lakehouse-api
  catalog` gives 550 pass.
- **The developer's three findings** become backlog item `DATA-39`, in the planner's docs PR:
  - two more reads of the registry without `FINAL`, in the assistant's code (`ai/data_map.rs`,
    `ai/tools/data.rs`);
  - the seed's other two registry tables are still `MergeTree`;
  - `list_body`'s column count takes the first row.

**Open: no BLOCKER, no SHOULD-FIX.** After this merges, the planner re-runs the bench and adds a
row to `DATA-11-RESULT.md`.
