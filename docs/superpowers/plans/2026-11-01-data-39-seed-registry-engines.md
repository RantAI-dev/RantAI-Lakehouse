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

### Handoff (developer, 2026-11-01)

**Commits on the branch.**

| commit | what |
| --- | --- |
| `887b902c` | the plan (planner) |
| `ad776cad` | T1 / D1 — the seed declares the four registry tables the way their owner declares them |
| (this handoff) | the `## 3` entry itself |

**T1's commit** is `ad776cad` — `fix(seed): the registry tables declare the
engine their owner declares (DATA-39)`. It changes
`scripts/seed-clickhouse.sql` only: the four tables
`lake.`bronze_meta.dataset_catalog``, `bronze_meta.dataset_sync` and their
`bronze_meta_sec` twins go from `ENGINE = MergeTree` to
`ENGINE = ReplacingMergeTree`, and the `ORDER BY slug` each of them already
had is the owner's sorting key.

**A note on who wrote that commit.** The branch already carried `ad776cad` when
this developer picked it up (the earlier worker had pushed it while this
handoff was being written). This developer therefore did **not** add a second
commit for T1 — a second commit would have been a duplicate of the same change.
Instead the commit on the branch was **verified independently**, against a
fresh throwaway container, and this handoff was appended on top. The commit's
own claims were not taken on trust; every acceptance criterion below was
re-run from scratch on the branch's final content. The change this developer
had staged before seeing the remote commit was functionally identical to
`ad776cad` (same four engines, same `ORDER BY`, same reason in the comment),
so nothing was lost by dropping it.

**What the owner declares** (`dagster/dispar_orchestrate/bronze_catalog.py`,
`_registry_schemas`, `:105-147`; `EXPECTED_SCHEMAS`, `:197-200`), read straight
out of the module:

| table | engine | `ORDER BY` |
| --- | --- | --- |
| `bronze_meta.dataset_catalog` | `ReplacingMergeTree` | `slug` |
| `bronze_meta.dataset_sync` | `ReplacingMergeTree` | `slug` |
| `bronze_meta_sec.dataset_catalog` | `ReplacingMergeTree` | `slug` |
| `bronze_meta_sec.dataset_sync` | `ReplacingMergeTree` | `slug` |

**No version column.** The plan's D1 says to include any version column passed
to `ReplacingMergeTree(...)`. The owner passes **none**: `TableSchema.engine`
is the bare string `"ReplacingMergeTree"` for all four tables
(`bronze_catalog.py:117`, `:133`, and the `_sec` twins through the same
`_registry_schemas` call), so the seed declares the bare engine, with no
`ReplacingMergeTree(<version>)` argument. The owner's own DDL string, printed
from the module itself, is the reference:

```
CREATE TABLE IF NOT EXISTS lake.`bronze_meta.dataset_catalog` (slug String, title String, description String, tier String, updated_at String, table_name String) ENGINE = ReplacingMergeTree ORDER BY slug
CREATE TABLE IF NOT EXISTS lake.`bronze_meta.dataset_sync` (slug String, title String, description String, table_name String, total UInt64, author String, frekuensi String, satuan String, klasifikasi String) ENGINE = ReplacingMergeTree ORDER BY slug
```

The two `dataset_column` tables were already `ReplacingMergeTree ORDER BY
(slug, key_asli)` from #308 (`DATA-24 T2`), so all six registry tables now
agree with the one owner.

**Acceptance, run on the branch's final content.** A throwaway
`clickhouse/clickhouse-server:26.8` container (server reports `26.8.30.8`),
started with `docker run -d --name data39-ch -e CLICKHOUSE_SKIP_USER_SETUP=1`
and removed with `docker rm -f data39-ch` afterwards (verified: `docker ps -a
--filter name=data39-ch` is empty).

The exact commands and their exit codes:

```
$ docker rm -f data39-ch                      # clean slate
$ docker run -d --name data39-ch -e CLICKHOUSE_SKIP_USER_SETUP=1 clickhouse/clickhouse-server:26.8
$ docker exec data39-ch clickhouse-client --query "SELECT version()"
26.8.30.8
$ docker exec -i data39-ch clickhouse-client --multiquery < scripts/seed-clickhouse.sql
RUN_1_EXIT=0
$ docker exec -i data39-ch clickhouse-client --multiquery < scripts/seed-clickhouse.sql
RUN_2_EXIT=0
$ docker rm -f data39-ch
data39-ch
$ docker ps -a --filter name=data39-ch --format '{{.Names}}' | wc -l
0
```

Both runs of the whole seed script exit 0. (`-i` matters: without it `docker
exec` does not forward stdin and the client reads an empty script, which
still exits 0 and creates nothing. The first attempt in this session did
exactly that — the `lake` database did not exist afterwards. Every number
below is from the `-i` runs.)

`SHOW CREATE TABLE` on each of the four tables:

```
=== SHOW CREATE TABLE lake.`bronze_meta.dataset_catalog` ===
ENGINE = ReplacingMergeTree
ORDER BY slug
=== SHOW CREATE TABLE lake.`bronze_meta.dataset_sync` ===
ENGINE = ReplacingMergeTree
ORDER BY slug
=== SHOW CREATE TABLE lake.`bronze_meta_sec.dataset_catalog` ===
ENGINE = ReplacingMergeTree
ORDER BY slug
=== SHOW CREATE TABLE lake.`bronze_meta_sec.dataset_sync` ===
ENGINE = ReplacingMergeTree
ORDER BY slug
```

`system.tables` reads the same four back as `engine=ReplacingMergeTree`,
`sorting_key=slug`. The **owner's own R10 guard** was then run against the
seeded container — `bronze_catalog.py::_assert_or_create_all(target,
EXPECTED_SCHEMAS)`, the function that raises `SchemaDriftError` when an
existing table's engine, sorting key or columns differ from the owner's. It
**passed** for all six registry tables, so a seeded stack is accepted by the
owner rather than refused by it.

Seeded row counts after the second run (`system.parts`, active): 4 / 4 for the
`bronze_meta` `dataset_catalog` / `dataset_sync`, 15 for
`bronze_meta.dataset_column`, 2 / 2 for the `bronze_meta_sec` twins, 4 for
`bronze_meta_sec.dataset_column` — unchanged between the two runs.

**D2 was not touched.** No Rust, no TypeScript, no Dagster change: the diff is
`scripts/seed-clickhouse.sql` alone. The assistant's reads without `FINAL`
(`routes/ai/data_map.rs`, `routes/ai/tools/data.rs`) are the AI team's.

**D3 — the per-table column count: it CAN still be wrong, and `FINAL` did not
fix it.** The backlog's phrase ("the catalog's column count takes the first row
it sees") is still literally true of the code, and it is worse than "takes the
first row" — the first row is not even stable between identical reads.

- The count comes from `col_rows` at
  `rust/crates/lakehouse-api/src/routes/catalog.rs:533-539`: two `GROUP BY
  slug` counts, one per registry, `UNION ALL`-ed.
- `col_of` at `rust/crates/lakehouse-api/src/routes/catalog.rs:554-559` takes
  `.iter().find(|c| str_col(c, "slug") == slug)` — **the first row for that
  slug**, and its number is that row's `n`.
- `FINAL` (added by #308) fixed a *different* defect, and it does fix it: the
  same `(slug, key_asli)` row written again by a second connector run used to
  be counted twice. Measured on the seeded container, with
  `SYSTEM STOP MERGES` so the two copies stay in two separate parts: the same
  `(slug, key_asli)` inserted by two separate `INSERT`s counts as **2**
  without `FINAL` and as **1** with it. A plain `MergeTree` control table
  with the same two rows counts **2** either way — it has no merged copy for
  `FINAL` to ask for, which is exactly why the seed must declare the owner's
  engine.
- What `FINAL` does **not** do is merge the two registries' counts. A slug
  present in **both** `bronze_meta.dataset_column` and
  `bronze_meta_sec.dataset_column` yields **two** rows in `col_rows`, and
  `col_of` reports only one of them — never the sum. Measured: a slug with 3
  columns in `bronze_meta` and 2 in `bronze_meta_sec` is reported as `3` or
  as `2`, and **which one varies between identical runs** (8 runs of the same
  query gave `3,2,2,2,2,2,2,2`): `UNION ALL` gives no order guarantee, so
  `.find()` takes whichever branch the engine happened to return first.
- The **detail page disagrees with the list page** for such a slug: the
  detail path's query (`catalog.rs:2351-2353`, also with `FINAL`) `UNION
  ALL`s the column *rows* of both registries and counts them with
  `cols.len()` (`catalog.rs:2456`), so it reports `5` where the list reports
  `3` or `2`. Measured on the same data.
- The same "first row" pattern also reaches the **catalog and sync** reads,
  which have no `FINAL` at all: `cat` (`catalog.rs:507-518`) and `sync_rows`
  (`catalog.rs:519-525`). With the registry now a `ReplacingMergeTree`, a
  second connector run for the same slug leaves two rows until the merge
  catches up, and `.find()` picks one of them. Measured with merges stopped,
  over 20 identical runs of the `sync_rows` query, the first row for one slug
  came back as `999 / baru` **14** times and as `720 / Badan Pusat Statistik`
  **6** times — the same query, the same data, two different answers. And
  `cat` returned the slug twice, so the list shows the asset **twice**
  (`catalog.rs:567-591` maps one asset per `cat` row). The detail path's own
  `LIMIT 1` over the same two `dataset_sync` rows
  (`catalog.rs:2330-2344`) is the same hazard.

So: **yes, the per-table column count can still be wrong now that `FINAL` is
used.** `FINAL` made each registry's own count right; the count that reaches
the UI is still "the first row the engine returned for this slug", drawn from
a `UNION ALL` of two registries whose order the engine does not promise. The
fix belongs to the AI team or to a new backlog item — D2 forbids this
developer from changing Rust, so nothing was changed here.

**Not verified, and why.**

- **No cargo, bun or pytest was run.** The diff is one SQL file; per
  `AGENTS.md`'s "Keep build time down", a commit that touches no Rust runs no
  cargo command, and this one touches no TypeScript and no Python either.
- **The change was not exercised against a running stack** — no `docker
  compose up`, no API reading the seeded registry. The evidence is the
  throwaway `ClickHouse` container and the owner's own guard, not the console.
- **The Rust tests were not run** (and none were changed): the `FINAL`
  semantics above were measured against the real engine, not against the
  repository's `wiremock` fake.
- **A stack seeded before this change keeps `MergeTree`.**
  `CREATE TABLE IF NOT EXISTS` never alters a table that already exists, so
  the four tables stay `MergeTree` until they are dropped and created again.
  The seed file says so in a note next to the DDL, the way the `dataset_column`
  note already did. The change drops no table, because the seed is meant to
  be safe to re-run against a live stack.
- **The count question above was measured on the seeded data plus
  hand-inserted probe rows** (`probe-both`, a slug in both registries;
  `probe-dup2`, a duplicated `(slug, key_asli)`; a duplicated
  `wisman-bulanan` row), with `SYSTEM STOP MERGES` on the probed tables so the
  copies stayed in separate parts. The probes were made on throwaway
  containers that have since been removed; the seeded state was left
  untouched.

### Handoff (developer, 2026-11-01) — the verification block, and D3 on the merged tree

**Who this is.** I wrote T1's commit, `ad776cad`. While I was running the
verification block, another worker picked up the same branch and appended the
handoff above (`7b0bc07e`), and the planner then reviewed it (`2da01498`, which
reworded the seed's comment into English and merged `main` into the branch).
This section is **my** handoff for the commit I wrote, added on top of the
branch's final content (`a7576a32`). It adds what the handoff above does not
have: the **full verification block with its counts** (the handoff above did
not run the Rust, TypeScript or Dagster suites), and the D3 answer with the
line numbers of the **merged** tree — the detail page's lines shifted when
`main` (which carries `DATA-38`'s `catalog_search_cache.rs` and
`routes/catalog.rs`) came in.

**Commits on the branch.**

| commit | what |
| --- | --- |
| `887b902c` | the plan (planner) |
| `ad776cad` | T1 / D1 — the seed declares the four registry tables the way their owner declares them (**mine**) |
| `7b0bc07e` | the first handoff (the parallel worker) |
| `2da01498` | review round 1 — the comment in English; the review's findings |
| `a7576a32` | `main` merged into the branch |
| (this section) | the verification block |

The branch's diff against `origin/main` (`303c7b4d`) is **`scripts/seed-clickhouse.sql` and this plan file** — no Rust, no TypeScript, no Python, no compose, and nothing in the assistant's area (D2, the AI team's).

**Accept, re-run on the branch's final content** (`a7576a32`), in the foreground:

```
$ docker run -d --name data39-seed-check clickhouse/clickhouse-server:26.8
$ docker exec data39-seed-check clickhouse-client --query "SELECT version()"
26.8.36.3
$ docker exec -i data39-seed-check clickhouse-client --multiquery < scripts/seed-clickhouse.sql
RUN 1 exit: 0
$ docker exec -i data39-seed-check clickhouse-client --multiquery < scripts/seed-clickhouse.sql
RUN 2 exit: 0
```

`SHOW CREATE TABLE` on each of the four tables, after the second run:

```
lake.`bronze_meta.dataset_catalog`      ENGINE = ReplacingMergeTree  ORDER BY slug
lake.`bronze_meta.dataset_sync`         ENGINE = ReplacingMergeTree  ORDER BY slug
lake.`bronze_meta_sec.dataset_catalog`  ENGINE = ReplacingMergeTree  ORDER BY slug
lake.`bronze_meta_sec.dataset_sync`     ENGINE = ReplacingMergeTree  ORDER BY slug
```

The owner's own R10 guard was then run against the seeded container —
`bronze_catalog.py::_assert_or_create_all(target, EXPECTED_SCHEMAS)`, the
function that raises `SchemaDriftError` when an existing table's engine,
sorting key or columns differ from the owner's. It **passes for all six**
registry tables (`engine=ReplacingMergeTree`, `sorting_key=slug` for the
catalog and sync twins, `slug, key_asli` for the column twins, the column
tuples equal to `EXPECTED_SCHEMAS`), so a seeded stack is accepted by the
owner rather than refused by it. `docker rm -f data39-seed-check` afterwards;
`docker ps -a --filter name=data39-seed-check` is empty.

**The full verification block.** Rust: `CARGO_TARGET_DIR=/home/uaa/rantai/cargo-target`,
`CARGO_BUILD_JOBS=4`, `CARGO_INCREMENTAL=0`.

| command | result |
| --- | --- |
| `cd rust && cargo fmt --check` | clean, exit 0 (10s) |
| `cd rust && cargo clippy --workspace --all-targets --all-features -- -D warnings` | exit 0, no warnings (6m03s) |
| `cd rust && cargo test --workspace` | **2868 passed, 5 failed, 18 ignored** in `lakehouse-api --lib`; exit 101 (cargo stops at the first failing target) |
| `cd rust && cargo test --workspace --no-fail-fast` | 163 test binaries, exit 101; 4 targets red, every other target green — listed below |
| `bun run typecheck` | exit 0 |
| `bun run lint` | exit 0 — 0 errors, 10 warnings (none in a file this branch touches) |
| `bun run test` | 3080 pass, 1 skip, 0 fail (3081 tests across 313 files) |
| `python3 ops/lint/check_intra_package_imports.py` | OK: all intra-package `dispar_orchestrate` imports resolve |
| `python3 ops/lint/check_bare_iceberg_count.py` | R11 guard: no bare `count()` against a Bronze Iceberg table found |
| `(cd dagster && python -m pytest dispar_orchestrate -q)` | 2398 passed, 1 skipped, **1 failed**, 33 subtests passed (25m53s) — the one failure is a flake, below |
| `docker compose --profile '*' config --quiet` | exit 0, with throwaway values for the five required variables (`CONNECTOR_SECRETS_KEY`, `POSTGRES_PASSWORD`, `RUSTFS_ACCESS_KEY`, `RUSTFS_SECRET_KEY`, `LAKEKEEPER_ENCRYPTION_KEY`); no compose file was changed |
| `git status --short` | clean — nothing generated staged |

`cargo test --workspace --no-fail-fast`, the red targets:

- `lakehouse-api --lib`: 2868 passed, **5 failed**, 18 ignored
- `lakehouse-api --bin lakehouse-api`: 2906 passed, **5 failed**, 18 ignored
- `lakehouse-api --test agents_approval`: 15 passed, **2 failed**
- `lakehouse-api --test ai_tool`: 24 passed, **1 failed**
- `lakehouse-clickhouse --lib`: **1 failed** (`the_byte_stream_hands_the_body_over_as_it_arrives`)

**None of these failures is this change's.** The branch's Rust tree is
byte-identical to `origin/main` (`git diff --stat origin/main -- rust/` is
empty; the branch's only code diff is `scripts/seed-clickhouse.sql`, plus this
plan file), and no Rust test reads `scripts/`. Nine distinct tests fail, all
on the network:

- `connector_probe::tests::postgres_unreachable_host_reports_real_elapsed_time_and_failure`
- `connector_probe::tests::postgres_loopback_host_is_reachable_when_internal_hosts_allowed`
- `connector_probe::tests::wizard_postgres_connector_dials_from_its_dial_not_its_host`
- `pdf::tests::a_gotenberg_that_is_down_is_unreachable`
- `routes::connectors::tests::deprovision_reads_slot_and_publication_from_dial_for_cdc_adapter`
- `lakehouse-clickhouse`'s `tests::the_byte_stream_hands_the_body_over_as_it_arrives` — "Connection reset by peer" while streaming a body
- `agents_approval::approve_executes_the_tool_and_flips_run_status` and `agents_approval::rename_board_confirmed_passes_the_gate_and_is_audited_as_failed` — HTTP 408 where 200 is expected
- `ai_tool::approve_delete_board_executes_the_dispatch_and_audits_it` — HTTP 408 where 200 is expected

One measured cause for the first five: on this machine a TCP connect to a
closed loopback port is **dropped, not refused**, so a dial that should fail
fast with "connection refused" hangs until the 5s timeout and the error
variant the test asserts never arrives (`socket.connect(("127.0.0.1", 1))`
times out after 3.10s instead of raising `ConnectionRefusedError`; the
failures read "PostgreSQL connection timed out after 5s", "the source database
did not answer within 5s" and "the PDF was not ready in time"). The other four
are the same class (a body stream reset, and three requests that time out).
Per `AGENTS.md`, a check already red on `main` does not block a PR that did
not cause it — the reviewer names it in the review, and it needs a backlog
item.

The one Dagster failure, `test_sink_history.py::test_a_composite_key_closes_only_the_pair_that_changed`,
is a flake in the test's own ordering assumption: the helper sorts the
versions by `_valid_from` (`test_sink_history.py:43`), and when both versions
of a key land in the same timestamp granularity the sort ties and the order
comes from the engine's row order. It failed once in the full-suite run (two
versions of key `(1, 2)` came back current-first) and passes 3/3 when run
alone (`python3 -m pytest dispar_orchestrate/test_sink_history.py::test_a_composite_key_closes_only_the_pair_that_changed -q`:
3 runs, 1 passed each). The Python tree is byte-identical to `origin/main`
too, and this commit touches no Python.

One environment note for the reviewer: this machine had none of the code
location's Python dependencies installed (`pytest`, `dlt`, `pyarrow`,
`pyiceberg`, `dagster` itself were all missing), so the Dagster suite was run
against an isolated `pip install --target /tmp/opencode/dag39-pydeps` of the
pinned versions, with `PYTHONPATH` pointing at it — the machine's own
site-packages were not touched. One extra package was needed there
(`pyopenssl`, because the system's is incompatible with
`cryptography==50.0.2`), so CI's environment is not bit-identical to this run.

**D3 — the per-table column count, on the merged tree.** My answer agrees
with the handoff above and the review: **a wrong count is still possible**,
and `FINAL` did not fix it. The line numbers below are the branch's own
(`origin/main` at `303c7b4d`; the detail page's lines shifted by the merge,
so they are `:2364-2365` and `:2468` where the handoff above says `:2351-2353`
and `:2456`).

- The count comes from `col_rows` (`rust/crates/lakehouse-api/src/routes/catalog.rs:535-536`):
  two `GROUP BY slug` counts, one per registry, `UNION ALL`-ed.
- `col_of` (`rust/crates/lakehouse-api/src/routes/catalog.rs:554-559`) takes
  `.iter().find(|c| str_col(c, "slug") == slug)` — **the first row for that
  slug** — and reports that row's `n`. It feeds `"columnCount"`
  (`catalog.rs:583` → `catalog.rs:1225`).
- A slug present in **both** `bronze_meta.dataset_column` and
  `bronze_meta_sec.dataset_column` yields **two** rows in `col_rows`, and
  `col_of` reports one of them — never the sum. Measured on the throwaway
  container: a slug with 6 columns in `bronze_meta` and 1 in
  `bronze_meta_sec` is reported as `6`, while the **detail page** counts the
  union of both registries' column *rows* (`catalog.rs:2364-2365`,
  `let col_count = cols.len();` at `catalog.rs:2468`) and reports `7`. The
  list and the asset's own page therefore disagree about the same dataset.
- What `FINAL` did close is the other defect: the same `(slug, key_asli)`
  written again by a second connector run. Measured on the container, the
  duplicated row counts **7 without** `FINAL` and **6 with** it — the 6
  distinct columns the table has.
- The same "first row" pattern also reaches the **catalog and sync** reads,
  which have no `FINAL` at all: `cat` (`catalog.rs:507-518`) and `sync_rows`
  (`catalog.rs:519-525`). With the registry now a `ReplacingMergeTree`, a
  second connector run for the same slug leaves two rows until the merge
  catches up, and `.find()` picks one of them.
- Nothing in the code forbids one slug in both registries: there is no
  uniqueness guard anywhere (`register_bronze_table` only ever writes
  `bronze_meta`, with `tier='primer'`; `bronze_meta_sec` is written by
  another producer or by hand), and the two tables are `UNION ALL`-ed on every
  read. The bench spreads its datasets across the two registries
  (`ops/bench/catalog_search_bench.py:207-209`), so it never produces the
  collision, and the tests do not pin the behaviour either way.

This is the review's `DATA-40`; nothing here was changed (D3 is investigation
only), and no code in the assistant's area was touched (D2).

## Review round 1 (planner, 2026-11-01, at `7b0bc07e`)

- **T1 matches D1.** The four tables are now `ReplacingMergeTree` with their existing
  `ORDER BY slug`. This is what `bronze_catalog.py:117-118` and `:133-134` declare, with no
  version column. The handoff's two runs on a fresh 26.8 container, the `SHOW CREATE TABLE`
  lines and the owner's own R10 guard passing on the seeded stack are the proof.
- **D2 is respected.** No Rust, TypeScript or Dagster changed.
- **Fixed by the planner, at review: the new comment was in Indonesian** and replaced an English
  one (`AGENTS.md`: comments are in English). It is rewritten in English with the same content.
  Only comments changed, so the acceptance run still holds. The rest of the file's older
  Indonesian header is out of scope here.
- **D3: the column count is still wrong for a slug in both registries.** The handoff measured
  it, and the reading is right. `col_of` (`routes/catalog.rs:554-559`) takes the first of two
  per-registry counts from a `UNION ALL`, so the list page shows one registry's count, not
  their sum. Which one varies between runs, and it disagrees with the detail page
  (`:2456`, which counts both). The catalog and sync reads (`:507-525`) have no `FINAL`, so the
  list can also show such a slug twice. This becomes a new backlog item, `DATA-40` (Data, P3).
  Its first question is whether one slug in both `bronze_meta` and `bronze_meta_sec` is a state
  the product allows, before it decides between summing and refusing.

**Open: no BLOCKER, no SHOULD-FIX.**
