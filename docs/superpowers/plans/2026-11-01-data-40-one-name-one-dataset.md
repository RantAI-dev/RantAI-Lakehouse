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
