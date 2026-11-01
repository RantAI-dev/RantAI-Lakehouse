# DATA-24: catalog search follow-ups

Backlog `DATA-24` (Task, Data, P2), from the delivery review of 2026-10-12. Reassigned to the
planner lane on 2026-11-01. Base: `main` at `9131dced`. Planner: Claude Opus. Developers: other agents
(`AGENTS.md`). If the code disagrees, stop and report it.

## 0. The four sub-items, checked

1. **The 500 ms target on 10,000 tables was never measured** (`catalog-search.md:60`,
   acceptance row 14 accepted as an exception).
   - `docs/plans/DATA-11-RESULT.md` does not exist.
   - The harness exists, `ops/bench/catalog_search_bench.py`, with two subcommands:
     - `seed` writes 10,000 datasets of 20 columns. It needs
       `BENCH_THROWAWAY_CATALOG=true` and `BENCH_CLICKHOUSE_URL`.
     - `run` sends 50 searches to each route, after waiting out the 30-second cache, and prints
       JSON.
   - It has never run against a stack.
2. **The Owner and Tags filters vanish on a laptop.**
   - `src/features/catalog/data-explorer-page.tsx:189-190` hides `tags` below 1760 px and
     `owner` below 1500 px.
   - Hidden columns are **removed** from the column list (`:223-227`), so their filters go with
     them (`:175`).
3. **The column rows are read without `FINAL`.**
   - `search_column_index` (`rust/crates/lakehouse-api/src/routes/catalog.rs:749-755`) reads
     `bronze_meta*.dataset_column`, a `ReplacingMergeTree ORDER BY (slug, key_asli)`
     (`dagster/dispar_orchestrate/bronze_catalog.py:136-146`) that each connector run writes
     again.
   - The per-slug column count (`catalog.rs:528-529`) has the same problem, so `col_count` can
     be inflated.
   - `scripts/seed-clickhouse.sql:170,207` declares those tables as plain `MergeTree`, which
     disagrees with Dagster.
4. **Acceptance rows 10, 11 and 13 already passed** on 2026-10-23
   (`catalog-search.md:99-112`). **Nothing to do.** The backlog row is out of date here.

## 1. Decisions

- **D1 (sub-item 2). Hide, do not remove.** At narrow widths the Owner and Tags columns become
  **hidden** through the table's column visibility, not removed from the column list. Their
  filters therefore stay available in the toolbar's filter menu at every width.
  - Keep the same thresholds and the existing "columns hidden" note, which now also says
    "filters still work".
- **D2 (sub-item 3).**
  - `search_column_index` and the column count read with `FINAL`, the way the asset-detail
    query already does (`catalog.rs:2337-2345`, which says why).
  - `scripts/seed-clickhouse.sql` declares both tables as
    `ReplacingMergeTree ORDER BY (slug, key_asli)`, matching `bronze_catalog.py`.
- **D3 (sub-item 1).** A new manual workflow, `.github/workflows/catalog-search-bench.yml`
  (`workflow_dispatch`), starts the core stack on a GitHub runner, runs the bench's `seed`
  into a throwaway catalog, then `run`, and writes the JSON to the step summary.
  - **The planner runs it** and writes `docs/plans/DATA-11-RESULT.md`: the time, the command and
    the run, and whether 500 ms was met. A miss is reported, not hidden (decision 1 of the feature
    page).
  - Fix whatever the bench needs to run (it has never run). Do not change what it measures.
- **D4.** No change for sub-item 4. The planner corrects the backlog row.

## 2. Tasks

- **T1 (console, D1).**
  - Test in `data-explorer-page.test.tsx`: at a width of 1280 the Owner and Tags filters are
    offered, while the columns are not shown.
  - *Accept:* `bun run typecheck`; `bun run lint`; `bun test src/features/catalog`.
- **T2 (Rust and SQL, D2).**
  - A test that a column re-registered by a second run is counted once (unit, or a route test
    with the ClickHouse fake the catalog tests use).
  - *Accept:* `cargo fmt --check`; `cargo clippy -p lakehouse-api --all-targets -- -D warnings`;
    `cargo test -p lakehouse-api catalog`.
- **T3 (D3).** The workflow, plus any bench fix.
  - *Accept:* `actionlint` on the workflow; `python3 -m py_compile ops/bench/catalog_search_bench.py`;
    the bench's own tests, if any.

T1 can be built on its own. T2 and T3 follow on the same branch.

## 3. Handoff (developers append below)

### Handoff T1 (developer, 2026-11-01)

Commit `3c643ceb` — "fix(data): keep the Owner and Tags filters at a narrow window (DATA-24 T1)" — on `fix/data-24-catalog-search-follow-ups`, on top of `d7745735` (the plan commit). One commit, console only.

What it does:

- The Data Explorer no longer drops the columns that do not fit the window from the table's column list. They are hidden through the table's visibility instead, so their filters keep working in the toolbar's filter menu at every width (decision D1). The thresholds are unchanged (`1760`, `1500`, `1280`, `1100`, `950`, `860`).
- The hide covers **every** column the window is too narrow for — Tags, Owner, Size, Freshness, Type, Layer — not only the two the report named. D1 is "hide, do not remove", and the note under the table now says "The filters still work", which is only true if every hidden column keeps its filters. Narrowing it to Owner and Tags is a one-line change in `data-explorer-page.tsx` if the reviewer wants that; the mechanism is the same either way.
- `useDataTable` takes `hiddenColumns` (`src/hooks/use-data-table.ts`). The page's hides ride on top of the stored layout instead of replacing it, and `keepStoredVisibility` leaves the user's stored choice for those columns alone when the layout is written — so a resize cannot overwrite it, and a wider window brings back what the user had.
- The settings menu leaves a page-hidden column out of its "View" list: it is not the user's to toggle, it comes back with the width.
- The "columns hidden" note now ends with "The filters still work."

Checks run in the foreground on the final commit (`3c643ceb`):

| Command | Result |
| --- | --- |
| `bun run typecheck` | clean, no output |
| `bun run lint` | 0 errors, 10 warnings — the same 10 as on `d7745735` (checked by re-running the lint with the change taken out) |
| `bun test src/features/catalog` | 341 pass, 0 fail, 1095 expect() calls, 16 files |
| `bun run test` (the whole console suite) | 3063 pass, 1 skip, 0 fail, 8336 expect() calls, 313 files |

New test file `src/features/catalog/data-explorer-page.test.tsx` (5 tests):

- at 1280px the Owner and Tags columns are not on screen;
- at 1280px the filter picker still offers the Owner and Tags filters;
- at 1800px the Owner and Tags columns are on screen;
- the note says the filters still work;
- the window's hide is not written into the saved layout — hiding a column the user can see stores that choice and leaves `owner`/`tags` out of `app:table-layout:/data`.

Checked that the tests bite: against the pre-change code (`d7745735`) 2 of the 5 fail — "offers the Owner and Tags filters at 1280px all the same" and "says the filters still work when it hides a column". The saved-layout test fails too when `keepStoredVisibility` is taken out (1 of 5), so the guard is pinned.

Not verified, and why:

- **Rust, Python and compose checks were not run.** T1 touches the console only; per AGENTS.md a commit runs the checks for the languages it touched. T2 (Rust, SQL) and T3 (workflow, bench) are untouched by this commit.
- **The widths themselves are not verified.** The thresholds are the estimates already in the file (derived from the column caps, not measured); this change does not move them, and the plan does not ask for a measurement.
- **One consequence to be aware of.** The sort/filter whitelist in `useDataTable` is built from the table's column ids, which now always include the columns the window hides. A hand-written `?sort=tags` (the UI never offers sorting by tags — the column has `enableSorting: false`, and the server refuses to sort by a list) is therefore forwarded to the server at every width and answered with a 400, where before a narrow window dropped it silently. At a wide window this was already the case, so the behaviour is now the same at every width instead of different. Not changed here: the whitelist is shared by every table and is out of T1's scope.

### Handoff T3 (developer, 2026-11-01)

Commit `1cf100cb` — "feat(bench): the manual workflow that runs the catalog search bench (DATA-24 T3)" — on `fix/data-24-catalog-search-follow-ups`, on top of `2ba78608` (the T1 handoff commit). Three files: the new workflow, the bench, the bench's new tests. No Rust, no console, no compose change.

What it does:

- **`.github/workflows/catalog-search-bench.yml` (new).** The manual run of the bench (decision D3), modelled on `.github/workflows/g9-upload-large.yml`: `workflow_dispatch` only, the runner's disk freed first, a CI-only `.env`, `docker compose up` the stack, wait for `/health`, the bench's `seed` with `BENCH_THROWAWAY_CATALOG=true`, then `run`, with the JSON (and the command that produced it) in `$GITHUB_STEP_SUMMARY`. The actions are pinned the way this repo's other workflows pin them (`actions/checkout@v7`, `actions/setup-python@v7`). It is not in `ci-required`, for the same reason `g9-upload-large.yml` is not: a measurement is not a gate.
  - The stack it brings up is `lakehouse-api` **and what the API needs** (Postgres, ClickHouse, the OIDC mock — the API's own `depends_on`), not Lakekeeper, RustFS, the Iceberg init job or Dagster. The reason: the search reads the catalog out of ClickHouse's registry tables, and the Iceberg enrichment the same route does is bounded to 300 ms (`ICEBERG_ENRICHMENT_BUDGET`) and degrades to "the list is unchanged" when Lakekeeper is not there. Bringing the Iceberg side up would add minutes and a second set of failure modes to a job that measures the catalog search.
  - `timeout-minutes: 180`. The API image is a release build of the Rust workspace: it took ~25 minutes on the machine this was developed on, from a cold cache.
- **The bench needed two fixes to run at all.** Neither changes what it measures (the terms, the routes, the rounds, the statistics and the JSON are untouched):
  - **The registry tables are now created before the seed writes into them**, by their ONE owner (`dagster/dispar_orchestrate/bronze_catalog.py`'s `_assert_or_create_all`, the same call `register_bronze_table` makes). A stack that never applied the demo SQL has no `lake` database at all, so the seed's INSERTs had nothing to write into. The bench carries no copy of that DDL (R10), and a table that exists with another shape is a refusal (`SchemaDriftError` → a `BenchFailure` with the drift in the message), never a silent tolerance.
  - **The sign-in now rotates the bootstrap credential** when the account says a rotation is pending. The bootstrap admin is seeded with `must_change_password = true` (`lakehouse-api`'s `main.rs`, `bootstrap_admin`), and the API refuses every route except the three in `policy.rs`'s `allowed_while_must_change_password` for such a session — the first search would have been answered 403, not measured. The rotation revokes every session of the account (the one used to make the call included), so the sign-in is repeated after it. The same two steps `ops/g3/g3_test.py` does.
- **`ops/bench/test_catalog_search_bench.py` (new).** The bench had no tests. This one covers the command line (the sizes the workflow passes, the defaults the feature page asks for) and the JSON `run` prints — the shape `docs/plans/DATA-11-RESULT.md` is written from — at a tiny size, with a fake session and no network. It also covers the owner loader: the owner file has to go into `sys.modules` *before* it is executed, or every `dataclass` in it fails at import time (this was a real bug, found by running the bench, and the test pins it).

Checks run in the foreground on the final commit (`1cf100cb`):

| Command | Result |
| --- | --- |
| `python3 -m py_compile ops/bench/catalog_search_bench.py` | clean |
| `python3 -m unittest discover -s ops/bench -p 'test_*.py'` | 16 tests, 16 pass |
| `python3 -m pytest ops/bench/test_catalog_search_bench.py -q` | 16 passed |
| `actionlint .github/workflows/catalog-search-bench.yml` | no finding (actionlint v1.7.7) |
| `python3 ops/lint/check_intra_package_imports.py` | OK |
| `python3 ops/lint/check_bare_iceberg_count.py` | OK |
| `docker compose --profile '*' config --quiet` (with the `.env` the workflow writes) | clean, every profile resolves |

The tests bite, checked by mutation on a copy of the bench: dropping the password rotation fails 1 of 16; renaming the JSON key `p95Ms` fails 2 of 16; changing the seed's default size fails 1 of 16; dropping the `supported: false` guard fails 1 of 16; dropping the `sys.modules` registration of the owner fails 1 of 16.

The bench was also run **end to end on this machine** — the API image built, the stack up, `seed --datasets 10000 --columns 20`, then `run --rounds 50`:

- `seed` wrote 10,000 tables and 200,000 columns into the two registries (2.8 s).
- `run` printed the JSON the workflow puts in the step summary, exit code 0: `catalog` p50 798 ms / p95 968 ms, `catalogQuery` p50 528 ms / p95 1,050 ms, first search 12.4 s (it builds the search copy).

**These numbers are a smoke run, not the measurement.** They come from a shared development box with other containers running and with the Iceberg side of the stack absent — not from a GitHub runner, which is where the plan says the number comes from. The planner runs the workflow; this run only proves that the bench runs, that the JSON is produced, and that nothing in the bench needs a fix that the workflow cannot cover. Whether 500 ms was met is the planner's to write in `docs/plans/DATA-11-RESULT.md`.

Not verified, and why:

- **The workflow has never run on GitHub.** Running it is the planner's step (D3), and this agent does not trigger workflows. What is verified is that `actionlint` is clean on it, that every command it runs was run here in the same order against the same compose file, and that the `.env` it writes satisfies `docker compose --profile '*' config`. The one part of it that was not reproduced here is GitHub's own runner: `actions/checkout@v7` and `actions/setup-python@v7` are the pins this repo's other workflows use.
- **Rust, TypeScript and console checks were not run.** T3 touches Python, a workflow and this plan; per AGENTS.md a commit runs the checks for the languages it touched. T2's Rust and SQL are untouched by this commit.
- **The compose stack was proven by `up`, not by `config` alone** — but only for the services this job starts. `lakehouse-api` came up with Postgres, ClickHouse and the OIDC mock healthy, the API answered `/health`, and the migrations and the bootstrap admin ran. The services this job does NOT start (Lakekeeper, RustFS, the Iceberg init job, Dagster) were never brought up, so nothing about them is claimed here.
- **The bench's tests are not wired into `ci.yml`.** `ci.yml`'s "Unit-test the connector gate coverage tooling" step discovers `ops/lint` and `ops/gates`; adding `ops/bench` there is one line, but it touches `ci.yml`, which is outside T3's scope as the plan states it. The reviewer may want it; the tests run under `python3 -m unittest discover -s ops/bench -p 'test_*.py'` as they are.
- **One pre-existing `actionlint` finding is not mine**: `.github/workflows/deploy-staging.yml:17` names the self-hosted runner label `staging`, which `actionlint` does not know. It is on this branch already and is untouched by T3.
- **A local-machine fact, not a workflow fact**: on this host something outside Docker holds port 8080, so the local run published the API on 18080 (`API_HOST_PORT`). The workflow uses the compose default (8080), which is free on a GitHub runner.

## 4. Review and results (planner appends below)
