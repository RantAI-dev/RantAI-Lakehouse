# DATA-42: the primary registry wins in the four other readers

Backlog `DATA-42` (Task, Data, P3), the follow-up of `DATA-40` (#320). Base: `main` at
`9c066426`. Planner: Claude Opus. Developer: another agent (`AGENTS.md`).

## 0. What is wrong, with evidence

`DATA-40` decided that one slug names one dataset. When a slug is in both Bronze registries,
`lake.bronze_meta.*` (primary) and `lake.bronze_meta_sec.*` (secondary), the primary wins. It
applied the rule in `routes/catalog.rs` only, through the constant `NOT_IN_PRIMARY`
(`catalog.rs:566`):

```rust
const NOT_IN_PRIMARY: &str = "NOT IN (SELECT slug FROM lake.`bronze_meta.dataset_catalog` FINAL)";
```

Its handoff's D6 table listed the other readers that `UNION ALL` the two registries
(`docs/superpowers/plans/2026-11-01-data-40-one-name-one-dataset.md`, handoff, "D6"). Four of
them show something different on a clash, and a fifth has an unordered `LIMIT 1`. At
`9c066426` they are unchanged:

| Reader | `file:line` at `9c066426` | What a clash does today |
| --- | --- | --- |
| `governance::classification` | `routes/governance.rs:433-458` (SQL `:436-437`) | Two rows with the same id `c-<slug>`, one per registry |
| `overview` assets and stale | `routes/overview.rs:62-71` (SQL `:65-69`) | The slug is counted twice in `assets`, and its sync row twice in `stale` |
| `overview` warm assets | `routes/overview.rs:82-88` (SQL `:85`) | The dataset is counted twice |
| `storage` warm rows and assets | `routes/storage.rs:74-82` (SQL `:77-80`) | The row count is summed twice and the asset count is one too high |
| `lineage` publishers | `routes/lineage.rs:247-254` (SQL `:249-250`), in `add_catalog` (`:207`) | Two publisher edges for one dataset when the registries name different authors |
| `catalog_profile::bronze_table_name` | `routes/catalog_profile.rs:275-293` (SQL `:277-282`) | `LIMIT 1` over the union with no order. Which table the asset page and a restore read is whichever row the engine answers first |

None of these reads takes `FINAL` either
(`git grep FINAL` over the five files finds nothing). A re-run writes the same slug again, and
until `ClickHouse` merges the parts, the same double count happens within one registry. That
is the reason `DATA-40` D4 and its review's SHOULD-FIX 1 gave for `catalog.rs`.

## 1. Decisions

- **D1. The rule is `DATA-40`'s, unchanged.** A slug the primary registry holds is not read
  from the secondary one. Every secondary branch in the five readers gets
  `WHERE slug {NOT_IN_PRIMARY}`, or `AND slug {NOT_IN_PRIMARY}` after an existing `WHERE`.
- **D2. One constant (rule 4).** `NOT_IN_PRIMARY` becomes `pub(crate)` where it is, in
  `routes/catalog.rs:566`, and its doc gains one line saying which other readers use it
  (`DATA-42`). The readers import it as `crate::routes::catalog::NOT_IN_PRIMARY`. It is not
  copied, and the string is not re-typed. *(planner default: leave it in `catalog.rs` rather than
  move it to a new module. It is one line, and its doc already explains it there.)*
- **D3. Every registry table these readers touch is read with `FINAL`**: `dataset_catalog` and
  `dataset_sync`, in both branches. This is the reason `DATA-40` D4 gives (a
  `ReplacingMergeTree ORDER BY slug` that each run writes again). Without it, D1 fixes the clash
  between registries but not the double count within one.
  - Write the alias **before** `FINAL` (`table AS s FINAL`). `DATA-40`'s fix round measured that
    `table FINAL AS s` is a syntax error on `ClickHouse` 26.8.
  - `overview.rs:65-69` has a `LEFT JOIN` over two unions. Each union is deduplicated on its
    own, so the join meets one catalog row and one sync row per slug.
- **D4. `catalog_profile::bronze_table_name` answers the primary's table.** The secondary branch
  gains `AND slug {NOT_IN_PRIMARY}`, and both branches take `FINAL`, so at most one row can
  answer. `LIMIT 1` stays as a guard. The doc says why the answer no longer depends on order,
  as `registered_slug`'s does (`catalog.rs:2896-2900`).
  - *Note:* `NOT_IN_PRIMARY` asks the primary **catalog**, and this reader reads **sync**. A
    slug that is in the primary sync table but not in the primary catalog would still be read
    from both. `catalog.rs`'s sync reads accept the same edge (`:631-633`). Keep it identical
    here, and do not invent a second rule. *(planner default)*
- **D5. `governance::classification`'s `LIMIT 500` keeps its meaning.** In `ClickHouse`, a
  `LIMIT` after the last `SELECT` of a `UNION ALL` binds to that `SELECT` unless the union is
  parenthesized. The developer checks this on the test `ClickHouse` (rule 1) and writes the
  answer in the handoff. Do not change what the limit caps in this item. If the check shows the
  cap is not what `DATA-40`'s handoff assumed ("takes one of the 500 slots"), report it, and the
  planner files it. *(planner default)*
- **D6. These are not touched**, with the reason from `DATA-40` D6:
  - `catalog_governance.rs:616-630`: a clash maps both table names to the same slug, so what it
    shows does not change;
  - `sql_rewrite.rs`: a SQL shape, not a read;
  - `routes/ai/*`: the AI team's (`DATA-39`).
- **D7. Tests on a real `ClickHouse`, one shared fixture (rule 4).**
  - `catalog.rs`'s `mod registry_clash` (`:7260`) has the helpers: `clickhouse()`,
    `wait_until_ready`, `create_registry`, `seed_catalog`, `seed_sync`, `seed_column`,
    `state_for` and `pinned`. They move to one `#[cfg(test)] pub(crate) mod` in the crate,
    proposed as `src/routes/bronze_registry_fixture.rs`, declared `#[cfg(test)]` in
    `routes/mod.rs`. `registry_clash` uses them from there, unchanged.
  - Each reader gets a `mod registry_clash` of its own that uses the fixture.
  - `overview::get_body` also needs Dagster and the probes. Its two registry statements become
    named constants (`ASSETS_SQL`, `WARM_ASSETS_SQL`), and the test runs the constants against
    the container, so the statement tested is the statement shipped.
  - `storage::get_body(ch)`, `governance::classification(ch, None)`, `lineage::add_catalog(ch,
    &mut g)` and `catalog_profile::bronze_table_name(&state, slug)` are called directly.

## 2. Anchors (verified at `9c066426`)

- `rust/crates/lakehouse-api/src/routes/catalog.rs`:
  - `NOT_IN_PRIMARY` `:552-566`;
  - its uses `:627`, `:632`, `:644`, `:1027`, `:2621`, `:2636`, `:2910`;
  - `registered_slug` `:2896-2915`;
  - `mod registry_clash` `:7260`, helpers `:7274-7420`.
- `rust/crates/lakehouse-api/src/routes/governance.rs:431-458`, test module `:1808`.
- `rust/crates/lakehouse-api/src/routes/overview.rs:57-88`, test module `:583`.
- `rust/crates/lakehouse-api/src/routes/storage.rs:67-82`, test module `:220`.
- `rust/crates/lakehouse-api/src/routes/lineage.rs:207`, `:247-268`, test module `:626`.
- `rust/crates/lakehouse-api/src/routes/catalog_profile.rs:266-293`, test module `:386`.
- `rust/crates/lakehouse-api/src/routes/mod.rs` (module declarations; `#[cfg(test)]` at
  `:1652`, `:1867`).
- `DATA-40` plan, handoff D6 table, and its fix round ("the alias comes BEFORE `FINAL`").

## 3. Tasks (one commit each, in order)

This is all Rust in one crate. Build it in one sitting, with `cargo check -p lakehouse-api`
between edits.

- **T1. The shared fixture.**
  - Move the helpers out of `catalog.rs`'s `registry_clash` into the fixture module (D7),
    unchanged.
  - `pub(crate) const NOT_IN_PRIMARY` (D2).
  - No behaviour change.
  - *Accept:* `cargo test -p lakehouse-api --lib registry_clash` runs the same 5 tests, all
    passing. Quote the count; it must equal the count before the move.
- **T2. `governance` and `lineage`.**
  - Apply D1 and D3, with a comment at each site citing `DATA-42` / `DATA-40` D3.
  - Tests:
    - one slug in both registries with different titles and authors. `classification` returns
      one `c-<slug>`, with the primary's title. The graph has one publisher edge for the
      dataset, from the primary's author;
    - a slug written twice to the primary before a merge (`SYSTEM STOP MERGES`, as `DATA-40`'s
      fix test does) is listed once.
  - The D5 check, with its result in the handoff.
  - *Accept:* `cargo test -p lakehouse-api --lib governance::` and
    `cargo test -p lakehouse-api --lib lineage::`.
- **T3. `overview` and `storage`.**
  - Extract the constants (D7), then apply D1 and D3.
  - Tests:
    - a clash with different `total`s: `assets` counts the slug once, `stale` follows the
      primary's `total`, and `storage`'s `rows` is the primary's `total`, not the sum;
    - the twice-written primary slug is counted once.
  - *Accept:* `cargo test -p lakehouse-api --lib overview::` and
    `cargo test -p lakehouse-api --lib storage::`.
- **T4. `catalog_profile::bronze_table_name`.**
  - Apply D4.
  - Test: a clash with different `table_name`s returns the primary's on repeated reads, under
    `pinned` (`max_threads=1`), with the secondary row written first.
  - Prove the test fails on the old statement (as `DATA-40` did), and quote the failure.
  - *Accept:* `cargo test -p lakehouse-api --lib catalog_profile::`, then
    `cargo fmt --check` and `cargo clippy -p lakehouse-api --all-targets -- -D warnings`.

For each of T2–T4, the handoff says whether the new test failed on the old code (measured, not
assumed). `docker ps` after the run must show no container left behind.

Full block before handoff: the Rust line of `AGENTS.md` "Verification", plus
`python3 ops/lint/check_bare_iceberg_count.py` (it scans Bronze counts).

## 4. Out of scope

- `catalog_governance.rs`, `sql_rewrite.rs` and `routes/ai/*` (D6).
- Changing what `classification`'s `LIMIT 500` caps (D5).
- Moving or merging rows that already clash; the write guard is `DATA-40` T1.
- Moving `CLASH_SQL`'s warning to these readers. One warning per list build is enough.

## 5. Handoff (developer appends below)

### Handoff (developer, 2026-11-02)

Branch `fix/data-42-primary-wins-everywhere`, base `main` at `9c066426`. Four commits, one per
task, in plan order:

| Commit | Task | What it changes |
| --- | --- | --- |
| `0e661f89` | T1 | The `ClickHouse` registry fixture moves into `routes::bronze_registry_fixture` (D7); `NOT_IN_PRIMARY` becomes `pub(crate)` (D2). No behaviour change. |
| `75781140` | T2 | `governance::classification` and `lineage`'s publisher edges carry `NOT_IN_PRIMARY` on the secondary branch and read with `FINAL` (D1/D3). |
| `c77152b0` | T3 | `overview`'s `assets`/`stale`/warm counts and `storage`'s warm tier carry the same clause and `FINAL` (D1/D3); the overview's two statements become the named constants `ASSETS_SQL`/`WARM_ASSETS_SQL` (D7). |
| `20a0159b` | T4 | `catalog_profile::bronze_table_name` answers the PRIMARY registry's table name (D4), with `FINAL` on both branches (D3) and `NOT_IN_PRIMARY` on the secondary one (D1). |

#### The commands that ran, and what they counted

All cargo commands ran through `/home/shiro/rantai/bin/cargo-locked`, from `rust/`, with `-j 2`
and `RUST_TEST_THREADS=1` (the machine ran out of memory on an earlier run of this slice).

```
cargo fmt --check -p lakehouse-api                                  -> exit 0
cargo clippy -p lakehouse-api --all-targets -j 2 -- -D warnings     -> clean, no warnings
cargo test -p lakehouse-api --lib registry_clash -j 2                -> 16 passed, 0 failed
cargo test -p lakehouse-api --lib governance:: -j 2                  -> 75 passed, 0 failed
cargo test -p lakehouse-api --lib lineage:: -j 2                     -> 55 passed, 0 failed
cargo test -p lakehouse-api --lib overview:: -j 2                    -> 17 passed, 0 failed
cargo test -p lakehouse-api --lib storage:: -j 2                     ->  6 passed, 0 failed
cargo test -p lakehouse-api --lib catalog_profile:: -j 2             -> 10 passed, 0 failed
cargo test -p lakehouse-api --test sec11_guard --test route_auth \
  --test catalog_restore --test upstream_errors -j 2
      -> sec11_guard 4 passed, route_auth 72 passed,
         catalog_restore 9 passed, upstream_errors 7 passed (0 failed)
python3 ops/lint/check_bare_iceberg_count.py                         -> exit 0 ("no bare count()")
python3 ops/lint/check_intra_package_imports.py                      -> exit 0
```

The 16 `registry_clash` tests are DATA-40's 5 in `catalog` plus this slice's 2 each in
`governance`, `lineage`, `overview` and `storage`, plus T4's 3 in `catalog_profile`. Each runs
against a real `ClickHouse` (`testcontainers`, the compose file's own `26.8` tag) through the
shared fixture of D7.

#### Did the new tests fail on the old code? (measured, not assumed)

- **T2** — the four new tests in `governance` and `lineage` FAIL on the old statements: measured
  `4 failed, 5 passed` (the list carried two rows with the same id `c-<slug>`, and the graph drew
  two publisher edges). *(Measured in the T2 sitting; the numbers are the ones the T2 commit
  records.)*
- **T3** — the four new tests in `overview` and `storage` FAIL on the old statements: measured
  `4 failed, 9 passed` (4 assets instead of 3, 2 stale instead of 1, 23 980 bytes instead of
  6 600). *(Measured in the T3 sitting; the numbers are the ones the T3 commit records.)*
- **T4** — two of the three new tests in `catalog_profile` FAIL on the old statement: measured
  `2 failed, 1 passed`.
  - `the_secondary_does_not_answer_for_a_slug_the_primary_holds`:
    `assertion left != right failed: the secondary's table must not answer for a slug the primary
    holds — left: "secondary_raw", right: "secondary_raw"`.
  - `a_slug_written_twice_to_the_primary_is_answered_once`:
    `assertion left == right failed: the newer row's table name, not the older one — left:
    "twice_profile_raw", right: "twice_profile_raw_2"`.
  - **The clash test itself (`the_primary_registry_answers_the_table_name_on_every_read`) PASSES
    on the old statement** — measured, and the reason is in the test's comment: with
    `max_threads=1` the pinned union answers in the ORDER OF THE STATEMENT'S BRANCHES, so the old
    statement answers with the primary's row first (20 of 20 reads on the test `ClickHouse`
    26.8.36.3); with the default settings the parallel union answers with the SECONDARY's row in
    3 of 20 reads. The plan's T4 asks for the clash test to be pinned, and a pinned test cannot
    fail on the old statement — so the rule the fix states ("the secondary never answers for a
    slug the primary holds") is pinned down by the second test, which fails every time. This is a
    deviation from the plan's *test method*, not from its decision: D4's statement is the one the
    plan asks for, and the plan's acceptance ("prove the test fails on the old statement") is met
    by the two tests that do.

#### D5: what `LIMIT 500` caps (the finding the plan asked for)

Measured on the compose file's own `ClickHouse` 26.8.36.3 (a scratch container, removed again),
on the statement the route now sends and on the one it sent before:

```sql
-- now
SELECT slug, title, tier FROM lake.`bronze_meta.dataset_catalog` FINAL
UNION ALL SELECT slug, title, tier FROM lake.`bronze_meta_sec.dataset_catalog` FINAL
WHERE slug NOT IN (SELECT slug FROM lake.`bronze_meta.dataset_catalog` FINAL) LIMIT 500
```

**The `LIMIT 500` binds to the LAST `SELECT` of the unparenthesized `UNION ALL`, so it caps the
SECONDARY branch only. The primary branch is not capped at all, and the union as a whole is not
capped at 500 rows.** The numbers (rows returned by the whole statement):

| primary rows | secondary rows | old statement | new statement |
| --- | --- | --- | --- |
| 3 | 600 | 503 | 503 |
| 604 | 600 | 1104 | 1104 |
| 604 | 3 | 607 | 607 |

So what the limit caps did **not** change with this slice (D5's requirement: the old and the new
statement return the same number of rows in all three shapes), and the plan's item is untouched.
But the finding to file is this: the classification list can hold **more than 500 rows** (607 and
1104 in the measurements above), and a row of the PRIMARY branch never takes a slot of the 500.
`DATA-40`'s handoff assumed a clashing slug "takes one of the 500 slots" — it does not: the
clashing slug's row lives in the primary branch, which the limit does not cap. Nothing in this
slice changes that; the planner files it.

#### Containers

`docker ps` after the run is empty: the `testcontainers` fixtures drop their container with the
test, and the scratch `ClickHouse` of the D5 measurement was removed. Nothing was left behind.

#### Not verified

- **The full workspace run of `AGENTS.md` "Verification" is NOT verified.** This machine ran out
  of memory on it, so the Rust line was scoped to the crate this slice touches
  (`cargo fmt --check -p lakehouse-api`, `cargo clippy -p lakehouse-api --all-targets`,
  `cargo test -p lakehouse-api --lib <filter>`) with `-j 2`. `cargo clippy --workspace
  --all-features` and `cargo test --workspace` did not run here; CI runs them on the pull request.
  `--all-features` was not used either.
- **`cargo test -p lakehouse-api` without a filter did not run** (the whole lib test set is ~2 957
  tests, most of them unrelated to this slice). The filters above cover every module this slice
  touches (`catalog`, `governance`, `lineage`, `overview`, `storage`, `catalog_profile`) plus the
  four integration test binaries that read the changed route.
- **The TypeScript, Dagster and compose lines did not run**: this slice touches no `.ts`, no
  Python and no compose file, so `AGENTS.md`'s "only build what changed" leaves them out. The two
  Python lints the plan asks for did run (above).
- The `catalog_profile` clash test's dependence on the engine's union order was measured on
  `ClickHouse` 26.8.36.3 only (the tag the compose file pins).
- No pull request was opened, and nothing was pushed to `main`.

## 6. Review (planner appends below)

---

### Proposed BACKLOG.md row

The row exists (`BACKLOG.md:272`, Todo). The proposed text replaces it:

| `DATA-42` | Task | Four readers still count a slug that is in both Bronze registries twice, and none of them reads with `FINAL`: `governance` classification (`routes/governance.rs`), `overview` (`routes/overview.rs`), `storage` (`routes/storage.rs`) and `lineage` publishers (`routes/lineage.rs`). `catalog_profile::bronze_table_name` takes an unordered `LIMIT 1` over both. Apply `DATA-40`'s rule (the primary wins) through the same `NOT_IN_PRIMARY` constant, add `FINAL`, and test each reader on a real `ClickHouse` with one shared fixture. The list is the `DATA-40` handoff's D6 table, with `file:line` | Data | P3 | Todo |  | `superpowers/plans/2026-11-01-data-40-one-name-one-dataset.md` |

### Review round 1 (planner, 2026-11-02, at `713fd6c0`)

- **T1–T4** match D1–D4. Every reader of both Bronze registries reads with `FINAL` and drops the slugs the primary holds, through the one `NOT_IN_PRIMARY` clause (rule 4). The shared registry fixture moved into `bronze_registry_fixture.rs` instead of a copy per module.
- **The tests fail on the old statement.** This was measured: T4 had 2 failed and 1 passed. The developer recorded that the pinned clash test passes on the old statement too, so the other two tests carry the proof. Accepted.
- **D5 is a new finding, not fixed here.** The catalog list's `LIMIT 500` binds to the last `SELECT` of the unparenthesized `UNION ALL`, so the primary branch is uncapped (503, 1104 and 607 rows measured). It behaves the same before and after this PR. Filed as a backlog item in the backlog docs PR (`DATA-43`).

**Open: no BLOCKER, no SHOULD-FIX.** Merge waits for the PR's CI, which runs the full workspace.
