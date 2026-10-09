# `DATA-11` Search that finds columns and tags — Implementation Plan

**Status:** built and reviewed; waits for the product owner's check in the
console. Decisions 1–8 on the feature page were signed by the product owner
on 2026-10-09, all as proposed.
Written 2026-10-09 by the planner (Claude Opus) for a developer agent, under
the role split in `AGENTS.md`.

**Spec:** `docs/core/specs/data-11.md`. Backlog `DATA-11`.
**Feature page:** `docs/core/features/catalog-search.md`. Decision numbers
below (D1–D8) are its rows.

**Why.** Search treats the typed text as one piece, never looks at column
names, forgives no typo and ranks nothing. The ⌘K box and the Data Explorer
search different fields with two separate matchers.

---

## 1. Slice and base

One pull request, Rust first, then the console.

| Content | Base | Branch | Worktree |
| --- | --- | --- | --- |
| One matcher, column search, ranking, the search copy, the tag filter, the ⌘K box, the Data Explorer | `origin/main` at `c338862` | `feat/data-11-catalog-search` | `/home/hv/lakehouse-data11` |

Commits stay local. The planner pushes and opens the pull request after its
review and after the product owner has tried the console.

## 2. What exists today

Anchors verified at `origin/main` `c338862`. `R` is
`rust/crates/lakehouse-api/src/routes`. Re-find by symbol if a line has
moved.

| # | Spec row | Code | Verdict |
| --- | --- | --- | --- |
| F1 | What is searched | Two matchers. `filter_assets_by_query` (`R/catalog.rs:426`) serves `GET /api/catalog?q=`: name, description, id, annotation description, annotation tags. `apply_search` (`R/catalog_query.rs:317`) serves `GET /api/catalog/query?search=` over `SEARCHABLE_FIELDS` (`:77`): id, name, namespace, description, owner. | Duplicated guard (rule 4), and the two disagree: tags match only in ⌘K, owner and namespace only in the Data Explorer. |
| F2 | What is searched | Column names reach neither matcher. `list_body` reads only a count per table (`R/catalog.rs:503`, and `system.columns` counted at `:589`). Column names and descriptions are read one table at a time by the detail route (`:1883`). | Columns are not searchable. |
| F3 | Matching | Both matchers lower-case the whole term and call `contains`. | One substring. No word order, no typo, no ranking. |
| F4 | Matching | `apply_sort` (`R/catalog_query.rs:356`) with no sort given still orders by `id`. | A ranked result would be re-ordered by id unless the sort is skipped. |
| F5 | Filters | `FILTERABLE_FIELDS` (`R/catalog_query.rs:32`) has type, layer, owner and sixteen more; no `tags`. `matches_filter` (`:254`) reads a field as one string. `SORTABLE_FIELDS` is an alias of it (`:56`). The page offers name, namespace, type, layer, tier, freshness, size, id (`src/features/catalog/data-explorer-columns.tsx`). | No tag filter. Owner is accepted by the API and not offered. |
| F6 | Where | `src/components/command-palette.tsx`: opens on ⌘K / Ctrl+K (`:81`), searches through `assetService.listAssets` (`:67`), mounted on every signed-in page (`src/components/app-shell/app-frame.tsx`). `Command.Dialog` (`:115`) keeps `cmdk`'s own filter on, and an asset item's `value` is `asset <id> <name>` (`:177`). | By reading, not run: `cmdk` hides an asset the server matched by description or tag, because the typed text is not in its `value`. |
| F7 | Speed | Both routes rebuild the catalog on every request: `list_body` (six ClickHouse queries), then `apply_sla_targets`, `enrich_bronze_assets`, `apply_annotations`, `apply_badges` (`R/catalog.rs:226-229`, `:361-364`). No measurement exists. | Not measured. |
| F8 | Permissions | `POLICY_TABLE` gives all three catalog reads `catalog:read` (`policy.rs:179-181`). `catalog_tenant_refusal` (`R/catalog.rs:164`) runs first in `list` and `query` and admits or refuses the whole catalog. No per-asset check exists anywhere. | Search must keep both checks. Per-table visibility is not this task (D5). |
| F9 | *Not in the spec* | `ListQuery` and the palette return the full asset list; the palette cuts to 8 in the browser (`src/lib/palette-search.ts`). | Kept (D7). |
| F10 | *Not in the spec* | Usage per table is computed on the detail route only, by parsing `query_history` SQL (`R/catalog_governance.rs`, `usage`, `summarize_usage`, `reads_any`; `lakehouse-store/src/queries.rs:143` `history_mentioning`). | "Ranked by use" has no list-wide source today. |

## 3. Decisions already made by the planner

- **One matcher.** A new pure module `R/catalog_search.rs` replaces both
  `filter_assets_by_query` and `apply_search`. Both routes call it. It takes
  no `ChClient`, like `catalog_query.rs`, so all of it is unit-tested.
- **Words, not a phrase.** The term is lower-cased (`to_lowercase`, as
  today) and split on whitespace. Every word must match the asset somewhere
  (any field, any order). At most 8 words are used; the rest are ignored.
- **A word matches a field** when the field's lower-cased text contains it
  (exact), or, for a word of 4 or more characters, when one token of the
  field is one edit away (approximate, D3). Tokens are the field split on
  every character that is not a letter or digit. One edit is one
  substitution, insertion, deletion, or swap of two adjacent characters,
  counted in `char`s. Write this check by hand (it is a single pass); add no
  dependency.
- **Fields and weights.** Per word, the best-scoring field counts; an
  asset's score is the sum over its words.

  | Field | Source on the asset | Exact | Approximate |
  | --- | --- | --- | --- |
  | name, when the word equals the whole name | `name` | 120 | n/a |
  | name | `name` | 100 | 50 |
  | id, namespace | `id`, `namespace` | 80 | 40 |
  | tag | `tags[]` | 70 | 35 |
  | column name | the search copy | 60 | 30 |
  | owner | `owner` | 40 | 20 |
  | description | `description` | 30 | 15 |
  | column description | the search copy | 20 | 10 |

  Order: score descending, then use descending (D4), then `id` ascending.
- **Why it matched.** A returned asset carries
  `matchedOn: { field, value, approximate }` for the field that scored
  highest on its first word, unless that field is the name. `field` is one
  of `id`, `namespace`, `tag`, `column`, `owner`, `description`,
  `columnDescription`. `value` is the tag or column name; for the two
  descriptions it is the empty string. Nothing is added when no term was
  sent.
- **The search copy (D2).** A search does not rebuild the catalog. It reads
  `CatalogSearchSnapshot`: the assembled assets (after the same four
  `apply_*` steps), the columns of every asset, and the use counts. The copy
  is kept in `AppState`, rebuilt when it is older than
  `SEARCH_SNAPSHOT_TTL` (30 seconds), by one request at a time (the others
  wait for it, they do not each rebuild). A successful `put_annotation`
  drops it, so a console edit shows at once. A rebuild that fails answers
  what the routes answer today when the catalog is unreachable (`503`, the
  fixed shape); a stale copy is never served past its age.
- **An empty term changes nothing.** With no `q` / `search`, both routes run
  exactly the code they run today, live.
- **The copy is the same for every caller.** The four `apply_*` steps take
  no `Principal`. If the developer finds any per-caller value in the
  assembled asset rows, stop and report; do not cache it.
- **Checks stay per request.** `catalog_tenant_refusal` runs before the copy
  is read, on both routes, as today. No route is added, so `POLICY_TABLE`
  and `tests/route_auth.rs` do not change.
- **Columns.** Two queries, run only when the copy is rebuilt: Bronze
  `slug, key_asli, deskripsi` from both `dataset_column` registries, and
  `database, table, name, comment` from `system.columns` for `silver` and
  `serving`. Keys are the asset ids the row builders give
  (`bronze_catalog_row`: the slug; `silver_catalog_row`: `silver.<name>`;
  `gold_catalog_row`: `serving.<name>`). Each query carries
  `LIMIT SEARCH_COLUMN_ROWS_MAX` (500,000). When a query returns exactly the
  limit, log a warning and set `columnSearch: "partial"` on the search
  response, so a caller can see column search was cut (principle 2).
- **Use (D4).** When the copy is rebuilt: the newest 5,000 `query_history`
  rows of the last 7 days, each parsed once with
  `sql_rewrite::referenced_tables` the way `reads_any` does (ClickHouse
  dialect, then generic), counted per asset id. No Postgres, or a failed
  read: every count is 0 and a warning is logged; ranking is by relevance
  alone. Use the same 7 days constant the asset page uses (`USAGE_DAYS`);
  do not add a second one.
- **Tag filter.** `tags` joins `FILTERABLE_FIELDS`. `SORTABLE_FIELDS`
  becomes its own list without `tags` (a list has no order to sort by);
  `GROUPABLE_FIELDS` is unchanged. For an array field, `eq`, `iLike` and
  `inArray` match when any entry matches; `ne`, `notILike` and `notInArray`
  match when no entry does; `isEmpty` and `isNotEmpty` look at the array's
  length. The ordering operators match nothing on an array.
- **The ⌘K box keeps `cmdk`'s filter for pages and actions.** Asset items
  and their group are marked `forceMount` (`cmdk` 1.1.1 has it on `Item` and
  `Group`), because the server has already decided they match.

## 4. Tasks

One task per commit, in this order. Cite `DATA-11` and the finding (`F1`…)
at each fix site and in the commit body (rule 13).

### Rust

**R1. The matcher.** `R/catalog_search.rs`, pure: tokenising, the one-edit
check, the weight table of section 3 as named constants, and
`search(assets, columns, usage, term) -> Vec<Value>` returning ranked assets
with `matchedOn`. Module doc says why it is pure and why there is one
matcher. *Check:* unit tests, names as full sentences:
- every old case of `filter_assets_by_query_*` (`R/catalog.rs:2758-2801`)
  and `search_*` (`R/catalog_query.rs`, the four search tests) is carried
  over; where the answer changes, the test says why in a comment (rule 2).
  The one intended change: several words no longer need to be adjacent.
- two words in the opposite order of the description match;
- a word that matches nothing excludes the asset even when another matches;
- `revnue` finds `revenue` (deletion), `revneue` finds it (swap), `rvn`
  does not find `rev` (under 4 characters), `ravanue` does not (two edits);
- a column name finds its table and `matchedOn` is
  `{field: "column", value: <the column>, approximate: false}`;
- a tag, an owner, a namespace and a column description each find their
  table;
- name beats tag beats column beats description for the same word; an exact
  hit beats an approximate one in the same field; equal scores order by use,
  then id;
- an empty or all-whitespace term returns every asset in input order with no
  `matchedOn`; a ninth word is ignored.

**R2. Columns and use for the copy.** In `R/catalog.rs`, a function that
reads the two column queries of section 3 into `HashMap<String, Vec<(name,
description)>>`. In `lakehouse-store/src/queries.rs`, `history_recent(pool,
days, limit)` returning the SQL text of the newest rows (bound values, `#
Errors`). In `R/catalog_governance.rs`, next to `reads_any`, a function
that turns those rows into `HashMap<String, u32>` by asset id; reuse the
parsing `reads_any` does, do not write a second parser path. *Check:* unit
tests on the pure parts (row → key mapping for the three id shapes; a query
reading two tables counts for both; unparseable SQL counts for none); a
`sqlx::test` for `history_recent` (newest first, respects days and limit).

**R3. The search copy.** `CatalogSearchSnapshot` and its cache in
`state.rs` (follow how `AppState` holds its other shared values;
`tokio::sync` primitives, no new dependency). Build function in
`R/catalog.rs` that runs `list_body` and the four `apply_*` steps exactly as
`list` does today, then R2. Constants `SEARCH_SNAPSHOT_TTL`,
`SEARCH_COLUMN_ROWS_MAX`, each with a comment naming D2 or the cap's reason.
`put_annotation` drops the copy after a successful write. *Check:* unit
tests with an injected clock or an explicit age: a fresh copy is reused, an
old one is rebuilt, a drop forces a rebuild, a failed rebuild returns the
error and leaves nothing cached.

**R4. Both routes use the matcher.** `list`: with a non-empty `q`, read the
copy and return `search(...)` as `assets` (namespaces as today, from the
copy). `query`: with a non-empty `search`, read the copy, `search(...)`,
then `apply_filters`; call `apply_sort` only when the request names a sort
(F4), then group and paginate as today. Add `columnSearch: "partial"` when
R3 says so. Delete `filter_assets_by_query`, `apply_search` and
`SEARCHABLE_FIELDS`; correct every doc comment that describes them
(`R/catalog.rs` `list` doc, `catalog_query.rs` module doc). The parity
corpus (`rust/tests/parity/corpus/catalog-list.json`) has no `q`; if a
parity request uses one, report it. *Check:* the tenant tests in `mod
tenant_scoping` still pass unchanged; new tests: a refused caller gets the
`supported: false` shape and the copy is not read; `search` with no `sort`
keeps rank order; `search` with a `sort` obeys the sort.

**R5. Tag filter.** Section 3's rules in `catalog_query.rs`. *Check:* unit
tests for each operator on an array field, `tags` rejected as a sort and as
a group field with a 400, and an asset without `tags` behaving as an empty
list.

Rust check for R1–R5, once the batch is in: section 5.

### Console

**T1. Contract.** `src/services/contracts/assets.ts`: `Asset` gains
`matchedOn?: { field: string; value: string; approximate: boolean }` with a
comment listing the field names; `Pagination<Asset>` consumers and
`listAssets` carry it through unchanged. No mock gains a field (remove a
consumer rather than optional-ing one). *Check:* `bun run typecheck`.

**T2. The ⌘K box.** `src/components/command-palette.tsx`: asset items and
their group `forceMount` (F6); a second line under the name from
`matchedOn` ("column revenue_amount", "tag finance", "description", with
"approximate match" when set), the wording in one small function in
`src/lib/palette-search.ts`; a last row "See all results" that opens
`/data?search=<term>` (the Data Explorer reads `search` from the URL,
`src/components/data-table/data-table-search.tsx:39`; confirm the key the
page uses). `src/components/app-shell/app-navbar.tsx:93` placeholder:
"Search pages, tables, columns… (⌘K)". A failed search shows "Catalog search
is unavailable", not an empty list (principle 2; today it shows nothing).
*Check:* new `src/components/command-palette.test.tsx`: an asset returned
for a term that is in neither its id nor its name is shown; its reason line
is shown; at most 8 assets; "See all results" carries the term; a rejected
request shows the unavailable line. Tests for the wording function.

**T3. Data Explorer.** `src/features/catalog/data-explorer-page.tsx:303`
placeholder: "Search by name, column, tag, description or owner…". Under the
asset's name, the same reason line when `matchedOn` is present.
`data-explorer-columns.tsx`: an Owner column and a Tags column (tags as the
pills `asset-about.tsx` uses; reuse, do not copy), each filterable with the
text variant; the Tags filter sends `iLike`. With a search term and no sort
chosen, the page must not send a default `sort` (F4); confirm what it sends
today and report if it always sends one. *Check:* tests beside the existing
ones in `src/features/catalog/`: the reason line renders; the Tags filter
produces the `filters` parameter the API accepts.

**T4. Measurement (D1).** `ops/bench/catalog_search_bench.py`, narrative
module docstring, `requests` with `timeout=` and `raise_for_status()`. Two
commands. `seed`: writes N synthetic datasets (default 10,000, 20 columns
each) into the two Bronze registries of the ClickHouse named by the
environment; it refuses to run unless `BENCH_THROWAWAY_CATALOG=true` is set,
and its docstring says never to point it at a shared stack. `run`: signs in
with credentials from the environment, sends 50 searches to
`/api/catalog?q=` and 50 to `/api/catalog/query?search=` (names, column
names, typos, two words), and prints the first-search time (copy built) and
p50 / p95 of the rest. Run it on a throwaway compose project, never the
shared dev stack. The planner writes `docs/plans/DATA-11-RESULT.md` from the
raw output. If no throwaway stack can be run on this machine, the handoff
says *not verified* and the Speed row stays open; no number is written.

**T5. Docs (planner, same branch).** `CHANGELOG.md`; `docs/core/BACKLOG.md`
(the per-table visibility item of D5, and the lineage-gate and Trino
time-travel findings recorded in the `DATA-13` and `DATA-16` specs);
`docs/core/PRODUCT.md` after merge.

## 5. Build limits on this machine

- Shared target dir and two jobs:
  `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-catalog-target CARGO_BUILD_JOBS=2`.
  Check `df -h /` before each cargo command.
- Other worktrees share that target dir. Before a clippy run whose result
  is quoted, run `touch rust/crates/*/src/lib.rs
  rust/crates/lakehouse-store/src/*.rs` in this worktree; a workspace clippy
  that finishes in seconds is *not verified*.
- Do not build test binaries here (`cargo test` has crashed this VM). Run
  `cargo fmt --check` and `cargo clippy -p lakehouse-api -p lakehouse-store
  --all-targets -- -D warnings`, which type-check the tests. Every Rust test
  is written *not verified* in the handoff; its first run is CI's.
- TypeScript: `bun run typecheck && bun run lint && bun run test` in full.
  Python: the two `ops/lint` lines.
- No `sqlx` migration is added by this plan.

## 6. Out of scope

- The certification filter and tags with keys and values (`DATA-12`).
- Per-table visibility (D5).
- The assistant's `list_datasets` tool and plain-language search (`AI-16`).
- Showing tags or marks in the chart builder's table picker or Query Studio
  (`DATA-12`).
- Making the live catalog list (empty search) faster.
- A search index outside the API process, and any new dependency.

## Handoff

### Rust slice (R1-R5), developer, 2026-10-09

Commits on `feat/data-11-catalog-search`, in order (none pushed):

- `15d6b54` feat(api): one catalog matcher that ranks words, tags, columns and typos (R1)
- `e9ed1ff` feat(api): read column names and query use for the catalog search copy (R2)
- `7f57a35` feat(api): keep a 30-second search copy of the assembled catalog (R3)
- `a344d30` feat(api): both catalog routes search through the one ranked matcher (R4)
- `8155a28` feat(api): filter the catalog by tag (R5)
- `dfae2ad` refactor(api): one catalog assembly for list, query and the search copy (clippy follow-up of R2-R4)

Commands run, from `/home/hv/lakehouse-data11/rust`, with
`CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-catalog-target CARGO_BUILD_JOBS=2`,
`df -h /` checked before each (58G free):

- `cargo check -p lakehouse-api` (and `-p lakehouse-store`) between edits: no errors.
- After the batch: `touch rust/crates/*/src/lib.rs rust/crates/lakehouse-store/src/*.rs`,
  then `cargo fmt --check`: exit 0.
- Then `cargo clippy -p lakehouse-api -p lakehouse-store --all-targets -- -D warnings`:
  first run failed with two findings (`too_many_lines` on `query`,
  `match_same_arms` in `use_keys`), fixed in `dfae2ad`; the rerun
  finished clean (`Finished dev profile`, 2m 08s, `lakehouse-api` and
  `lakehouse-store` re-checked). `--all-targets` type-checks every new test.

*Not verified:* every Rust test. None was run (`cargo test` is forbidden on
this machine); the first run is CI's. That covers the new tests in
`catalog_search.rs`, `catalog_search_cache.rs`, `catalog_governance.rs`
(`use_keys`, `use_counts`), `catalog.rs` (`collect_columns`,
`order_results`, the refused-search tenant test), `catalog_query.rs` (tags),
and the `sqlx::test` `history_recent_is_newest_first_and_respects_days_and_limit`
in `lakehouse-store/tests/queries.rs`. Also not verified: the full workspace
clippy and test run of the verification block, any request against a live
`ClickHouse` (the two column queries, the `UNION ALL` subquery with `LIMIT`,
and `system.columns` were never run), and the speed budget (T4).

Not covered by a test: that `put_annotation` drops the copy (it needs a
write through Postgres and a primed cache; the drop itself is covered at the
cache); a route-level search through a mock `ClickHouse` returning rows.

Deviations and choices the plan did not spell out:

- The cache lives in its own module `catalog_search_cache.rs` (like
  `bronze_stats_cache`), not inside `state.rs`: `routes::catalog_search` is
  private to `routes`, so `AppState` could not name its types. `AppState`
  holds `catalog_search_cache`.
- `SEARCH_COLUMN_ROWS_MAX` is defined in R2 (the query needs it); the plan
  listed it under R3. The Bronze column query is a `UNION ALL` wrapped in a
  subquery so one `LIMIT` caps the union.
- `USAGE_DAYS` became `pub(crate)` and the 5,000-row read is
  `USE_RANKING_ROWS` in `catalog_governance.rs`.
- `history_recent` returns `Vec<String>` (SQL text only, all statuses).
- A failed rebuild also drops an expired copy, and `invalidate` waits for a
  rebuild in flight, so `put_annotation` can wait up to one rebuild.
- `apply_annotations` now returns nothing (it handed the rows back only for
  the deleted matcher). `assemble_catalog` is the one assembly for `list`,
  `query` and the copy.
- The 503 body of both routes still carries `js_error(err)`, as before; the
  plan says to answer what the routes answer today. It is the known
  principle-4 gap, not a new one.
- No per-caller value was found in the assembled rows: the four `apply_*`
  steps take only `AppState`.
- The parity corpus has no `q` or `search` request on the catalog routes
  (the `"search"` in `ai-chat-ok.json` is the assistant's `list_datasets`
  argument).

### Review fixes, console (T1-T3) and benchmark script (T4), developer, 2026-10-09

Commits on `feat/data-11-catalog-search` after the first entry, in order
(none pushed):

- `71184f6` fix(api): forgive a typo in a search word that has punctuation (review SHOULD-FIX 1)
- `fe4d136` fix(api): count a dataset's use under the Silver table its page reads (review SHOULD-FIX 2)
- `46f5f6e` test(api): an annotation edit drops the search copy, a refused one does not (review SHOULD-FIX 3)
- T1 `4f02196` feat(console): carry why a search matched on the asset contract
- T2 `5d0c385` feat(console): the Cmd+K box shows why a table matched and links to all results
- T3 `7df1f88` feat(console): Data Explorer searches columns and tags, filters by owner and tag
- T4 `cee90a3` feat(ops): a benchmark for catalog search on 10,000 tables

Commands run, with counts:

- Rust, from `/home/hv/lakehouse-data11/rust`, `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-catalog-target
  CARGO_BUILD_JOBS=2`, `df -h /` checked before each (54-55G free):
  `cargo check -p lakehouse-api` and `cargo check -p lakehouse-api --tests`
  between edits: no errors. Then `touch rust/crates/*/src/lib.rs
  rust/crates/lakehouse-store/src/*.rs`; `cargo fmt --check` failed on two
  closures of my SHOULD-FIX 3 helpers only; `cargo fmt` fixed them (7 lines
  added, 15 removed, all in my hunks), rerun `cargo fmt --check`: exit 0.
  `cargo clippy -p lakehouse-api -p lakehouse-store --all-targets -- -D
  warnings`: finished clean (1m 23s, every crate re-checked). That clippy run
  came before the `cargo fmt` rewrite of those two closures; no clippy was
  re-run after it.
- TypeScript, from the worktree root, after `bun install --frozen-lockfile`
  (772 packages, lockfile unchanged): `bun run typecheck` exit 0; `bun run
  lint`: 0 errors, 6 warnings, none in a file this slice touches (alerts-page,
  open-format-card.test, use-data-table, dashboard-specs and two more);
  `bun run test`: 928 pass, 1 skip, 0 fail, 929 tests across 102 files.
  New: 5 in `command-palette.test.tsx`, 4 in `palette-search.test.ts`, 5 in
  `data-explorer-columns.test.tsx`.
- Python: `python3 -m py_compile ops/bench/catalog_search_bench.py` ok;
  `python3 ops/lint/check_intra_package_imports.py` and
  `python3 ops/lint/check_bare_iceberg_count.py` both OK.

Choices the plan did not spell out:

- SHOULD-FIX 1: `best_hit` lost its `word_chars` argument and `search` no
  longer builds `word_chars`; the new `approximately_in` applies the 4-character
  rule per word token, so a word such as `ab_cd` is not approximated as a whole.
  Every part of a punctuated word that is contained in the field counts as
  matching, so `id_customer` finds a field `customer_id` (flagged approximate).
- SHOULD-FIX 2: `use_keys` and `use_by_asset` take the `silver_fallback`
  value (`iceberg_query_db.is_none()`); a listed Silver asset keeps its own key.
- SHOULD-FIX 3: the two tests prime and read the cache through
  `get_or_build` with a flag in the build closure.
- T2: the box shows "No results." only when the server returned no asset
  (the force-mounted rows would otherwise sit above cmdk's own empty line);
  "See all results" appears only when at least one asset is listed; a
  `supported: false` refusal still shows "No results.", not the unavailable
  line (the API answers 200 with an empty list).
- T3: the plan did not say what happens to the new columns on a narrow window.
  Tags hides below 1400 px and Owner below 1280 px, ahead of Size, through the
  page's existing `hiddenByWidth`; while hidden, their filters are not
  offered, as for Type and Layer today. Tags has `enableSorting: false`
  because the server refuses a `tags` sort.
- T3 sort check: the Data Explorer sends no default `sort`. `useDataTable`
  is given no initial `sorting` and `toQueryParams` leaves an empty sort out.
  A sort the user chose (a header click, which puts it in the
  URL) is sent and is obeyed by the API; saved table memory was not read.
- T4: the benchmark's registry columns come from the `SELECT`s in `catalog.rs`
  (`dataset_catalog`, `dataset_sync`, `dataset_column`); the real Iceberg
  table definitions are not in this repo. `run` waits `--cold-wait` (35 s) so
  the first search builds the copy. The query terms assume 20 columns per
  table.

*Not verified:*

- Every Rust test, new and old (no `cargo test` on this machine): the
  `custmer_id` / `silver.ordrs` / `custmer_xx` test, the `use_keys`
  both-settings test and the two `put_annotation` copy tests. First run is CI's.
  They are type-checked by `clippy --all-targets` and `cargo check --tests`.
- The workspace-wide Rust verification block, and the Python test suite
  `(cd dagster && python -m pytest ...)` (no Python change in `dagster/`).
- `ops/bench/catalog_search_bench.py` was never run against a stack: `seed`
  was not run, no compose project was started, and `run` was only
  started without settings to see it refuse. No number exists; the Speed row
  (T4, 500 ms) stays open. Whether ClickHouse accepts the INSERT into the
  Iceberg registries is unknown.
- No browser check of the Cmd+K box or the Data Explorer; the user interface
  was exercised only through happy-dom tests with a stubbed `fetch`.
- The `forceMount` on `Command.Item` alone: removing only the item-level
  prop left the tests passing, removing the group-level one too made two fail,
  so the group-level `forceMount` is what the tests pin.

### Review fixes SHOULD-FIX 4 to 7, developer, 2026-10-09

Commits on `feat/data-11-catalog-search`, one per finding (none pushed):

- `f8c8c90` fix(console): the Cmd+K box tells a refusal and a catalog reason from an outage (SHOULD-FIX 4)
- `796bd5e` fix(api): a search word whose parts all match exactly is not marked approximate (SHOULD-FIX 5)
- `f0e3fc2` fix(console): "See all results" stays after the tables in the Cmd+K box (SHOULD-FIX 6)
- `0dd2b2e` fix(console): the Data Explorer fits beside the open sidebar at 1280 to 1536 px (SHOULD-FIX 7)

Commands run:

- Rust, from `rust/` with `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-catalog-target
  CARGO_BUILD_JOBS=2` (`df -h /` first: 49G free), after `touch
  rust/crates/*/src/lib.rs rust/crates/lakehouse-store/src/*.rs`: the first
  `cargo fmt --check` failed on one `assert!` in my new test; I formatted that
  hunk by hand. Then `cargo fmt --check` exit 0 and `cargo clippy -p
  lakehouse-api -p lakehouse-store --all-targets -- -D warnings` finished clean
  (1m 17s), both after the last Rust edit.
- TypeScript, worktree root, on the final code commit: `bun run typecheck`
  exit 0; `bun run lint` 0 errors, 6 warnings (same as before, none in a
  touched file); `bun run test` 932 pass, 1 skip, 0 fail, 933 tests in 102
  files. New tests: 2 in `command-palette.test.tsx` for 4, 1 for 6 (red before
  the fix: See all results was row 0), 1 in `data-explorer-columns.test.tsx`
  for 7.

Notes:

- 4: `loadCatalog` now throws `errorFor(res.status, ...)` (the helper the other
  methods of the file use), so a `403` is `permission_denied`. A `supported:
  false` answer to a search (the contract has no field for it on
  `listAssets`) is thrown as `ServiceError("unavailable", reason, 200)`; the
  palette shows the message only when the status is `200` (no failed request
  has it), so upstream error text still never reaches the box. Only a search
  throws; `listNamespaces` and the unsearched lists keep reading the empty
  answer. This status-200 marker is a choice the review did not spell out.
- 5: `approximately_in` returns `Option<bool>`; the weight is the approximate
  one either way. The test orders `silver.exact` (column `id_customer`) before
  `silver.swapped` (`customer_id`, `approximate: false`); `custmer_id` stays
  `approximate: true` (also pinned by the older SHOULD-FIX 1 test).
- 6: `cmdk` 1.1.1 scores each row's `value` against the typed text and
  reorders; the row's value is now one arrow character (no letter or digit, so
  it scores zero for any word) and stays last on equal scores.
- 7: the page measures the window (`useWindowWidth`), not the table's
  container; the mechanism is unchanged. Tags now shows from 1760 px (was
  1400) and Owner from 1500 px (was 1280). The figures are estimates from the
  column caps (Owner 10rem, Tags 14rem) and the comment's existing
  window-minus-296px rule, with ~55 px to spare.

*Not verified:*

- Every Rust test (no `cargo test` here); the new test is type-checked by
  clippy only. First run is CI's.
- SHOULD-FIX 7 in a browser: no width was opened at 1280, 1440 or 1536 px. The
  thresholds are not measured, and a table with long names, many tags or
  other data could still be wider than estimated.
- SHOULD-FIX 4 and 6 against the real API and a real browser: tested only with
  `happy-dom` and a stubbed `fetch`.
- The workspace-wide Rust block and the Python lines.

## Review

### Rust slice (R1–R5), reviewed 2026-10-09 at `4b61231`

No `BLOCKER`. Three `SHOULD-FIX`, the first a mistake in this plan.

**SHOULD-FIX 1. A typed word with punctuation is never matched
approximately (plan error, section 3).** The plan split the term on
whitespace only, and compared the whole word with the field's tokens, which
never contain punctuation. `custmer_id` therefore cannot find `customer_id`:
the word has an underscore, no token has one. *Fix, in `best_hit`
(`R/catalog_search.rs`):* when the word is not contained in the field, split
the word the way fields are split (`tokens`). The field matches
approximately when every word token is either contained in the field or, at
4 characters or more, one edit from one of the field's tokens. A word with
no letter or digit matches nothing. Tests: `custmer_id` finds a column
`customer_id` (approximate); `silver.ordrs` finds `silver.orders`;
`custmer_xx` does not find `customer_id`.

**SHOULD-FIX 2. Use is counted under a different table name than the asset
page counts it.** `use_keys` (`R/catalog_governance.rs`) gives a Bronze
dataset the one key `bronze.<table>`. The asset page counts a dataset's
queries under `bronze.<table>` and under the key of the table its page
reads (`source.policy_key`, `R/catalog.rs:2165-2170`), which is
`silver.<table>` on a deployment that cannot read Bronze. There, search
ranks every Bronze dataset as unused, and D4 ("the count the asset page
already shows") does not hold. *Fix:* when `Config::iceberg_query_db` is
unset, `use_keys` also maps `silver.<table>` to the dataset, unless a listed
asset already has that id. Test for both settings.

**SHOULD-FIX 3. Nothing tests that an annotation edit drops the search
copy.** The drop is one line in `put_annotation`, tested only at the cache.
*Fix:* a `sqlx::test` beside the existing `put_annotation_*` tests: put a
copy in the cache, call `put_annotation` as a member, assert the next
`get_or_build` runs its build; and one asserting a refused write leaves the
copy in place.

**Checked and correct.**
- One matcher: `filter_assets_by_query`, `apply_search` and
  `SEARCHABLE_FIELDS` are gone; every old search test is carried into
  `catalog_search.rs`, none weakened.
- The weight table, the 4-character rule, the 8-word cap and the ordering
  match section 3. `within_one_edit` is correct for substitution, insertion,
  deletion and adjacent swap, on `char`s.
- Both routes run `catalog_tenant_refusal` before the copy is read. No route
  was added; `POLICY_TABLE` is untouched. No dependency was added.
- The copy holds nothing per caller: `assemble_catalog` takes `AppState`
  only. Single-flight is the `tokio` mutex held across the rebuild. A failed
  rebuild caches nothing.
- An empty term runs the live assembly on both routes.
- `tags` filters with the any/none rules, is refused as a sort or group
  field, and a test pins the sortable list to the filterable one.
- SQL: `history_recent` binds both values; the two column queries
  interpolate only the constant limit.
- The developer's own changes to the plan are accepted: the cache is its own
  module `catalog_search_cache.rs` (a private route module cannot be named
  from `state.rs`); `assemble_catalog` removes the duplicated assembly.

**Carried, not caused.** The `503` body of both routes still passes
`js_error(err)`, upstream text, as before this branch (principle 4). The
search path reuses that one response; no new handler leaks. Backlog `SEC-6`.

**Not a finding, for T4.** `search` lower-cases every field of every asset
on each call. If the measurement shows the matcher is where the time goes,
the lower-cased text moves into the copy; not before it is measured.

**Verification, by the planner, at `4b61231`.** From `rust/`, with the
shared target dir and two jobs, after `touch` of the crate sources:
`cargo fmt --check` exit 0; `cargo clippy -p lakehouse-api -p lakehouse-store
--all-targets -- -D warnings` finished clean in 1m14s.

**Not verified.** No Rust test has been run by anyone (`cargo test` is not
run on this machine); their first run is CI's. The two column queries have
never run against a ClickHouse. The workspace-wide clippy and the 500 ms
target (T4) are open.

### Review fixes, console and benchmark, reviewed 2026-10-09 at `f1c4931`

No `BLOCKER`. `SHOULD-FIX` 1 to 3 are fixed as written (`71184f6`,
`fe4d136`, `46f5f6e`). Two new `SHOULD-FIX`, both small, left open until the
product owner has tried the console, so that they are fixed with whatever
that finds.

**SHOULD-FIX 4. A user without `catalog:read` reads "Catalog search is
unavailable" in the ⌘K box.** The `403` makes `loadCatalog`
(`src/services/clients/assets.ts`) throw the same error as an outage. *Fix:*
the palette tells a refusal (`403`) from a failure and says "You do not have
access to the catalog" for the first. Likewise, a `supported: false` answer
shows the API's `reason`, not "No results.".

**SHOULD-FIX 5. A word whose parts all match exactly is marked
approximate.** `id_customer` finds `customer_id` through `approximately_in`
and is shown as "approximate match" though no letter is wrong. *Fix:*
`approximately_in` reports whether any part needed an edit; with none, the
hit keeps the approximate weight (the order was wrong) and `approximate` is
false.

**Checked and correct.**
- T1: `matchedOn` is on the contract only; no mock gained a field.
- T2: the asset group and items are force-mounted; the reason line, "See
  all results" (`/data?search=`), the placeholder and the failure line are
  as planned; the wording is one function shared with the Data Explorer.
- T3: Owner and Tags columns reuse `Pill`; Tags is not sortable; the page
  sends no default sort, so a search keeps its rank order.
- T4: the script refuses `seed` without `BENCH_THROWAWAY_CATALOG=true` and
  takes every address and credential from the environment.

**Verification, by the planner, at `f1c4931`.** After `touch` of the crate
sources: `cargo fmt --check` exit 0; `cargo clippy -p lakehouse-api -p
lakehouse-store --all-targets -- -D warnings` clean in 1m17s. `bun run
typecheck` exit 0; `bun run lint` 0 errors, 6 warnings, none in a changed
file; `bun run test` 928 pass, 1 skip, 0 fail, 929 tests in 102 files.
`check_intra_package_imports.py` and `check_bare_iceberg_count.py` OK.

**Not verified.**
- No Rust test has been run by anyone; CI runs them first.
- The workspace-wide clippy and `cargo test --workspace`.
- Nothing has been tried in a browser or against a running API and
  ClickHouse: the two column queries, the tag filter and the ⌘K box are
  proven only by unit tests with stubs.
- The benchmark has not run. The 500 ms target (D1) is open and
  `docs/plans/DATA-11-RESULT.md` does not exist.

**Before the pull request.** The product owner tries the console (feature
page, acceptance rows 1 to 12) and signs or changes decisions 1 to 8; `main`
is merged into the branch; then the planner pushes and opens the pull
request, and CI gives the Rust tests their first run.

### First run on a live stack, by the planner, 2026-10-09 at `3f3e663`

A separate API (own port, own empty Postgres database, the dev stack's
ClickHouse) and console were started from this branch; nothing shared was
rebuilt. Catalog of 29 assets.

**Now verified.** The two column queries run. `GET /api/catalog?q=id`
answered in 60 ms on the first search (copy built) and 12 ms on the second.
`custmer_id` finds the tables with a `customer_id` column, marked
approximate; two words in either order match; words matching nothing return
an empty list; a tag saved with `PUT …/annotation` is found by the next
search; the `tags` filter with `iLike` returns the tagged table; `tags` as a
sort is a 400. In a headless browser: the Data Explorer shows the reason
line and the Owner and Tags columns, and the ⌘K box lists a table found
only by its tag. These are 29 assets, not 10,000: D1 is still not measured.

Two more `SHOULD-FIX` from that run. With 4 and 5 above they are fixed now,
before the product owner's check.

**SHOULD-FIX 6. "See all results" is listed above the tables in the ⌘K
box.** `cmdk` orders items by its own score and the row's `value` holds the
typed text. *Fix:* the row is the last entry of the "Catalog assets" group
whatever is typed (for example its own group after the assets, or a `value`
that cannot outrank them); test the order.

**SHOULD-FIX 7. The Data Explorer scrolls sideways at 1500 px.** With the
two new columns the table is wider than the page beside the open sidebar;
Size is cut off. The thresholds (`tags` under 1400, `owner` under 1280) were
chosen without the sidebar. *Fix:* choose them so the table fits without a
sideways scroll at 1280, 1440 and 1536 px with the sidebar open, hiding Tags
first, then Owner; cap the Owner cell's width with a truncation and the full
value as its title, as the Name cell does.

### `SHOULD-FIX` 4 to 7, reviewed 2026-10-09 at `89afb60`

No `BLOCKER`, no open `SHOULD-FIX`. All four are fixed as written
(`f8c8c90`, `796bd5e`, `f0e3fc2`, `0dd2b2e`).

**Checked and correct.**
- 4: a `403` reads "You do not have access to the catalog", a `supported:
  false` answer shows the API's reason, anything else "Catalog search is
  unavailable"; no upstream text reaches the box. The reason travels as a
  `ServiceError` with status 200, a convention of this client, said in a
  comment there.
- 5: live, `id_customer` finds `customer_id` with `approximate: false`,
  `custmer_id` with `true`.
- 6: live, "See all results" is the last row for the term `customer`.
- 7: live in a headless browser with the sidebar open, the table has no
  sideways scroll at 1280, 1440, 1536 and 1920 px. Owner shows from 1500 px
  and Tags from 1760 px, so on a common laptop screen both columns are
  hidden and their filters with them (as Type and Layer already behave).
  The page measures the window, not the table; unchanged.

**Verification, by the planner, at `89afb60`.** After `touch` of the crate
sources: `cargo fmt --check` exit 0; `cargo clippy -p lakehouse-api -p
lakehouse-store --all-targets -- -D warnings` clean in 2m14s. `bun run
typecheck` exit 0; `bun run lint` 0 errors, 6 warnings; `bun run test` 932
pass, 1 skip, 0 fail. The API was rebuilt from this commit and the separate
instance restarted for the live checks above. The full block runs again
after `main` is merged in, before the pull request.

**Not verified.** No Rust test has run (CI first). D1, the 500 ms target on
10,000 tables, is not measured. The product owner has not yet tried the
console.

### From the product owner's check, 2026-10-09

**SHOULD-FIX 8. A result found through its second word shows no reason
(plan error, section 3).** The product owner typed `northwind customer` in
the ⌘K box. `Northwind Orders` is listed, correctly: `northwind` is in its
name and `customer` is in its column `customer_id`. But it shows no reason
line, because the plan took `matchedOn` from the first word only, and the
first word matched the name. The reader cannot tell why Orders is there.
*Fix, in `search` (`R/catalog_search.rs`):* `matchedOn` comes from the first
word, in typed order, whose best field is not the name. When every word's
best field is the name, there is no `matchedOn`, as today. Update the module
doc, the doc comment of `search`, the contract comment in
`src/services/contracts/assets.ts` and the feature page wording if it says
"first word". Tests: `northwind customer` on an asset named `Northwind
Orders` with a column `customer_id` gives `{field: "column", value:
"customer_id", approximate: false}`; on `Northwind Customers` gives no
`matchedOn`; the existing test `the_reason_comes_from_the_first_word` is
changed to the new rule, with the reason in a comment (rule 2).

