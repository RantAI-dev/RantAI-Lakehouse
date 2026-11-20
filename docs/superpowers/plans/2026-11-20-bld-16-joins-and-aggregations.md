# `BLD-16` (`BLD-1` part 2): joins, aggregations and filters as form steps, and a form step as SQL

Planner and reviewer: Claude Opus. Developers: GLM 5.3 for S1 (Rust, the
assistant, the gate); a separate **Opus developer agent** for S2 (new console
UI: three new step panes, product owner's rule of 2026-11-04 in `AGENTS.md`).
Nobody reviews their own slice.

Base: `origin/main` at `6b2242ff6`. Every `file:line` below was read at that
commit. Spec: `docs/core/specs/bld-1.md` (rows marked part 2). Feature page:
`docs/core/features/pipeline-joins-and-sql.md`, capabilities 10 to 13,
decisions 1 to 18. Part 1's plan,
`docs/superpowers/plans/2026-11-12-bld-1-part-1-steps-and-sql.md`, still holds:
its D1 (the shape), D2 (limits and names), D4/D4a/D4b (the compiler and
lineage), D6a (names are matched exactly), D7 (errors name the item), D8 (the
run route), D9/D10 (the check at save, who it reads as), D16 (the preview).
This plan only adds to them.

| Slice | Branch | What | Developer | Blocked by |
| --- | --- | --- | --- | --- |
| S1 | `feat/bld-16-form-steps-api` | Three new step kinds in the store and the compiler, the step-as-SQL route, the assistant's schema, the `g7` case | GLM 5.3 | The owner signing section 2's planner defaults (decisions 19 to 27 on the feature page); a `BLD-16` row in `BACKLOG.md` |
| S2 | `feat/bld-16-form-steps-console` | The three panes, "Show as SQL" and "Switch to SQL step" on every form step, the read view | An Opus developer agent | S1 merged; the product owner has looked at part 1's editor in a browser (the same hold the text-form plan has, `2026-11-13-bld-1-part-1-pipeline-as-text.md`) |

**Why two pull requests, not one.** The owner's rule of 2026-11-04 gives new
console UI to an Opus developer and everything else to the other agents, and
the reviewer may not build what it reviews. S1 is safe to merge alone: a
console that does not know a step's kind already shows it, does not edit it
and sends it back as stored (`knownStep`, `src/lib/pipeline-editor.ts:71`,
part 1 S3 review R1-S2), so a pipeline the assistant makes with a join step
between S1 and S2 is not damaged by a save from the part 1 editor.

## 1. The problem

Read from `main` at `6b2242ff6`:

- A step is one of two kinds: `transform` (the five operations, as grammar
  strings) or `sql` (`rust/crates/lakehouse-store/src/pipelines.rs:262`
  `PipelineStep`). There is no join, no aggregation, and a filter is one
  comparison; several `filter(...)` transforms are joined by `AND` only
  (`rust/crates/lakehouse-api/src/pipeline_compile.rs:546`
  `render_transform_step`; `transform_grammar.rs:192` `parse_filter` refuses
  `OR`). A person who cannot write SQL cannot build a join or a sum.
- The compiler renders a step in `render_step` (`pipeline_compile.rs:520`) and
  checks it in `validate` (`:262`, the `match step` at `:340` to `:377`). Both
  `match`es are exhaustive over the two kinds.
- `steps` is a `JSONB` column with no check on `kind`
  (`rust/migrations/0140_pipeline_inputs_and_steps.sql:114`), so new kinds
  need **no migration**.
- The console already keeps a kind it does not know as stored
  (`src/lib/pipeline-editor.ts:60` `EditorStep` `unknown`, `:71`
  `knownStep`); the read view shows its JSON
  (`src/features/pipelines/pipeline-definition.tsx:265`) and the flow draws it
  without edges (`src/features/pipelines/pipeline-flow-layout.ts:193`).
- Nothing turns a form step into SQL. The console has no SQL renderer of its
  own, and must not get one (`AGENTS.md` rule 4: one renderer, in the API).
- The assistant's `create_pipeline` (`rust/crates/lakehouse-api/src/routes/ai/registry.rs:1717`)
  offers `sql` and `transform` steps, and tells the model to use SQL for any
  join or group-by (`:1719`).

Findings, each handled below or named for the backlog:

- **F1. A form join without column names would produce duplicate or dotted
  column names.** `SELECT *` over a join in `ClickHouse` names a duplicate
  right-side column `<alias>.<column>`, and the compiler cannot know the
  columns (it is pure and reads no schema). J3 makes the output columns an
  explicit list.
- **F2. An aggregate's alias can capture a column.** In `ClickHouse` an alias
  is visible in the whole query, so `sum(amount) AS amount, avg(amount) AS a`
  makes the second `avg(sum(amount))`, an engine error. J4 refuses a measure
  name that is a column the step names.
- **F3. An outer join's missing side is the type's default, not empty.**
  `ClickHouse` fills it with `0` or `''` unless `join_use_nulls = 1`; the BI
  builder sets it for its own joins (`rust/crates/lakehouse-bi/src/builder.rs:475`,
  `BI-7`), a pipeline does not: the orchestrator appends its own `SETTINGS`
  after the compiled statement (`dagster/dispar_orchestrate/authored_factory.py:326`),
  so the compiler cannot add one. This is true of a SQL step on `main` today.
  J3 keeps the engine's default for both kinds and says so (planner default,
  owner may change; a change is its own backlog item, because it changes SQL
  steps too).
- **F4. A column that does not exist is accepted at save** (true of SQL steps
  today). The save runs the gate and the rewrite, which check tables, not
  columns. A wrong column fails the preview and the run with the engine's
  error, reported the way part 1 reports it. Not changed here; named under
  limits (J10).
- **F5. There is no `BLD-16` row in `docs/core/BACKLOG.md`** at `6b2242ff6`
  (nor anywhere under `docs/`). `AGENTS.md` loop step 0 needs one. The
  planner adds it in the docs PR that carries the feature page's new
  decisions (section 9); this plan does not edit `BACKLOG.md`.

## 2. Decisions

The product owner's, by number on the feature page: 1 (scope: joins, basic
aggregations), 3 (reads as its author), 4 (5 inputs), 6 (the assistant covers
the same part), 7 (an editor of named steps; a step is SQL or a form), 13
(the preview reads as the person looking), 14 (an edit moves who it reads
as), 15 (20 steps). The spec's part 2 rows: four join types, one or more
column pairs, equality only (Metabase's four); group by any columns with
`count`, `count distinct`, `sum`, `avg`, `min`, `max`, a filter on an
aggregated column; several filter conditions, all or any; a form step shown
as SQL and switched to a SQL step, one way.

Everything marked **(planner default, owner may change)** is not signed. The
planner proposes them as decisions 19 to 27 on the feature page (section 9);
**S1 does not start until the owner has signed them** (spec, "Before anyone
builds it").

- **J1. Three new step kinds** (planner default, owner may change): `join`,
  `aggregate` and `filter`, beside `transform` (the five operations stay,
  capability 4) and `sql`. A filter is its own kind, not a flag on
  `transform`, so a step stored with "any" is a kind the part 1 console keeps
  as stored instead of a `transform` step whose "any" it would drop on save,
  and so the legacy columns (`definition_columns`, `pipelines.rs:343`) and
  part 1's legacy-form edit (`legacy_form_over_stored`,
  `routes/authored_pipelines.rs:1147`) need no change.

- **J2. The stored shape.** Fields are strings and lists of strings in the
  store, exactly as `transforms` is, and every field has `#[serde(default)]`:
  the store holds what the route sent, and the **compiler** parses each field
  into a checked value and refuses it with a path (D7). A missing or wrong
  field is therefore a 400 that names `steps[i].<field>`, never serde's
  "body JSON is invalid".

  ```json
  { "name": "joined", "kind": "join", "left": "orders", "right": "customers",
    "joinType": "left",
    "on": [ { "left": "customer_id", "right": "id" } ],
    "columns": [
      { "side": "left",  "column": "order_id" },
      { "side": "left",  "column": "amount" },
      { "side": "right", "column": "country" },
      { "side": "right", "column": "id", "name": "customer_ref" } ] }

  { "name": "by_country", "kind": "aggregate", "reads": "joined",
    "groupBy": ["country"],
    "measures": [ { "function": "sum",   "column": "amount", "name": "revenue" },
                  { "function": "count",                     "name": "order_count" } ],
    "havingMatch": "all",
    "having": [ { "column": "revenue", "operator": ">", "value": "1000" } ] }

  { "name": "big_or_local", "kind": "filter", "reads": "by_country",
    "match": "any",
    "conditions": [ { "column": "revenue", "operator": ">=", "value": "5000" },
                    { "column": "country", "operator": "=",  "value": "ID" } ] }
  ```

  - `joinType`: `inner`, `left`, `right`, `full` (spec).
  - `on`: 1 to 10 pairs (planner default, owner may change); equality only
    (spec).
  - `columns`: 1 to 300 output columns (planner default, owner may change);
    `side` is `left` or `right`; `name` is optional and defaults to `column`.
  - `function`: `count`, `countDistinct`, `sum`, `avg`, `min`, `max`.
    `column` is optional for `count` only (`count()` counts rows; `count(c)`
    counts rows where `c` is not empty).
  - `groupBy`: 0 to 20 columns; `measures`: 0 to 50; at least one of the two
    (planner default, owner may change). No `groupBy` gives one row.
  - `match` and `havingMatch`: `all` or `any`; empty or missing means `all`
    (the assistant may leave it out). All and any are not nested (planner
    default, owner may change; Metabase does not nest either).
  - `conditions`: 1 to 20; `having`: 0 to 20 (planner default, owner may
    change). `operator` is one of the six the transform grammar already
    allows, `= != < > <= >=` (`transform_grammar.rs:54`
    `ALLOWED_FILTER_OPERATORS`), shared, not copied.
  - `value`: any text of at most 1,000 characters (planner default, owner may
    change). It is compared as `ClickHouse` compares a column with a string
    literal, exactly as `filter(col > '5')` does today.

- **J3. A join.** `left` and `right` each name an input or an earlier step,
  exactly (D6a); they may be the same item (a self-join). The output is the
  listed columns, each `` `__left`.`c` AS `name` `` or
  `` `__right`.`c` AS `name` ``. `__left` and `__right` are fixed aliases; no
  input or step may be called that, because D2 refuses a name starting with
  `__` (`pipeline_compile.rs:143` `valid_name`), so they cannot collide. The
  output names are unique without regard to case. Unmatched rows of an outer
  join get the type's default on the missing side (F3; planner default,
  owner may change). The console pre-fills `columns` (J9); the assistant
  lists them (J8).

- **J4. An aggregate.** `groupBy` columns are output as themselves, then each
  measure as `<expression> AS name`. Output names are unique without regard
  to case. A measure's `name` may not be, without regard to case, any column
  the step names (a `groupBy` column, a measure's `column`, a `having`
  column that is a group column) (F2). A `having` condition names a
  `groupBy` column or a measure's `name`, exactly, and is rendered with that
  column, or with **the measure's expression, not its alias**, so the alias
  question of F2 does not arise. A `having` value compared with a `count`,
  `countDistinct`, `sum` or `avg` must be a decimal number
  (`-?[0-9]{1,30}(\.[0-9]{1,30})?`, checked character by character, no new
  dependency) and is rendered as written, unquoted; every other value is a
  string literal.

- **J5. A filter** reads one input or earlier step and keeps the rows that
  meet all, or any, of its conditions. It outputs every column of what it
  reads (`SELECT *`).

- **J6. A step as SQL.** One function in the compiler,
  `pipeline_compile::step_sql`, returns the body the compiler puts inside a
  step's `WITH` entry: the `SELECT` over the inputs and earlier steps **by
  their names**, before the access rewrite (the rewritten text would show
  another person's masking expressions, and a SQL step is written over names,
  not tables). The CTE body of a form step and the text `step_sql` returns are
  produced by the same function, so they cannot differ. A new route,
  `POST /api/pipelines/step-sql` (`pipeline:write`, like the preview), takes
  `{ inputs, steps, step }` and answers `{ sql }`; nothing is stored and
  nothing is read from `ClickHouse`. "Switch to SQL step" is the console's:
  it replaces the form step with `{ name, kind: "sql", sql: <that text> }`,
  same name, same place, after a confirm that says it cannot be turned back
  (spec: one way). Every form kind has both, the five-operation `transform`
  step included ("any form step").

- **J7. What does not change.** The compile route (D8), the save check (D9,
  D10), the preview (D16) and the orchestrator take the new kinds with no
  code of their own: they all call `compile`/`validate_and_compile`, so the
  whole compiled statement still goes through the tenant gate and the access
  rewrite. A pipeline with a new kind is not today's shape
  (`is_legacy_shape`, `pipeline_compile.rs:223`, stays as it is), so it
  compiles to the `WITH` form and is not noted for column lineage (D4b,
  `DATA-57`); table lineage shows every input as before. No Python changes
  except the `g7` gate case.

- **J8. The assistant** (decision 6; **for the AI team's review**).
  `create_pipeline_schema` gains the three kinds and their fields (section
  4). Its description says: prefer a form step (`join`, `aggregate`,
  `filter`, `transform`) when it can say what the person asked, because the
  person can then change it in the form; use a SQL step for anything else
  (a `CASE`, a calculated column, a window function, a join that is not on
  equal columns, all and any mixed); list a join's output columns, which you
  look up first with `describe_mart`, `describe_dataset` or `run_sql`
  (planner default, owner and AI team may change). The tool already calls
  `routes::pipelines::create` (`routes/ai/tools/pipelines.rs:479`), so every
  check applies with no second copy. The fixture is regenerated. One
  `ops/ai_eval/cases.json` case (not run in CI, like `build_pipeline_draft`
  at `:497`).

- **J9. The console** (S2; decision 7). Every rule below is a pure function in
  `src/lib/`, with unit tests; components only show and edit
  (`CODE-STANDARD.md` 4.6).
  - The rail's add buttons (`src/features/pipelines/pipeline-editor.tsx:694`
    to `:701`) become **SQL**, **Join**, **Aggregate**, **Filter** and
    **Operations**. "Operations" is today's "Form step"; with four kinds of
    form the word "Form" names none of them (planner default, owner may
    change). The pane header (`pipeline-editor-panes.tsx:309`) says the same
    words.
  - **Join pane**: left and right (pickers of the inputs and earlier steps),
    the join type (four choices, each with one line: "only rows that match",
    "every row of the left", "every row of the right", "every row of both"),
    pairs of columns (add, remove), the output columns (a checkbox per column
    of each side, and its output name). When both sides' columns are known,
    a new join step, or a change of side, fills the output with every column
    of the left, then every column of the right; a right column whose name a
    left column already has is named `<right item name>_<column>`, then
    `_2`, `_3` while still taken. The pane says, for left, right and full:
    "A row with no match gets 0 or an empty text in the other side's columns,
    not a blank." (F3).
  - **Aggregate pane**: group-by columns (multi-pick), measures (function,
    column, name; a new measure's name is `<function>_<column>`, or `count`
    for `count` with no column, made unique), and "Only keep groups where"
    with all/any and conditions over the group columns and the measures.
  - **Filter pane**: all/any and the conditions (column, operator, value).
  - The column pickers offer the columns of what the step reads: an input's
    from `getTableDetail` (as today, `columnsRead`, `pipeline-editor.ts:668`),
    a step's from its last preview. When they are not known the pane shows a
    "Load columns" button that runs that item's preview (D16). A column name
    the API would refuse (not letters, digits and underscores) is not
    offered, and the pane says: "Columns whose names are not letters, digits
    and underscores are left out; use a SQL step to read them."
  - **Show as SQL** on every form step: calls the step-SQL route for the
    current state and shows the text read-only in `CodeView`
    (`src/components/patterns/code-view.tsx:23`), formatted with
    `formatSql` (`src/lib/sql-format.ts`) for display only. Stale after an
    edit, as the preview is.
  - **Switch to SQL step**: a confirm ("This step becomes a SQL step. It
    cannot be turned back into a form."); then the step is replaced with the
    route's text as returned (not the formatted one), same id, name and
    place. Nothing is saved until Save.
  - A refusal `steps[i].<field>[j]` (J2's paths) selects step `i` and marks
    row `j` of that field.
  - The read view (`pipeline-definition.tsx:227` `AuthoredFlow`) shows each
    new kind in words (`left join of orders and customers on customer_id =
    id`, `group by country: revenue = sum of amount, …`, `any of 2
    conditions`), and the flow (`pipeline-flow-layout.ts:193`) draws a join
    with two edges in and the other kinds with one.
  - Not in S2: a form that can say all and any together, a form for
    calculated columns, a run button in the editor.

- **J10. Limits to tell a customer** (added to the feature page in S2-T13): a
  form reads only columns whose names are letters, digits and underscores; an
  outer join's missing side is `0` or empty text, not blank (as in a SQL
  step); a column that does not exist is found by the preview or the run, not
  by the save; a pipeline with a join, aggregate or filter step has no column
  lineage yet (`DATA-57`); "Switch to SQL step" cannot be undone except by
  restoring an earlier version.

## 3. Security rules (conditions, not options)

Each rule has the tests that prove it (named in section 6). A change that
breaks one is a `BLOCKER` in review.

1. **One `SELECT`, through the access rewrite.** A form step is rendered into
   the same single compiled statement as every other step. No new code path
   sends SQL to `ClickHouse` except the existing preview and the existing
   compile route, and both put the whole compiled statement through
   `check_and_rewrite` (`routes/authored_pipelines.rs:1334`) or the
   preview's `rewrite_as_caller` (`routes/pipeline_preview.rs:165`). The
   step-SQL route sends nothing to `ClickHouse` and reads no table.
2. **A run and a save read as the author** (decisions 3, 14): unchanged code
   (`compile_pipeline`, `authored_pipelines.rs:2043`; D9). Proven again for
   a join step: a masked column of an input, carried through a join, comes
   back masked from the compile route.
3. **A preview reads as the person looking** (decision 13): unchanged code;
   proven again for an aggregate step: a column masked for the caller is
   masked in the preview's rows.
4. **No identifier from user text reaches SQL unchecked.** Every column, every
   output name, every step and input name in a form step goes through
   `lakehouse_core::ident::Ident::new` (`rust/crates/lakehouse-core/src/ident.rs:58`)
   — and names through D2's `valid_name` — before it is formatted, and is
   then written between backticks. A name that fails is refused with the
   path, never escaped, trimmed or guessed at. `format!` in the new code
   formats only checked values and constant text.
5. **No value from user text reaches SQL unescaped.** A pipeline's statement
   is one text the orchestrator runs (`INSERT … SELECT`), so a value cannot
   be a bind parameter here; every value goes through
   `lakehouse_core::ident::SqlLiteral` (`ident.rs:146`), the type today's
   `filter(...)` uses, or — only for J4's numbers — the decimal check, and is
   rendered unquoted only when every character is a digit, one leading `-` or
   one `.` between digits. Tests prove that hostile values (`'`, `\`, `\'`,
   `--`, `/*`, `;`, a line break) give exactly one string literal both to
   `sqlparser` (what the gate and the rewrite see) and in the text.
6. **Errors are fixed text.** Every refusal is a `DefinitionError` with a
   `&'static str` message from section 4's list and a path; the routes answer
   it through `definition_message` (`authored_pipelines.rs:1228`), so
   `tests/sec11_guard.rs` stays green with no allowlist entry.
7. **Every new route is classified.** `POST /api/pipelines/step-sql` is in
   `POLICY_TABLE` as `pipeline:write`, asserted both ways in
   `tests/route_auth.rs`, and in `audit_coverage.rs` as read-like, the
   preview's row (`audit_coverage.rs:263`).

## 4. Contracts

```
step = …part 1's two kinds…
     | { name, kind: "join", left: string, right: string,
         joinType: "inner" | "left" | "right" | "full",
         on: { left: string, right: string }[],
         columns: { side: "left" | "right", column: string, name?: string }[] }
     | { name, kind: "aggregate", reads: string,
         groupBy: string[],
         measures: { function: "count" | "countDistinct" | "sum" | "avg" | "min" | "max",
                     column?: string, name: string }[],
         havingMatch?: "all" | "any",
         having?: condition[] }
     | { name, kind: "filter", reads: string, match?: "all" | "any", conditions: condition[] }
condition = { column: string, operator: "=" | "!=" | "<" | ">" | "<=" | ">=", value: string }

POST /api/pipelines/step-sql            (pipeline:write)
  body { inputs: input[], steps: step[], step: string }
  200  { sql: string }
  400  body not JSON; `step` missing; any definition refusal (D7 path)
```

`POST /api/pipelines`, `PUT /api/pipelines/{id}`, `POST /api/pipelines/preview`,
`GET /api/pipelines/{id}` and the compile route are unchanged in shape; their
`steps` may now hold the new kinds.

### The rendered text (tests pin it byte for byte)

The bodies for J2's example:

```sql
SELECT `__left`.`order_id` AS `order_id`, `__left`.`amount` AS `amount`, `__right`.`country` AS `country`, `__right`.`id` AS `customer_ref` FROM `orders` AS `__left` LEFT JOIN `customers` AS `__right` ON `__left`.`customer_id` = `__right`.`id`
SELECT `country`, sum(`amount`) AS `revenue`, count() AS `order_count` FROM `joined` GROUP BY `country` HAVING sum(`amount`) > 1000
SELECT * FROM `by_country` WHERE `revenue` >= '5000' OR `country` = 'ID'
```

- One line, single spaces, no trailing space.
- Joins: `INNER JOIN`, `LEFT JOIN`, `RIGHT JOIN`, `FULL OUTER JOIN`; pairs
  joined by ` AND `.
- Measures: `count()`, `count(`c`)`, `count(DISTINCT `c`)`, `sum(`c`)`,
  `avg(`c`)`, `min(`c`)`, `max(`c`)`.
- No `groupBy`: no `GROUP BY`. No `having`: no `HAVING`. Conditions joined by
  ` AND ` (all) or ` OR ` (any), no brackets (one kind of joiner per list).
- In the compiled `WITH`, a form step's entry is
  `` `name` AS (<body>) ``, the shape a `transform` step has today
  (`pipeline_compile.rs:530`); a SQL step keeps `(\n<sql>\n)`.
- A `transform` step's body is exactly what it is today:
  `render_transform_step` over `` `reads` `` (columns unquoted). Its text
  does not change in this plan (D4a).

### The fixed sentences (paths in brackets)

| Constant | Path | Text |
| --- | --- | --- |
| `JOIN_TYPE` | `steps[i].joinType` | `a join is inner, left, right or full` |
| `JOIN_NO_PAIR` | `steps[i].on` | `a join needs at least one pair of columns to match on` |
| `JOIN_TOO_MANY_PAIRS` | `steps[i].on` | `a join matches on at most 10 pairs of columns` |
| `JOIN_NO_COLUMN` | `steps[i].columns` | `a join needs at least one output column` |
| `JOIN_TOO_MANY_COLUMNS` | `steps[i].columns` | `a join has at most 300 output columns` |
| `JOIN_SIDE` | `steps[i].columns[j]` | `an output column comes from the left or the right side` |
| `COLUMN_NAME` | `steps[i].on[j]`, `.columns[j]`, `.groupBy[j]`, `.measures[j]`, `.conditions[j]`, `.having[j]` | `a column must be a plain name (letters, digits and underscores, not starting with a digit)` |
| `DUPLICATE_OUTPUT` | `steps[i].columns[j]`, `.groupBy[j]`, `.measures[j]` | `another output column of this step has this name; give one of them another name` |
| `MATCH_MODE` | `steps[i].match`, `steps[i].havingMatch` | `match is all or any` |
| `NO_CONDITION` | `steps[i].conditions` | `a filter needs at least one condition` |
| `TOO_MANY_CONDITIONS` | `steps[i].conditions`, `steps[i].having` | `a step has at most 20 conditions` |
| `OPERATOR` | `….conditions[j]`, `….having[j]` | `a condition's operator is one of = != < > <= >=` |
| `VALUE_TOO_LONG` | `….conditions[j]`, `….having[j]` | `a condition's value is at most 1,000 characters` |
| `AGGREGATE_EMPTY` | `steps[i]` | `an aggregation needs a column to group by or a measure` |
| `TOO_MANY_GROUP_COLUMNS` | `steps[i].groupBy` | `an aggregation groups by at most 20 columns` |
| `TOO_MANY_MEASURES` | `steps[i].measures` | `an aggregation has at most 50 measures` |
| `MEASURE_FUNCTION` | `steps[i].measures[j]` | `a measure is count, countDistinct, sum, avg, min or max` |
| `MEASURE_NEEDS_COLUMN` | `steps[i].measures[j]` | `this measure needs a column` |
| `MEASURE_NAME_IS_COLUMN` | `steps[i].measures[j]` | `a measure's name cannot be a column this step reads; choose another name` |
| `HAVING_UNKNOWN` | `steps[i].having[j]` | `a condition here names a group-by column or a measure of this step` |
| `HAVING_NOT_A_NUMBER` | `steps[i].having[j]` | `a count, sum or average is compared with a number` |
| `STEP_UNKNOWN` | `step` | `step must name a step of this pipeline` |

`left`, `right` and `reads` reuse part 1's `READS_NOT_INPUT` and
`READS_LATER_STEP` (`pipeline_compile.rs:132`, `:133`) at `steps[i].left`,
`steps[i].right` and `steps[i].reads`. A measure `name` that fails D2's name
rule reuses `INVALID_NAME` (`:127`) at `steps[i].measures[j]`; an output
column `name` of a join that fails `Ident::new` uses `COLUMN_NAME`.

## 5. Anchors (`main` at `6b2242ff6`)

Rust (`rust/crates/`):

- `lakehouse-store/src/pipelines.rs`: `PipelineInput` `:235`; `PipelineStep`
  `:262` (the `serde(tag = "kind", rename_all = "camelCase")` at `:261`
  renames variants only, not fields); `impl PipelineStep` `:282` (`name`
  `:285`, `is_transform` `:292`); `definition_columns` `:343` (its `_` arm
  already gives no `transforms` for any other kind).
- `lakehouse-store/tests/pipelines.rs` (module of `tests/it.rs:48`): the
  `PipelineStep::Sql` round trip near `:3688`.
- `lakehouse-api/src/lib.rs:46` (`pub mod pipeline_compile;`), `:76`
  (`pub mod transform_grammar;`).
- `lakehouse-api/src/transform_grammar.rs:54` `ALLOWED_FILTER_OPERATORS`
  (private today).
- `lakehouse-api/src/pipeline_compile.rs`: module doc `:1` to `:57`; limits
  `:68` to `:72`; `DefinitionError` `:81` (`at` `:87`, `step` `:95`); the
  fixed sentences `:120` to `:139`; `valid_name` `:142`; `key` `:148`;
  `compile` `:163`; `is_legacy_shape` `:223`; `validate_and_compile` `:243`;
  `validate` `:262` (the per-step `match` `:340` to `:377`; the `transform`
  read check `:352` to `:368`); `render_step` `:520`;
  `render_transform_step` `:546`; tests from `:600`.
- `lakehouse-api/src/sql_guard.rs:568` `check_pipeline_step_sql`.
- `lakehouse-api/src/routes/authored_pipelines.rs`: `definition_message`
  `:1228`; `catalog_db` `:1239`; `check_and_rewrite` `:1334`; `step_tables`
  `:1370`; `refusal_path` `:1382`; `compile_pipeline` `:2043` (the lineage
  note `:2116`).
- `lakehouse-api/src/routes/pipeline_preview.rs`: `PreviewBody` `:77`;
  `preview` `:102`; `rewrite_as_caller` `:165`.
- `lakehouse-api/src/routes/mod.rs`: `mod pipeline_preview;` `:88`;
  `pipeline_preview_router` `:440`; its merge `:474`.
- `lakehouse-api/src/policy.rs:603` (the preview's `POLICY_TABLE` row).
- `lakehouse-api/src/audit_coverage.rs:263` (the preview's coverage row).
- `lakehouse-api/tests/route_auth.rs:4733`
  `the_pipeline_preview_route_authorization_is_asserted_both_ways`.
- `lakehouse-api/tests/pipeline_preview.rs` (helpers `app` `:73`,
  `mount_preview` `:110`, `preview` `:135`, `seed_mask` `:189`).
- `lakehouse-api/tests/pipeline_compile_route.rs` (`seed_pipeline_reading`
  `:195` with `FixtureStep` `:185`;
  `a_join_of_two_governed_inputs_comes_back_with_both_rewritten` `:889`;
  `a_masked_column_inside_a_sql_step_comes_back_masked` `:1029`).
- `lakehouse-api/tests/pipeline_definition_save.rs`
  (`new_body` `:129`; `saving_a_table_the_author_may_not_read_is_422_naming_the_input` `:260`).
- `lakehouse-api/src/routes/ai/registry.rs`: `create_pipeline_schema`
  `:1717`; `write_tool_schema_fixture` `:3309`;
  `tool_schemas_snapshot_is_byte_identical` `:3321`. Fixture:
  `lakehouse-api/tests/fixtures/tool_schemas.json`.
- `lakehouse-api/src/routes/ai/tools/pipelines.rs`: `create_pipeline` `:467`;
  `create_pipeline_accepts_inputs_and_a_sql_step` `:1179` (the pattern for
  the new tool test).
- `lakehouse-api/src/routes/ai/prompt.rs:550` (the pipelines line of the
  build prompt); the pipeline domain `:983` to `:999`; `ALWAYS` `:564`
  (`describe_dataset`, `describe_mart` and `run_sql` are always offered, so
  the domain needs no new tool).

Python and gates:

- `ops/g7/g7_pipeline_test.py:1214`
  `step_a_two_input_sql_pipeline_matches_a_direct_query` (the pattern);
  `main` calls it at `:1535`.
- `ops/ai_eval/cases.json:497` `build_pipeline_draft` (the pattern).

Console (`src/`):

- `services/contracts/pipelines.ts`: `PipelineTransformStep` `:168`,
  `PipelineSqlStep` `:171`, `PipelineOtherStep` `:182`, `PipelineStep`
  `:189`; `PipelineService.previewPipelineStep` `:478`.
- `services/clients/pipelines.ts:177` `previewPipelineStep`; its tests
  `services/clients/pipelines.test.ts:108`.
- `lib/pipeline-editor.ts`: `EditorStep` `:60`; `knownStep` `:71`;
  `editorStateFrom` `:110`; `readableItems` `:195`; `readsName` `:214`;
  `addStep` `:243`; `renameItem` `:272`; `updateStep` `:312`;
  `ProblemField` `:361`; `editorProblems` `:381`; `definitionBody` `:491`;
  `errorTarget` `:639`; `columnsRead` `:668`; `inputIsNamed` `:826`. Tests:
  `lib/pipeline-editor.test.ts`.
- `features/pipelines/pipeline-editor.tsx`: `onAddStep` `:359`; the step pane
  switch `:444` to `:470`; the add buttons `:694` to `:701`.
- `features/pipelines/pipeline-editor-panes.tsx`: `StepHeader` `:291` (kind
  label `:309`); `TransformStepPane` `:337`; `SqlStepPane` `:421`;
  `UnknownStepPane` `:528`.
- `features/pipelines/pipeline-definition.tsx`: the kind label `:264`; the
  unknown-kind branch `:265`; the transform branch `:277`.
- `features/pipelines/pipeline-flow-layout.ts:193` to `:215`.
- `features/pipelines/pipeline-create-page.test.tsx` (the editor's page
  tests).
- `components/patterns/code-view.tsx:23` `CodeView`; `lib/sql-format.ts`.

## 6. Tasks

One task per commit, in order. **Push after every commit** (worker sessions
end without warning). Per commit, run only the scoped checks named in the
task (`AGENTS.md`, "Keep build time down"); all cargo through the machine's
shared `CARGO_TARGET_DIR` and, where the machine has it, the `cargo-locked`
wrapper; never `cargo clean`. The reviewer opens a draft pull request after
the first code commit is pushed, so CI runs the whole suite and the gates.

If the code at any anchor is not what this plan says, **stop and report the
mismatch**; do not improvise a different design.

### S1 (`feat/bld-16-form-steps-api`, GLM 5.3)

The Rust tasks T1 to T5 sit together so `lakehouse-api` is built in one
sitting; T5 (the assistant) is Rust in the same crate, so it comes before the
console rather than after it. T6 is Python.

- **S1-T1. The stored shape (J1, J2).** In
  `lakehouse-store/src/pipelines.rs`:
  - New public types, each `#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]`
    and `#[serde(rename_all = "camelCase")]`, every field `#[serde(default)]`,
    each with a doc comment: `JoinPair { left: String, right: String }`,
    `JoinColumn { side: String, column: String, name: String }` (`name` also
    `#[serde(default, skip_serializing_if = "String::is_empty")]`),
    `StepCondition { column: String, operator: String, value: String }`,
    `Measure { function: String, column: String, name: String }` (`column`
    also `skip_serializing_if = "String::is_empty"`).
  - Three new variants of `PipelineStep`, after `Sql`:
    `Join { name, left, right, join_type, on: Vec<JoinPair>, columns: Vec<JoinColumn> }`,
    `Aggregate { name, reads, group_by: Vec<String>, measures: Vec<Measure>, having_match: String, having: Vec<StepCondition> }`,
    `Filter { name, reads, match_mode: String, conditions: Vec<StepCondition> }`.
    Every field except `name` is `#[serde(default)]`. Multi-word fields get
    an explicit `#[serde(rename = "…")]`: `joinType`, `groupBy`,
    `havingMatch`, and `match_mode` → `"match"`. Do **not** add
    `rename_all_fields`; the explicit renames are what a reader checks.
    `having_match` and `having` are `skip_serializing_if` empty, so a stored
    aggregate with no having has no such keys.
  - `PipelineStep::name` covers the new variants. `is_transform` is
    unchanged.
  - Module and type docs say why the fields are strings (the compiler
    checks them and names the field, J2).

  *Check:* new tests in `lakehouse-store/tests/pipelines.rs`:
  `a_join_step_round_trips_through_json_in_the_api_shape` (J2's join JSON
  decodes to `PipelineStep::Join` with each field, and encodes back to the
  same JSON value), `an_aggregate_step_round_trips_through_json_in_the_api_shape`,
  `a_filter_step_reads_match_from_the_match_key`,
  `a_new_kind_with_missing_fields_decodes_with_empty_fields`
  (`{"name":"j","kind":"join"}` decodes, fields empty),
  `a_definition_with_a_join_step_is_stored_and_read_back` (create through
  `create_pipeline`, read with `get_definition`, equal),
  `a_definition_with_a_new_kind_writes_no_legacy_transforms`
  (`definition_columns` gives an empty `transforms`). Each of the first four
  fails on `main` (`unknown variant`).
  So that every commit compiles, T1 also adds the three arms to the two
  exhaustive `match`es of `pipeline_compile.rs` (`validate` `:340`,
  `render_step` `:521`), each refusing with a new constant
  `KIND_NOT_BUILT = "this kind of step is not supported yet"` at
  `steps[i]` ("unsupported, honestly", `AGENTS.md` rule 14). T3 replaces
  both arms and removes the constant; no commit after T3 may still contain
  it.
  `cargo fmt --check && cargo clippy -p lakehouse-store -p lakehouse-api --all-targets -- -D warnings && cargo test -p lakehouse-store --test it pipelines::`.

- **S1-T2. The form renderer (J2 to J5; security rules 4 and 5).** New
  module `lakehouse-api/src/pipeline_forms.rs`, `pub mod pipeline_forms;` in
  `lib.rs` next to `pipeline_compile` (`:46`). It is pure: no I/O, no state.
  - In `transform_grammar.rs`, make `ALLOWED_FILTER_OPERATORS` `pub(crate)`
    and add `pub(crate) fn filter_operator(text: &str) -> Option<&'static str>`
    (the operator from the list equal to `text.trim()`). Use it from
    `pipeline_forms`; do not write a second list.
  - Typed values, all `pub` with a doc comment (a `pub` item of a `pub mod`
    is not dead code before T3 calls it, so `clippy -D warnings` stays green
    on this commit): `enum JoinKind { Inner, Left, Right, Full }`,
    `enum Side { Left, Right }`, `enum MatchMode { All, Any }` (`""` and
    `"all"` → `All`, `"any"` → `Any`, anything else refused),
    `enum MeasureFunction { Count, CountDistinct, Sum, Avg, Min, Max }`,
    `struct Condition { column: Ident, operator: &'static str, value: ConditionValue }`,
    `enum ConditionValue { Text(SqlLiteral), Number(String) }`.
  - Parsers, each taking the stored fields and the step index and returning
    the typed form or a `DefinitionError` with the path and constant of
    section 4: `parse_join`, `parse_aggregate`, `parse_filter`, and the shared
    `parse_condition(index, field, j, &StepCondition)`. They check every rule
    of J2 to J5 **except** `left`/`right`/`reads` (the compiler checks those,
    T3). `DefinitionError::at` is private to `pipeline_compile`; make it
    `pub(crate)` rather than adding a second constructor.
  - Renderers: `render_join(left_name, right_name, &JoinForm) -> String`,
    `render_aggregate(reads, &AggregateForm) -> String`,
    `render_filter(reads, &FilterForm) -> String`, producing exactly section
    4's text. Identifiers are formatted only from `Ident` values (and step or
    input names already checked by `valid_name`), between backticks;
    `ConditionValue::Text` through `SqlLiteral`'s `Display`;
    `ConditionValue::Number` as checked. Parsers and renderers are `pub` too,
    each with `# Errors` where it returns a `Result`.
  - `is_decimal(text: &str) -> bool`: J4's rule, by characters; no `regex`
    dependency.
  - Module doc: what each kind is, why the output columns of a join are
    listed (F1), why a measure's name may not be a column (F2), why `having`
    repeats the expression, why outer joins give defaults (F3), and section
    3's rules 4 and 5 as the invariant this module keeps.

  *Check:* unit tests in `pipeline_forms.rs` (`#[cfg(test)]`):
  - Text, byte for byte: `a_left_join_renders_the_listed_columns_and_the_pairs`
    (J2's join), `each_join_type_renders_its_keyword` (four),
    `two_pairs_are_joined_by_and`, `a_self_join_uses_two_aliases`,
    `an_aggregate_renders_group_by_measures_and_having_with_the_expression`
    (J2's aggregate), `each_measure_function_renders_its_expression` (seven
    forms, `count` with and without a column), `an_aggregate_with_no_group_by_has_no_group_by_clause`,
    `a_filter_with_any_joins_by_or` and `a_filter_with_all_joins_by_and`,
    `a_missing_match_means_all`.
  - Refusals, each asserting the exact path and constant: a join type
    `cross`; no pair; eleven pairs; no output column; a side `middle`; a
    column `amount; DROP TABLE x`, `` a`b ``, `2col`, `""` in each of `on`,
    `columns`, `groupBy`, `measures`, `conditions`, `having`; two output
    columns `id` and `ID`; a function `median`; `sum` with no column; a
    measure named `amount` that sums `amount`; a measure named `country` when
    grouping by `country` (`DUPLICATE_OUTPUT`); a having on `nope`; a having
    `revenue > '1000 OR 1=1'` on a `sum` (`HAVING_NOT_A_NUMBER`); an operator
    `LIKE`; a value of 1,001 characters (and 1,000 accepted); twenty-one
    conditions; an aggregate with neither group-by nor measures; a match
    `some`.
  - Security rule 5: `a_hostile_value_is_one_string_literal` — for each of
    `x' OR '1'='1`, `a\`, `a\'b`, `-- c`, `/* c`, `;DROP TABLE t`, `line\nbreak`:
    render a filter, parse the text with `sqlparser` (`ClickHouseDialect`),
    assert one `Statement::Query` whose `WHERE` is one `BinaryOp` with a
    `Value::SingleQuotedString` right side equal to the original value.
  - `is_decimal`: `1000`, `-2.5` yes; `1e3`, `1.`, `.5`, `1 OR 1`, `''`,
    31 digits no.

  `cargo clippy -p lakehouse-api --all-targets -- -D warnings && cargo test -p lakehouse-api --lib -- pipeline_forms transform_grammar`.

- **S1-T3. The compiler takes the new kinds; a step as SQL (J6, J7).** In
  `pipeline_compile.rs`:
  - Factor the `transform` arm's read check (`:352` to `:368`) into
    `fn check_read(index: usize, field: Option<&str>, reads: &str, inputs, steps) -> Result<(), DefinitionError>`;
    `field: None` keeps today's path `steps[i]` for `transform` (no existing
    message changes), `Some("left")`, `Some("right")`, `Some("reads")` give
    `steps[i].left` and so on. Exact match, case included (D6a).
  - `validate`'s `match step` gains three arms: `Join` checks `left` and
    `right` with `check_read`, then `pipeline_forms::parse_join`;
    `Aggregate` and `Filter` check `reads`, then their parser.
  - `fn step_body(step, index) -> Result<String, DefinitionError>`: the
    `SELECT` of one step over names: `transform` → today's
    `render_transform_step(&format!("`{reads}`"), …)`, `sql` → its text as
    stored, the three new kinds → their renderer over their parsed form.
    `render_step` calls it: `sql` keeps `` `name` AS (\n{sql}\n) ``, every
    other kind `` `name` AS ({body}) ``. The `transform` text does not change.
  - `pub fn step_sql(inputs, steps, step_name) -> Result<String, DefinitionError>`:
    `validate(inputs, steps, Some(step_name))`; refuse with `STEP_UNKNOWN` at
    path `step` when `step_name` is not a step's name exactly (an input's
    name included); return `step_body` of that step. Doc: J6, and that the
    text is before the access rewrite on purpose.
  - The module doc's list of rules gains the new kinds and points at
    `pipeline_forms`.

  *Check:* unit tests in `pipeline_compile.rs`:
  `a_join_step_compiles_into_the_with_form` (two inputs and J2's join,
  whole statement pinned), `j2s_three_steps_compile_into_one_statement`
  (the three steps of J2 after a two-input definition, whole statement
  pinned), `a_join_reading_a_later_step_is_refused_at_its_left_or_right`,
  `a_join_reading_a_name_in_another_case_is_refused`,
  `an_aggregate_reading_an_unknown_name_is_refused_at_reads`,
  `the_legacy_predicate_is_false_for_join_aggregate_and_filter_steps`,
  `up_to_a_join_step_stops_there`,
  `step_sql_of_a_form_step_is_its_with_body` (for each of the four form
  kinds: `step_sql` equals the text between `AS (` and the matching `)` of
  that step in `compile`'s output),
  `switching_a_form_step_to_sql_compiles_to_the_same_statement` (for each
  of the four form kinds: replace the step with
  `PipelineStep::Sql { name, sql: step_sql(..) }` at the same index; both
  definitions pass `validate_and_compile`; parse both compiled statements
  with `sqlparser` and assert their `to_string()` are equal — this proves the
  SQL a person switches to is accepted as a SQL step, D5 and D6, and computes
  the same thing),
  `step_sql_of_an_input_name_is_refused`, `step_sql_of_an_unknown_name_is_refused`,
  `step_sql_refuses_a_bad_earlier_step` (a broken step before the named one
  is refused at its own index). Every existing test of `pipeline_compile`,
  `transform_grammar` and `sql_guard` passes unchanged.
  `cargo fmt --check && cargo clippy -p lakehouse-api --all-targets -- -D warnings && cargo test -p lakehouse-api --lib -- pipeline_compile pipeline_forms transform_grammar sql_guard`.

- **S1-T4. The step-SQL route; the new kinds through save, run and preview
  (J6, J7; security rules 1, 2, 3, 6, 7).**
  - `routes/pipeline_preview.rs`: `struct StepSqlBody { inputs, steps, step: String }`
    (camelCase, `#[serde(default)]` on each, like `PreviewBody` `:77`) and
    `pub async fn step_sql(State(_state), Extension(_principal), body: Bytes) -> ApiResult<ApiJson<Value>>`:
    body not JSON → 400 `body JSON is invalid`; `step` blank → 400
    `step is required.`; `pipeline_compile::step_sql` refusal → 400 through
    `definition_message`; else `{ "sql": … }`. No `ClickHouse`, no
    Postgres, no tenant: say so in its doc, and why it still needs
    `pipeline:write` (it is part of authoring, and it shows the pipeline's
    names). The module doc gains the route.
  - `routes/mod.rs:440`: add
    `.route("/api/pipelines/step-sql", axum::routing::post(pipeline_preview::step_sql))`
    to `pipeline_preview_router`, and widen its doc.
  - `policy.rs`, after `:603`:
    `("POST", "/api/pipelines/step-sql", Policy::RequiresPermission("pipeline:write")),`
  - `audit_coverage.rs`, after `:263`:
    `("POST", "/api/pipelines/step-sql", Coverage::Excluded { reason: EX_READ_LIKE }),`

  *Check:* tests, all in existing files (no new test module, so `tests/it.rs`
  is unchanged):
  - `tests/route_auth.rs`:
    `the_pipeline_step_sql_route_authorization_is_asserted_both_ways`, a copy
    of `:4733` for the new path (no session 401, `query:read` 403,
    `pipeline:write` past the gate). Fails on `main` (no row).
  - `tests/pipeline_preview.rs`:
    `step_sql_answers_the_sql_of_a_join_step` (J2's join, exact text),
    `step_sql_answers_400_naming_the_field_at_fault` (a join type `cross`
    → `steps[0].joinType: a join is inner, left, right or full`),
    `step_sql_sends_nothing_to_the_engine` (the mock `ClickHouse` received no
    request), `previewing_an_aggregate_step_runs_it_as_the_caller_with_their_mask`
    (a column masked for the caller, `seed_mask` `:189`, read by a join then
    an aggregate's `min`: the SQL sent to the engine carries the mask
    expression; security rule 3), `previewing_a_filter_step_sends_its_where`.
  - `tests/pipeline_compile_route.rs`: extend `FixtureStep` (`:185`) with
    `Steps(Vec<PipelineStep>)`; add
    `a_masked_column_carried_through_a_join_step_comes_back_masked`
    (security rule 2), `a_join_of_two_governed_inputs_through_a_join_step_comes_back_with_both_rewritten`
    (the form twin of `:889`), `a_form_step_pipeline_is_not_noted_for_column_lineage`
    (no row in the column-lineage store after the compile).
  - `tests/pipeline_definition_save.rs`:
    `a_join_step_reading_a_table_the_author_may_not_read_is_422_naming_the_input`
    (twin of `:260`), `a_join_step_with_a_bad_column_is_400_naming_the_column`
    (`steps[0].on[0]: a column must be …`), `a_pipeline_with_join_aggregate_and_filter_steps_is_saved_and_read_back`
    (the detail returns the same steps), `an_edit_of_a_form_step_by_another_user_moves_the_author`
    (D10 still applies to the new kinds).
  - `tests/sec11_guard.rs` passes with no new allowlist entry.
  `cargo fmt --check && cargo clippy -p lakehouse-api --all-targets -- -D warnings && cargo test -p lakehouse-api --test it -- route_auth:: pipeline_preview:: pipeline_compile_route:: pipeline_definition_save:: sec11_guard:: audit_coverage::`.

- **S1-T5. The assistant (J8). For the AI team's review.**
  - `routes/ai/registry.rs:1717` `create_pipeline_schema`: the step item's
    `kind` enum becomes `["sql", "transform", "join", "aggregate", "filter"]`;
    new properties on the step item, each with a short description naming
    the kind it belongs to: `left`, `right`, `joinType` (enum of four), `on`
    (array of `{ left, right }`), `columns` (array of `{ side` enum,
    `column`, `name }`, required `side` and `column`), `groupBy` (array of
    string), `measures` (array of `{ function` enum of six, `column`, `name }`,
    required `function` and `name`), `havingMatch` and `match` (enum
    `all`, `any`), `having` and `conditions` (array of
    `{ column, operator` enum of six, `value` string `}`, all required). The
    description replaces "Use a SQL step whenever the person asks for a join,
    a group-by, an aggregate…" with J8's guidance, and states the limits:
    equality joins only; list a join's output columns after looking them up;
    a measure's name cannot be a column it reads; a having names a group-by
    column or a measure; values are text, except a count, sum or average is
    compared with a number. Keep the schema's existing fields and order.
  - `routes/ai/prompt.rs:550`: the pipelines line adds one clause: "a
    pipeline step may be a join, an aggregate or a filter form, or SQL".
  - Regenerate the fixture:
    `cargo test -p lakehouse-api --lib write_tool_schema_fixture -- --ignored`,
    then commit `tests/fixtures/tool_schemas.json`. The diff of the fixture
    must touch only `create_pipeline`; the handoff says so.
  - `routes/ai/tools/pipelines.rs`: test
    `create_pipeline_accepts_join_and_aggregate_form_steps` (pattern `:1179`):
    two inputs, a `join` and an `aggregate` with a `having`; the created
    definition has those steps, and the pipeline is a draft.
    `create_pipeline_refuses_a_form_step_with_the_field_named`: a measure
    `median` comes back as an error naming `steps[1].measures[0]`.
  - `ops/ai_eval/cases.json`: case `build_pipeline_join_aggregate_form`,
    `"note": "not run in CI"`, mode `build`, turn: "Create a pipeline named
    ai_eval_sales_by_country that joins silver orders and silver customers on
    customer_id, sums amount per country, keeps countries over 1000, and
    writes gold.ai_eval_sales_by_country. Keep it manual."; `expect_tool`
    `create_pipeline` with `args_contains` `{ "targetTable": "ai_eval_sales_by_country" }`;
    `forbid` `["approvals"]`. If `ops/ai_eval/test_ai_eval.py` validates the
    file, run it.

  *Check:* `cargo clippy -p lakehouse-api --all-targets -- -D warnings && cargo test -p lakehouse-api --lib -- routes::ai::registry routes::ai::tools::pipelines routes::ai::prompt`
  (includes `tool_schemas_snapshot_is_byte_identical`, which fails on
  `main` against the new fixture); `(cd ops/ai_eval && python -m pytest test_ai_eval.py -q)`.

- **S1-T6. The gate (`g7`).** In `ops/g7/g7_pipeline_test.py`, after
  `step_a_two_input_sql_pipeline_matches_a_direct_query` (`:1214`), add
  `step_a_form_step_pipeline_matches_its_sql_twin_and_a_direct_query()` and
  call it from `main` after `:1535`. It creates, on the same Bronze table
  under two names as the existing case, a pipeline with steps: a `join`
  (`inner`, on `id` = `id`, output `customer` from the left, `amount` from
  the left, and the right's `id` named `id` — an output name equal to a key
  column, the alias question of F2 for joins), an `aggregate` (group by
  `customer`, `sum` of `amount` named `total`, `count` named `rows_n`,
  having `rows_n >= 1`), and a `filter` (`any` of `total > '0'` and
  `customer != ''`). It runs it, then calls `POST /api/pipelines/step-sql`
  for each step, creates a second pipeline whose steps are those SQL texts
  as SQL steps, runs it, and asserts: both outputs equal each other and equal
  the same query run directly against `CATALOG_DB` (the pattern at `:1303`).
  Name the targets `g7_form_steps_out` and `g7_form_steps_sql_out`.

  *Check:* `python3 -m py_compile ops/g7/g7_pipeline_test.py`;
  `python3 ops/lint/check_intra_package_imports.py && python3 ops/lint/check_bare_iceberg_count.py`.
  The gate itself runs in CI job `g3a-dagster` on the draft PR; locally it is
  *not verified* unless the developer brings the stack up, and the handoff
  says which.

Verification before the S1 handoff, on the final commit, in the foreground:

```bash
cd rust && cargo fmt --check
cargo clippy -p lakehouse-store -p lakehouse-api --all-targets -- -D warnings
cargo test -p lakehouse-store --test it pipelines::
cargo test -p lakehouse-api --lib -- pipeline_compile pipeline_forms transform_grammar sql_guard routes::ai
cargo test -p lakehouse-api --test it -- route_auth:: pipeline_preview:: pipeline_compile_route:: pipeline_definition_save:: column_lineage_pipeline:: sec11_guard:: audit_coverage::
cd .. && python3 ops/lint/check_intra_package_imports.py && python3 ops/lint/check_bare_iceberg_count.py
python3 -m py_compile ops/g7/g7_pipeline_test.py
(cd ops/ai_eval && python -m pytest test_ai_eval.py -q)
git status
```

The whole workspace (`cargo clippy --workspace --all-targets --all-features`,
the three `nextest` partitions) and the gates run in CI on the draft pull
request; the reviewer re-runs the block above before merge.

### S2 (`feat/bld-16-form-steps-console`, an Opus developer agent)

- **S2-T7. Contracts and client.** `services/contracts/pipelines.ts`:
  `PipelineJoinStep`, `PipelineAggregateStep`, `PipelineFilterStep`,
  `PipelineStepCondition`, `PipelineMeasure`, `PipelineJoinPair`,
  `PipelineJoinColumn` (`type`, camelCase as section 4, string unions with
  `| string` widening where the API may add values, `CODE-STANDARD.md` 4.2);
  `PipelineStep` gains the three. `PipelineService.stepSql(input: PipelineStepSqlInput, signal?)`
  `→ Promise<{ sql: string }>` with a doc like `:473`. `services/clients/pipelines.ts`:
  `stepSql` through `postJson` (as `:177`).
  *Check:* `services/clients/pipelines.test.ts`:
  `posts the definition and step to /api/pipelines/step-sql and returns the sql`,
  and a 400's message reaches the caller as the preview's does (`:132`).
  `bun run typecheck && bun test src/services/clients/pipelines.test.ts`.

- **S2-T8. The editor's rules (J9), pure, in `lib/pipeline-editor.ts`.**
  - `EditorStep` gains `join` (`left: StepRead`, `right: StepRead`,
    `joinType`, `on`, `columns`), `aggregate` (`reads: StepRead`, `groupBy`,
    `measures`, `havingMatch`, `having`) and `filter` (`reads: StepRead`,
    `match`, `conditions`). Reads follow the item by id, as `transform` does
    (part 1 S3 review R1-B2).
  - `knownStep` accepts each new kind only when every field has its type
    (as it does for `transform`, `:74`); anything else stays `unknown`.
  - `editorStateFrom`, `renameItem`, `updateStep` (a patch type per kind),
    `readsName`, `inputIsNamed` (`:826`, a join reads by id on both sides)
    and `definitionBody` cover the new kinds. `addStep(state, kind)` accepts
    `"join" | "aggregate" | "filter"`; a new join reads the last item on the
    left and nothing on the right; a new aggregate or filter reads the last
    item.
  - `editorProblems`: per new kind, only what the page can say without the
    API: a read not chosen or later (the `transform` sentences, `:399` to
    `:403`); a join with no pair or no output column; an aggregate with
    neither group-by nor measure; a filter with no condition; an empty
    column or operator in a row. Everything else is the API's (D7). New
    `ProblemField` values: `left`, `right`, `on`, `columns`, `groupBy`,
    `measures`, `having`, `conditions`.
  - `joinOutputColumns(leftName, leftCols, rightName, rightCols)`: J9's fill
    rule. `measureName(fn, column, taken)`: J9's naming rule.
    `formColumns(columns)`: drops the names `Ident::new` would refuse
    (same regular expression as `IDENT`, `:136`) and says whether any was
    dropped.
  - `columnsRead` generalizes to every form kind and to a join's two sides.
  - `switchToSql(state, id, sql)`: the step becomes `{ kind: "sql", id, name, sql }`
    at its place; no other item changes.
  - `errorTarget` (`:639`) also returns `field` and `row` for
    `steps[i].(on|columns|groupBy|measures|conditions|having)[j]`; the
    `transforms[j]` case is unchanged.

  *Check:* `lib/pipeline-editor.test.ts`: a stored join, aggregate and filter
  open as those kinds and are sent back equal (the API's JSON, J2); a stored
  join with `on` that is not a list stays `unknown`; renaming an input a
  join reads keeps the join reading it; removing it gives the "Choose an
  input" problem; `joinOutputColumns` names a colliding right column
  `customers_id`, then `customers_id_2`; `measureName` gives `sum_amount`,
  `count`, `count_2`; `formColumns` drops `order date`; `switchToSql` keeps
  the id, the name and the place; `errorTarget` of
  `steps[2].measures[1]: …` is step 2, field `measures`, row 1; every
  existing test passes unchanged.
  `bun run typecheck && bun test src/lib/pipeline-editor.test.ts`.

- **S2-T9. The three panes and the rail (J9).** `pipeline-editor-panes.tsx`:
  `JoinStepPane`, `AggregateStepPane`, `FilterStepPane`, built from the
  existing patterns (`SelectField`, `Input`, `Label`, `PaneProblem`,
  `tie`); no native `<select>` (the lint refuses it). A shared
  `ConditionRows` for filter and having. `pipeline-editor.tsx`: the step
  pane switch (`:444`) and the add buttons (`:694`) per J9; "Load columns"
  runs the existing preview for the read item.
  *Check:* `features/pipelines/pipeline-create-page.test.tsx`: adding a join
  of two inputs whose columns are known fills the output columns and saves
  the J2 shape; the outer-join sentence shows for left, right and full and
  not for inner; adding a measure names it `sum_amount`; a column with a
  space is not offered and the sentence says why; a filter's any/all is
  sent as `match`; a 400 `steps[0].on[0]: …` selects the join and marks its
  first pair; the add buttons read SQL, Join, Aggregate, Filter,
  Operations. Tests that looked for the label "Form step" or "· Form" are
  changed to "Operations" in this commit, and the commit body names each one
  and why (`AGENTS.md` rule 2: the label changed on purpose).
  `bun run typecheck && bun run lint && bun test src/features/pipelines/`.

- **S2-T10. Show as SQL and Switch to SQL step (J6, J9).** On every form
  pane (the three new ones and `TransformStepPane`, `:337`).
  *Check:* page tests: Show as SQL calls `stepSql` with the current
  definition and the step's name and shows the text; it is marked stale after
  an edit; Switch asks first, then the rail shows the step as SQL with the
  route's text in the `SqlEditor`, at the same place and name, and Save
  sends `kind: "sql"`; cancelling the confirm changes nothing; a 400 from
  the route is shown in the pane.
  `bun test src/features/pipelines/`.

- **S2-T11. The read view and the flow (J9).** `pipeline-definition.tsx`
  and `pipeline-flow-layout.ts`.
  *Check:* `features/pipelines/pipeline-flow-layout.test.ts`: a join draws
  two edges in, an aggregate and a filter one each, with J9's sublabels;
  `pipeline-detail-page.test.tsx`: a pipeline of J2's steps shows each step
  in words, and a kind still unknown still shows its JSON.
  `bun test src/features/pipelines/`.

- **S2-T12. Words.** The feature page's "Limits to tell a customer" gains
  J10's lines; its Plan row names this plan; module docs of every touched
  file say why (principle 1). No change to decisions or the checklist (the
  planner's).
  *Check:* `git diff --stat` touches only `docs/core/features/pipeline-joins-and-sql.md`
  and comments.

Verification before the S2 handoff, on the final commit, in the foreground:

```bash
bun run typecheck && bun run lint
bun test src/lib/ src/features/pipelines/ src/services/clients/pipelines.test.ts
python3 ops/lint/check_bare_iceberg_count.py && python3 ops/lint/check_intra_package_imports.py
git status
```

The whole console suite runs in CI. How the panes look in a browser is the
developer's to state: which panes were opened, in which theme, against a
running API or not; otherwise *not verified*.

## 7. Tests that fail on `main`

Each of these fails at `6b2242ff6` and passes after its task, which is how the
reviewer checks that the test tests the change:

- The store round trips of S1-T1 (`unknown variant 'join'`).
- Every `pipeline_forms` and new `pipeline_compile` test (no such kinds, no
  `step_sql`).
- `the_pipeline_step_sql_route_authorization_is_asserted_both_ways` (no
  `POLICY_TABLE` row and no route).
- The `step_sql_*` route tests (no route), and the save, compile and preview
  tests with a join, aggregate or filter step (the body does not parse).
- `tool_schemas_snapshot_is_byte_identical` against the regenerated fixture,
  and the two new tool tests.
- S2's tests (no such kinds in the console; no `stepSql`).

## 8. Verification for the handoff (the full block)

Run once per slice, before its handoff, for the languages it touched
(`AGENTS.md`):

```bash
cd rust && cargo fmt --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace
bun run typecheck && bun run lint && bun run test
python3 ops/lint/check_intra_package_imports.py && python3 ops/lint/check_bare_iceberg_count.py
(cd dagster && python -m pytest dispar_orchestrate -q)
git status
```

Per the owner's rule of 2026-10-30 and the memory of this repo, CI's cargo run
on the draft PR stands in for the full local `cargo test --workspace`; the
developer runs the scoped blocks of section 6 locally and quotes CI's run for
the rest. S1 runs no `bun`; S2 runs no cargo. No compose change, so no
`docker compose up`. `(cd dagster && …)` is in the block because `ops/g7`
is Python; nothing under `dagster/` changes.

## 9. Before S1 starts (planner, not the developer)

1. Add `BLD-16` to `docs/core/BACKLOG.md` (Task, Build, under `BLD-1`, "part
   2: joins, aggregations and filters as form steps, a form step as SQL",
   **Blocked by** nothing open) and to the base (F5).
2. Write decisions 19 to 27 on the feature page from J1 to J6 and J8's
   planner defaults, and acceptance rows `BLD-1-AC15` to `BLD-1-AC21`:
   - AC15: a left join form of two inputs where some rows have no match;
     run; the row count equals the same query in Query Studio, and unmatched
     rows have `0` or empty text on the right side.
   - AC16: an aggregate form with all six functions and a having; output
     equals the step's "Show as SQL" text run in Query Studio.
   - AC17: a filter with two conditions, once all, once any; row counts
     equal the same `WHERE … AND` and `WHERE … OR` in Query Studio.
   - AC18: Switch a form step to SQL and run; the output equals the run
     before the switch; the step cannot be turned back.
   - AC19: as an author for whom column C is masked, join C through a form
     step; C is masked in the output; as an editor who sees C unmasked, the
     preview shows it unmasked.
   - AC20: ask the assistant for AC15's join and a sum by a column with a
     having; a draft with join and aggregate form steps is created.
   - AC21: a form step naming a column that does not exist; saving works, the
     preview shows the engine's error on that step (J10).
3. Get the owner's signature on 19 to 27; then hand S1 to the developer.

## 10. Out of scope

- Part 3: publishing a step as a table, output options, quality rules before
  publishing (feature page capabilities 14 and 15).
- `BLD-12` (refreshing only changed rows), Python steps (`DEC-1`), dbt
  (`DEC-2`), storing a pipeline in Git.
- `join_use_nulls` for pipelines (F3), for form or SQL steps: its own item,
  if the owner wants it.
- Checking at save that a form step's columns exist (F4).
- Column lineage through a `WITH` (`DATA-57`).
- Non-equality joins, cross joins, more than two inputs in one join step,
  calculated columns as a form, all and any nested, `IN`, `LIKE`, `IS NULL`
  operators.
- The pipeline as text (planned in `2026-11-13-bld-1-part-1-pipeline-as-text.md`,
  not started). It writes any step with the fields it was stored with (its
  X2), so the new kinds need nothing from it; whichever of the two console
  branches lands second rebases on the other (both touch
  `pipeline-editor.tsx`).
- Any change to `check_sql_source`, Query Studio, the dashboard preview, the
  BI builder, the orchestrator, or the five-operation grammar beyond making
  its operator list shareable.

## 11. Handoff

### S1 — `bld-16-joins-aggregations` (built on one branch by agreement with the orchestrator; the plan named `feat/bld-16-form-steps-api`)

Commits (T1–T6, one per task, plus one carrying a file a message claimed but
a `git add` had missed):

- `ef4dd844a` — T1 the stored shape of the three new step kinds
- `4a3b7c908` — T2 `pipeline_forms`, the parsers and renderers
- `57ee4312b` — T3 the compiler takes the new kinds; `step_sql`
- `22a913ca8` — T4 the step-SQL route; save/run/preview proof
- `6edd8fcf4` — T5 the assistant's schema, prompt line, fixture, tool tests
- `b76b32dd0` — T5 follow-up: `ops/ai_eval/cases.json` (the T5 commit's
  `git add` covered `rust/` only, so the case file itself was never staged;
  committed as its own commit rather than rewritten history)
- `efcfe63a2` — T6 the `g7` gate case

Commands run, scoped per commit (the counts as they passed on this branch):

- T1: `cargo fmt --check`; `cargo clippy -p lakehouse-store -p lakehouse-api
  --all-targets -- -D warnings` (clean); `cargo test -p lakehouse-store
  --test it pipelines::` — **80 passed, 0 failed** (6 new: the J2 join,
  aggregate and filter JSON round trips, the `match` key, a bare new kind,
  stored-and-read-back, no legacy transforms).
- T2: `cargo clippy -p lakehouse-api --all-targets -- -D warnings` (clean);
  `cargo test -p lakehouse-api --lib -- pipeline_forms transform_grammar` —
  **60 passed, 0 failed** (39 in `pipeline_forms`, the rest the untouched
  grammar tests).
- T3: `cargo fmt --check`; `cargo clippy -p lakehouse-api --all-targets -- -D
  warnings` (clean); `cargo test -p lakehouse-api --lib -- pipeline_compile
  pipeline_forms transform_grammar sql_guard` — **143 passed, 0 failed**.
- T4: `cargo fmt --check`; `cargo clippy -p lakehouse-api --all-targets -- -D
  warnings` (clean); `cargo test -p lakehouse-api --test it -- route_auth::
  pipeline_preview:: pipeline_compile_route:: pipeline_definition_save::
  sec11_guard:: audit_coverage::` — **187 passed, 0 failed**.
- T5: `cargo clippy -p lakehouse-api --all-targets -- -D warnings` (clean);
  `cargo test -p lakehouse-api --lib -- routes::ai::registry
  routes::ai::tools::pipelines routes::ai::prompt` — **118 passed, 0 failed,
  1 ignored** (the ignored one is `write_tool_schema_fixture`, run by hand to
  regenerate the fixture; the diff touches only `create_pipeline`, checked by
  comparing the committed and regenerated JSON per top-level entry).
- T6: `python3 -m py_compile ops/g7/g7_pipeline_test.py`;
  `python3 ops/lint/check_intra_package_imports.py`;
  `python3 ops/lint/check_bare_iceberg_count.py` — all pass.

S1 verification block (final commit, foreground):

- `cargo fmt --check` — clean.
- `cargo clippy -p lakehouse-store -p lakehouse-api --all-targets -- -D
  warnings` — clean.
- `cargo test -p lakehouse-store --test it pipelines::` — **80 passed,
  0 failed**.
- `cargo test -p lakehouse-api --lib -- pipeline_compile pipeline_forms
  transform_grammar sql_guard routes::ai` — **764 passed, 0 failed, 1
  ignored**.
- `cargo test -p lakehouse-api --test it -- route_auth:: pipeline_preview::
  pipeline_compile_route:: pipeline_definition_save::
  column_lineage_pipeline:: sec11_guard:: audit_coverage::` — **194 passed,
  0 failed**.
- `python3 ops/lint/check_intra_package_imports.py`,
  `python3 ops/lint/check_bare_iceberg_count.py`,
  `python3 -m py_compile ops/g7/g7_pipeline_test.py` — all pass.
- `git status` — clean.

Not verified, and why:

- `ops/ai_eval/test_ai_eval.py` (the plan's T5 check line): the session's
  Python has neither `pytest` nor `requests`, which the harness imports at
  module load. `cases.json` parses and the new case follows
  `build_pipeline_draft`'s shape; the rescore test replays a saved run by id,
  so a new case cannot change its numbers. CI runs the suite.
- The `g7` gate itself: the compose stack was not brought up in this session.
  It runs in CI's `g3a-dagster` job on the draft PR.
- The whole workspace and the gates: CI's run on the draft PR stands in for
  the full local `cargo test --workspace` (the plan's own rule, section 8).
- The assistant's actual behaviour with the new kinds (the `ai_eval` case) is
  not asserted here; it is "not run in CI" by design (J8).

Notes for the reviewer:

- The T5 commit message says the fixture's diff touches only
  `create_pipeline`; that was verified programmatically (per-entry comparison
  of the old and new fixture JSON), not by eye.
- One deviation from the plan's letter, inside the plan's own rules: the
  existing `step_a_two_input_sql_pipeline_matches_a_direct_query` gate case
  now shares the new `_create_and_run_authored_pipeline` helper instead of
  carrying a byte-identical copy of the create-activate-run sequence
  (AGENTS.md rule 4, "a duplicated guard is a finding"). Only the inline
  sequence moved; the assertion and the direct query are unchanged.

## 12. Review

*(the reviewer appends findings per slice and round, tagged `BLOCKER` or
`SHOULD-FIX`)*
