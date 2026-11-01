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
