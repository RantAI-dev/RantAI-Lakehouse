# `DATA-11` Search that finds columns and tags — Implementation Plan

**Status:** ready to build on the feature page's defaults. Decisions 1–8 are
**not signed**; the product owner asked on 2026-10-09 for phase 2 to start
with this task. A decision signed differently later changes the task that
cites it.
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

## Review

*(planner)*
