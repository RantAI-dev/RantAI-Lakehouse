# `BLD-1` part 1: a pipeline is inputs, steps and an output; a step can be SQL

Planner and reviewer: Claude Opus. Developers: a non-Opus agent for S1 and S2
(Rust and Python); a separate Opus developer agent for S3 (new console UI,
product owner's rule of 2026-11-04). Nobody reviews their own slice
(`AGENTS.md`).

Base: `origin/main` at `657c2502`. Spec: `docs/core/specs/bld-1.md`. Feature
page: `docs/core/features/pipeline-joins-and-sql.md` (decisions 1 to 18).

| Slice | Branch | What | Waits for |
| --- | --- | --- | --- |
| S1 | `feat/bld-1-steps-api` | Definition, compiler, save check, run route, orchestrator, target lag inputs, assistant schema | Nothing: decisions 12 to 18 were signed by the product owner on 2026-11-12 |
| S2 | `feat/bld-1-steps-preview` | Preview route | S1 merged |
| S3 | `feat/bld-1-steps-console` | The editor, the read view, the contracts | S1 merged (S2's contract is in section 3) |

The pipeline as text (feature page capability 8, decision 10) is the last
slice of part 1. Its plan is written after S3 merges, because every anchor it
needs is in files S3 creates.

## 1. The problem

Read from `main` at `657c2502`:

- A definition is one `source` (`"zone.table"`), a list of `transforms`
  strings and one `target` (`lakehouse-store/src/pipelines.rs:220`
  `AuthoredDefinition`, migration `0036`).
- The orchestrator builds the statement. `authored_factory.py:506`
  `_build_select_sql` turns the transforms into one `SELECT`, `:462`
  `_enforced_sql` posts it to `POST /api/pipelines/runnable/{id}/enforce`
  (`routes/authored_pipelines.rs:1460`), and `:582` `_write_clickhouse_table`
  runs what comes back: a staging table, `INSERT ... SELECT`, `EXCHANGE
  TABLES`. Every run rebuilds the table.
- The console is a five-step wizard (`pipeline-create-page.tsx:127`
  `PipelineEditor`) with one source table and rows of the five operations.

Findings on `main`, each handled below or in the backlog:

- **F1. Access is not checked when a pipeline is saved.** `create`
  (`routes/pipelines.rs:3098`) and `update` (`authored_pipelines.rs:1671`)
  call neither `gate_statement` nor `rewrite_sql_for_roles`. The spec asked
  for it; PR #173 built only the run-time half. D9 builds it.
- **F2. The API does not check the zone and table names it stores.** No
  `Ident::new` in `routes/pipelines.rs`, `routes/authored_pipelines.rs` or
  `lakehouse-store/src/pipelines.rs`. Only the orchestrator refuses an
  unsafe name, when it runs (`authored_factory.py:726`). D4 moves the check
  to the API.
- **F3. `kind` and `incrementalColumn` do nothing.** The orchestrator never
  reads `incrementalColumn`, and `kind` is not in what it receives
  (`RunnablePipeline`, `pipelines.rs:1408`). Backlog `BLD-12`; S3 removes
  the two fields from the page (decision 18).
- **F4. A comment claims a shared test file that one side does not load.**
  `authored_transforms.py` says `transform_test_vectors.json` is loaded by
  `transform_grammar.rs`'s tests too. Nothing in Rust reads it. S1 T2 makes
  the claim true (`AGENTS.md` rule 1).
- **F5. A draft from `generate` skips the checks `create` runs**
  (`routes/pipelines.rs:3340`), and moving a draft to ready checks nothing
  (`:3575`). Backlog `BLD-13`. Not in this plan. After S1 such a pipeline
  still fails closed when it runs, because the run route compiles and checks
  it.
- **F6. An editor can make a pipeline read what only its author may read.**
  `update` keeps `run_as`. Backlog `SEC-71`; D10 closes it.

## 2. Decisions (planner; the product owner's are cited by number)

- **D1. The shape.** A definition holds `inputs` and `steps`.

  ```json
  {
    "inputs": [
      { "name": "orders", "zone": "bronze", "table": "orders" },
      { "name": "customers", "zone": "bronze", "table": "customers" }
    ],
    "steps": [
      { "name": "paid", "kind": "transform", "reads": "orders",
        "transforms": ["filter(status = 'paid')"] },
      { "name": "by_country", "kind": "sql",
        "sql": "SELECT c.country, sum(p.amount) AS revenue FROM paid AS p LEFT JOIN customers AS c ON p.customer_id = c.id GROUP BY c.country" }
    ]
  }
  ```

  The output is what the last step returns. With no steps there is exactly
  one input and the pipeline copies it (today's plain copy).

- **D2. Limits and names.** 1 to 5 inputs (decision 4). 0 to 20 steps
  (decision 15). A SQL step is at most 20,000 characters (decision 16; the
  constant `sql_guard.rs:259` `SQL_SOURCE_MAX_CHARS` already holds that
  number for dashboards: share it, do not write a second one). A name is an
  `Ident` (`lakehouse-core/src/ident.rs`) of at most 48 characters, not
  starting with `__`, unique across inputs and steps without regard to case.
  A `transform` step's `reads`, and every table a SQL step names, is an
  input or an earlier step. The output table is not one of the inputs.

- **D3. One body, old callers keep working.** `POST /api/pipelines`,
  `PUT /api/pipelines/{id}` and the assistant's `create_pipeline` accept
  either `inputs` and `steps`, or the fields of today (`sourceZone`,
  `sourceTable`, `transforms`). Today's fields are turned into D1's shape by
  one function: one input named after the table (or `input_1` when the table
  name is not a usable name), and, when there are transforms, one
  `transform` step named `transformed` (`transformed_1` if the input has
  that name). Sending both forms is a 400. Responses carry `inputs` and
  `steps`, and still carry `sourceZone`, `sourceTable` (the first input) and
  `transforms` (the transforms of a pipeline that is one input and at most
  one `transform` step; otherwise empty), so nothing that reads today's
  fields breaks in the same release.

- **D4. The API compiles; the orchestrator does not.** One pure function in
  a new module `lakehouse-api/src/pipeline_compile.rs` turns a definition
  into one statement:

  ```sql
  WITH
    `orders` AS (SELECT * FROM icecat_api.`bronze.orders`),
    `customers` AS (SELECT * FROM icecat_api.`bronze.customers`),
    `paid` AS (SELECT * FROM `orders` WHERE status = 'paid'),
    `by_country` AS (
  SELECT c.country, sum(p.amount) AS revenue FROM paid AS p ...
  )
  SELECT * FROM `by_country`
  ```

  - An input is named the way `authored_factory.py:367` `_resolve_source`
    names it today: Bronze as `` <iceberg query db>.`bronze.<table>` ``
    (`state.config.iceberg_query_db`), any other zone as
    `` <zone>.`<table>` ``. The rule moves to Rust and leaves Python.
  - The compiler formats only checked names: `zone` and `table` through
    `Ident::new`, step and input names per D2 (F2). A stored definition that
    fails this is refused when compiled, with a fixed message.
  - A `transform` step is rendered by a Rust port of `_build_select_sql`
    over `Transform::render_clickhouse` (`transform_grammar.rs:118`, dead
    code today; remove its `#[allow(dead_code)]`). Same rules: the last
    `select` wins, `rename` and `cast` apply to selected columns, filters are
    joined by `AND`, the last dedupe key gives `ORDER BY k LIMIT 1 BY k`.
  - A SQL step's text goes in as written, between line breaks, so a comment
    at its end cannot swallow the closing bracket.
  - `compile(definition, up_to)` stops after the named input or step. The
    preview (S2) uses it.

- **D5. What a SQL step may be.** A new `sql_guard::check_pipeline_step_sql`
  that shares one private core with `check_sql_source` (`sql_guard.rs:329`):
  not blank, within D2's length, no `{{placeholder}}`, starts with `SELECT`
  or `WITH`, passes `is_read_only`, no `;`, no `SETTINGS` or `FORMAT`. On top
  of the shared core: the text parses, with `ClickHouseDialect`, to exactly
  one statement and that statement is a query (spec rule 1).
  `check_sql_source` keeps its behaviour and its tests; only its body is
  split so the two do not drift (`AGENTS.md` rule 4). Window functions are
  not refused (decision 12). Table functions, `system.*` and dictionary
  functions are already refused by the rewrite (`RewriteError`).

- **D6. A step reads only inputs and earlier steps** (spec rule 2). After
  compiling up to step *k*, `sql_rewrite::referenced_tables`
  (`sql_rewrite.rs:118`) of the compiled text must be a subset of the
  inputs' `zone.table` names. It leaves out CTE names and gives Bronze read
  through the catalog database as `bronze.<table>` (`canonicalize`, `:576`),
  so the comparison is exact. The first step that adds a table outside the
  set is the one refused: "this step reads a table that is not an input; add
  it under Inputs".

- **D7. Errors name the step.** A refusal from D2, D5, D6 or D9 is a 400 or
  422 whose message starts with `steps[<i>]` (or `inputs[<i>]`), the form
  the console already parses for `transforms[<i>]`
  (`src/lib/transform-draft.ts:75`). A transform inside a step is
  `steps[<i>].transforms[<j>]`. The rest of the message is fixed text
  written in the API, never the parser's, the rewriter's or ClickHouse's.

- **D8. The run route.** `POST /api/pipelines/runnable/{id}/compile`
  replaces `POST /api/pipelines/runnable/{id}/enforce`. No body. Same
  caller rule (the orchestrator's token or an unrestricted principal), same
  409 cases (no author, author not active, no tenant), same gate
  (`gate_statement` as the author in the pipeline's tenant), same rewrite
  (`rewrite_sql_for_roles`), same lineage note (`note_pipeline_sql`). It
  compiles the stored definition and returns
  `{ "sql": "<rewritten>", "readsIceberg": <bool> }`; `readsIceberg` is true
  when any input is Bronze. The orchestrator no longer sends SQL to the API.
  The `enforce` route, its policy row and `EnforceBody` are removed; its
  tests move to the new route.

- **D9. The check when saved** (F1, spec rule 3). After D2 to D6, `create`,
  `update` and `restore_version` compile the whole definition and run the
  gate and the rewrite for the person the pipeline will read as, exactly as
  D8 does. The code that turns a user id into the principal, tenant and
  placeholder values the rewrite needs is in `enforce_pipeline_sql` today
  (`authored_pipelines.rs:1484` to `1569`); make it one function that the
  save path and D8 both call. A refusal is a 422 per D7. When the pipeline
  will have nobody to read as (created by a service identity), the save
  runs every check except the rewrite, and the pipeline does not run until
  an administrator assigns an author, as today.

- **D10. Who it reads as after an edit** (decision 14, closes `SEC-71`).
  On `update` and `restore_version`: when the caller is a user, is not the
  current `run_as`, and the inputs or steps differ from the stored ones,
  `run_as` becomes the caller, in the same transaction, and D9 checks as the
  caller. An edit that changes neither (the schedule, the description, the
  retries) leaves `run_as` alone. The audit entry of the update records the
  old and the new `run_as`. `PUT /api/pipelines/{id}/run-as` is unchanged.

- **D11. Target lag follows every input** (decision 9). A new column
  `input_tables TEXT[]` holds the inputs as `zone.table`, written by the
  store from `inputs` in one place. Every reader of the single `source`
  column for "what does this pipeline read" uses it:
  `lag_reader_predicate` (`pipelines.rs:2216`),
  `upstream_downstream_pipelines_conn` (`:2333`),
  `find_target_lag_cycle_conn` (`:2482`), `pipeline_lag::inputs_of_conn`
  (`pipeline_lag.rs:409`), and any other the developer finds with
  `grep -n "\.source\b\|p\.source\|source =" ` over `lakehouse-store` and
  `lakehouse-api`. Each one is listed in the handoff with what was done.
  `source` stays, as the first input. `dependsOn` is unchanged: it is still
  how a pipeline runs after a connector load or a re-upload.

- **D12. Connector and per-table triggers keep to the first input.** A
  pipeline with a connector has exactly one input, checked by
  `validate_connector_source` (`authored_pipelines.rs:549`) as today.
  `ingest_job` and `file_ingest_job` in `dependsOn` are accepted when the
  first input is Bronze (`validate_per_table_upstream`, `:594`), and watch
  the first input, as today. `DEC-22` was decided on 2026-11-12 (a
  re-upload also runs the pipelines further down); `BLD-14` builds that on
  D11's order after this part lands, not here.

- **D13. Storage.** Migration `0137_pipeline_inputs_and_steps.sql` (the next
  free number when the PR is opened; check every open branch first). Adds
  `inputs JSONB NOT NULL DEFAULT '[]'`, `steps JSONB NOT NULL DEFAULT '[]'`,
  `input_tables TEXT[] NOT NULL DEFAULT '{}'` to `pipeline_definition`, and
  fills them for every row from `source` and `transforms` by D3's rule. A
  why-header; additive; never edited once applied. The version snapshot
  (`PipelineDefinitionSnapshot`, `pipelines.rs:373`) gains `inputs` and
  `steps` with a serde default, so an old snapshot still reads and is turned
  into D1's shape by D3's function when restored. `transforms` and `source`
  keep being written (D3).

- **D14. The orchestrator.** `authored_factory._op_for_pipeline` (`:654`)
  calls D8's route, then `_write_clickhouse_table` with the returned SQL,
  with `allow_database_iceberg = 1` when `readsIceberg`. It keeps: the check
  of the target zone and table names it formats itself, the refusal of a
  Bronze target, the refusal of a connector on a non-Bronze source (it reads
  `sourceZone`, still delivered), FBIC's skipped verb, retries, the
  materialization. It loses: `_build_select_sql`, `_resolve_source`, the
  transform parsing, and with them `authored_transforms.py`,
  `test_authored_transforms.py` and the Python reading of
  `transform_test_vectors.json`. Python no longer builds SQL from stored
  text, so the second grammar has nothing left to guard; say so in the
  commit and the module doc (`AGENTS.md` rule 2: this removes code and the
  tests of that code, it does not weaken a test of code that stays).

- **D15. The assistant** (decision 6). `create_pipeline_schema`
  (`routes/ai/registry.rs:1705`) gains `inputs` and `steps` as D1 describes
  them, keeps today's fields, and its description says when to use a SQL
  step. The tool already calls `routes::pipelines::create`, so D2 to D10
  apply to it with no second copy. The fixture is regenerated with
  `cargo test -p lakehouse-api --lib write_tool_schema_fixture -- --ignored`.
  The AI team reviews this task.

- **D16. The preview** (S2; decisions 5, 13, 17).
  `POST /api/pipelines/preview`, permission `pipeline:write`, in
  `POLICY_TABLE` and asserted both ways in `tests/route_auth.rs`.
  Body `{ "inputs": [...], "steps": [...], "upTo": "<input or step name>" }`;
  nothing is stored, so it works before the pipeline is saved. It runs D2 to
  D6 on the part up to `upTo`, compiles it, and runs it **as the caller**
  through the same gate and rewrite Query Studio uses (`GateActor::Caller`),
  wrapped as `SELECT * FROM (<rewritten>) AS step LIMIT 100`, with a 30
  second time limit and the result caps the dashboard preview uses
  (`dashboard_sources.rs:446` `preview` is the precedent for the wrapper,
  the column probe and the settings; reuse its helpers where they fit, do
  not copy them). Response
  `{ "columns": [{ "name": "...", "type": "..." }], "rows": [[...]], "truncated": <bool> }`.
  An error from ClickHouse is reported the way `POST /api/query/run` reports
  one for the same SQL; no second classification is written.

- **D17. The console** (S3; decision 7). One editor for create and edit,
  replacing the wizard:
  - `FormPage` (`src/components/patterns/form-page.tsx:21`) as the shell,
    with two tabs (`Tabs`, line variant): **Build** and **Schedule and
    settings**.
  - Build: a left rail (Inputs, Steps, Output) and, to its right, the pane
    for the selected item above a preview. The chart builder is the layout
    precedent (`chart-builder.tsx:1787`).
  - Input pane: zone, table, the name in this pipeline, the columns with
    their types (`lakehouseService.getTableDetail` already returns types).
  - `transform` step pane: what it reads, and the existing five-operation
    rows, moved as they are. Part 2 redesigns them; this slice does not.
  - SQL step pane: `SqlEditor` (`src/components/sql-editor.tsx:90`) with
    completion for the inputs, the earlier steps and the inputs' columns
    (`sql-completion-source.ts:83`), and Format (`src/lib/sql-format.ts`).
  - Output pane: target zone and table, and one line: "Every run rebuilds
    this table."
  - Preview: D16 for the selected input or step; column names, types and
    rows; marked stale when the definition changed since it ran; a line when
    the person looking is not the author (decision 13).
  - Schedule and settings: the schedule field, the connector, the two
    trigger switches, the description. All exist today.
  - Kind and incremental column are not on the page (decision 18). Create
    sends `kind: "batch"`; edit sends the stored `kind` back unchanged.
  - A 400 or 422 naming `steps[<i>]` or `inputs[<i>]` selects that item and
    shows the message in its pane (D7).
  - The read view on the pipeline page (`pipeline-flow-layout.ts:155`
    `authoredFlow`, `pipeline-definition.tsx:225` `AuthoredFlow`) draws
    inputs, steps and the output; a SQL step shows its SQL in `CodeView`.
  - Not in this slice: an assistant button inside the editor (the list
    page's builder dialog and the assistant panel stay as they are), a run
    button inside the editor, the text form.

### Plan changes after the S1 review, round 1 (planner, 2026-11-12)

These change section 2. Where one differs from a decision above, this one
holds.

- **D3a. A body in today's form cannot flatten a pipeline.** On `PUT`, a body
  in today's form (`sourceZone`, `sourceTable`, `transforms`) is accepted
  only when the stored definition is one input and at most one `transform`
  step reading it. It then keeps the stored names of that input and step and
  changes only the zone, the table and the transforms. For any other stored
  definition it is refused, 409, with fixed text saying the pipeline has
  inputs and steps this form cannot hold. Reason: until S3 ships, the console
  sends today's form on every save, and it would store one input and no step
  over a pipeline the assistant made with SQL steps; the next run would then
  overwrite the output with a copy of the first input.
- **D4a. A pipeline of today's shape compiles to today's statement.** A
  definition that is one input and either no step or exactly one `transform`
  step reading that input compiles with no `WITH`:
  `SELECT <columns> FROM <the input's table> [WHERE ...] [ORDER BY k LIMIT 1 BY k]`,
  and `SELECT * FROM <the input's table>` with no step. The text is what
  `_build_select_sql` gave on `main` for the same pipeline. `up_to` follows
  the same rule for the part it renders. Every other definition compiles as
  D4 says. Reason: column lineage (`DATA-13`) reads this statement and cannot
  see through a `WITH`; with D4 alone every existing pipeline lost its column
  mappings.
- **D4b. No lineage row for a column the output does not have.** For a
  definition D4a does not cover, the column lineage of a run is either right
  or absent. Until the mapper can read a `WITH` (a backlog item the planner
  adds), the compiled statement of such a pipeline is not noted for column
  lineage, and the route's doc says so. Table lineage (D11) is unaffected.
- **D6a. A step names what it reads exactly.** `reads`, every bare table name
  in a SQL step, and `upTo` match an input's or an earlier step's name
  exactly, case included. Uniqueness of names stays without regard to case.
  A qualified name (`zone.table`) in a SQL step is refused with the sentence
  for a table that is not an input: a step reads an input by its name in the
  pipeline. Reason: `ClickHouse` matches a `WITH` name by exact case, while
  the gate and the rewrite treat any casing of it as that name; a reference
  that differs only in case would be looked up as a real table in the
  connection's database, unseen by both.
- **D10a. An edit by a service identity.** When the caller is not a user and
  the inputs or steps differ from the stored ones: an unrestricted identity
  leaves `run_as` as it is; any other identity clears `run_as`, in the same
  transaction, and the audit entry records it. The pipeline then does not run
  until an administrator assigns who it reads as (the existing rule for a
  pipeline with no author). Reason: decision 14 covers "someone other than
  the author"; a service identity with `pipeline:write` is one, and it cannot
  be read as.
- **D11a. Two more readers of the single source.** `routes/lineage.rs` (the
  edge from a pipeline's source to its target) draws one edge per input, and
  the orchestrator's lag sensor (`authored_factory._pipeline_source`) takes
  every input of the definition. The four readers in the store and the API
  use one rule for a row with no `input_tables`.
- **D17a. For S3.** Saving a change to the inputs or steps of a pipeline that
  reads as someone else says so before it is saved: "This pipeline will read
  as you from now on." An administrator who edits another person's pipeline
  makes its output unmasked; the feature page lists this as a limit.

## 3. Contracts

Request and response fields are camelCase, as the rest of the API.

```
POST /api/pipelines            body: today's fields, with either
PUT  /api/pipelines/{id}         { inputs, steps }  or  { sourceZone, sourceTable, transforms }
GET  /api/pipelines/{id}       definition: { inputs, steps, sourceZone, sourceTable, transforms, ... }

input  = { name: string, zone: string, table: string }
step   = { name: string, kind: "transform", reads: string, transforms: string[] }
       | { name: string, kind: "sql", sql: string }

POST /api/pipelines/runnable/{id}/compile     (orchestrator only)
  200 { sql: string, readsIceberg: boolean }
  409 no author, author not active, no tenant      422 refused (fixed message)

POST /api/pipelines/preview                   (pipeline:write)
  body { inputs: input[], steps: step[], upTo: string }
  200 { columns: { name: string, type: string }[], rows: unknown[][], truncated: boolean }
  400 shape, limits, names, not a single SELECT, not an input   422 refused by the gate or the rewrite
```

## 4. Tasks

One task per commit, in order. Push after every commit. Per commit, run only
the scoped checks for what it touched (`AGENTS.md`, "Keep build time down");
all cargo through the shared wrapper and target directory the brief names.
The reviewer opens a draft pull request once the first code commit is
pushed, so CI runs the full suite.

### S1 (`feat/bld-1-steps-api`)

- **S1-T1. Storage (D13, D3).** The migration; `PipelineInput` and
  `PipelineStep` in `lakehouse-store`; `AuthoredDefinition`, the create and
  update inputs and the snapshot carry them; the store writes `source`,
  `transforms` and `input_tables` from them in one place; D3's function.
  *Check:* store tests: a row with transforms is filled as one input and one
  `transform` step; a row without, as one input and no steps; the
  migration's result equals D3's function on the same fixtures; an old
  snapshot (no `inputs`) restores to the right shape; a table name that is
  not a usable name becomes `input_1`.
- **S1-T2. Compiler and step check (D2, D4, D5, D6, F4).**
  `pipeline_compile.rs`; `check_pipeline_step_sql`; the port of
  `_build_select_sql`; `transform_test_vectors.json` moves under
  `rust/crates/lakehouse-api/tests/fixtures/` and `transform_grammar.rs`'s
  tests load it.
  *Check:* unit tests pin the compiled text for: one input and no steps; one
  input and one `transform` step with select, rename, cast, filter and
  dedupe (the statement `test_authored_factory.py:209` pins today, inside
  the `WITH`); two inputs and a SQL step; a SQL step ending in a `--`
  comment; `up_to` an input and `up_to` a middle step. Refusals, each naming
  the right index: a sixth input, a twenty-first step, a duplicate name
  differing only in case, a name starting with `__`, `reads` naming a later
  step, a SQL step that is `DROP TABLE`, `INSERT`, two statements, has
  `SETTINGS`, reads `silver.other`, reads a bare unknown name, the output
  equal to an input, an unsafe zone or table. A SQL step with a window
  function, a subquery, its own `WITH`, and `LIMIT 1 BY` is accepted.
  `check_sql_source`'s existing tests pass unchanged.
- **S1-T3. Save path (D3, D7, D9, D10, D12).** `create`, `update`,
  `restore_version`; the shared "principal for a user id" function.
  *Check:* API tests: today's body still creates a pipeline and the response
  has both forms; both forms at once is 400; a SQL step reading a table the
  author's role may not read (`GOV-7`) is 422 naming the step; a masked
  column passes the save; a service identity can save and the pipeline has
  no author; an edit of the steps by another user moves `run_as` to them and
  is refused when they may not read an input; an edit of only the schedule
  by another user leaves `run_as`; restoring an old version runs the same
  checks; a pipeline in another tenant is the same 404 as an unknown one.
- **S1-T4. Run route (D8).** The route, the policy row, the removal of
  `enforce`.
  *Check:* `tests/pipeline_enforce.rs` renamed to
  `tests/pipeline_compile_route.rs` and retargeted with every existing case
  kept, plus: a SQL-step pipeline compiles; a masked column of
  an input read inside a step comes back masked; a join of two governed
  inputs comes back with both rewritten; `readsIceberg` is true only with a
  Bronze input. `tests/route_auth.rs`: the new route asserted both ways, the
  old one gone from `POLICY_TABLE` and from `routes/mod.rs`.
- **S1-T5. Target lag inputs (D11).**
  *Check:* store and API tests: a pipeline with two inputs is a reader of
  both producing pipelines; `inputs_of_conn` returns both; the cycle check
  sees a cycle through a second input; an old single-source pipeline behaves
  as before. The handoff lists every reader of `source` found and what was
  done with it.
- **S1-T6. Orchestrator (D14).**
  *Check:* `python -m pytest dispar_orchestrate -q`. The enforcement tests
  of `test_authored_factory.py` (`:433` to `:521`) are kept against the new
  route; the SQL-building tests (`:200`, `:215`) and the source-naming tests
  that tested Python code now deleted are removed with the code, and the
  handoff names each removed test and the Rust test that replaces it.
  `ops/g7/g7_pipeline_test.py` gains a case: two Bronze inputs, a SQL step
  that joins them and sums by a column, run, and the totals compared with
  the same query run directly. `ops/g7/bld1_pipeline_author_test.py` uses
  the new route. Both run in CI job `g3a-dagster`; locally they are *not
  verified* unless the developer brings the stack up.
- **S1-T7. Assistant (D15).**
  *Check:* the fixture test `tool_schemas_snapshot_is_byte_identical` passes
  after regeneration; a tool test creates a pipeline with a SQL step through
  `create_pipeline`.
- **S1-T8. Words.** Module docs of every touched module say why (principle
  1); `docs/OPERATIONS.md`'s pipeline section, if it describes how a run
  gets its SQL, is brought up to date.

Verification before handoff, on the final commit:

```bash
cd rust && cargo fmt --check
cargo clippy -p lakehouse-store -p lakehouse-api --all-targets -- -D warnings
cargo test -p lakehouse-store pipelines
cargo test -p lakehouse-api --lib -- pipeline_compile sql_guard transform_grammar
cargo test -p lakehouse-api --test pipeline_compile_route --test route_auth
python3 ops/lint/check_intra_package_imports.py && python3 ops/lint/check_bare_iceberg_count.py
(cd dagster && python -m pytest dispar_orchestrate -q)
git status
```

The whole workspace and the gates run in CI on the draft pull request.

### S2 (`feat/bld-1-steps-preview`)

- **S2-T1. The route (D16).** Handler, policy row, `route_auth` both ways.
  *Check:* API tests: `upTo` an input returns that table's rows; `upTo` a
  step returns the step's; a column masked for the caller is masked in the
  rows; a table the caller may not read is 422; a non-`SELECT` step is 400
  naming the step; a caller without `pipeline:write` is 403; more than 100
  rows gives 100 and `truncated: true`.
- **S2-T2. Limits and errors.** The time limit and caps; the error path of
  `POST /api/query/run` reused.
  *Check:* a test that the wrapper carries the limits; a test that a
  ClickHouse error reaches the caller in the same form `query/run` gives. A
  preview of a Bronze input through the catalog database is proven on a
  running stack or written down as *not verified*.

### S3 (`feat/bld-1-steps-console`)

- **S3-T1. Contracts and client.** `PipelineInput`, `PipelineStep`;
  `AuthoredDefinition`, `CreatePipelineInput`, `UpdatePipelineInput` gain
  `inputs` and `steps`; `previewPipelineStep` on `PipelineService`.
- **S3-T2. Pure logic in `src/lib`.** The editor's state and its rules
  (add, remove, rename, reorder; the names a step may read; D2's limits;
  the body to send; the index in a D7 message), with unit tests. No rule
  lives in a component (`CODE-STANDARD.md` 4.6).
- **S3-T3. The editor (D17).** Replaces `PipelineEditor`. The preview table
  reuses `SqlRowsTable` and `SqlColumnChips`
  (`src/features/dashboards/sql-source-panel.tsx:15`, `:34`), moved to
  `src/components/patterns/` so two features can use them.
- **S3-T4. The read view (D17).**
- **S3-T5. Tests.** Every behaviour the 22 tests of
  `pipeline-create-page.test.tsx` cover today is covered in the new editor
  or named in the handoff with the reason it no longer applies (Kind and
  incremental column: decision 18). New tests: a pipeline made before this
  change opens as one input and one form step; adding a sixth input is not
  offered; a SQL step's completion offers the inputs and earlier steps; a
  422 naming `steps[1]` selects the second step and shows the message; the
  preview is marked stale after an edit; the create body has `inputs` and
  `steps` and no `sourceZone`.

Verification before handoff: `bun run typecheck && bun run lint && bun run test`.
How the page looks in a browser is the developer's to check and to state:
which pages were opened, in which theme, or *not verified*.

## 5. Out of scope

- Joins, aggregations and richer filters as forms; a form step shown as SQL
  (part 2).
- Publishing a step as a table; output options (part 3).
- The text form of a pipeline (last slice of part 1, planned after S3).
- Python (`DEC-1`), dbt (`DEC-2`), refreshing only changed rows (`BLD-12`),
  the checks on `generate` and on ready (`BLD-13`).
- Any change to `check_sql_source`'s behaviour, to Query Studio, or to the
  dashboard preview.

## 6. Handoff

*(the developer appends one entry per slice)*

### Handoff S1 (developer, 2026-11-12)

Branch `feat/bld-1-steps-api`, base `origin/main` at `512f6e37c`.

**Commits**

| Commit | Subject |
| --- | --- |
| `a5214a2f5` | feat(pipelines): store a pipeline as inputs and steps (S1-T1) |
| `f62b05e1d` | feat(pipelines): compile a definition into one statement (S1-T2) |
| `9e1e35524` | wip: S1-T3 save path (intermediate, kept because sessions die) |
| `b8338e4e4` | feat(pipelines): check access when a pipeline is saved (S1-T3) |
| `4bf4b1509` | feat(pipelines): the orchestrator compiles instead of sending SQL (S1-T4) |
| `5b0a66bec` | feat(pipelines): target lag follows every input (S1-T5) |
| `5b56c0f28` | feat(dagster): the orchestrator asks the API for the SQL (S1-T6) |
| `db8b6e529` | feat(ai): the assistant authors inputs and SQL steps (S1-T7) |
| `a2df5fcd7` | docs(operations): a run gets its SQL from the compile route (S1-T8) |
| (this commit) | docs(plan): BLD-1 part 1 S1 handoff |

**Migration.** `0137_pipeline_inputs_and_steps.sql`. Checked every remote
branch first: the highest number any of them holds is `0136`, so 0137 was
free. It adds `inputs JSONB`, `steps JSONB` and `input_tables TEXT[]` (all
`NOT NULL DEFAULT`), two `IMMUTABLE` SQL functions (`pipeline_legacy_input`,
`pipeline_legacy_steps`, plus the name predicate) and one backfill `UPDATE`.
The functions exist so the store test can compare the migration's answer with
`pipelines::legacy_definition` instead of restating the rule in SQL.

**Verification, run on the final commit (counts are real).**

```
cd rust && cargo fmt --check                                  # clean
cargo-locked clippy -j 2 -p lakehouse-store -p lakehouse-api --all-targets -- -D warnings
                                                              # clean
cargo-locked test -j 2 -p lakehouse-store --test pipelines     # 70 passed, 0 failed
cargo-locked test -j 2 -p lakehouse-api --lib -- pipeline_compile sql_guard transform_grammar
                                                              # 69 passed, 0 failed
cargo-locked test -j 2 -p lakehouse-api --test pipeline_compile_route --test route_auth
                                                              # 20 passed; 90 passed; 0 failed
python3 ops/lint/check_intra_package_imports.py               # OK
python3 ops/lint/check_bare_iceberg_count.py                  # R11 guard: none found
```

Other scoped runs during the slice (each on the commit that needed it):
`-p lakehouse-api --lib -- pipeline` 284 passed (all create/update/status/
detail/connector/restore/depends-on route tests); `--lib -- pipeline_lag` 42
passed; `--test pipeline_definition_save` 10 passed; `--test
column_lineage_pipeline` 7 passed; `--test table_access` 14; `--test
pipeline_lag_check` 7; `--test pipeline_lag_page` 6; `--lib --
create_pipeline_accepts_inputs_and_a_sql_step` 1 passed.

**Python.** `pytest dispar_orchestrate` was run in full after S1-T6: 2511
passed, 1 skipped, 25 subtests (`/home/shiro/rantai/venv-pr186/bin/python`;
see "not verified" below). After S1-T7/T8 only Rust and Markdown changed, so
the re-run was scoped: `test_authored_factory.py` + `test_op_source_metadata.py`
111 passed, 18 subtests.

**D11: every reader of the single `source` column, and what was done.**
Found with `grep -n "\.source\b\|p\.source\|source ="` over
`lakehouse-store` and `lakehouse-api`:
1. `lag_reader_predicate` (store) — the `p.source = candidate` term became
   `candidate = ANY (p.input_tables)`.
2. `upstream_downstream_pipelines_conn` (store) — starts from every input;
   `source` is the fallback for a row with an empty array.
3. `find_target_lag_cycle_conn` (store) — the walk starts from every input of
   each lag pipeline, same fallback.
4. `pipeline_lag::inputs_of_conn` (API) — parses every input, same fallback.
5. `Pipeline::source` itself, and the create/update write of the column, are
   the only others; both are the definition's storage shape (D3) and were not
   readers of "what does this pipeline read". `pipeline_lag` snapshots
   (`inputs_of`) feed `InputTable`s, not `source`.
No fifth reader exists. `Pipeline` gained `input_tables`, `#[serde(skip)]` so
the wire shape does not change in this release.

**Python tests removed with their code (D14), and the Rust test that
replaces each**
- `BuildSelectSqlTest::test_select_rename_cast_filter_dedupe_compose_into_one_statement`
  → `pipeline_compile::tests::a_transform_step_renders_select_rename_cast_filter_and_dedupe`
- `BuildSelectSqlTest::test_no_transforms_selects_star_with_no_where_or_order`
  → `pipeline_compile::tests::one_input_and_no_steps_is_a_plain_copy`
- `BronzeSourceTest::test_a_bronze_source_is_read_through_the_api_iceberg_query_database`
  → `pipeline_compile::tests::a_bronze_input_is_read_through_the_catalog_database`
- `BronzeSourceTest::test_resolve_source_marks_only_bronze_as_an_iceberg_read`
  → same compiler test, plus `tests/pipeline_compile_route.rs::reads_iceberg_is_true_only_with_a_bronze_input`
- `BronzeSourceTest::test_a_silver_source_is_read_exactly_as_before`
  → `pipeline_compile::tests::one_input_and_no_steps_is_a_plain_copy`
- `BronzeSourceTest::test_this_module_creates_no_catalog_database_of_its_own`
  → nothing: the module no longer creates a database at all
- `OpForPipelineTest::test_a_rejected_transform_raises_a_non_retryable_failure_wrapping_transform_error`
  → `pipeline_compile_route.rs`'s 422 refusal cases (the grammar left Python)
- `test_authored_transforms.py` (whole file) → `transform_grammar::shared_vectors::the_shared_accept_reject_vectors_match_this_grammar`
The enforcement tests were kept, renamed `CompileSqlTest`, against the new
route.

**Existing Rust tests changed because behaviour changed by plan, and why**
- `tests/pipeline_enforce.rs` → `tests/pipeline_compile_route.rs` (D8).
- `tests/column_lineage_pipeline.rs`: the noted SQL is now the COMPILED
  statement, whose inputs are CTEs; the mapper records `partial: true` with
  the reason "read through the CTE …, which is not traced" instead of an
  invented source column, and the test now asserts that answer. **See
  "mismatch" below.**
- `tests/pipeline_lag_check.rs::unparseable_source_preserves_existing_state`
  corrupts `input_tables` (D11's column) instead of `source`.
- route-test fixtures that build a pipeline as a tenantless stand-in principal
  or as a user whose real roles cannot read the table now meet the save-time
  gate (D9); they use the seeded Platform Admin (`fajar`), and the test
  `Config`s name `ICEBERG_QUERY_DB` because D4 compiles a Bronze input
  through it.
- `tests/table_access.rs` and the store's `update_input_for` needed no change.

**Mismatch between plan and code (for the planner)**

1. `docs/core/features/pipeline-joins-and-sql.md` still shows decisions 12–18
   as *(proposed)* with an empty Signed cell, although the brief says they
   were signed on 2026-11-12. I built to 12 (window functions allowed), 15
   (20 steps), 16 (20,000 characters) and 18 (nothing reaches the page from
   this slice); the feature page should be updated.
2. **DATA-13 lineage degrades for pipelines.** D8 says "same lineage note",
   and the note is the compiled statement. Because every input is a CTE and
   the outer select is `SELECT * FROM <last step>`, the lineage mapper can no
   longer prove which source column a target column came from: it writes
   `partial: true` with an honest reason instead of the previous exact
   mapping. The target table/columns are still recorded. Fixing it needs
   either a CTE-aware mapper or noting the last step's SQL expanded; it is
   not in this slice's scope and should be a backlog item (BLD-1 follow-up or
   DATA-13 follow-up) before AC1-style acceptance reads lineage.
3. The save-time gate (D9) refuses a pipeline saved by a restricted user when
   the pipeline has no tenant (the gate's `TenantReach::NoTenant`), which is a
   new refusal for a deployment where a user creates before an admin assigns
   a tenant. Platform Admins are unaffected. This follows from "exactly as D8
   does" (D8 answers 409 before the gate) but the plan did not say it.
4. `validate_per_table_upstream` and the connector rule now read the FIRST
   input (D12) rather than a `sourceZone` field; no behaviour change for a
   one-input pipeline, which is every pipeline a connector can have.

**Not verified locally**
- `ops/g7/g7_pipeline_test.py` (the new two-input SQL-join case) and
  `ops/g7/bld1_pipeline_author_test.py`: they need the whole stack. Both were
  `python3 -m py_compile`d and checked by eye; CI job `g3a-dagster` runs them.
  In the new gate case the two "inputs" are the same Bronze table under two
  names, because the G7 stack guarantees one loaded Bronze table; the case
  still exercises two inputs, a join, an aggregate, the compile, the rewrite
  and the run.
- The assistant's end-to-end behaviour against a live model; only the schema
  snapshot test and the tool test above were run.
- `pytest` under `/home/shiro/rantai/venv-bld3/bin/python`: that venv cannot
  import the package (`ModuleNotFoundError: fastavro`), so the full suite was
  run under `venv-pr186/bin/python`, which has the same `dagster 1.13.20` and
  the missing dependency. CI runs the full suite in the `g3a-dagster` job.
- `cargo test --workspace` and the full clippy workspace run: not run here
  (the brief scopes local runs); CI runs them on the draft PR.

**For S2/S3**
- `pipeline_compile::compile(inputs, steps, Some(upTo), catalog_db)` is the
  preview's seam; `DefinitionError::path()` gives the `inputs[i]`/`steps[i]`
  prefix the editor selects an item with.
- The wire shape S3 can rely on: `AuthoredDefinition` carries `inputs` and
  `steps` (camelCase, `PipelineStep` internally tagged by `kind`), and keeps
  `sourceZone`/`sourceTable`/`transforms`.
- `POST /api/pipelines` and `PUT /api/pipelines/{id}` refuse a body that
  sends both forms, and a body that sends neither.

### Handoff S1, fix round 1 (developer, 2026-11-12)

Branch `feat/bld-1-steps-api` (base `origin/main` as of `1a30be00a`'s merge;
`main` merged again at `64055f47a`). One line per finding:

- **R1-B1 / D4a, D4b** — `52d1ecc9c`. Today's shape (one input, no step or one
  `transform` step reading it) compiles with NO `WITH` to exactly the text
  `_build_select_sql` gave on `main`; `up_to` follows the same rule. Every
  other definition keeps the `WITH`, and the route notes no SQL for column
  lineage for it (D4b). `tests/column_lineage_pipeline.rs` is `origin/main`'s
  file again, retargeted to `.../compile`: the exact `price_f` walk and the
  three pairs are back; a new test pins that a SQL-step pipeline writes no
  column row. The G7 gate's exact-mapping assertion (`:650`) holds again.
- **R1-B2** — `ecd8abb22` (in the wip, final in `6536878b1`'s round).
  `tests/table_access.rs`'s GOV-7 case posts to `.../compile` with the hidden
  table as the pipeline's input; it still proves the 404 and the SEC-20
  sentence.
- **R1-B3 / D3a** — `aea0e2eac`. Today's form on `PUT` is accepted only for a
  stored definition of today's shape, keeping that input's and step's names;
  anything else is 409 `LEGACY_FORM_CANNOT_EDIT`. Two tests: a 409 that
  changes nothing over a two-input SQL-step pipeline, and a save that keeps
  `orders_named`/`my_step` and does not move the author.
- **R1-B4** — `361678213`. The migration splits `source` at the FIRST dot
  (`pipeline_legacy_zone`/`pipeline_legacy_table`), picks the step name
  without regard to case, and the backfill is a function the test runs over
  rows it inserts. Fixtures: no dot, two dots, an unusable name,
  `Transformed`.
- **R1-B5** — `ccd35aabb`. `StoreError::CorruptDefinition` (422); decode
  failures propagate from `get_definition`; `PipelineDefinitionSnapshot` is
  `TryFrom`, so a write never records an empty snapshot; both writes use `?`.
  Two tests: an undecodable `steps` is refused; a refused delete writes no
  snapshot and leaves the row.
- **R1-B6 / D6a** — `439bf7a5f`. `reads`, bare names in a SQL step and `upTo`
  match exactly (case included); a qualified `zone.table` in a step is
  refused as not-an-input; uniqueness stays case-insensitive. Four tests.
- **R1-B7 / D11a** — `f215471ba`. `routes/lineage.rs` draws one edge per
  input; `_pipeline_sources` gives the lag sensor every input; the "no
  `input_tables`" fallback is ONE store rule (`input_tables_of` /
  `input_tables_or_source`) used by all four readers and those two. Tests: a
  two-input pipeline has both edges; a lag pipeline is watched at its second
  input.
- **R1-B8** — `ecd8abb22`/`6536878b1`. `pipeline_compile_route.rs`'s fixtures
  are today's shape again, so the four statements are exact `assert_eq!`
  again; "an author with no obligation gets no mask" is asserted; the
  `!sql.contains(SQL)` line can fail again; the two SQL-step cases are
  additional tests that pin the whole answer and the mask's presence/absence.
  In `routes/pipelines.rs`, the 7 non-save call sites are back on
  `fixture_member_principal`; the save-path fixtures now use a restricted
  member who MAY read the table (their tenant claims it with
  `uploads::claim_table`), and the two that create a pipeline with NO tenant
  use `fixture_admin_no_tenant_principal` (a real seeded `*:*` author) —
  a restricted caller cannot create a tenantless pipeline (D9).
- **R1-B9 / D10a** — `f34f325ef`. `RunAsChange` (`Keep`/`Clear`/`Set`) on
  `UpdatePipelineInput`; a service identity that changes the definition clears
  `run_as` unless it holds `*:*`; the audit entry records the change. Tests:
  the rule, the store clear, a service-created pipeline has no author, and an
  editor who may not read an input is refused with `inputs[i]`.
  **Not reachable through HTTP today**: a restricted service identity has no
  tenant, so `in_scope` (`authored_pipelines.rs`, rule 7) answers 404 before
  `update` runs. The `Clear` branch is unit- and store-tested, not
  route-tested.
- **R1-S1** — `d42b7bc02`. Only the gate's own refusals are a 422 at save; an
  outage keeps its status (`EnforcementRefusal::into_api_error`).
- **R1-S2** — `1cba5c09e`. 404 (GOV-7) joins the refused statuses, so the
  orchestrator does not retry it or report its address. `OPERATIONS.md` lists
  409/404/422, which carry `inputs[i]`/`steps[i]`, and that the orchestrator
  no longer needs `ICEBERG_QUERY_DB`.
- **R1-S3** — `1cba5c09e`. The old-flow comments and the transform-grammar
  references are gone from `authored_factory.py`, `AuthoredPipelineConfig` no
  longer reads `iceberg_query_db`, `backup_job.py` no longer cites the deleted
  module, the vacuous assertion is replaced by one that can fail, and the lost
  test is back (no `allow_database_iceberg` when `readsIceberg` is false).
- **R1-S4** — `6536878b1`. `inputs_of` returns both inputs; an old snapshot
  restores through the route; an unsafe zone; the `--` comment and both
  `up_to` cases pin the whole text; a `transform`-step pipeline through the
  compile route; an uncompilable stored definition is 422 naming `steps[0]`.
- **R1-S5 / R1-S6** — `e40e5de21`. The G7 direct query is the same self-join
  with `SETTINGS allow_database_iceberg = 1`; the doc above `MAX_INPUTS` says
  what the constant is; the tool test's input zone is `silver`, which the
  tool's schema lists.
- **R1-CI1** — `efa1039e1`. `definition_message(&DefinitionError)` builds the
  sentence from `path()` and `message()`; both response sites use it, so no
  response line calls `.to_string()` on an error. The guard's stale
  `pipelines.rs` allowlist entry (the loop moved to `authored_pipelines.rs` in
  S1-T3) is removed; the pattern was not loosened.
- **R1-CI2** — `0ed866cc6`. `ICEBERG_QUERY_DB` is gone from
  `tests/common/mod.rs`; `pipeline_compile_route.rs` and
  `column_lineage_pipeline.rs` set it in their own overrides.

**Verification, on the final commit (real counts).**

```
cd rust && cargo fmt --check                                  # clean
cargo-locked clippy -j 2 -p lakehouse-store -p lakehouse-api --all-targets -- -D warnings
                                                              # clean
cargo-locked test -j 2 -p lakehouse-store --test pipelines     # 74 passed
cargo-locked test -j 2 -p lakehouse-api --lib -- pipeline_compile sql_guard transform_grammar
                                                              # 78 passed
cargo-locked test -j 2 -p lakehouse-api --test pipeline_compile_route --test route_auth
                                                              # 24 + 94 passed
cargo-locked test -j 2 -p lakehouse-api --test table_access --test column_lineage_pipeline \
  --test pipeline_definition_save --test pipeline_lag_check   # 14, 8, 14, 7 passed
cargo-locked test -j 2 -p lakehouse-api --test sec11_guard    # 4 passed
cargo-locked test -j 2 -p lakehouse-api --test upload_routes  # 175 passed
cargo-locked test -j 2 -p lakehouse-api --test quality_anomalies --test quality_on_load \
  --test pipeline_lag_page                                    # 6, 5, 16 passed
python3 ops/lint/check_intra_package_imports.py               # OK
python3 ops/lint/check_bare_iceberg_count.py                  # none found
(cd dagster && /home/shiro/rantai/venv-pr186/bin/python -m pytest dispar_orchestrate -q)
                                                              # 2514 passed, 1 skipped, 25 subtests
```

Other scoped runs: `--lib -- pipeline` 294 passed; `--lib -- lineage` 61;
`--lib -- pipeline_lag` 42; `--test pipeline_definition_save` 14;
`test_authored_factory.py` 102.

**Not verified locally**
- `ops/g7/g7_pipeline_test.py` (the R1-S5 fix) and
  `ops/g7/bld1_pipeline_author_test.py`: `py_compile`d only; they need the
  stack. CI job `g3a-dagster` runs them.
- The `pytest` run used `/home/shiro/rantai/venv-pr186/bin/python` again:
  `venv-bld3` cannot import the package (`fastavro` is missing there).
- `cargo test --workspace` / full workspace clippy: not run here; CI runs them.

### Handoff S1, fix round 2 (developer, 2026-11-12)

One BLOCKER this round, then `main` merged in.

- **R2-B1 / D6a** — `edc1e083f`. The SQL step check no longer reads its names
  through `sql_rewrite::referenced_tables`, which lowercased every name and
  dropped a bare name equal to the statement's own `WITH` names. The step is
  parsed once and walked with `sqlparser`'s visitor (`StepReads`), so every
  table relation at any depth — a subquery, a join, the step's own CTE bodies
  — is checked AS WRITTEN: a one-part name equal, case included, to an
  input's name, an earlier step's name, or a `WITH` name the same step
  declares. A qualified name is refused (D6a); a table function is left to the
  rewrite. `sql_rewrite.rs` is unchanged.
  Tests: `FROM ORDERS` over input `orders` is refused; input `Orders` with
  `FROM Orders` compiles (whole text pinned); a step's own `WITH t` read as
  `T` is refused and as `t` accepted; a name inside a subquery and inside a
  join is refused; the round-1 D6a tests still pass.

**The merge of `main`** — `67252c473` (no conflicts: `git merge origin/main`
auto-merged; the files both sides touched — `lib.rs`, `policy.rs`,
`routes/mod.rs`, `route_auth.rs` — kept both sides' additions, and
`route_auth.rs` already held both tests from round 1's merge). It brought
`main`'s `0137_connector_run_timed_out.sql`,
`0138_data_engineer_catalog_read.sql` and `0139_alert_rule_delivery.sql`.

**The migration is 0140 since the merge of main** (`0137` and `0138` were
taken on `main`, `0139` by an open branch that has since merged). Renamed in
`b1f488848` from `0137_pipeline_inputs_and_steps.sql` to
`0140_pipeline_inputs_and_steps.sql`; it was never applied to a real
database, so the rename is safe. Every reference that means OUR migration now
says `0140` (the store's comments and test names, the API's target-lag
comment, the migration's own header). The plan's earlier entries keep their
history as written.

**Verification, on the head commit (real counts).**

```
cd rust && cargo fmt --check                                  # clean
cargo-locked clippy -j 2 -p lakehouse-store -p lakehouse-api --all-targets -- -D warnings
                                                              # clean
cargo-locked test -j 2 -p lakehouse-api --lib -- pipeline_compile sql_guard transform_grammar
                                                              # 82 passed
cargo-locked test -j 2 -p lakehouse-api --test pipeline_compile_route \
  --test pipeline_definition_save --test column_lineage_pipeline \
  --test route_auth --test sec11_guard --test table_access    # 24, 14, 8, 94, 4, 14 passed
cargo-locked test -j 2 -p lakehouse-store --test pipelines     # 74 passed
```

After the rename, and before the rest: `--test pipelines` 74 and
`--test pipeline_definition_save` 14, rebuilt so the embedded migration set
is the one with `0140`.

**Not verified**
- CI's full run, including `g3a-dagster` (the G7 case and the author gate):
  no local run covers them, and neither gate has passed on any commit of this
  slice yet.
- The Python suite was not re-run this round: neither commit touches
  `dagster/` (the merge brought `main`'s changes, which its own CI covered).
- `cargo test --workspace`: not run here.

### Handoff S2 (developer, 2026-11-13)

Branch `feat/bld-1-steps-preview`, base `origin/main` at `be2c92910` (S1's
merge, PR #481). Rust only; no migration, no dependency change, no `src/`,
`dagster/`, compose or CI change.

**Commits**

| Commit | Subject |
| --- | --- |
| `264380ad6` | feat(pipelines): preview one input or step as the caller (S2-T1) |
| `0dfed1599` | feat(pipelines): a preview's limits and the query/run error form (S2-T2) |
| (this commit) | docs(plan): BLD-1 part 1 S2 handoff |

**What was built.** `POST /api/pipelines/preview` in the new
`routes/pipeline_preview.rs`, `pipeline:write` in `POLICY_TABLE` (deny by
default would otherwise answer 500), its own `pipeline_preview_router()`
merged into `pipelines_router()` so that function stays under
`clippy::too_many_lines`, and asserted both ways in `tests/route_auth.rs`.
The handler parses `{ inputs, steps, upTo }`, requires `upTo`, compiles
through `pipeline_compile::compile(inputs, steps, Some(upTo), catalog_db)`,
runs the tenant gate as `GateActor::Caller` and then
`policy_engine::rewrite_sql_for_roles` (decision 13: as the person looking,
never as any pipeline's author), probes the columns, and runs
`SELECT * FROM (<rewritten>) AS step LIMIT 101 SETTINGS max_result_rows =
101, result_overflow_mode = 'break', max_execution_time = 30`. The response
is `{ columns: [{name, type}], rows, truncated }`, rows as arrays in column
order, at most 100 of them.

**Reuse, and what moved (AGENTS.md rule 4).** `placeholders_for` and the
`DESCRIBE` column probe moved from `routes/dashboard_sources.rs` to
`routes/support.rs`; the dashboard preview calls them and its behaviour and
its 7 unit tests and `dashboard_tenant_scope.rs` (13) are unchanged. The
gate's 404/422-to-422 rule is `authored_pipelines::gate_refusal_as_unprocessable`,
made `pub(crate)` rather than copied; the rewrite's refusal sentence is
`policy_engine::enforcement_error_message`, as the save path uses it. The
settings text is `lakehouse_bi::builder::SQL_SOURCE_SETTINGS` swapped by
`with_run_settings`, not a second `SETTINGS` string. The
`ClickHouse` error path is `upstream_error::ch_failure_for_author` — the
function `POST /api/query/run` itself uses. One `tests/sec11_guard.rs`
allowlist entry was added with its reason (the query-author exception,
spec rule 7); no existing entry or test was changed or weakened.

**Verification, run on the final commit (`0dfed1599`), in the foreground.**

```
cd rust && cargo fmt --check                                  # clean
cargo-locked clippy -j 2 -p lakehouse-api --all-targets -- -D warnings
                                                              # clean
cargo-locked test -j 2 -p lakehouse-api --lib -- pipeline_compile sql_guard
                                                              # 61 passed, 0 failed
cargo-locked test -j 2 -p lakehouse-api --lib -- preview      # 30 passed, 0 failed
cargo-locked test -j 2 -p lakehouse-api --test pipeline_preview \
  --test route_auth --test sec11_guard --test pipeline_compile_route \
  --test pipeline_definition_save --test dashboard_tenant_scope
                        # 10, 95, 4, 24, 14, 13 passed; 0 failed in each
python3 ops/lint/check_intra_package_imports.py               # OK
python3 ops/lint/check_bare_iceberg_count.py                  # R11 guard: none found
```

On the T1 commit (`264380ad6`), the same scoped set: `--test
pipeline_preview` 8 passed, `--test route_auth` 95, `--test sec11_guard` 4,
`--lib -- preview_tests` 7, `--test dashboard_tenant_scope` 13.

The route tests are in `tests/pipeline_preview.rs` and cover S2-T1's and
S2-T2's checks: `upTo` an input answers that table's rows (and the compiled
text reads Bronze through `icecat_api.`); `upTo` a step answers that step;
a column masked for the caller is masked in the statement the engine runs
and in the returned rows; an input the caller may not read is a 422 with the
fixed sentence and NOTHING reaches `ClickHouse`; a `DROP TABLE` step is a
400 whose message starts `steps[0]: `; a caller without `pipeline:write` is
403; 101 rows answer 100 with `truncated: true`; the row query carries the
limits; and a `ClickHouse` statement failure answers `422` with the same
text `POST /api/query/run` gives for it (compared up to the reference id).

**Mismatch between plan and code (for the planner)**

1. **D16's wrapper is `LIMIT 100`; the built wrapper is `LIMIT 101`.** The
   S2-T1 check requires "more than 100 rows gives 100 and `truncated: true`",
   and `ClickHouse` reports only the rows it returned, so `LIMIT 100` cannot
   tell a cut from an exact fit. One more row is asked and 100 answered —
   the rule Query Studio already follows ("one row more than the route keeps
   is asked for, so `truncated` stays honest"). The `SETTINGS` cap is 101 for
   the same reason.
2. **A table the caller may not read is a 422 here, and a 404 everywhere
   else.** S2-T1's check and section 3's contract say 422, so the gate's own
   404 refusal (`SEC-20`) and the rewrite's `TableNotReadable` (`GOV-7` D5,
   which `routes::support::enforcement_refusal` answers 404) are answered
   422 with the same fixed sentence. That is the save path's rule (D9), but
   it differs from Query Studio and the dashboard preview, which keep the
   404. No sentence changes, so nothing about a hidden table leaks; the
   console must not read this route's 422 as "bad definition" when the
   message is an access sentence. If the planner wants the 404 instead, it
   is one `map_err` and one test assertion.
3. Not a mismatch, noted for the review: a preview that hits its 30 second
   limit answers the engine's fixed failure sentence (`ch_error`'s), not
   Query Studio's "stopped after N seconds, the limit for your role", because
   that sentence names a configured limit and the preview's 30 seconds is
   decision 17's own.

**Not verified, with the reason**

- **A preview of a Bronze input through the catalog database on a running
  stack: not verified.** The wiremock proof pins that the compiled text is
  `` SELECT * FROM icecat_api.`bronze.<table>` ``, but no `ClickHouse` was
  asked. Running it needs the full stack (`docker compose --profile '*'`), a
  loaded Bronze table and `ICEBERG_QUERY_DB`; none was brought up here.
- No browser or console check: S3 owns the page, and nothing in `src/` was
  touched.
- A real `ClickHouse`'s `DESCRIBE` over a masked subquery, and its
  `max_execution_time` behaviour at 30 s: wiremock only.
- `cargo test --workspace`, the full workspace clippy run and the acceptance
  gates: not run here (the brief scopes local runs); CI runs them on the
  pull request.
- The Python suite was not run: no `dagster/` file changed.

### Handoff S2, fix round 1 (developer, 2026-11-13)

Branch `feat/bld-1-steps-preview` (base for this round `1d025c342`, which
carries the reviewer's entry and the merge of `main`).

**Commits**

| Commit | Subject |
| --- | --- |
| `53c1aceed` | test(pipelines): the preview asks nothing about the previewed table (R1-B1) |
| `513c81e9b` | fix(pipelines): several inputs need a step (R1-S1) |
| (this commit) | docs(plan): BLD-1 part 1 S2 fix round 1 handoff |

**R1-B1 — which request, and which case.** I could not observe it, so I do
not quote a body, and I say that plainly rather than invent one. The failure
did not reproduce here:

- the test alone, one process per run: 40 runs, all passed;
- the test alone through the built binary, eight processes in parallel, 15
  runs each: 120 runs, all passed;
- the whole file, three loops of 10 runs at `--test-threads=16`: 30 runs,
  all passed;
- the whole file through `cargo-locked`, 20 runs before the final
  comment-only polish and 20 after: 40 runs, all passed.

Every one of those runs used an assertion that prints the received requests,
and none ever received one. The route also cannot produce it: the tenant
gate reads Postgres, and `rewrite_sql_for_roles` refuses at `table_access`
**before** `prefetch_for_tables` can ask `ClickHouse` for
`system.columns`/`system.tables`; `describe_columns` and the row query run
only after the rewrite returns `Ok`, and either would have answered a
different sentence than the one the reviewer saw pass (`The database request
failed. Reference: …`). So this is **case 2**: a request from something else,
not about the previewed table — this machine runs many test processes at
once, every mock binds an ephemeral port, and a request one of them queued in
a background task (the column-lineage build pass is one writer that reads
`ClickHouse` after its route has answered) can land on whichever server binds
that port next.

What changed: the assertion now says what the test means — no request the
engine received names the previewed table (`silver.orders`), which is the
compiled statement, its `DESCRIBE` and the row query wrapping it. The shape
alone (`DESCRIBE`, `AS step LIMIT`) is deliberately not used, because another
preview test's probe has the same shape and would be a false failure; the
failure message prints every received request (method, url, body) so the next
failure names the intruder. The assertion is still there; no retry, no sleep,
no route change (there was no order defect to fix). If the reviewer can
reproduce it, the printed request will say which process sent it.

**R1-S1 — several inputs need a step (D1).** `pipeline_compile::validate`
refuses a definition with more than one input and no step at `inputs[1]` with
the fixed sentence "with more than one input, add a step that reads them",
when `up_to` is `None`; with `up_to` naming an input it still compiles that
input (the editor's preview of a pipeline whose step is not written yet). The
module doc's rule 1 says so. No fixture in `pipeline_definition_save.rs`,
`pipeline_compile_route.rs`, `pipeline_preview.rs`, or elsewhere in the
crate's `--lib -- pipeline` tests saved several inputs with no step, so no
existing fixture needed a step.

**Verification, on the code head (`513c81e9b`), in the foreground.**

```
cd rust && cargo fmt --check                                  # clean
cargo-locked clippy -j 2 -p lakehouse-api --all-targets -- -D warnings
                                                              # clean
cargo-locked test -j 2 -p lakehouse-api --lib -- pipeline_compile
                                                              # 38 passed, 0 failed
cargo-locked test -j 2 -p lakehouse-api --test pipeline_definition_save \
  --test pipeline_compile_route --test pipeline_preview        # 14, 24, 10 passed
cargo-locked test -j 2 -p lakehouse-api --lib -- pipeline      # 304 passed
cargo-locked test -j 2 -p lakehouse-api --test sec11_guard --test route_auth
                                                              # 4, 95 passed
```

The 20-run proof of the R1-B1 fix, `for i in $(seq 20); do cargo-locked test
-j 2 -p lakehouse-api --test pipeline_preview 2>&1 | grep '^test result';
done`, printed the same line twenty times:

```
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

(finished between 4.53 s and 6.43 s). That is 20 of 20, none failed.

### Handoff S2, fix round 2 (developer, 2026-11-13)

- **Commits:** `2c15ea883` `test(pipelines): the preview test's comment says what is known (S2 review R2-S1)` — the comment above the assertion only, rewritten to say what is known (what the test asserts and why; the gate is Postgres and the rewrite refuses at `table_access` before its prefetch; the "received nothing at all" form failed twice in the reviewer's runs on 2026-11-13, the stray request was never captured and did not reproduce in 230 runs; a request from another test process on a reused port is a guess, not a finding; the assertion prints everything received). The assertion and every other line are unchanged. `42932cdb4` `Merge remote-tracking branch 'origin/main' into feat/bld-1-steps-preview` — no conflict.
- **Counts on `42932cdb4`:** `cargo fmt --check` clean; `cargo-locked clippy -j 2 -p lakehouse-api --all-targets -- -D warnings` clean; `--test pipeline_preview` 10 passed, `--test sec11_guard` 4 passed, `--test route_auth` 95 passed, 0 failed in each.
- **What the merge touched:** 31 files, +2063 −91: 27 under `src/` (the console's WEB-11 round), `CHANGELOG.md`, the new plan `docs/superpowers/plans/2026-11-12-qa-round-web-11.md`, and `docs/core/features/pages-per-role.md`. No Rust and no `dagster/` file, so the branch's Rust is exactly what round 1 verified. No new migration: `0140` (this feature's) is still the highest, and this slice adds none.

### Handoff S3 (developer, 2026-11-13)

Branch `feat/bld-1-steps-console`, base `origin/main` at `be2c92910` (S1
merged, #481); `main` merged in once (below). Console only: no file under
`rust/`, `dagster/`, `ops/`, migrations, `docker-compose.yml` or
`design-system/` was touched, and no cargo command was run.

**Commits**

| Commit | Subject |
| --- | --- |
| `fbe642eb4` | feat(console): the pipeline contracts carry inputs and steps; preview client (S3-T1) |
| `362bcc1fa` | feat(console): the pipeline editor's state and rules, as pure functions (S3-T2) |
| `3d43f6f19` | feat(console): one pipeline editor of inputs, steps and an output (S3-T3) |
| `2c65d03fa` | feat(console): the pipeline page draws inputs, steps and the output (S3-T4) |
| `c855c9559` | test(console): the editor's own behaviours, as the plan lists them (S3-T5) |
| `5fbf13bfe` | Merge origin/main (#497, WEB-11) into feat/bld-1-steps-console |
| (this commit) | docs(plan): BLD-1 part 1 S3 handoff |

Correction: `362bcc1fa`'s message says `pipeline-editor.test.ts` has 37
tests. It had 34; S3-T4 added 2, so it has 36.

**What was built, by file**

- `src/services/contracts/pipelines.ts`, `clients/pipelines.ts`:
  `PipelineInput`, `PipelineStep`, `PipelinePreviewInput`, `PipelinePreview`;
  `AuthoredDefinition` requires `inputs`/`steps` (the API on `main` always
  sends them); create/update inputs may send them and their fields of before
  are optional (D3); `previewPipelineStep` posts through `sendJson`.
- `src/lib/pipeline-editor.ts`: state (ids that are never reused), add,
  remove, rename (a form step's `reads` follows a renamed name; SQL text is
  not rewritten), reorder, D2's limits and name rule, readable names (exact
  case, D6a), the connector's fixed first input, the check before saving,
  the save body, `definitionChanged`, D17a's rule and sentence, the preview
  body and key, `errorTarget`/`errorItemId` (D7), the columns a form step
  offers, the SQL step's completion, `sqlStepReads` (read view edges).
  `transform-draft.ts` gains `emptyDraftFor`, `draftIsComplete`,
  `TRANSFORM_VERBS`, moved unchanged from the old page.
- `src/features/pipelines/pipeline-editor.tsx` (shell, rail, preview, save),
  `pipeline-editor-panes.tsx` (input, form step, SQL step, output),
  `pipeline-transform-rows.tsx` (the five-operation rows, moved as they
  were). `pipeline-create-page.tsx` renders the editor;
  `pipeline-edit-page.tsx` imports it.
- `src/components/patterns/sql-rows-table.tsx`: `SqlRowsTable` and
  `SqlColumnChips` moved from `features/dashboards/sql-source-panel.tsx`,
  with optional `noRowsText`/`staleText` whose defaults are the dashboard's
  words; `chart-builder.tsx` and `sql-source-panel.tsx` import them.
- Read view: `pipeline-flow-layout.ts` `authoredFlow`, a `step` node kind in
  `pipeline-flowchart.tsx`, and `AuthoredFlow`'s panel in
  `pipeline-definition.tsx` (a SQL step's SQL in `CodeView`).

**The merge of `main`** (`5fbf13bfe`, #497 `WEB-11`). One conflict:
`src/services/clients/pipelines.test.ts`, resolved hunk by hunk keeping both
sides (main's refusal tests for `listPipelines`, `getPipeline` and the
writes; this branch's three `previewPipelineStep` tests). The merge also
brought:
- `src/services/clients/pipelines.ts`, merged by itself: `getJson`,
  `sendJson` and `listPipelines` call the shared `deniedFor`.
  `previewPipelineStep` goes through `sendJson`, so it follows that mapping
  with no code of its own.
- In the editor, the save error and the preview error show
  `ServiceError.message` and map nothing themselves. A 401/403 is therefore
  #497's sentence. `errorTarget` finds no `steps[i]` in that sentence, so it
  is shown at page level (save) or in the preview (preview).
- Two tests were added with the merge: a 403 on the preview is
  `permission_denied` with the shared sentence (client, `BLD-1-AC11`), and
  the editor's preview shows that sentence.

**Verification, on the head commit `5fbf13bfe` (real counts)**

```
bun run typecheck                                        # exit 0
bun run lint                                             # exit 0; 0 errors, 11 warnings (the 11 on main, none in a file this branch touched)
flock /home/shiro/rantai/.heavy.lock bun test src/features/pipelines/    # 143 pass, 0 fail (12 files)
flock /home/shiro/rantai/.heavy.lock bun test src/services/              # 253 pass, 0 fail (22 files)
flock /home/shiro/rantai/.heavy.lock bun test src/components/patterns/   #  32 pass, 0 fail (6 files)
flock /home/shiro/rantai/.heavy.lock bun test src/lib/                   # 1330 pass, 0 fail, 1 skip (129 files)
flock /home/shiro/rantai/.heavy.lock bun test src/features/dashboards/   # 636 pass, 0 fail (79 files)
python3 ops/lint/check_bare_iceberg_count.py                              # R11 guard: none found
python3 ops/lint/check_intra_package_imports.py                           # OK
git status                                                                # clean
```

Total: 2394 pass, 0 fail, 1 skip. The skip is
`src/lib/connectors/brand-marks.test.ts`, which this branch does not touch.
The dashboard count (636) is the same before and after the move of the two
components, and no dashboard test was edited.

**Old tests of `pipeline-create-page.test.tsx` (22) and where each is now**

| Old test | Now |
| --- | --- |
| re-upload: offered and ON for Bronze, new and stored | same name, on the Schedule and settings tab, "first input" |
| re-upload: not offered when the zone is not Bronze | same, for a Silver first input |
| re-upload: carries every other upstream through a save | same |
| re-upload: clears the upload upstream when off | same |
| re-upload: adds it on create, no other upstream | same (input, form step, output instead of wizard steps) |
| suggestions: real table names, typing still allowed | same; the datalist id is per input (`input-table-options-<id>`) |
| WEB-8: namespaces as the source zone | "an input's zone" (combobox "Zone") |
| WEB-8: namespaces as the target zone | "the output zone" (Output pane) |
| WEB-8: columns from the schema (was: the incremental column picker) | the input pane lists the columns with types, and a form step's Key column picks from them. The incremental column left the page (decision 18) |
| WEB-8: free text while the table does not exist | same line shown; a form step's Key column is a text box |
| BLD-7: zone fixed, connector's tables only, trigger is its load | same |
| BLD-7: sends the chosen table and the connector's load | same; asserts `inputs[0]` instead of `sourceZone`/`sourceTable` (D3) |
| BLD-7: run the ingest first, link to Ingest | same, plus Create is disabled |
| BLD-7: plain form without a connector in the URL | same |
| BLD-9: every connector and No connector, null when cleared | same, across the two tabs |
| BLD-9: sets the connector picked | same; asserts `inputs` |
| BLD-10: FBIC offered as not available, disabled and off | **no longer applies**: the FBIC row is not on the page (see mismatch 1). What is sent is still pinned by the two tests below |
| BLD-10: Next disabled with no transform until plain copy | "Create disabled on a pipeline with no step until the plain copy is switched on" |
| BLD-10: plain copy off and disabled when a transform is added | "… when a step is added" |
| BLD-10: no transform shown as a plain copy on Review | there is no Review step; "says that a pipeline with no step is a plain copy of its input" (the rail's line) |
| BLD-10: sends fbicEnabled false on create | same |
| BLD-10: sends a stored fbicEnabled true back on edit | same |

New tests (S3-T5): opens a pipeline of before as one input and one form step;
no sixth input; a SQL step completes the inputs and earlier steps; a 422
naming `steps[1]` selects step 2, shows the message, and the rail item is
described by it; the preview posts the selected input and goes stale after
an edit; a preview refusal is shown at the preview; the preview shows the
shared 403 sentence; create sends `inputs`, `steps` and `kind: "batch"` and
none of the old fields; edit sends the stored kind and no incremental
column; D17a appears on a step change for a pipeline that reads as someone
else, not on a schedule-only change and not when it reads as you; decision
13's line; move, remove and the rename that a form step follows. Read view:
two tests in `pipeline-detail-page.test.tsx`, a two-input SQL case in
`pipeline-flow-layout.test.ts`. Pattern: `sql-rows-table.test.tsx` (4).

**Existing tests changed, and why**
- `pipeline-flow-layout.test.ts`: "the transforms stacked in one column"
  became "an input, one form step, then the target", because a form step is
  now one node and its operations are listed in its panel. In the
  plain-copy test, only the input's node id changed (`input:0` instead of
  `source`); its assertions are the same.
- Fixtures in `contracts/pipelines.test.ts`, `pipeline-flow-layout.test.ts`
  and `pipeline-detail-page.test.tsx` gain `inputs`/`steps`, which the API
  returns and the type requires. What they assert is unchanged.

**Mismatches and choices for the planner**

1. **FBIC row removed from the page.** D17 lists what Schedule and settings
   holds (schedule, connector, two switches, description), and FBIC is not
   in it. The body still sends `fbicEnabled`: false on create, the stored
   value on edit (BLD-10 D1). The read view keeps its FBIC row. If the row
   should stay on the editor, it is one block to restore.
2. **Where the pipeline name lives.** The plan does not say. It is above the
   two tabs, so a create can name the pipeline from either tab.
3. **Checks the API does not make, added on the client.** A form step with
   no operation is refused before save ("Add an operation, or remove this
   step."), because BLD-10 D2 says a copy must be chosen. The API accepts
   such a step as `SELECT *`. The plain-copy switch is kept on the Output
   pane for a pipeline with no step. A second input beside a connector is
   refused on the page (D12).
4. **The preview body.** An input is previewed alone
   (`{inputs:[that one], steps:[], upTo}`). A step is sent with every input
   and only the steps up to it, so an unfinished later item does not block
   a preview. The shape is section 3's. Whether S2 accepts a body cut this
   way is **not verified**: S2 is not on `main`, and the tests stub `fetch`.
5. **D17a is also said when the pipeline has no author**, because D10 then
   makes it read as the caller. It is also said while the session is still
   loading. It is not said when it already reads as you, or for a change to
   anything but inputs and steps.
6. **Read view edges of a SQL step** come from `sqlStepReads` (names after
   `FROM`/`JOIN`, as written). It leaves out what it cannot place, such as
   `FROM a, b`, rather than invent an edge, and the page says how SQL edges
   are drawn. It is a drawing aid, not a check.
7. The flowchart's `transform` node kind is no longer produced by
   `authoredFlow`. It was kept because `layoutFlow`'s test uses it; removing
   it is a cleanup the planner may want.

**Not verified**
- **Not verified in a browser.** There is no browser on this machine, so how
  either page looks, in either theme, was not seen.
- **Not verified against a running API.** The preview route (S2) is not on
  `main`; every preview, save and refusal here is against stubbed `fetch`.
- **The whole console suite (`bun run test`) was not run.** One process
  reaches 7 GB here. The five touched folders were run one by one (above);
  CI runs the whole suite on the pull request.

### Handoff S3, fix round 1 (developer, 2026-11-13)

Branch `feat/bld-1-steps-console`. One commit per finding, or per group that
touches the same lines; each commit and each fix site cites the finding.
Console only; no cargo command was run.

**Per finding**

- **R1-B1** — `a6a3e69b6`. An edit sends the stored `incrementalColumn`
  back unchanged, as it does the kind. The comment is corrected (the store
  writes `incremental_column = $7` with no COALESCE). The test that pinned
  the loss now pins that `updated_at` is sent back.
- **R1-B2** — `0cdfb5516`. A form step holds the id of the item it reads
  (`StepRead`), and the name is written only when the body is built. A
  stored `reads` that names no item is written back as it was and flagged.
  A step moved before the step it reads says so. Tests: a rename typed
  through the empty name (lib, and on the page in `2711b9d54`), the reorder,
  the round trip.
- **R1-B3** — `0cb0a84dc`. The test switches the plain copy on (the page
  says the output is a copy of the input), then off (it does not).
- **R1-S1** — `a4895ebba`. An input's name follows its table only while no
  step names it (no form step reads it, no SQL step's text has the name). A
  SQL step that names, after FROM or JOIN, something that is not an input,
  an earlier step or a WITH name of its own gets a hint on its pane
  (`sqlStepHint`). The hint does not block a save.
- **R1-S2** — `8ea21aeb1`. The contract gains `PipelineOtherStep`
  (`kind: string`), and `knownStep` tells the kinds apart by their fields.
  In the editor such a step is shown with its kind and stored record, can
  be moved or removed, is not renamed or edited, and is saved back exactly
  as stored. The read view draws it with its name and kind and shows its
  record. Tests in lib, the editor, the flow layout and the detail page.
- **R1-S3** — `79b253578`. `saveBlockers` lists why Save is disabled (name,
  each item's problem, output, schedule). The page says each one next to
  the button, and each line goes to what it is about.
- **R1-S4** — `83e8762e7`. Until the connector's tables are read, or when
  that read fails, the stored input stays as it is: not flagged, and no
  change counted. Test with an unreadable ingest spec.
- **R1-S5** — `d9864d1b3`. `previewRequest` returns the body with the
  editor item of each input and step, and `errorItemId` reads a refusal's
  path through what was sent. The save path uses the same mapping. The rail
  marks the item a preview refusal names, the preview offers a button to
  it, and an old refusal is marked stale after an edit. A step preview
  leaves out inputs with no name or table.
- **R1-S6** — `cd6f27037`. `InputColumnsReader`, one per input, reads that
  input's columns, keyed on its own zone and table. Test: typing in the
  second input's table does not re-read the first input's columns.
- **R1-S7** — three commits, by the lines each touches:
  - `0cb0a84dc`: the rail's no-step line and the Output pane's "Written
    from" line are now true for each state (`noStepLine`,
    `outputSourceLine`).
  - `d9864d1b3`: the preview says why there is none, for the item it is
    about, and "It returned no rows." is said once
    (`SqlColumnChips noRowsText={null}`; the dashboard's default is
    unchanged).
  - `2dee9f0f6`: a refused operation's mark is kept only while the step's
    operations are the ones sent.
- **R1-S8** — `658c7d333`. Corrected: `runAs` in the contract (it changes
  on an edit, D10) and `assignRunAs` (no longer "the only way"); the create
  body's note about the builder dialog; `sql-rows-table.tsx` (two
  additions); "five-step form" in both trigger modules; the `Field` wrapper
  in `pipeline-pickers.tsx`. `sqlStepReads` was made true in `a4895ebba`:
  one scanner skips a FROM inside a function call.
- **R1-S9** — `658c7d333` and `db0f21583`. `sourceZone`, `sourceTable` and
  `transforms` left the create and update input types, and `inputs`/`steps`
  are now required there (`658c7d333`, the same lines as the comment).
  `parseTransformDraft` and `transformErrorRowIndex` are removed with their
  tests. The flowchart's `transform` node kind is removed with its card and
  verb icons; the layout test of an edge inside one column uses `step`.
- **R1-S10** — `ec3c732ec`.
  - Each rail item carries its own description element (problem, save
    refusal or preview refusal).
  - Every problem names its control (`Problem.field`), which gets
    `aria-invalid` and `aria-describedby`. `SelectField` and `ZoneField` take
    an `aria-describedby`.
  - The table label names a control only while there is one.
  - `aria-invalid` is off the list item.
  - Two existing assertions now look where the sentence is: the Output pane
    rather than the whole page (the rail has its own copy), and the rail
    item's description by text rather than by the pane's id.
- **R1-S11** — tests: the `steps[i].transforms[j]` highlight (`2dee9f0f6`);
  R1-B2's rename (`0cdfb5516` lib, `2711b9d54` page); R1-S1 (`a4895ebba`);
  R1-S2 in the editor and the read view (`8ea21aeb1`); R1-S4 (`83e8762e7`).

**The merge of `main`** — `e0bbddd3a`, no conflict. It brought PR #486
(dashboards QA round p1), which touches `chart-builder.tsx` and its test.
This branch's change to `chart-builder.tsx` is only the import of
`SqlRowsTable` from `components/patterns/`, which the merge kept.

**No finding was found wrong against the code.** Two notes on how they
were done:
- R1-S1 asks that the name follow the table only while no form step reads
  it either. Since R1-B2, a form step follows by id, so a rename would not
  break it; the rule still holds for form steps, as the review asks.
- R1-S5's "select or mark it": the item is marked on the rail and the
  preview offers a button to it. The selection is not moved by itself,
  because the person may be reading the step they previewed.

**Verification, on the head commit `e0bbddd3a` (real counts)**

```
bun run typecheck                                        # exit 0
bun run lint                                             # exit 0; 0 errors, 11 warnings (the 11 on main)
flock /home/shiro/rantai/.heavy.lock bun test src/features/pipelines/    #  154 pass, 0 fail (12 files)
flock /home/shiro/rantai/.heavy.lock bun test src/lib/                   # 1354 pass, 0 fail, 1 skip (129 files)
flock /home/shiro/rantai/.heavy.lock bun test src/components/patterns/   #   32 pass, 0 fail (6 files)
flock /home/shiro/rantai/.heavy.lock bun test src/services/              #  254 pass, 0 fail (22 files)
flock /home/shiro/rantai/.heavy.lock bun test src/features/dashboards/   #  652 pass, 0 fail (81 files)
python3 ops/lint/check_bare_iceberg_count.py                              # R11 guard: none found
python3 ops/lint/check_intra_package_imports.py                           # OK
git status                                                                # clean
```

Total: 2446 pass, 0 fail, 1 skip. The skip is
`src/lib/connectors/brand-marks.test.ts`, which this branch does not touch.
`src/features/dashboards/` was run because the shared `SqlColumnChips` and
`SelectField` changed. Its count rose from 636 to 652 with the 16 tests
`main` brought.

**Not verified**
- **Not verified in a browser.** There is no browser on this machine.
- **Not verified against a running API.** S2's preview route is still not on
  `main` (no `pipelines/preview` in `routes/mod.rs` at `43d36f55b`), so every
  preview, save and refusal here is against stubbed `fetch`.
- **The whole console suite was not run** (memory limit); the five folders
  above were run one by one, and CI runs the whole suite on the pull
  request.

## 7. Review


### S1, round 1 (reviewer, 2026-11-12)

Reviewed `origin/main...7b48cb44b` (36 files, +4452 −1180) against sections
2 and 4. `main` was merged into the branch before this entry (no conflict).

**Not re-run by the reviewer this round.** The slice goes back for the
findings below, and the reviewer's own scoped run happens on the fixed
commit. CI's full run on `7b48cb44b` had not finished when this was written;
its last finished runs were on unfinished commits.

What is right, so it is not redone: the compiler is one pure module with
fixed refusal text and paths; `check_sql_source` kept its behaviour and its
tests and shares one core with the step check; the route, the save path and
the restore path call one "who reads" function; the store writes `source`,
`transforms`, `input_tables`, `inputs` and `steps` from one place; the
orchestrator runs only what the API returns and refuses a 2xx without `sql`
or `readsIceberg`; the handoff is exact about what was not run.

#### BLOCKER

- **R1-B1. Every pipeline loses its column lineage (`DATA-13`).** The noted
  statement is now a `WITH`, which `sql_lineage` records as partial. The
  rewritten test `a_successful_run_records_partial_mappings_through_the_compiled_ctes`
  (`tests/column_lineage_pipeline.rs:289`) pins target columns `amount`,
  `id`, `price` for a step that outputs `id`, `amt`, `price_f`: rows for
  columns that do not exist (principle 2). `ops/g7/g7_pipeline_test.py:650`
  still requires the exact, non-partial mapping, so CI job `g3a-dagster`
  fails there. The handoff names the loss (mismatch 2) but not the gate.
  Fix: **D4a** and **D4b**. Restore the old assertions of the lineage test
  (the exact walk for `price_f`, the three pairs) on a pipeline of today's
  shape; add one test that a pipeline with a SQL step writes no column row.
- **R1-B2. A test still calls the removed route.**
  `tests/table_access.rs:942` posts to `/enforce`. Retarget it to the compile
  route and keep what it proves (`GOV-7`: an author whose role may not read
  the table is answered "not found" with the fixed sentence). The handoff's
  count for this file is from before S1-T4 removed the route.
- **R1-B3. A save from today's console can destroy a pipeline.**
  `definition_form` (`routes/authored_pipelines.rs:1055`) turns today's form
  into one input and at most one step on every `PUT`, and the store
  overwrites `inputs` and `steps`. Fix: **D3a**, with tests: a `PUT` in
  today's form on a two-input pipeline is 409 and changes nothing; on a
  pipeline of today's shape with its own names it keeps the names and does
  not move `run_as`.
- **R1-B4. The migration and the Rust rule disagree, and a migration cannot
  be edited once applied.** For a `source` with no dot or with two, the SQL
  takes `split_part(.., 1)` and `split_part(.., 2)` while
  `legacy_definition` splits at the first dot: `serving.sales.orders`
  becomes input `serving.sales` in SQL, a different table that compiles and
  is read. A table named `Transformed` gets a step named `transformed`, which
  the compiler then refuses as a duplicate, so a pipeline that ran stops.
  The comment "running this UPDATE again is idempotent" is not true for a
  row with several inputs or steps. Fix: one rule in both (split at the
  first dot; the step's name avoids the input's name without regard to
  case); correct the comment; the equality test gets fixtures with no dot,
  two dots, a table that is not a usable name, a table named `Transformed`,
  and runs the migration's `UPDATE` on existing rows.
- **R1-B5. A stored definition that cannot be decoded becomes a plain copy.**
  `lakehouse-store/src/pipelines.rs:1059`, `:1060` (and `:869`, `:871` for a
  snapshot) use `unwrap_or_default()`; `:1275` to `:1279` and `:1533` write
  `unwrap_or(Value::Null)`. A row whose `steps` do not decode compiles to
  `SELECT * FROM <input>` and overwrites its output with unfiltered rows
  (principle 3). Fix: a decode failure is a `StoreError`; the compile route
  answers its fixed refusal; the detail route does not invent an empty
  definition. Test both.
- **R1-B6. A reference that differs only in case is read as a real table.**
  `pipeline_compile.rs:292`, `:349`: `reads` and bare names are compared
  lowercased. Input `SECRET`, step `SELECT * FROM secret`: the compiler
  accepts it, the rewrite skips `secret` as a `WITH` name, and `ClickHouse`
  looks up `secret` as a table. Fix: **D6a**, with tests for `reads`, a bare
  name, `upTo`, and a qualified name.
- **R1-B7. Two readers of the single source were missed (D11).**
  `routes/lineage.rs:459` draws only `pipeline.source`;
  `authored_factory.py:1397` `_pipeline_source` gives the lag sensor only
  the first input, so a pipeline with a time lag whose second input is made
  by a `downstream` pipeline never has that pipeline run. The handoff's "no
  fifth reader exists" is not so; its own grep matches the first. Fix:
  **D11a**, with a test for each.
- **R1-B8. Tests were weakened (rule 2).**
  `tests/pipeline_compile_route.rs`: four assertions that were exact equality
  with the expected statement are now two `contains` (`:411`, `:584`,
  `:772`, `:833`); "an author with no obligation gets no mask" is no longer
  asserted anywhere; `!sql.contains("SELECT email FROM serving.enforced_x")`
  (`:387`) can no longer fail. With D4a the old fixtures give the old
  statements again: restore the exact assertions, and keep the SQL-step cases
  as additional tests that pin the whole compiled text and assert the mask
  expression is absent where it must be. In `routes/pipelines.rs`'s tests, 16
  call sites swapped a restricted principal for the seeded administrator:
  put the nine that do not pass through a save back, and give the seven that
  do a restricted user who may read the table, so they still exercise a
  caller who is not an administrator.
- **R1-B9. `SEC-71` is not closed for a service identity, and its refusal has
  no test.** `author_after_edit` (`routes/authored_pipelines.rs:1416`)
  ignores a caller who is not a user, so a service identity with
  `pipeline:write` can change what another person's pipeline reads and it
  still runs as that person. Fix: **D10a**. Add the tests the plan's S1-T3
  check lists and the slice lacks: a second user's edit is refused when they
  may not read an input; a service identity can create a pipeline and it has
  no author; and for D10a, a restricted service identity's edit of the steps
  clears `run_as` while an edit of the schedule does not.

#### SHOULD-FIX

- **R1-S1.** `save_check_compiled` turns every gate error into a 422 with
  `other.to_string()` (`gate_message`). An error that is not a refusal keeps
  its own status at save, as at the run route
  (`gate_refusal_as_unprocessable`); an outage must not read as a bad
  request.
- **R1-S2.** The orchestrator retries a 404 from the compile route (an
  author whose role may not read an input, `GOV-7`) and then fails with the
  HTTP client's text, which holds the API's internal address. Treat 404 as a
  refusal: not retried, the fixed sentence. `docs/OPERATIONS.md` then says
  what is true: which refusals carry an `inputs[i]` or `steps[i]` path, the
  404 case, and that the orchestrator no longer needs `ICEBERG_QUERY_DB`.
- **R1-S3.** Python leftovers (rule 1): the module doc and the comments at
  `authored_factory.py:9`, `:134` to `:149`, `:292`, `:337` to `:362`,
  `:584`, `:612` still describe the old flow or the deleted
  `authored_transforms.py`; `iceberg_query_db` is read and never used;
  `backup_job.py:322` cites the deleted file;
  `test_authored_factory.py:357` asserts something that cannot fail. Add the
  test that was lost: no statement carries `allow_database_iceberg` when
  `readsIceberg` is false; restore the assertion on which statements carry it
  when true.
- **R1-S4.** Tests the plan lists that are missing or thin: `inputs_of_conn`
  returns both inputs; an old snapshot restored through the route; an unsafe
  zone; the `--` comment and the two `up_to` cases pin the whole text; a
  `transform`-step pipeline through the compile route; a stored definition
  that does not compile answers the route's 422.
- **R1-S5.** The new G7 case: the direct query has no
  `SETTINGS allow_database_iceberg = 1`, which every other read of the
  catalog database in that file has; and a self-join on `id` equals the
  direct sum only if `id` is unique. Compare with the same join run
  directly.
- **R1-S6.** `pipeline_compile.rs:61`: the doc comment above `MAX_INPUTS`
  describes something else. The assistant's tool test uses zone `serving`,
  which the tool's own schema does not list: make the schema and the test
  agree on the zones an input may have.

#### Notes, nothing for the developer

- Handoff mismatch 1 is settled: the feature page on `main` carries the
  signatures (#480).
- Handoff mismatch 3 (a restricted user saving a pipeline that has no tenant
  is refused) stands as built; it is the run route's rule applied at save.
- A consequence of decision 14 the product owner should know: see **D17a**.
- Not in this slice, not verified on a running `ClickHouse`: the rewrite and
  the gate leave out a bare name that matches any `WITH` name without regard
  to case, in every surface that takes SQL. The planner checks it and adds a
  backlog item.

### S1, round 2 (reviewer, 2026-11-12)

Reviewed the fix round, `85a26c224..b5e22cd42` (14 commits by the developer,
`main` merged at `64055f47a`), finding by finding.

**Re-run by the reviewer on `b5e22cd42`**, in the developer's worktree, with
`cargo-locked -j 2`:

```
cargo fmt --check                                                          # clean
cargo test -p lakehouse-api --lib -- pipeline_compile sql_guard transform_grammar   # 78 passed
cargo test -p lakehouse-api --test column_lineage_pipeline                 # 8 passed
cargo test -p lakehouse-api --test pipeline_compile_route                  # 24 passed
cargo test -p lakehouse-api --test pipeline_definition_save                # 14 passed
cargo test -p lakehouse-api --test sec11_guard                             # 4 passed
cargo test -p lakehouse-api --test table_access                            # 14 passed
```

The counts equal the handoff's. Not re-run by the reviewer: the store's
`--test pipelines`, `route_auth`, `upload_routes`, clippy, and the Python
suite (the handoff's counts stand unchecked). CI's full run on `b5e22cd42`
had not started when this was written; the last full run that finished was on
`f215471ba` and failed the two tests that became R1-CI1 and R1-CI2. The G7
gate has not passed on any commit of this slice yet.

**Closed, read in the code:** R1-B1 (today's shape compiles with no `WITH`;
the lineage test is `main`'s again; no SQL is noted for other shapes), R1-B2,
R1-B3 (`legacy_form_over_stored`), R1-B4 (the migration splits at the first
dot and its backfill is a function the test runs), R1-B5, R1-B7, R1-B8 (the
four exact assertions are back; the principals are members again, not the
administrator), R1-B9 (`RunAsChange`), R1-S1 to R1-S6, R1-CI1 (the guard's
pattern is untouched; one stale allowlist entry is gone), R1-CI2.

**Accepted as reported:** the `Clear` branch of D10a cannot be reached over
HTTP today, because a restricted service identity has no tenant and
`in_scope` answers 404 before `update` runs. So the gap R1-B9 named could not
be used through the API; the rule is in place for the day a service identity
can have a tenant.

#### BLOCKER

- **R2-B1. D6a is not met for a SQL step: the names are compared after they
  were lowercased.** `check_step_tables` (`pipeline_compile.rs:372`) takes
  its names from `sql_rewrite::referenced_tables`, and that collector
  lowercases every name (`object_name_parts`, `sql_rewrite.rs:4801`) and
  drops any bare name that equals a `WITH` name of the step, also lowercased
  (`:541`). So:
  - input `orders`, step `SELECT * FROM ORDERS`: the collector gives
    `orders`, which equals the input's name, and the step is accepted.
    `ClickHouse` does not match `ORDERS` to the `WITH` name `orders` and
    looks it up as a table. This is the case R1-B6 was about; the test added
    for it (`a_read_that_differs_only_in_case_is_refused`, input `SECRET`,
    step `FROM secret`) covers only the other direction.
  - step `WITH T AS (SELECT 1) SELECT * FROM t`: `t` is dropped as the
    step's own `WITH` name and never checked.
  - input `Orders`, step `SELECT * FROM Orders`: refused, although it is
    exact. A name with a capital letter can never be read by a SQL step.

  Fix: the step check reads the names as written. Parse the step with the
  parser already used there and walk it with `sqlparser`'s visitor (the
  `visitor` feature is on for this crate): every table relation in the
  step, at any depth, is either a one-part name that equals, case included,
  an input's name, an earlier step's name, or a `WITH` name declared in that
  same step (also compared as written); or it is refused. A name of more
  than one part is refused (D6a). A table function is left to the rewrite,
  as today. `sql_rewrite` itself is not changed in this slice.
  Tests, each on the compiler: input `orders` with `FROM ORDERS` is refused;
  input `Orders` with `FROM Orders` is accepted and compiles; a step's own
  `WITH t` read as `T` is refused and read as `t` is accepted; a name inside
  a subquery and inside a `JOIN` is checked the same way; the existing D6a
  tests still pass.

#### Before this can leave draft

- R2-B1 fixed and handed off.
- `main` merged into the branch again (it is behind).
- CI's full run green on the head commit, including `g3a-dagster` (the G7
  case and the author gate), which no local run covers.

### S1, round 3 (reviewer, 2026-11-12)

Reviewed `e2cd356b2..911cd742a`: the fix for R2-B1, the merge of `main`, the
migration renamed to `0140`.

- **R2-B1: closed.** `check_step_tables` (`pipeline_compile.rs`) parses the
  step and walks it with `sqlparser`'s visitor (`StepReads`): it collects the
  step's own `WITH` names and every `TableFactor::Table` that is not a
  function call, at any depth, and compares each as written. A name of more
  than one part is refused. `sql_rewrite.rs` is unchanged. The tests cover
  both directions of a case difference, the step's own `WITH`, a subquery and
  a join.
- **The migration is `0140_pipeline_inputs_and_steps.sql`.** `main` took
  `0137`, `0138` and `0139` while this slice was in review. No reference to
  the old number is left in the code. D13's "0137" above is history.
- `main` merged again by the reviewer with this entry (documents only on
  `main`; no conflict).

**Re-run by the reviewer on `911cd742a`**, in the developer's worktree:
`cargo test -p lakehouse-api --lib -- pipeline_compile sql_guard transform_grammar`:
82 passed, the handoff's count. The other suites were re-run by the reviewer
in round 2 on `b5e22cd42` (24, 14, 8, 4, 14); since then only
`pipeline_compile.rs` and the migration's file name changed, and the
developer's counts for this round stand unchecked by the reviewer.

**No open BLOCKER and no open SHOULD-FIX.**

**Not yet true, and required before merge:** CI's full run green on the head
commit. No full run has finished green on any commit of this slice, and the
two gates in `g3a-dagster` (the G7 case with two inputs and a SQL step; the
author gate against the compile route) have never run to the end on it. The
pull request stays a draft until that run is green; a failure there reopens
the review.

### S2, round 1 (reviewer, 2026-11-13)

Reviewed `origin/main...30bc20a88` (10 files, +1073 −27) against D16 and
S2-T1, S2-T2. `main` was merged into the branch with this entry (documents
only; no conflict).

What is right: the route is one small module over the compiler's `up_to`
seam; it runs as the caller through `StatementGate` and
`rewrite_sql_for_roles`; the wrapper reuses the dashboard preview's settings
and `with_run_settings`; the column probe and the placeholder set were moved
to `routes/support.rs` and shared, not copied; an engine failure goes through
`ch_failure_for_author`, the function Query Studio uses, and the guard's new
allowlist entry names that function and the reason (the same exception
`query.rs` has; the guard's pattern is untouched); the policy row is asserted
both ways.

Accepted as reported in the handoff: the row query asks for 101 rows and
answers 100, so `truncated` is exact; a table the caller may not read is 422
here, as D16 and section 3 say; a timeout answers the engine's fixed
sentence.

**Re-run by the reviewer on `30bc20a88`**, in the developer's worktree, with
`cargo-locked -j 2`: `cargo fmt --check` clean; `--test sec11_guard` 4
passed; `--test pipeline_preview` **9 passed, 1 failed**. Not re-run by the
reviewer: clippy, `route_auth`, the dashboard preview's tests (the handoff's
counts stand unchecked). CI's full run on the head had not finished.

#### BLOCKER

- **R1-B1. `an_input_the_caller_may_not_read_is_refused_before_the_engine`
  fails some of the time.** `tests/pipeline_preview.rs:396` asserts that the
  mock engine received no request at all. In the reviewer's runs it failed
  once when the whole file ran and once in three runs of that test alone;
  the status and the sentence were right each time, and only the "received
  nothing" assertion failed. A test that fails one run in three cannot go to
  `main`, and what it asserts matters: the refusal for a table the caller may
  not read must come before anything about that table is asked of the engine
  (`SEC-50` says the same for the run route).
  Find out first which request reaches the engine: print the bodies the mock
  received on a failing run. Then one of two things is true, and each has its
  own fix:
  1. The request comes from the preview itself (the gate or the rewrite asks
     the engine about the statement before the refusal is decided, depending
     on a cache or on timing). That is a defect in the order of the route's
     steps: fix the order so the refusal is decided first, and keep the
     assertion as it is.
  2. The request comes from something else the test app starts (a background
     task, a warm-up, a health probe) and is not about the previewed table.
     Then the assertion is wrong, not the route: assert what the test means,
     that no request the engine received names the previewed table and none
     is a `DESCRIBE` or the row query, and say in the test which other
     request may arrive and why.
  Do not remove the assertion and do not add a retry or a sleep. Run the
  whole file 20 times in a row afterwards and quote the count in the
  handoff (`for i in $(seq 20); do ... --test pipeline_preview; done`, each
  through `cargo-locked`).

No SHOULD-FIX.

### S2, round 2 (reviewer, 2026-11-13)

Reviewed `1d025c342..315f7b464`: the fix for R1-B1, the planner's addition
R1-S1, the comment of R2-S1, and the merge of `main`.

- **R1-B1: closed, with what is not known stated.** The developer could not
  reproduce the failure (230 runs) and showed from the code that the route
  asks the engine nothing before the refusal: the gate reads Postgres, and
  the rewrite refuses at table access before its prefetch. The assertion now
  says what the test means (nothing the engine received names the previewed
  table) and prints everything received. Which request reached the mock in
  the reviewer's two failing runs was never captured; the test's comment says
  so and calls the reused-port explanation a guess (R2-S1, sent by message
  and fixed in `2c15ea883`).
- **R1-S1 (sent by message; plan D1): closed.** The compiler refuses several
  inputs with no step at `inputs[1]` when the whole definition is compiled,
  and still compiles one input for a preview. Four tests.
- `main` merged by the developer (`42932cdb4`; no Rust in it).

**Re-run by the reviewer**, in the developer's worktree with
`cargo-locked -j 2`, on `c405885e6`: `--lib -- pipeline_compile` 38 passed;
`--test pipeline_preview` passed all 10 tests in 10 of 10 runs. Since then
only a comment and the merge of `main` changed. Not re-run by the reviewer:
clippy, `route_auth`, `sec11_guard` on the head (the handoff's counts stand
unchecked).

**No open BLOCKER and no open SHOULD-FIX.** Merges on a green full run of
the head commit.

**Not verified by anyone:** a preview of a Bronze input through the catalog
database on a running stack, and the 30 second limit against a real engine.
The acceptance checklist's `BLD-1-AC3` covers the first.

### S3, round 1 (reviewer, 2026-11-13)

Reviewed `origin/main...87017487a` (24 files, +3798 −1381) against D17, D17a
and S3-T1 to S3-T5, and against the API on `main`.

**Re-run by the reviewer on `87017487a`**, in the developer's worktree:
`bun run typecheck` exit 0; `bun run lint` 0 errors, 11 warnings (the 11 on
`main`); under the lock, `bun test src/features/pipelines/` 143 pass, 0 fail;
`bun test src/lib/pipeline-editor.test.ts src/lib/transform-draft.test.ts
src/components/patterns/sql-rows-table.test.tsx
src/services/clients/pipelines.test.ts` 62 pass, 0 fail. Not re-run by the
reviewer: `src/lib/` as a whole, `src/services/` as a whole,
`src/features/dashboards/` (the handoff's counts stand unchecked); CI runs
the whole suite. **Not verified by anyone: how the page looks in a browser,
and the page against a running API.**

What is right: the rules are pure functions in `src/lib` with their own
tests; saves send only `inputs` and `steps`; the limits and the name rule
equal the compiler's; a preview's answer is filed by item and by the body
sent, a second run aborts the first, and an old answer cannot land on
another item; `useService` and `useServiceAction` throughout, no hand-rolled
fetch, no mock of a service, no native select; the connector entry, its
table list and both trigger switches behave as on `main`; the moved
`SqlRowsTable` and `SqlColumnChips` leave the dashboard panel as it was; 21
of the 22 old tests are carried over and the handoff maps each.

Accepted as reported: the FBIC row leaves the editor (the body still sends
`fbicEnabled`; the read view keeps its row; `BLD-10` allowed hiding it); the
pipeline's name sits above the tabs; D17a's sentence is also said for a
pipeline with no author; a second input beside a connector is refused on the
page (D12); the read view's edges for a SQL step are a drawing aid.

#### BLOCKER

- **R1-B1. Every edit erases the stored incremental column, and the comment
  says it does not.** `pipeline-editor.tsx:243`: "the incremental column is
  not sent, which the API reads as 'leave it alone'". The API writes
  `incremental_column = $7` with no `COALESCE`
  (`lakehouse-store/src/pipelines.rs:1611`), so an edit of the schedule alone
  sets the column to NULL. Decision 18 took the field off the page, not out
  of the row. Send the stored `incrementalColumn` back unchanged on edit, as
  the stored `kind` is, correct the comment, and turn the test at
  `pipeline-create-page.test.tsx:754` round: it pins the loss.
- **R1-B2. Clearing a name and typing a new one detaches every form step
  that read it.** `src/lib/pipeline-editor.ts:214` follows a rename only
  while `old !== ""`. Backspacing `orders_raw` to nothing moves each reader's
  `reads` to `""`; the next keystroke no longer follows. A form step's
  `reads` must follow the item it reads, not the text of its name: hold the
  read item's id in the editor's state and write the name when the body is
  built. Test: a rename typed one character at a time through the empty
  string keeps the reader attached.
- **R1-B3. A test was carried over without what it asserted.** Old "BLD-10:
  no transform shown as a plain copy on Review" switched the plain copy on
  and asserted the summary. The new test (`:592`) asserts a line the rail
  shows for any pipeline with no step, and passes when the copy was never
  chosen. Assert what the old one did: with the plain copy switched on, the
  page says the output is a copy of the input, and with it off it does not
  (rule 2).

#### SHOULD-FIX

- **R1-S1. Changing an input's table renames the input under a SQL step.**
  `pipeline-editor.ts:243`: the name follows the table while it still equals
  the suggestion. Input `orders`, step `SELECT * FROM orders`; the table
  becomes `orders_v2`, the name with it, and the SQL now names something
  that is not an input, which the page does not say until the server refuses
  the save. Let the name follow the table only while no step reads it (no
  form step reads it and no SQL step's text names it). And flag on the page,
  as a hint, a SQL step that names something after `FROM` or `JOIN` that is
  neither an input, an earlier step nor a `WITH` name of its own; the server
  stays the authority.
- **R1-S2. A step of a kind the console does not know crashes the editor and
  the read view.** `contracts/pipelines.ts:174` is a closed union
  (`CODE-STANDARD.md` 4.2 asks for `| string`); `pipeline-editor.ts:78`
  treats anything not `"sql"` as a form step and spreads `transforms`;
  `pipeline-flow-layout.ts:192` tokenises an undefined `sql`. Part 2 adds
  kinds. An unknown step is shown as one the page cannot edit, is kept
  exactly as stored when the pipeline is saved, and is drawn in the read
  view with its name and kind.
- **R1-S3. The page does not say why Save is disabled.** A stored pipeline
  the API accepts can fail a rule only the page has (a form step with no
  operation, several inputs with no step), and a missing pipeline name or
  schedule disables Save with no word. Next to the button, say what is
  missing and where, and select the item on a click.
- **R1-S4. On a pipeline fed by a connector, the page reports a change
  nobody made.** `pipeline-editor.tsx:137`: while the connector's tables
  load, or when that read fails, the first input's table is empty, so D17a's
  sentence shows and the input is flagged. Until the tables are loaded, keep
  the stored input as it is and count no change.
- **R1-S5. A preview's refusal is not put on the item it names.** The client
  comment (`clients/pipelines.ts:174`) says the editor reads it; the editor
  prints the text. An input is previewed alone, so a refusal about the third
  input says `inputs[0]`; a step preview sends every input, so one unfinished
  input blocks every step's preview. Map the path in the answer back to the
  item of the body that was sent and select or mark it; leave an input with
  no table out of a preview body; mark an old refusal stale after an edit,
  as an old answer is.
- **R1-S6. Each keystroke in one table name re-reads the columns of every
  input.** `pipeline-editor.tsx:175`: one read keyed on all inputs. Read per
  input, and only when that input's zone or table changed.
- **R1-S7. Words on the page that are not true.** The Output pane says "No
  steps: the output is a copy of the input." with several inputs or with the
  copy not chosen, and its preview hint says "Give it a name first" where no
  name is missing; "It returned no rows." appears twice in the preview; a
  refused operation's red row moves to another row when an earlier one is
  removed.
- **R1-S8. Comments that are not true (rule 1).**
  `contracts/pipelines.ts:60` and `:453` (the author never changes on edit:
  false since D10); `:279` (the old fields kept "for the assistant's builder
  dialog": nothing in the console sends them); `sql-rows-table.tsx:11` (one
  addition: there are two); `pipeline-editor.ts:545` (an edge it draws is one
  the step has: not for `EXTRACT(x FROM paid)`); "five-step form" in
  `pipeline-reupload-trigger.ts:5` and `pipeline-connector-trigger.ts:7`;
  "`Field` wrapper" in `pipeline-pickers.tsx:111`.
- **R1-S9. Leftovers.** `parseTransformDraft` and `transformErrorRowIndex`
  have no caller outside tests; the flowchart's `transform` node kind is no
  longer produced; `sourceZone`, `sourceTable` and `transforms` were made
  optional on the create and update inputs although the console never sends
  them (remove them from those types; `AGENTS.md`: remove consumers rather
  than optional a field).
- **R1-S10. Accessibility.** `aria-describedby` on a rail item points at an
  element that exists only while that item's pane is open; a hint from the
  page's own rules has no id and is not tied to its field; the table label
  has no target while the connector's tables load; `aria-invalid` sits on an
  `<li>`.
- **R1-S11. Tests for what was found:** the highlight of
  `steps[i].transforms[j]`; R1-B2's rename; R1-S1's table change with a SQL
  step; R1-S2's unknown step kind in the editor and in the read view; R1-S4.

#### For the API, not for this developer

- D1 says a pipeline with no step has exactly one input. The compiler does
  not refuse several inputs with no step; it copies the first. The page's
  own rule ("With more than one input, add a step that reads them") is
  right; the compiler gets the same rule in S2's fix round, so the two agree.

### S3, round 2 (reviewer, 2026-11-13)

Reviewed the fix round, `229885ea3..33882bd7f` (15 commits by the developer;
`main` merged at `e0bbddd3a`), against the fourteen findings of round 1.

**Closed, read in the code:** R1-B1 (an edit sends the stored
`incrementalColumn` back, `pipeline-editor.tsx:256`), R1-B2 (a form step
holds the id of what it reads, `StepRead` in `src/lib/pipeline-editor.ts`;
the name is written when the body is built), R1-S2 (the step type is
widened; `knownStep` guards the two kinds the page edits; another kind is
kept as stored). **Closed as reported in the handoff, not each read line by
line by the reviewer:** R1-B3 and R1-S1, R1-S3 to R1-S11. The developer's
two notes are accepted: the name of an input still stops following its table
once a form step reads it, and a preview's refusal marks the item and offers
a button to it instead of moving the selection.

`SelectField` gained one prop, `aria-describedby`. It names a sentence, not
a style, so the component stays the data-only wrapper `WEB-9` made it.

**Re-run by the reviewer on `33882bd7f`**, in the developer's worktree:
`bun run typecheck` exit 0; `bun run lint` 0 errors, 11 warnings (the 11 on
`main`); under the lock, `bun test src/features/pipelines/` 154 pass, 0 fail;
`bun test src/lib/pipeline-editor.test.ts src/lib/transform-draft.test.ts
src/components/patterns/ src/services/clients/pipelines.test.ts
src/services/contracts/pipelines.test.ts` 109 pass, 0 fail. Not re-run by
the reviewer: `src/lib/` and `src/services/` as a whole,
`src/features/dashboards/` (the handoff's counts stand unchecked). CI runs
the whole console suite.

**No open BLOCKER and no open SHOULD-FIX.**

**Before merge:** S2 (#500, the preview route) merges first, because this
page calls it; then `main` is merged into this branch (the plan file needs
both slices' entries kept) and CI's full run must be green on the head.

**Not verified by anyone:** how the editor and the read view look in a
browser, in either theme, and the page against a running API (every save,
preview and refusal was tested against a stubbed `fetch`). The product owner
looks at it; `BLD-1-AC1` to `AC14` on the feature page are the checklist.
