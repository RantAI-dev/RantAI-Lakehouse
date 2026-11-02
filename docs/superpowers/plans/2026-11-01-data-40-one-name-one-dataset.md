# DATA-40: one dataset name, one dataset

Backlog `DATA-40` (Task, Data, P3), found in `DATA-39` D3 (#315). Base: `main` at `5661a043`.
Planner: Claude Opus. Developer: another agent (`AGENTS.md`).

## 0. What is wrong

The catalog reads two registries, `lake.bronze_meta.*` (primary) and `lake.bronze_meta_sec.*`
(secondary), and `UNION ALL`s them. A dataset's `slug` is its identity: the console's address
and every other read use it. When one slug is in both registries:

- the list's column count is one registry's, and which one varies between runs: `col_of`
  takes the first row (`routes/catalog.rs:554-559` over the union at `:533-539`);
- the detail page counts both registries' columns (`:2364-2365`, `cols.len()`), so it
  disagrees with the list;
- the detail page's header row is `LIMIT 1` over the union with no order (`:2341-2357`), so
  which registry it shows varies too;
- the list's catalog and sync reads (`:507-525`) have no `FINAL`, so the slug can be listed
  twice. A slug the same registry holds twice, before `ClickHouse` merges a re-run, can be
  listed twice the same way.

The product's own writer, `register_bronze_table`
(`dagster/dispar_orchestrate/bronze_catalog.py:292`), writes only to `bronze_meta`. The
secondary registry is filled only by an older bootstrap or by hand.

## 1. Decisions (product owner, 2026-11-01)

- **D1. One slug names one dataset.** Two datasets never share a slug across the registries.
- **D2. When writing, refuse.** `register_bronze_table` first checks whether the slug is in
  `bronze_meta_sec.dataset_catalog`. If it is, it inserts nothing and raises a new
  `SlugConflictError`. The message names the slug and says that a secondary dataset already
  uses it and that the load needs another table name. The check reads with `FINAL`, and the
  slug is bound through `_sql_string_literal`, as the inserts are.
- **D3. When reading, the primary wins, everywhere in `routes/catalog.rs`.** A slug that is in
  both registries is shown once, from `bronze_meta`. Every secondary branch of a union gets
  `AND slug NOT IN (SELECT slug FROM lake.\`bronze_meta.dataset_catalog\` FINAL)`, or a
  `WHERE` that says the same. That applies to the list's catalog, sync and column-count reads,
  the search copy's column read (`:776-782`), the detail page's header and column reads
  (`:2341-2365`), and `registered_slug` (`:2628-2636`). A clash that already exists therefore
  gives one answer, never a varying one.
- **D4. The list's catalog and sync reads take `FINAL`**, for the reason the column reads
  already give (`:526-532`): a re-run writes the same slug again.
- **D5. A clash is reported, without upstream text.** While the list is built, if any slug is
  in both registries, the API logs one `tracing::warn!` with the number of such slugs and the
  first few slugs. Nothing is added to the response.
- **D6. Only `routes/catalog.rs` changes on the read side.** These other readers also union
  the two registries: `catalog_governance.rs`, `catalog_profile.rs`, `governance.rs`,
  `lineage.rs`, `overview.rs`, `storage.rs` and `sql_rewrite.rs`. The developer lists each one
  in the handoff, with `file:line`, and says whether a clash changes what it shows. The
  planner then decides on a follow-up. `routes/ai/*` is the AI team's (`DATA-39`) and is not
  touched or listed.

## 2. Tasks (one commit each, in order)

- **T1. The write guard (D2).**
  - Add the guard to `bronze_catalog.py`, with `SlugConflictError` next to
    `SchemaDriftError`.
  - Unit tests with the fake ClickHouse target the module's tests already use (no network):
    - a slug the secondary registry holds is refused, and nothing is inserted;
    - a free slug is registered as before;
    - the slug in the check query is a bound literal.
  - Test names are full sentences.
  - *Accept:* `(cd dagster && python -m pytest dispar_orchestrate -q -k bronze_catalog)` and
    `python3 ops/lint/check_intra_package_imports.py`.
- **T2. The reads (D3–D5).**
  - Change `routes/catalog.rs` as D3 and D4 say, with a comment at each site citing `DATA-40`.
  - Add the warn of D5.
  - Tests in the crate's existing ClickHouse-backed catalog tests (the test at `:6707-6717`
    already writes to both registries). Seed one slug in both registries, with a different
    column count in each, and assert:
    - the list shows it once, with the primary's count, on several reads;
    - the detail page shows the primary's header and only the primary's columns;
    - `registered_slug` answers the primary's slug;
    - a slug written twice to the primary before a merge is listed once.
  - *Accept:* `cargo fmt --check`, `cargo clippy -p lakehouse-api --all-targets -- -D
    warnings`, and `cargo test -p lakehouse-api catalog`.
- **T3. The handoff list of D6** goes in the handoff entry. It needs no code.

## 3. Out of scope

- The AI team's reads (`DATA-39`).
- Any other reader than `routes/catalog.rs` (D6).
- Moving or merging rows that already clash.

## 4. Handoff (developer appends below)

### Handoff (developer, 2026-11-01)

**Commits** — branch `fix/data-40-one-name-one-dataset`, base `60527153` (the plan commit):

- `0005ab8f` `fix(bronze-catalog): one slug names one dataset, on the write side (DATA-40 T1)` — T1, D2.
- `4cc2517b` `fix(catalog): one slug names one dataset, on the read side (DATA-40 T2)` — T2, D3/D4/D5.
- this commit — T3, the D6 list below (no code).

**Where the change sits**

- T1 (D2): `dagster/dispar_orchestrate/bronze_catalog.py:63` (`SlugConflictError`), `:302`
  (`_assert_slug_is_free`), `:365` (the call, after the R10 schema bootstrap and before the
  first insert); tests in `dagster/dispar_orchestrate/test_bronze_catalog.py:174`, `:193`, `:212`.
- T2 (D3): `rust/crates/lakehouse-api/src/routes/catalog.rs:502` (`NOT_IN_PRIMARY`), the
  list's catalog/sync/column-count reads at `:555`/`:565`/`:577`, the search copy's column
  index at `:847`, the detail page's header read at `:2418` and column read at `:2441`,
  `registered_slug` at `:2715`.
- T2 (D4): `FINAL` on the list's catalog and sync reads (`:555`, `:565`) and on the
  secondary branch of every union through `NOT_IN_PRIMARY`.
- T2 (D5): `CLASH_SQL` at `:511` and the one `tracing::warn!` at `:589` — the number of
  slugs both registries hold and the first few of them; nothing is added to the response.
- T2 (tests): `rust/crates/lakehouse-api/src/routes/catalog.rs:7050` (`mod registry_clash`),
  plus the fake engine's `table_read` at `:6870`.

**Commands run** (every cargo command through `/home/shiro/rantai/bin/cargo-locked`, from
`rust/`, with the shared `CARGO_TARGET_DIR`; the Python run from `dagster/`):

| command | result |
| --- | --- |
| `python -m pytest dispar_orchestrate -q -k bronze_catalog` | **10 passed**, 2393 deselected (7 pre-existing + the 3 new) |
| `python3 ops/lint/check_intra_package_imports.py` | `OK: all intra-package dispar_orchestrate imports resolve` (exit 0) |
| `python3 ops/lint/check_bare_iceberg_count.py` | `R11 guard: no bare count() against a Bronze Iceberg table found.` (exit 0) |
| `cargo fmt --check` | exit 0, no output |
| `cargo clippy -p lakehouse-api --all-targets -- -D warnings` | finished, no warnings |
| `cargo test -p lakehouse-api catalog` | **568 passed, 0 failed** |

The 568 are: the crate's lib test binary 274, its bin test binary 276, and the integration
binaries the filter matches 18 (`ai_sql_help` 2, `catalog_grants` 1, `gold_export_auth` 3,
`iceberg_door` 2, `query_tenant_scope` 4, `route_auth` 5, `semantic_api` 1).

**The new tests fail without the fix** (measured, not assumed): with the production change
reverted, all four fail — the list shows the clashing slug twice, the detail page counts 5
columns (both registries) instead of 2, `registered_slug` answers the secondary's row, and
the twice-written slug is listed twice. The tests run against a REAL `ClickHouse` 26.8 in a
`testcontainers` container, one container per TEST (a local, so it is dropped — and
removed — when the test ends; a container in a `static` is never dropped, because the test
harness exits the process). `docker ps` after the run: no container left behind.

**D6 — the other readers that union the two registries** (all untouched; `routes/ai/*` is
the AI team's and is not listed, per D6). "A clash" below means one slug in both registries:

| reader | file:line | does a clash change what it shows? |
| --- | --- | --- |
| `sql_rewrite` (not a read) | `rust/crates/lakehouse-api/src/sql_rewrite.rs:5746` | **No.** The string is a constant in `CLICKHOUSE_SHAPES` — a SQL *shape* the table substitution must accept (copied from the AI team's catalog view). Nothing is executed and nothing is counted. |
| `governance::classification` | `rust/crates/lakehouse-api/src/routes/governance.rs:437` | **Yes.** The classification list gets two rows with the same id `c-<slug>` (the primary's and the secondary's title/tier), and the duplicate takes one of the 500 `LIMIT` slots. |
| `overview` (assets, stale) | `rust/crates/lakehouse-api/src/routes/overview.rs:67` and `:69` | **Yes.** The slug is counted twice in `assets`, and the joined sync row twice in `stale`. |
| `overview` (warm assets) | `rust/crates/lakehouse-api/src/routes/overview.rs:85` | **Yes.** The clashing dataset's `total` is counted twice. |
| `catalog_governance::use_by_asset` | `rust/crates/lakehouse-api/src/routes/catalog_governance.rs:616` | **No, for the slug.** Both rows map to the same slug, so the slug shown is the same; what changes is that the secondary's table name becomes a second key pointing at it. |
| `storage` (warm rows, assets) | `rust/crates/lakehouse-api/src/routes/storage.rs:80` | **Yes.** The clashing dataset's row count is summed twice and the asset count is one too high. |
| `catalog_profile::bronze_table_name` | `rust/crates/lakehouse-api/src/routes/catalog_profile.rs:280` | **Not today.** The primary's row comes first in the union, so the primary's table name is the answer — but nothing pins it (`LIMIT 1`, no order). The same rule is now pinned for `registered_slug` in `routes/catalog.rs`. |
| `lineage` (publishers) | `rust/crates/lakehouse-api/src/routes/lineage.rs:250` | **Yes.** One publisher edge per registry, so two publishers for one dataset when the two registries name different authors. |

**Not verified, with the reason**

- The full workspace run (`cargo clippy --workspace --all-targets --all-features -- -D
  warnings`, `cargo test --workspace`) was **not** run: this machine's rule scopes cargo to
  `lakehouse-api`, and CI runs the workspace.
- `bun run typecheck && bun run lint && bun run test` and `docker compose --profile '*'
  config --quiet` were **not** run: this change touches no TypeScript and no compose file.
- The whole-directory Python suite (`python -m pytest dispar_orchestrate -q`, without
  `-k`) was **not** run: the venv this worktree used
  (`/home/shiro/rantai/venv-pr186/bin/python`) runs it, but the shared
  `~/.cache/rantai-dagster-venv` has no `fastavro`, so seven test modules fail at
  collection there — a pre-existing gap in that venv, not a change of this branch. Only the
  `-k bronze_catalog` selection and the two lints above were run for the Python side.
- The detail page's **header** read still has no `FINAL`: a slug the PRIMARY registry holds
  twice (a re-run before `ClickHouse` merges the parts) can still give two header rows
  there, and `LIMIT 1` picks one. D4 scoped `FINAL` to "the list's catalog and sync reads",
  so I did not add it to the detail read — the planner decides.
- `registered_slug` now answers "not registered" for the SECONDARY registry's table name of
  a clashing slug (D3's rule: the primary wins, so the secondary's row is not read).
  `routes/uploads` asks this function whether a table name is taken, so a name the
  secondary registry holds for a clashing slug reads as free. That is what D3 prescribes;
  the planner decides whether the follow-up should say so.
- The clash read (D5) is one more query per list build. It runs inside the same
  `tokio::try_join!` as the three registry reads, so it costs no extra round trip, and it
  does not run at all when no list is built.
- The seeded deployment has no clash: `scripts/seed-clickhouse.sql`'s slugs are disjoint
  across the two registries (`wisman-bulanan`, `kunjungan-dtw`, `event-pariwisata`,
  `gci-readiness` in the primary; `usaha-kuliner`, `atlas-poi` in the secondary), so D5's
  warning stays quiet there. Not run against a seeded stack — only the statements were
  checked against a real `ClickHouse` 26.8.

### Fix for review round 1 (developer, 2026-11-01)

**Finding fixed.** `SHOULD-FIX 1` — the detail page's header read (`bronze_asset_detail_body`)
had no `FINAL`.

**Commit** — branch `fix/data-40-one-name-one-dataset`:

- `747c633b` `fix(catalog): the detail page's header reads the merged row (DATA-40 SHOULD-FIX 1)`.

**What changed**

- `rust/crates/lakehouse-api/src/routes/catalog.rs:2439-2445` (the header read of
  `bronze_asset_detail_body`): `FINAL` on BOTH tables of BOTH branches of the read —
  `bronze_meta.dataset_sync`, `bronze_meta.dataset_catalog`, `bronze_meta_sec.dataset_sync`
  and `bronze_meta_sec.dataset_catalog` — with a comment at the site citing
  `DATA-40 review round 1 SHOULD-FIX 1`. The alias comes BEFORE `FINAL`
  (`table AS s FINAL`): the engine takes the alias as part of the table expression and
  `FINAL` as the table's modifier (`table FINAL AS s` is a syntax error on `ClickHouse`
  26.8).
- `rust/crates/lakehouse-api/src/routes/catalog.rs:7524-7577` (the test, in `mod
  registry_clash`): `the_detail_header_shows_the_newer_total_of_a_slug_written_twice` — the
  slug's primary sync row written twice (`total` 10, then 20) before any merge, and the
  detail header must show 20. The test pins the state, so the answer is one thing and not a
  coin toss: `SYSTEM STOP MERGES` on the sync table (the two copies stay apart — "before any
  merge") and `max_threads=1` on the client's URL (a URL setting, so the statement the
  module sends is the statement that runs — the engine reads one part at a time, in the
  order the parts were written).
- `rust/crates/lakehouse-api/src/routes/catalog.rs:7214` — the test helper `pinned`, which
  builds that URL.

**The new test FAILS on the old code** (measured, not assumed): with the four `FINAL`s
removed and everything else as it is, the test fails with `left: Number(10), right:
Number(20)` — the page showed the row count of the run BEFORE the last one. With the fix it
passes.

Why the old read could answer either way: it is a `LIMIT 1` over the union with no order,
so its answer was whichever of the two rows the engine answered first. Measured on a
`ClickHouse` 26.8 with the same fixture and no `FINAL`, the same statement answered `20,
10, 20, 20, 20` over five runs — the answer varied with the thread count. With `FINAL` the
answer is the merged row: `20` on every run.

**Commands run** (every cargo command through `/home/shiro/rantai/bin/cargo-locked`, from
`rust/`, with the shared `CARGO_TARGET_DIR`):

| command | result |
| --- | --- |
| `cargo fmt --check` | exit 0, no output |
| `cargo clippy -p lakehouse-api --all-targets -- -D warnings` | finished, no warnings |
| `cargo test -p lakehouse-api --lib registry_clash` | **5 passed**, 0 failed (the 4 of DATA-40 T2 + the new one) |
| `cargo test -p lakehouse-api --lib registry_clash::the_detail_header_shows_the_newer_total_of_a_slug_written_twice` on the old code (the four `FINAL`s removed) | **FAILED**: `left: Number(10), right: Number(20)` |
| `cargo test -p lakehouse-api --lib catalog` (extra, beyond the plan's scope) | **275 passed**, 0 failed |
| `python -m pytest dispar_orchestrate -q -k bronze_catalog` (from `dagster/`, with `/home/shiro/rantai/venv-pr186/bin/python`) | **10 passed**, 2393 deselected |

`docker ps` after the test run: no container left behind (each `ClickHouse`
testcontainer is dropped with its test).

**The branch carries T1 and T2 again.** The review commit `940829f0` dropped them: it put
`routes/catalog.rs` back to the pre-T2 blob, `bronze_catalog.py` and
`test_bronze_catalog.py` back to the pre-T1 blobs, and it removed the handoff entry above
from this plan — so the branch carried the plan's review of code it no longer had.
`747c633b` restores them byte for byte from the reviewed commits `4cc2517b` (T2) and
`0005ab8f` (T1), and carries the fix on top of them.

**Not verified, with the reason**

- The full workspace run (`cargo clippy --workspace --all-targets --all-features -D warnings`,
  `cargo test --workspace`) was **not** run: this machine's rule scopes cargo to
  `lakehouse-api`, and CI runs the workspace.
- `bun run typecheck && bun run lint && bun run test` and `docker compose --profile '*'
  config --quiet` were **not** run: this change touches no TypeScript and no compose file.
- The whole-directory Python suite (`python -m pytest dispar_orchestrate -q`, without
  `-k`) was **not** run: only the `-k bronze_catalog` selection was.
- The fix was **not** run against a seeded deployment: only the statements were checked
  against a real `ClickHouse` 26.8 (a `testcontainers` one).

## 5. Review (planner appends below)

### Review round 1 (planner, 2026-11-01, at `0799551b`)

- **T1 matches D2.** The guard reads `bronze_meta_sec.dataset_catalog` with `FINAL`, binds the
  slug through `_sql_string_literal`, raises `SlugConflictError` before any insert, and is tested
  without a network.
- **T2 matches D3–D5.**
  - Every secondary branch in `routes/catalog.rs` is filtered by `NOT_IN_PRIMARY`.
  - The list's catalog and sync reads take `FINAL`.
  - The clash warning is one `INTERSECT` query, run with the other three by `try_join!`, and
    it logs a count and at most five slugs.
  - The four new tests ran against the old code and failed there.
- **D6 list.** Four other readers do change what they show on a clash: `governance`
  classification, `overview`, `storage` and `lineage`. That becomes a follow-up backlog item,
  `DATA-42`.
- **SHOULD-FIX 1. The detail page's header read has no `FINAL`.** `bronze_asset_detail_body`
  reads `dataset_sync` LEFT JOIN `dataset_catalog` with `LIMIT 1` and no order. A re-run writes
  the slug's sync row again, so before `ClickHouse` merges, the page can show the older
  `total`. D4 named only the list's reads, which was the planner's miss. *Fix:* `FINAL` on both
  tables of both branches of that read, with a test of a twice-written slug's detail header
  showing the newer total.

**Open: SHOULD-FIX 1.**
