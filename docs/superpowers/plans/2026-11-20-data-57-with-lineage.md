# `DATA-57`: column lineage through a `WITH`

Planner and reviewer: Claude Opus. Developer: `Hy4 flash` (OpenCode). Nobody
reviews their own work (`AGENTS.md`).

Base: `origin/main` at `c7cd01a76`. Every `file:line` below was read at that
commit unless it says `#619`. Branch: `data-57-with-lineage` (this plan is
its first commit). Backlog: `DATA-57`, under `DATA-13` and `BLD-1`. Feature
page: `docs/core/features/column-lineage.md` (decisions 1 to 5 still hold).
Origin: `docs/superpowers/plans/2026-11-12-bld-1-part-1-steps-and-sql.md`,
D4b and review R1-B1.

**Blocked by PR #619 (`BLD-16`, branch `bld-16-joins-aggregations`), for
tasks T6 to T8 only.** #619 is a draft and merges first. T1 to T5 touch no
file #619 touches and are built now; T6 to T8 are built after #619 merges
and this branch is rebased onto `main` (section 6).

Parallel-fix check, 2026-11-20: `gh pr list --state open` has no PR for
`DATA-57`, `sql_lineage` or column lineage; `git log origin/main -i
--grep='DATA-57\|sql_lineage\|DATA-13'` shows nothing newer than `be2c92910`
(BLD-1 part 1 S1, which created the gate this plan lifts).

## 1. Why

A pipeline is compiled into one statement (`pipeline_compile.rs:1-25`). A
pipeline of one input and at most one `transform` step compiles to a plain
`SELECT` (D4a); every other pipeline — a SQL step, several inputs, more than
one step, and after #619 every join, aggregate and filter step — compiles to
a `WITH` that names each input and each step, ending in
`SELECT * FROM <last step>`.

The column mapper (`rust/crates/lakehouse-api/src/sql_lineage.rs`) treats
every `WITH` name as opaque (`:17`, `:407-416`): a column read through one is
a partial row. On that shape that is worse than partial: the outer
`SELECT *` cannot be expanded, so `expand_star_rows`
(`routes/lineage/columns.rs:594`) spreads one partial row over whatever
columns the target was probed with — rows for columns the output may not
have (R1-B1). BLD-1 part 1 therefore stopped noting such statements at all
(`routes/authored_pipelines.rs:2106-2118`, gated on
`pipeline_compile::is_legacy_shape`, `pipeline_compile.rs:219-233`). Today
those pipelines have table lineage and no column lineage.

This item teaches the mapper to follow a `WITH` name to the query behind it
and then notes every compiled statement. The rule from `DATA-13` holds
unchanged: **a mapping is stored only when the statement proves it; anything
else is a partial row with a fixed reason, never an edge.**

The mapper has three callers, and all three gain the same reading:
pipeline runs (`columns.rs:649` `record_pipeline_run`), the query-log build
pass (`columns.rs:844`), and dashboard SQL sources (`columns.rs:189`
`record_sql_source`). No caller changes except the pipeline gate (T5).

## 2. Decisions

- **D1. A `WITH` name resolves to the outputs of its query.** Each CTE is
  read with the same rules as a top-level `SELECT` (the existing
  `outputs_of_body`), giving one `Out` per output column: its name, its
  source columns, its reasons and its transform. A `FROM` entry that names a
  CTE then carries that list, and a column read through it takes the CTE
  column's sources and reasons. CTEs are read in the order written; a CTE
  may read any CTE before it in the same `WITH` or in an enclosing one.
- **D2. A nested `WITH` is read the same way.** A SQL step's own text may
  start with `WITH` (it becomes a CTE whose query has its own `with`); its
  names are read first, are visible only inside that query, and shadow an
  outer name of the same spelling.
- **D3. CTE names match exactly, case included.** `ClickHouse` names are
  case sensitive, and the compiler already guarantees the steps' names are
  used exactly (BLD-1 part 1 D6a). A one-part table name that equals a CTE
  name only without regard to case is NOT a real table and NOT that CTE: it
  is opaque, with the reason `MSG_CTE_CASE`. A column read through a CTE also
  matches its output name exactly (unlike a real table, whose known columns
  come from the engine and keep today's single case-folded match,
  `sql_lineage.rs:690-705`).
- **D4. A `SELECT *` over a CTE expands when the CTE's columns are all
  known.** This is the compiled shape itself (every input is
  `SELECT * FROM <table>`, a filter step is `SELECT * FROM <reads>`, the
  last line is `SELECT * FROM <last step>`), so refusing it would leave
  nothing to record. A CTE's columns are "all known" when none of its
  outputs is an unexpanded `*` (`Out::unexpanded_star`). A CTE that holds an
  unexpanded `*` is opaque with that `*`'s own reason (for example
  `the columns of bronze.x are not known`), and so is everything read
  through it.
- **D5. Transforms compose.** The outer transform applied to the inner one:
  | outer \ inner | copy | cast | expression |
  | --- | --- | --- | --- |
  | copy | copy | cast | expression |
  | cast | cast | expression | expression |
  | expression | expression | expression | expression |

  A cast of a cast is `expression` (planner default, owner may change): the
  stored word means "one `CAST` of one column", and two are not that. An
  outer copy of a CTE column that reads no column (a literal, `count()`) is
  a constant: no row, as today (`sql_lineage.rs:30-31`).
- **D6. An alias of the same `SELECT` captures an unqualified name.** In
  `ClickHouse` an alias is visible in the whole query (`BLD-16` plan F2,
  `#619 pipeline_forms.rs:25-28`): in `SELECT amount AS id, id AS orig FROM t`
  the second item reads `amount`, not `t.id`. Today the mapper would record
  `orig ← t.id`, a wrong edge, and SQL steps make the shape likely. So an
  unqualified identifier whose name is the alias of ANOTHER item of the same
  projection resolves to nothing: the item gets the partial reason
  `MSG_ALIAS_CAPTURE`. Not captured: a qualified identifier (`t.id`), the
  item's own alias inside its own expression (`CAST(price AS Float64) AS
  price` keeps today's mapping), and an alias that is the bare column of the
  same name (`id AS id`). This applies to every `SELECT`, with or without a
  `WITH` (it fixes the same wrong edge on a plain statement).
- **D7. Unsupported shapes, each a partial row with a fixed reason and no
  edge:**
  | Shape | Reason constant |
  | --- | --- |
  | `WITH RECURSIVE` (every name of that `WITH`) | `MSG_CTE_RECURSIVE` |
  | A CTE with a column list, `WITH c(x, y) AS (...)` | `MSG_CTE_COLUMNS` |
  | A CTE with `MATERIALIZED` / `NOT MATERIALIZED` or a `FROM` hint | `MSG_CTE_HINT` |
  | A name read before its CTE is defined (a later CTE of the same `WITH`) | `MSG_CTE_FORWARD` |
  | Two CTEs of one `WITH` with the same name | `MSG_CTE_DUPLICATE` (both) |
  | A CTE whose query cannot be read at all (`VALUES`, a non-`SELECT` body) | `MSG_CTE_UNREADABLE` |
  | A `UNION`/`INTERSECT`/`EXCEPT` inside a CTE | `MSG_UNION`, carried through (today's rule, `:362-374`) |
  | A subquery in `FROM` or a table function inside a CTE | `MSG_SUBQUERY_FROM` / `MSG_TABLE_FUNCTION`, carried through |
  | `*` over a CTE whose columns are not all known (D4) | the `*`'s own reason |
  | A column the CTE does not output | `the column {c} is not a column of the CTE {n}` |
  | A CTE with two outputs of the same name, read by that name | `the CTE {n} has more than one column named {c}` |
  | An unqualified column found in more than one joined relation, or in a relation whose columns are not known | today's sentences (`:800-820`) |
  | An alias captured as in D6 | `MSG_ALIAS_CAPTURE` |
  | The case-only name of D3 | `MSG_CTE_CASE` |

  Fixed sentences (copy exactly; `{n}` and `{c}` go through `short()`):
  - `MSG_CTE_RECURSIVE`: `"read through a recursive WITH, which is not traced"`
  - `MSG_CTE_COLUMNS`: `"read through a WITH name with its own column list, which is not traced"`
  - `MSG_CTE_HINT`: `"read through a WITH name with a MATERIALIZED or FROM hint, which is not traced"`
  - `MSG_CTE_FORWARD`: `"read through a WITH name before it is defined, which is not traced"`
  - `MSG_CTE_DUPLICATE`: `"read through a WITH name that is defined twice, which is not traced"`
  - `MSG_CTE_UNREADABLE`: `"read through a WITH name whose query cannot be read, which is not traced"`
  - `MSG_CTE_CASE`: `"the name differs from a WITH name only in letter case, which is not traced"`
  - `MSG_ALIAS_CAPTURE`: `"the name is also an alias in this SELECT, and ClickHouse reads the alias; rename one"`
- **D8. Subqueries in `FROM` stay opaque.** The same machinery could read
  them, but `DATA-57` is the `WITH`; a subquery in `FROM` is out of scope
  (section 8) and keeps `MSG_SUBQUERY_FROM`.
- **D9. Every compiled pipeline statement is noted.** The `is_legacy_shape`
  gate at `authored_pipelines.rs:2116` is removed; `note_pipeline_sql` runs
  for every definition. `is_legacy_shape` itself stays (D4a's compile and
  the legacy form edit at `authored_pipelines.rs:1159` still use it). The
  stored rows still come from the COMPILED SQL, not the rewritten one
  (`:2106-2109`, unchanged reason).
- **D10. Storage rules do not change.** `rows_for`, `expand_star_rows`,
  `unmappable_rows` and the 20,000-character limit (`columns.rs:631-647`)
  are untouched. A mapping row and a partial row for the same output (an
  expression reading one traceable and one untraceable column) are written
  as today.

Only one item in this plan is the owner's to change: D5's cast-of-a-cast as
`expression`. Everything else follows from `DATA-13` decision 2 and the
"never a guess" rule (`sql_lineage.rs:9-14`).

## 3. Anchors (at `c7cd01a76`)

`rust/crates/lakehouse-api/src/sql_lineage.rs`:
- `:1-35` module doc; `:17` lists "a column read through a CTE" as partial.
- `:128-141` `Source` (`Table`, `Opaque`) and `Entry`.
- `:143-147` `Scope`. `:149-158` `Out`.
- `:160-167` the `MSG_*` constants.
- `:234-262` `map_select`; `:240-246` collects CTE names as lowercased strings.
- `:334-377` `outputs_of_body`; `:351-361` the nested-query `WITH` branch.
- `:380-389` `select_entries`; `:391-447` `push_factor`; `:407-416` the CTE branch.
- `:449-524` `outputs_of_item`; `:538-590` `expand_star`; `:577-588` the
  duplicate-name rule (only when more than one entry).
- `:633-643` `transform_of`; `:646-677` `analyse`.
- `:690-705` `lookup`; `:709-719` `on_table`; `:722-732` `resolve`;
  `:734-766` `resolve_qualified`; `:768-823` `resolve_unqualified`.
- `:826-878` test helpers (`known()`, `run`, `triples`, `t`, `reasons`);
  `:1029-1046` `a_cte_a_subquery_in_from_and_a_table_function_are_partial`.

`rust/crates/lakehouse-api/src/routes/lineage/columns.rs`: `:189` `record_sql_source`;
`:549-553` `note_pipeline_sql`; `:594-629` `expand_star_rows`;
`:631-647` `map_statement_or_unmappable`; `:649-692` `record_pipeline_run`;
`:844` the build pass's `statement_target`.

`rust/crates/lakehouse-api/src/routes/authored_pipelines.rs:2106-2118` — the
D4b gate and its comment.

`rust/crates/lakehouse-api/src/pipeline_compile.rs:219-233` —
`is_legacy_shape` and its doc ("the only shape whose statement is noted for
column lineage (D4b)"). In #619 the same doc is at `:240-243`, and #619 adds
a test doc at `:1547` with the same claim.

`rust/crates/lakehouse-api/tests/column_lineage_pipeline.rs`: `:40-41` `SQL`;
`:59-113` `app()` (the mock engine answers `id`, `amount`, `price` for EVERY
`system.columns` query, target included); `:127-180` `seed_pipeline`;
`:377-441` `a_pipeline_with_a_sql_step_writes_no_column_lineage_row`.

#619 (read at `origin/bld-16-joins-aggregations` `89bda5288`):
- `pipeline_forms.rs:658-688` `render_join`: `` SELECT `__left`.`c` AS `n`, ...
  FROM `l` AS `__left` <KIND> JOIN `r` AS `__right` ON `__left`.`a` = `__right`.`b` ``.
- `:691-746` `render_aggregate`: `` SELECT `g`, sum(`c`) AS `m`, count() AS `n`
  FROM `x` GROUP BY `g` HAVING sum(`c`) > 1000 ``; measures `count()`,
  `` count(`c`) ``, `` count(DISTINCT `c`) ``, `sum`, `avg`, `min`, `max`.
- `:752-765` `render_filter`: `` SELECT * FROM `x` WHERE `c` >= '5000' OR ... ``.
- `pipeline_compile.rs:633-634` wraps a form step as `` `name` AS (<body>) ``.
- `tests/pipeline_compile_route.rs:1272-1311`
  `a_form_step_pipeline_is_not_noted_for_column_lineage` — asserts the gate
  this plan removes; it FAILS after T5 and is changed in T6.
- `ops/g7/g7_pipeline_test.py` `step_a_form_step_pipeline_matches_its_sql_twin_and_a_direct_query`
  and `_create_and_run_authored_pipeline` (returns nothing today).
- `docs/core/features/pipeline-joins-and-sql.md:131-133` — the limit line
  this item removes.

**Files both this plan and #619 touch:** `src/pipeline_compile.rs` (a doc
comment, T6), `tests/pipeline_compile_route.rs` (T6),
`ops/g7/g7_pipeline_test.py` (T7), `docs/core/features/pipeline-joins-and-sql.md`
(T8), `CHANGELOG.md` (T8). All four are in T6 to T8, after the rebase. No
other file overlaps: #619 does not touch `sql_lineage.rs`,
`routes/lineage/columns.rs`, `routes/authored_pipelines.rs` or
`tests/column_lineage_pipeline.rs`.

## 4. Shape of the change (guidance, not a contract)

Keep it inside `sql_lineage.rs`; no new module.

- Replace the `ctes: &[String]` parameter with an environment, e.g.
  `struct CteEnv { defined: Vec<(String /*exact name*/, CteDef)>, pending: Vec<String> }`
  with `enum CteDef { Columns(Vec<Out>), Opaque(String) }`. Lookup walks
  `defined` from the end (D2 shadowing). `pending` holds names of the
  current `WITH` not yet read (for `MSG_CTE_FORWARD`) — compare exactly,
  and case-insensitively only to produce `MSG_CTE_CASE`.
- Add `fn outputs_of_query(query: &Query, env: &CteEnv, known) -> Result<Vec<Out>, Unmappable>`:
  read `query.with` (if any) into a child env in order, then
  `outputs_of_body(&query.body, &child_env, known)`. A CTE whose
  `outputs_of_query` is `Err` becomes `CteDef::Opaque(MSG_CTE_UNREADABLE)`;
  one with an `unexpanded_star` out becomes `Opaque(<that out's first reason>)`.
  `map_select` calls `outputs_of_query` with an empty env;
  `outputs_of_body`'s `SetExpr::Query` arm calls it too (replacing `:351-361`).
- Add `Source::Cte { name: String, columns: Vec<Out> }` (clone the `Out`s;
  at most 25 CTEs per pipeline, and 20,000 characters per statement).
  `push_factor` produces it, or `Source::Opaque` per D7.
- Change `resolve` (and the two below it) to return the resolved READ, not
  one pair: e.g. `struct Read { sources: Vec<(String, String)>, reasons: Vec<String>, transform: Transform }`.
  A real table gives one source and `Transform::Copy`. A CTE column gives
  its own sources, reasons and transform. `analyse` merges sources and
  reasons (deduplicated, as `:662-675` does) and, when the outer
  expression is one column or a cast of one (`transform_of` is `Copy` or
  `Cast`), composes the transform per D5.
- `resolve_unqualified` over CTE entries: a CTE's columns are always known,
  so it behaves like a table whose columns are known (`Lookup::Found` /
  `Absent`, exact match), and an `Opaque` entry still makes every
  unqualified name partial (`:771-777`, unchanged).
- `expand_star` over a `Source::Cte` pushes a clone of each of its `Out`s.
  Run the duplicate-name rule (`:577-588`) whenever an entry is a CTE, not
  only when there are several entries (D7, a CTE with two outputs of one
  name).
- D6: in the `SetExpr::Select` arm, collect the projection's aliases
  (excluding `Identifier(x) AS x`) with the index of the item that defines
  each; pass them to `analyse` via `Scope`, and in `resolve`, an
  unqualified name equal to an alias defined by a DIFFERENT item index is
  `Err(MSG_ALIAS_CAPTURE)`.

## 5. Tasks

One task per commit, in this order. Every Rust commit runs, from `rust/`:
`cargo-locked fmt --check`,
`cargo-locked clippy -p lakehouse-api --all-targets -- -D warnings`,
and the scoped test named in the task (`AGENTS.md`, "Keep build time down";
`cargo-locked` is the build machine's wrapper on `PATH`, not a repo file
— if your machine has none, use `cargo` with `-j 2`). Push after
every commit (`git push`).

In every unit test below, `known()` is the existing helper
(`silver.orders: id, Amount, region`; `silver.users: id, name`;
`serving.target: a, b`) and `triples`/`reasons` are the existing helpers.
"No edge" means `l.mappings` is empty.

### T1. The environment and `WITH` resolution (D1, D2, D3, D4, D7 rows 1-7)

Change `sql_lineage.rs` per section 4: `CteEnv`, `outputs_of_query`,
`Source::Cte`, `push_factor`, `expand_star`, and the new `MSG_CTE_*`
constants with the exact text of D7. Update the module doc `:16-26`: the
CTE bullet becomes the D7 list in one sentence each; add a short "# Through
a `WITH`" section stating D1 to D4. Do not touch D5/D6 yet (T2, T3): in this
commit a CTE column read by an outer expression contributes its sources and
reasons, and the transform is still `transform_of(expr)`.

Rule 2: `a_cte_a_subquery_in_from_and_a_table_function_are_partial`
(`:1029-1046`) asserts the behaviour this item removes. Drop its CTE half
(keep the subquery and table-function halves, renamed
`a_subquery_in_from_and_a_table_function_are_partial`), and say why in the
commit body.

Acceptance — new unit tests in `sql_lineage.rs`, exact values:

1. `a_column_read_through_a_cte_names_the_table_behind_it`:
   `WITH c AS (SELECT id, Amount AS amt FROM silver.orders) SELECT id, amt FROM c`
   → `triples == [t("id","silver.orders.id","copy"), t("amt","silver.orders.Amount","copy")]`,
   `partial` empty.
2. `a_star_over_a_cte_expands_to_its_columns`:
   `WITH c AS (SELECT * FROM silver.users) SELECT * FROM c`
   → `[t("id","silver.users.id","copy"), t("name","silver.users.name","copy")]`.
3. `a_nested_with_is_read_and_shadows_the_outer_name`:
   `WITH t AS (SELECT name FROM silver.users), s AS (WITH t AS (SELECT id FROM silver.orders) SELECT id FROM t) SELECT id FROM s`
   → `[t("id","silver.orders.id","copy")]`.
4. `a_cte_over_unknown_columns_leaves_the_star_partial`:
   `WITH c AS (SELECT * FROM silver.unknown) SELECT * FROM c`
   → no edge; `reasons == [("*", "the columns of silver.unknown are not known")]`.
5. `a_union_inside_a_cte_is_carried_through_as_partial`:
   `WITH c AS (SELECT id FROM silver.orders UNION ALL SELECT id FROM silver.users) SELECT id FROM c`
   → no edge; `reasons == [("id", MSG_UNION)]`.
6. `a_subquery_in_from_inside_a_cte_is_carried_through_as_partial`:
   `WITH c AS (SELECT id FROM (SELECT id FROM silver.orders) s) SELECT id FROM c`
   → no edge; `reasons == [("id", MSG_SUBQUERY_FROM)]`.
7. `a_recursive_with_is_partial`:
   `WITH RECURSIVE c AS (SELECT id FROM silver.orders) SELECT id FROM c`
   → no edge; `reasons == [("id", MSG_CTE_RECURSIVE)]`. If the
   `ClickHouseDialect` parser refuses `WITH RECURSIVE`, assert instead
   `map_select(...) == Err(Unmappable("the statement does not parse".into()))`
   and say so in the handoff.
8. `a_cte_with_a_column_list_is_partial`:
   `WITH c(x) AS (SELECT id FROM silver.orders) SELECT x FROM c` → no edge;
   `reasons == [("x", MSG_CTE_COLUMNS)]` (same parse fallback as 7).
9. `a_name_read_before_its_cte_is_defined_is_partial`:
   `WITH a AS (SELECT id FROM b), b AS (SELECT id FROM silver.orders) SELECT id FROM a`
   → no edge; `reasons == [("id", MSG_CTE_FORWARD)]`.
10. `a_name_that_differs_from_a_cte_only_in_case_is_partial`:
    `WITH c AS (SELECT id FROM silver.orders) SELECT id FROM C` → no edge;
    `reasons == [("id", MSG_CTE_CASE)]`.
11. `a_column_the_cte_does_not_output_is_partial`:
    `WITH c AS (SELECT id FROM silver.orders) SELECT Amount FROM c` → no edge;
    `reasons == [("Amount", "the column Amount is not a column of the CTE c")]`.
12. `a_cte_with_two_columns_of_one_name_is_partial_by_that_name`:
    `WITH c AS (SELECT o.id, u.id FROM silver.orders AS o JOIN silver.users AS u ON o.id = u.id) SELECT id FROM c`
    → no edge; `reasons == [("id", "the CTE c has more than one column named id")]`.
13. `an_unqualified_column_in_two_joined_ctes_is_partial`:
    `WITH a AS (SELECT id, name FROM silver.users), b AS (SELECT id FROM silver.orders) SELECT id FROM a JOIN b ON a.id = b.id`
    → no edge; `reasons == [("id", "the column id matches more than one table in FROM; qualify it")]`.
14. `an_insert_through_a_with_maps_by_position`:
    `INSERT INTO serving.target WITH c AS (SELECT id, name FROM silver.users) SELECT id, name FROM c`
    → `target == Some("serving.target")`,
    `[t("a","silver.users.id","copy"), t("b","silver.users.name","copy")]`.
    (If the parser wants the `WITH` elsewhere, use the form it accepts and
    say which in the handoff.)

Scoped run: `cargo-locked test -p lakehouse-api --lib sql_lineage`.

### T2. Transforms compose (D5)

`analyse` composes per D5's table. Doc comment on the composing function
cites D5 and states the cast-of-a-cast default.

Acceptance — unit tests:

1. `a_cast_of_a_cte_copy_is_a_cast_and_a_copy_of_a_cte_cast_is_a_cast`:
   `WITH c AS (SELECT id, CAST(Amount AS Float64) AS f FROM silver.orders) SELECT CAST(id AS String) AS s, f FROM c`
   → `[t("s","silver.orders.id","cast"), t("f","silver.orders.Amount","cast")]`.
2. `a_cast_of_a_cte_cast_is_an_expression`:
   `WITH c AS (SELECT CAST(Amount AS Float64) AS f FROM silver.orders) SELECT CAST(f AS String) AS g FROM c`
   → `[t("g","silver.orders.Amount","expression")]`.
3. `a_copy_of_a_cte_constant_is_a_constant`:
   `WITH c AS (SELECT 1 AS one, count() AS n FROM silver.orders) SELECT one, n FROM c`
   → no edge, `partial` empty, `constants == ["one","n"]`.
4. `the_compiled_shape_of_a_multi_step_pipeline_is_traced` — the text
   `pipeline_compile` produces (`pipeline_compile.rs:683-702`'s format):
   ```
   WITH
     `orders` AS (SELECT * FROM silver.`orders`),
     `paid` AS (SELECT * FROM `orders` WHERE region = 'EU'),
     `shaped` AS (
   SELECT id, CAST(Amount AS Float64) AS amount_f, upper(region) AS r FROM paid
   )
   SELECT * FROM `shaped`
   ```
   → `[t("id","silver.orders.id","copy"), t("amount_f","silver.orders.Amount","cast"), t("r","silver.orders.region","expression")]`, `partial` empty.
5. `two_inputs_joined_in_a_sql_step_map_each_column_to_its_own_table`:
   ```
   WITH
     `orders` AS (SELECT * FROM silver.`orders`),
     `users` AS (SELECT * FROM silver.`users`),
     `joined` AS (
   SELECT o.id AS order_id, u.name, o.Amount * 2 AS doubled FROM orders AS o JOIN users AS u ON o.id = u.id
   )
   SELECT * FROM `joined`
   ```
   → `[t("order_id","silver.orders.id","copy"), t("name","silver.users.name","copy"), t("doubled","silver.orders.Amount","expression")]`.

Scoped run as T1.

### T3. An alias captures an unqualified name (D6)

Acceptance — unit tests:

1. `an_alias_of_another_item_captures_an_unqualified_name`:
   `SELECT Amount AS id, id AS original FROM silver.orders`
   → `triples == [t("id","silver.orders.Amount","copy")]`;
   `reasons == [("original", MSG_ALIAS_CAPTURE)]`.
2. `the_same_capture_inside_a_cte_is_carried_through`:
   `WITH c AS (SELECT Amount AS id, id AS original FROM silver.orders) SELECT original FROM c`
   → no edge; `reasons == [("original", MSG_ALIAS_CAPTURE)]`.
3. `an_items_own_alias_and_a_qualified_name_are_not_captured`:
   `SELECT CAST(Amount AS Float64) AS Amount, o.id AS region_id, o.region AS id FROM silver.orders AS o`
   → `[t("Amount","silver.orders.Amount","cast"), t("region_id","silver.orders.id","copy"), t("id","silver.orders.region","copy")]`, `partial` empty.
4. `id_as_id_does_not_capture`:
   `SELECT id AS id, id + 1 AS next FROM silver.orders`
   → `[t("id","silver.orders.id","copy"), t("next","silver.orders.id","expression")]`.

Every existing test in the module still passes unchanged (if one does not,
stop and report: D6 is meant to change only the captured case).

Scoped run as T1.

### T4. The form steps of #619, by their rendered text

No #619 code is needed: the text is pinned in #619's own byte-for-byte
tests (`pipeline_forms.rs:870-1030` at #619). These tests are what make the
gate lift in T5 safe for the form steps.

Acceptance — unit tests:

1. `a_join_an_aggregate_and_a_filter_as_compiled_are_traced`:
   ```
   WITH
     `orders` AS (SELECT * FROM silver.`orders`),
     `users` AS (SELECT * FROM silver.`users`),
     `joined` AS (SELECT `__left`.`id` AS `id`, `__left`.`Amount` AS `amount`, `__right`.`name` AS `name` FROM `orders` AS `__left` LEFT JOIN `users` AS `__right` ON `__left`.`id` = `__right`.`id`),
     `by_name` AS (SELECT `name`, sum(`amount`) AS `revenue`, count() AS `order_count`, count(DISTINCT `id`) AS `ids` FROM `joined` GROUP BY `name` HAVING sum(`amount`) > 1000),
     `big` AS (SELECT * FROM `by_name` WHERE `revenue` >= '5000' OR `name` = 'ID')
   SELECT * FROM `big`
   ```
   → `triples == [t("name","silver.users.name","copy"), t("revenue","silver.orders.Amount","expression"), t("ids","silver.orders.id","expression")]`;
   `constants == ["order_count"]`; `partial` empty.
2. `a_self_join_as_compiled_maps_both_sides_to_the_one_table`:
   ```
   WITH
     `orders` AS (SELECT * FROM silver.`orders`),
     `tree` AS (SELECT `__left`.`id` AS `id`, `__right`.`id` AS `parent` FROM `orders` AS `__left` INNER JOIN `orders` AS `__right` ON `__left`.`id` = `__right`.`region`)
   SELECT * FROM `tree`
   ```
   → `[t("id","silver.orders.id","copy"), t("parent","silver.orders.id","copy")]`.
3. `every_measure_function_as_rendered_reads_its_column`: for each of
   `count(`id`)`, `count(DISTINCT `id`)`, `sum(`id`)`, `avg(`id`)`,
   `min(`id`)`, `max(`id`)`: ``WITH `o` AS (SELECT * FROM silver.`orders`), `m` AS (SELECT <expr> AS `x` FROM `o`) SELECT * FROM `m` ``
   → `[t("x","silver.orders.id","expression")]`.

Scoped run as T1.

### T5. Note every compiled statement (D9), and prove it through the routes

- `routes/authored_pipelines.rs:2106-2118`: remove the `is_legacy_shape`
  condition; call `note_pipeline_sql` for every definition. Rewrite the
  comment: keep the `DATA-13 (F2)` paragraph; replace the D4b paragraph with
  one line citing `DATA-57` (the mapper reads a `WITH`, so every compiled
  statement is noted; what it cannot prove is partial).
- `tests/column_lineage_pipeline.rs`: rule 2 — replace
  `a_pipeline_with_a_sql_step_writes_no_column_lineage_row` (`:377-441`),
  whose premise this item removes, with the three tests below; say why in
  the commit body. Factor the `create_pipeline` + `status = 'ready'` block
  into one helper taking `inputs` and `steps` (rule 4: three callers now).
  Rows are filtered to `producer_ref == <the pipeline's id>`.

Acceptance — integration tests (mock engine: every table has `id`,
`amount`, `price`):

1. `a_pipeline_with_a_sql_step_records_its_columns_through_the_with`:
   input `orders` = `bronze.orders`; step SQL `projected` =
   `SELECT id, amount AS amt FROM orders`; target `silver.orders_clean`.
   After compile, `pending(&pool, &id)` is `Some(_)`. After the success
   event, the rows sorted as `(from_table, from_column, to_column, transform, partial)` are exactly
   `[("bronze.orders","amount","amt","copy",false), ("bronze.orders","id","id","copy",false)]`.
2. `a_pipeline_with_two_inputs_and_two_steps_maps_each_column_to_its_input`:
   inputs `orders` = `bronze.orders`, `customers` = `bronze.customers`;
   steps SQL `joined` = `SELECT o.id AS id, c.price AS price FROM orders AS o JOIN customers AS c ON o.id = c.id`,
   SQL `scaled` = `SELECT id, price * 2 AS price2 FROM joined`.
   Rows exactly `[("bronze.customers","price","price2","expression",false), ("bronze.orders","id","id","copy",false)]`.
   (If the tenant gate refuses `bronze.customers` with this harness, stop
   and report; do not change the gate or `CATALOG_TENANT_ID`.)
3. `a_pipeline_whose_step_is_a_union_records_no_edge`:
   one input `orders`; step SQL `both` =
   `SELECT id FROM orders UNION ALL SELECT id FROM orders`.
   Every row for the pipeline has `partial == true` and `from_table == None`;
   the rows are exactly one, `to_column == "id"`, reason
   `"built with UNION, INTERSECT or EXCEPT; the branches are not traced"`.

The existing tests of the file (`:254-375`, today's shape) pass unchanged.

Scoped run: `cargo-locked test -p lakehouse-api --test it column_lineage_pipeline::`
(plus `--lib sql_lineage` once more).

**Stop here if #619 has not merged.** Write the handoff for T1 to T5 and
report; the planner decides whether to open T1 to T5 as their own PR
(section 6).

### T6. After the rebase: #619's test and doc comments

Rebase onto `origin/main` containing #619 (section 6). Then:

- `tests/pipeline_compile_route.rs` (#619 `:1272-1311`): rule 2 —
  `a_form_step_pipeline_is_not_noted_for_column_lineage` asserts the gate
  T5 removed. Rename it `a_form_step_pipeline_is_noted_for_column_lineage`,
  assert `noted == 1`, and rewrite its doc comment to cite `DATA-57` D9.
- `pipeline_compile.rs`: `is_legacy_shape`'s doc (#619 `:240-243`) — drop
  "and the only shape whose statement is noted for column lineage (D4b)";
  the test doc at #619 `:1547` — drop "and is not noted for column lineage
  (D4b)". Grep the crate for any other `noted for column lineage` /
  `D4b` claim about lineage (`rg -n "noted for column lineage|cannot read through a .WITH|WITH-aware" rust/`)
  and correct each.
- `tests/column_lineage_pipeline.rs`: add
  `a_pipeline_of_form_steps_records_its_columns`: inputs `orders` =
  `bronze.orders`, `customers` = `bronze.customers`; a `join` step `joined`
  (left `orders`, right `customers`, inner, on `id = id`, columns: left
  `id`, left `amount`, right `price` as `unit_price`); an `aggregate` step
  `by_id` (reads `joined`, group by `id`, measures `sum(amount)` as
  `total` and `count()` as `n`); a `filter` step `big` (reads `by_id`,
  match `all`, `total > 0`). Rows exactly
  `[("bronze.orders","amount","total","expression",false), ("bronze.orders","id","id","copy",false)]`
  (`n` is a constant; `unit_price` is dropped by the aggregate). Build the
  steps with #619's `pipelines::PipelineStep::{Join, Aggregate, Filter}`
  exactly as #619's own tests do.

Scoped run: `cargo-locked test -p lakehouse-api --test it pipeline_compile_route:: column_lineage_pipeline::`
and `--lib pipeline_compile`.

### T7. After the rebase: the `g7` gate reads the form pipeline's lineage

`ops/g7/g7_pipeline_test.py` (as #619 leaves it):

- `_create_and_run_authored_pipeline` returns the `pipeline_id`.
- Generalise `step_verify_column_lineage_of_the_pipeline_target` (`:618-664`
  at `c7cd01a76`) to take the table, the pipeline id and a list of
  `(to_column, from_table, from_column, transform)` (rule 4: one poller, not
  two); its existing caller (`:1670`) passes today's two expectations with
  transform `copy`, so what it asserts does not change. Assert the
  `transform` too, through the field the route returns (read
  `routes/lineage/columns.rs` for the JSON shape; do not guess it).
- In `step_a_form_step_pipeline_matches_its_sql_twin_and_a_direct_query`,
  after the form pipeline's run, verify `silver.<its target>.customer ←
  bronze.<BRONZE_TABLE_NAME>.customer` (`copy`, not partial) and `total ←
  bronze.<BRONZE_TABLE_NAME>.amount` (`expression`, not partial), recorded
  by that pipeline.

Check: `python3 -m py_compile ops/g7/g7_pipeline_test.py` and
`python3 ops/lint/check_intra_package_imports.py`. The gate itself runs only
in CI job `g3a-dagster`; the handoff says *not verified locally* for it.

### T8. After the rebase: what the customer is told

- `docs/core/features/column-lineage.md`, "Limits to tell a customer": add
  one line — mappings are read through `WITH` (every pipeline shape); not
  through a recursive `WITH`, a `WITH` name with its own column list, a
  subquery in `FROM`, `UNION`/`INTERSECT`/`EXCEPT`, an unqualified column
  two joined tables both have, or a name that is also another column's
  alias in the same `SELECT`; each of these is shown partial with its
  reason. Add `DATA-57` to the spec-row "Column mappings recorded..." text
  where it says what the parser cannot prove.
- `docs/core/features/pipeline-joins-and-sql.md`: delete the lines saying a
  pipeline with a SQL step / several inputs / more than one step, or with a
  join, aggregate or filter step, has no column lineage (`:129` and `:145`
  at `c7cd01a76`; #619 rewrites them, so find them by
  `grep -n DATA-57`).
- `CHANGELOG.md` `[Unreleased]`, one entry a customer can read: column
  lineage now covers every pipeline (SQL steps, several inputs, several
  steps, joins, aggregations, filters) from its next successful run; what
  it cannot prove is shown partial with the reason; a name that is also
  another column's alias in the same `SELECT` is now partial instead of
  mapped to the column (backlog `DATA-57`).

No `BACKLOG.md` edit by the developer (the reviewer sets status on merge).

## 6. PR slicing and the rebase

One PR, `data-57-with-lineage` → `main`, opened by the planner after T8 and
review. If #619 has not merged when T5 is done, the planner may open T1 to
T5 as the PR on its own; T6 to T8 then become a second PR on a new branch
off `main` after both merge. Either way, the PR that lands after #619 must
carry T6, because #619's `a_form_step_pipeline_is_not_noted_for_column_lineage`
fails once T5 is on `main`. If T1 to T5 merge first, #619 must be rebased
and that test flipped in #619 instead — the planner tells #619's developer.

Rebase, when #619 is on `main`: `git fetch origin && git rebase origin/main`.
T1 to T5 touch no file #619 touches, so no conflict is expected; if one
appears, resolve it hunk by hunk (rule 9), then rebuild and re-run every
scoped test of T1 to T5 before starting T6. `git push --force-with-lease`
on this feature branch only.

## 7. Verification before handoff

From the final commit, foreground, quoting each command and its counts:

```bash
cd rust && cargo-locked fmt --check \
  && cargo-locked clippy --workspace --all-targets --all-features -- -D warnings \
  && cargo-locked test -p lakehouse-api --lib sql_lineage \
  && cargo-locked test -p lakehouse-api --lib pipeline_compile \
  && cargo-locked test -p lakehouse-api --test it column_lineage_pipeline:: column_lineage_route:: column_lineage_sql_source:: pipeline_compile_route:: sec11_guard::
python3 -m py_compile ops/g7/g7_pipeline_test.py
python3 ops/lint/check_intra_package_imports.py && python3 ops/lint/check_bare_iceberg_count.py
git status   # nothing generated staged
```

The full `cargo test --workspace` is CI's (`AGENTS.md`, "Keep build time
down"; owner rule: CI's cargo run is the full one). No TypeScript, Dagster
code or compose changes, so those lines do not run. The handoff says
*not verified* for the `g7` gate (CI only) and for anything else not run.

## 8. Out of scope

- Subqueries in `FROM` (D8), table functions, `ARRAY JOIN`, set operations —
  all stay partial.
- `ClickHouse`'s scalar `WITH <expr> AS name` form (it does not parse as a
  CTE today and stays `Unmappable`).
- Lineage of `WHERE`, `JOIN ... ON`, `GROUP BY`, `HAVING` columns (not
  lineage here, `sql_lineage.rs:28-31`).
- Dropping D4a's no-`WITH` compile for today's shape (the `g7` gate and the
  legacy form edit depend on its text).
- Backfill: a pipeline gets rows at its next successful run, as `DATA-13`.
- Any change to the store, the route's JSON or the console.

## Handoff

## Review
