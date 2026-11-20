# `BLD-17` (`BLD-1` part 3): publish a step as its own table, choose what a run does when the columns change, and check quality rules before publishing

Planner and reviewer: Claude Opus. Developer: **Hy4 flash** (OpenCode), both
slices, as the orchestrator assigned. Nobody reviews their own slice.

> **Role note for the orchestrator.** S2 adds controls to panes that already
> exist (a publish section on a step pane, a choice and a list on the Output
> pane, one switch on the quality rule form, one node in the flow). The
> planner reads that as additions, not a revamp or a new screen. If the
> orchestrator reads it as new UI, `AGENTS.md`'s rule of 2026-11-04 gives S2
> to an Opus developer agent; nothing in this plan depends on which.

Base: `origin/main` at `fd325da69` (BLD-16 merged as #619). Every `file:line`
below was read at that commit. Spec: `docs/core/specs/bld-1.md` (the two
rows marked part 3). Feature page: `docs/core/features/pipeline-joins-and-sql.md`,
capabilities 14 and 15, decisions 1 to 18. Part 1's plan
(`2026-11-12-bld-1-part-1-steps-and-sql.md`) and part 2's
(`2026-11-20-bld-16-joins-and-aggregations.md`) still hold; this plan only
adds to them.

| Slice | Branch | What | Developer | Blocked by |
| --- | --- | --- | --- | --- |
| S1 | `feat/bld-17-publish-api` | Migration `0154`, the store, the compiler's published statements, the publish check route, the quality gate, lineage and order, the assistant, the orchestrator, the `g7` case | Hy4 flash | The owner signing section 2's planner defaults (P1 to P15) |
| S2 | `feat/bld-17-publish-console` | The publish section on a step, the column-change choice and the blocking rules list on the Output pane, the switch on a quality rule, the read view and the flow | Hy4 flash (see the role note) | S1 merged |

**Why two pull requests.** Same as BLD-16: the API slice is safe alone. A
console that does not know `published` or `onColumnChange` sends neither on
save, and the update route keeps both as stored when they are absent (S1-T4),
so a pipeline the assistant publishes between S1 and S2 is not damaged by a
save from the part 2 editor.

## 1. Why

Read from `main` at `fd325da69`:

- **A step cannot be reused.** A pipeline writes one table, the last step's
  result (`pipeline_definition.target`, `rust/migrations/0007_pipelines.sql:25`;
  `AuthoredDefinition.target_zone`/`target_table`,
  `rust/crates/lakehouse-store/src/pipelines.rs:612-614`). A cleaned table
  that two pipelines need is built twice, or a second pipeline is made only to
  write it.
- **A run takes whatever columns the last step returns.** The orchestrator
  builds the new rows in a staging table and swaps it in
  (`dagster/dispar_orchestrate/authored_factory.py:474-546`,
  `EXCHANGE TABLES` at `:539-543`). Nothing compares the columns; the
  `CREATE TABLE IF NOT EXISTS` of the live table does nothing once it exists
  (`:466-471`), so a removed or retyped column reaches every dashboard
  silently. The feature page lists this as a limit ("A dashboard that reads
  a column the pipeline no longer returns breaks").
- **Quality rules run after the data is live, and never block.** After
  `run-finished`, `quality_load::spawn_pipeline`
  (`rust/crates/lakehouse-api/src/routes/pipelines.rs:2848`) runs the target
  table's `on_load` rules in the background (`routes/quality.rs:1797`
  `run_on_load`), "nothing here can fail the event" (`quality.rs:1792-1796`).
  A failed rule opens an incident; the bad rows are already served.

What the code already gives us:

- The compiler can stop at any step: `compile(inputs, steps, up_to, ..)`
  (`rust/crates/lakehouse-api/src/pipeline_compile.rs:184`) renders the
  `WITH` up to the named step and `SELECT * FROM` it. That is the published
  table's statement, put through the same access rewrite as the output.
- The staging table exists, filled, before the swap
  (`authored_factory.py:522-534`). That is the one point where the columns
  and the quality rules can be checked against the new rows while readers
  still see the old ones.
- A quality check's SQL takes its `FROM` as a parameter
  (`quality.rs:486` `check_sql(check, from, per_load)`), and
  `catalog_source::clickhouse_source` (`routes/catalog_source.rs:102`)
  resolves any `db.table` to a `ReadSource` with its columns.

Findings, each handled below or named for the backlog (the planner adds the
rows after the owner signs; this plan does not edit them in):

- **F1. A `gold` output is written to a `ClickHouse` database named `gold`,
  which the catalog does not list.** The run writes `{targetZone}.{table}`
  as given (`authored_factory.py:612`, `:463`); the seed creates `serving`,
  `silver` and `lake` only (`scripts/seed-clickhouse.sql:34-36`); the
  catalog, schema versions and quality read `serving`
  (`routes/schema_versions.rs:91`, `routes/lineage/columns.rs:53-60`,
  `routes/quality_load.rs:41-43`). Not changed here. BLD-17 keys rules with
  `zone_key` (as `quality_load` does) and reads the staging table from the
  database the run wrote, so it behaves the same whichever way F1 is fixed.
- **F2. Two pipelines may write the same output table.** No unique index
  (`0007_pipelines.sql:50` is on `name`), no check in create
  (`routes/pipelines.rs:3106`) or update (`routes/authored_pipelines.rs:2220`).
  BLD-17 refuses a published table that any pipeline already writes (P3);
  the same rule for output tables is a separate item, because it can refuse
  a save that works today.
- **F3. The API has no name rule for the output table**; only the
  orchestrator checks it, at run time (`authored_factory.py:618-626`).
  Published tables get the step-name rule at save (P3). The output is left
  as it is (separate item).
- **F4. The first run creates an empty live table before its rows exist**
  (`_ensure_target_table` at `authored_factory.py:515`, before the insert).
  With checks before publishing, a first run that fails its checks would
  publish an empty table. S1-T9 moves the create after the checks.

## 2. Decisions

The product owner's, by number on the feature page: 3 (reads as its author,
so masking carries into every table a run writes), 8 (no Replace, Append or
Merge choice: every run rebuilds every table it writes), 9 (order between
pipelines is derived from the inputs), 11 (publishing a step is part 3), 14
(an edit moves who it reads as). Spec, part 3 rows: "A step can be published
as a table other pipelines read"; "A choice of what a run does when the
columns change; quality rules before the output is published".

Everything below is **(planner default, owner may change)** and is not
signed. The planner writes P1 to P15 on the feature page as the next free
decision numbers after BLD-16's (19 to 27), with acceptance rows (section
9). **S1 does not start until the owner has signed them.**

### For the owner to sign, in one pass

| # | Question | Planner default (recommendation) |
| --- | --- | --- |
| P1 | Which steps can be published | Any step except the last (the last one is the output table already). At most **5** published steps per pipeline, each to one table |
| P2 | The published table's layer and name | Any zone the output may use (not Bronze). The editor proposes the output's zone and the name `<output table>_<step name>` (cut to 48 characters); the user may change both |
| P3 | Name rules | Letters, digits and underscores, not starting with a digit or `__`, at most 48 characters (the step-name rule). Refused at save when it is the pipeline's own output, one of its inputs, another published table of the same pipeline, or a table **any other pipeline** writes (as output or published) |
| P4 | Who may read it | The same as any new Silver or Gold table: a role that reads all tables sees it; a role narrowed to named tables needs a grant. No automatic grant. It is computed as the pipeline's author, so a column masked for the author is masked in it (decision 3) |
| P5 | How it is built | Each run computes the published step as its own statement (the pipeline up to that step), through the same access rewrite. The step's work is done twice per run (once for its table, once inside the output). Reading the published table back instead is a later item |
| P6 | All or nothing | Every table of a run is built first, then checked, then swapped in. If any table fails a check, **no** table of that run is published. The swaps are one per table; a crash between two swaps can leave one table new and another old (no multi-table transaction in `ClickHouse`) |
| P7 | Unpublishing, deleting | Turning publishing off, or removing the step, leaves the table as it was; it stops being refreshed. Deleting the pipeline leaves it too (as it leaves the output today). The editor says so |
| P8 | Lineage and order | A published table is an output of its pipeline everywhere the product asks "which pipeline writes this table": table lineage (edges from every input of the pipeline, the rule the output already follows), a downstream target lag, the cycle check, and the quality follow-up after a run. No column lineage for it (`DATA-57`) |
| P9 | What a run can do when columns change | Three choices, per pipeline, for every table it writes: **Stop the run** (any column added, removed or retyped), **Allow new columns** (added is fine; removed or retyped stops the run), **Take the new columns** (today's behaviour). Compared by name and type; order is ignored. A table's first run never stops |
| P10 | The default | Pipelines that exist keep **Take the new columns**, so they run as before (`BLD-1-AC9`). A new pipeline, from the console or the assistant, starts with **Allow new columns** |
| P11 | Which quality rules block | A rule marked **"Stop pipelines from publishing when this check fails"** (a new switch on a rule, off by default; changing it needs `governance:write`, like any rule edit). Severity does not decide. Only a **failed** verdict blocks; a warning does not |
| P12 | A blocking rule that cannot be checked | Stops the publish (fail closed): its column is missing from the new rows, or the query fails. A rule whose check the evaluator cannot read cannot be marked blocking (refused at save), nor can a rule on a Bronze table (no pipeline writes Bronze) |
| P13 | What the user sees | The run fails and each table keeps its last version. The run's failure and log carry one fixed sentence naming the table and why: the removed, added or retyped columns (up to 10 names, then "and N more"), or the rule's name, its measured value and its threshold. The verdict goes into the rule's run history as run by the pipeline, and opens an incident like any failed rule. The usual failed-run alert fires. The Output pane lists the blocking rules a run will check |
| P14 | A run that is not published | Counts as a failed run (failure streak, alerts, the run list), not retried by the orchestrator: the same rows give the same answer |
| P15 | The assistant | `create_pipeline` gains `published` and `onColumnChange`; `draft_quality_rule` gains `blocksPublish`. Same review by the AI team as parts 1 and 2 |

### The shapes these give

- **D1. Stored shape.** `pipeline_definition` gains three columns
  (migration `0154`, S1-T1): `published JSONB` (the list as sent),
  `published_tables TEXT[]` (their `zone.table` locations, for the readers
  that match tables, the way `input_tables` serves inputs, `0140`), and
  `on_column_change TEXT` (`fail`, `add`, `replace`). `quality_rule` gains
  `blocks_publish BOOLEAN`.
- **D2. Wire shape.** On the definition (`GET /api/pipelines/{id}`,
  `POST /api/pipelines`, `PUT /api/pipelines/{id}`, the version snapshot,
  `GET /api/pipelines/runnable`):

  ```json
  { "published": [ { "step": "paid", "zone": "silver", "table": "orders_paid" } ],
    "onColumnChange": "add" }
  ```

  `step` names a step exactly (D6a of part 1). On the pipeline row:
  `publishedTables: ["silver.orders_paid"]`. On a quality rule:
  `blocksPublish: true`.
- **D3. Create and update.** Create: `published` absent means none;
  `onColumnChange` absent means `add` (P10). Update: either field absent
  keeps what is stored (the part 2 console sends neither). Restore of a
  version restores both; a snapshot written before `0154` restores none and
  `replace`.
- **D4. The compile answer** (`POST /api/pipelines/runnable/{id}/compile`)
  keeps `sql` and `readsIceberg` and adds:

  ```json
  { "published": [ { "step": "paid", "zone": "silver", "table": "orders_paid",
                     "sql": "<rewritten as the author>" } ],
    "onColumnChange": "add" }
  ```

  Each published `sql` is `compile(inputs, steps, Some(step), ..)` then the
  same gate and rewrite as `sql`. A refusal of any of them refuses the whole
  compile (422, the step named), as the output's does today.
- **D5. The publish check** — a new route
  `POST /api/pipelines/runnable/{id}/publish-check`, body `{ "runId": "<dagster run id>" }`.
  Same callers as compile (a service identity or a Platform Admin; same
  `pipeline:read` floor in `POLICY_TABLE`). The API derives every table the
  pipeline writes (output and published) and each one's staging table
  (`<zone>.__staging_<table>`, the orchestrator's name,
  `authored_factory.py:516`), and for each: reads both tables' columns,
  applies P9, then evaluates the blocking rules (P11) against the staging
  table. Answers:
  - `200 {"publish": true}`;
  - `200 {"publish": false, "reason": "<P13 sentence>"}` at the first table
    that fails, in the order output first, then published in list order;
  - `404` unknown pipeline; `409` no author (same as compile);
  - `503` when `ClickHouse` or Postgres cannot be read (the orchestrator
    retries; after its retries the run fails and nothing is swapped).
  A staging table that does not exist answers `publish: false` with
  `STAGING_MISSING` (the orchestrator did not build it; fail closed).
- **D6. The sentences** (fixed text in the new module, S1-T5; the column and
  rule names are the customer's own metadata, never engine text):
  - `"{zone}.{table} was not published: columns changed ({detail}). It keeps its last version. Change the steps, or allow the change under Output."`
    where `{detail}` is any of `removed: a, b`, `added: c`, `type changed: d`,
    joined by `; `, each list cut at 10 names with `and N more`.
  - `"{zone}.{table} was not published: quality rule \"{rule}\" failed ({value}; expected {threshold}). It keeps its last version."`
  - `"{zone}.{table} was not published: quality rule \"{rule}\" could not be checked against the new rows. It keeps its last version."`
  - `STAGING_MISSING = "the new rows were not found; nothing was published"`.
  - At save: `published[i].step`: `"this names no step of this pipeline"`,
    `"the last step is the output table already"`;
    `published[i].table`: the part 1 `INVALID_NAME` sentence,
    `"this table is already the output or an input of this pipeline"`,
    `"this table is already published by another step"`,
    `"another pipeline already writes this table"`;
    `published`: `"a pipeline may publish at most 5 steps"`;
    `published[i].zone`: part 1's Bronze sentence
    (`BRONZE_TARGET_REFUSED`, `authored_pipelines.rs:464`);
    `onColumnChange`: `"onColumnChange must be fail, add or replace"`.
  - On a rule: `"only a check the evaluator can read can stop a pipeline from publishing"`,
    `"a Bronze table's rule cannot stop a pipeline from publishing"`.

## 3. Security rules (conditions, not options)

1. Every published statement is compiled by the API and goes through the
   tenant gate and `check_and_rewrite` as the pipeline's author, at save and
   at every compile — exactly the output's path
   (`authored_pipelines.rs:2080-2105`). The orchestrator still builds no SQL
   but table names, which it checks with `_is_safe_ch_identifier`
   (`authored_factory.py:292-301`).
2. The publish check runs the quality SQL as the platform user, as every
   quality evaluation does today (`quality.rs:33-36`), returning aggregates
   only. It is reachable only by the orchestrator's identity or a Platform
   Admin (handler-level narrowing, copied from `compile_pipeline`,
   `authored_pipelines.rs:2048-2056`).
3. No upstream error text in any answer: a failed `ClickHouse` read is 503
   through `upstream_error`; the P13 sentences are built from fixed text and
   stored names only. `tests/sec11_guard.rs` must stay green.
4. Table names in the `ClickHouse` text come only from values that passed
   `Ident` (`lakehouse_core::ident::Ident`) or `valid_name`
   (`pipeline_compile.rs:163`), formatted between backticks; the staging
   prefix is a constant.

## 4. Anchors (`main` at `fd325da69`)

Rust, `rust/crates/`:

- `lakehouse-store/src/pipelines.rs`: `Pipeline` `:48` (`input_tables` `:72`);
  `PipelineStep` `:270`; `definition_columns` `:493`; `AuthoredDefinition`
  `:591`; `PipelineRow` `:626`; `PIPELINE_COLUMNS` `:735`, `:747`;
  `PipelineDefinitionSnapshot` `:763`; `get_definition` `:1222`;
  `CreatePipelineInput` `:1337`; `create_pipeline` `:1437` (insert `:1485`);
  `UpdatePipelineInput` `:1553`; `update_pipeline` `:1691` (`:1765`);
  `restore_pipeline` `:1712`; `RunnablePipeline` `:1940`;
  `targets_for_conn` `:2466`; `lag_reader_predicate` `:2748`;
  `lag_pipelines_without_readers` `:2828`;
  `upstream_downstream_pipelines_conn` `:2865`;
  `find_target_lag_cycle_conn` `:3016` (`target_of` `:3031`-`:3037`).
- `lakehouse-store/src/governance.rs`: `QualityRule` `:328`;
  `CreateQualityRuleInput` `:384`; `create_quality_rule` `:406`;
  `UpdateQualityRuleInput` `:428`; `update_quality_rule` `:445`;
  `list_quality_rules` `:154`.
- `lakehouse-api/src/pipeline_compile.rs`: `valid_name` `:163`; `compile`
  `:184`; `validate_and_compile` `:264`; `OUTPUT_IS_INPUT` `:148`.
- `lakehouse-api/src/routes/authored_pipelines.rs`:
  `validate_target_zone` `:464`; `save_check_compiled` `:1443`;
  `compile_and_save_check` `:1528`; `runnable` `:1669`;
  `compile_pipeline` `:2043`-`:2127`; `UpdateBody` `:2130`; `update` `:2220`;
  `delete` `:2380`; `restore_version` `:2482`.
- `lakehouse-api/src/routes/pipelines.rs`: `CreatePipelineBody` `:3020`;
  `create` `:3106`; `run_finished_event` `:2785` (**not touched**, see
  section 7).
- `lakehouse-api/src/routes/quality.rs`: `check_sql` `:486`; `Verdict` `:553`;
  `ensure_table`/`record_run` `:713`/`:743`; `resolve` `:952`; `evaluate`
  `:1245`; `UpdateRuleBody` `:1395`; `update_rule` `:1423`; `run_on_load`
  `:1797`. `routes/governance.rs`: `create_quality_rule` `:2056`.
- `lakehouse-api/src/routes/quality_load.rs`: `pipeline_table` `:41`;
  `pipeline_loaded` `:99`-`:115`.
- `lakehouse-api/src/routes/catalog_source.rs`: `ReadSource` `:44`;
  `clickhouse_source` `:102`.
- `lakehouse-api/src/routes/lineage.rs`: the pipeline edges `:449`-`:479`.
  `routes/lineage/columns.rs`: `zone_key` `:53`.
- `lakehouse-api/src/routes/mod.rs`: `pipeline_compile_router` `:425`-`:430`.
- `lakehouse-api/src/policy.rs`: the compile row `:638`.
- `lakehouse-api/src/audit_coverage.rs`: the compile row `:281`.
- `lakehouse-api/tests/route_auth.rs`: compile asserted both ways `:4706`.
- `lakehouse-api/src/routes/ai/registry.rs`: `draft_quality_rule_schema`
  `:1637`; `create_pipeline_schema` `:1717`; steps parameter `:1749`.
  `routes/ai/tools/pipelines.rs`: `create_pipeline` `:467` (forwards the
  args to `create` as the body, so no handler change).
  `routes/ai/tools/governance.rs`: `draft_quality_rule` `:299`-`:317`.
- Tests: `lakehouse-api/tests/pipeline_compile_route.rs`,
  `pipeline_definition_save.rs`, `pipeline_lag_check.rs`,
  `quality_on_load.rs`, `column_lineage_pipeline.rs`;
  `lakehouse-store/tests/pipelines.rs`, `quality.rs`, `governance.rs`;
  `lakehouse-api/tests/fixtures/tool_schemas.json`.

Python, `dagster/dispar_orchestrate/`:

- `authored_factory.py`: `_is_safe_ch_identifier` `:292`; `_reading` `:325`;
  `_compiled_sql` `:395`-`:443`; `_ensure_target_table` `:446`-`:471`;
  `_write_clickhouse_table` `:474`-`:546`; `_op_for_pipeline` `:549`
  (target checks `:611`-`:640`, compile `:653`-`:661`, write `:662`,
  materialization `:671`-`:678`); `_pipeline_target` `:1401`;
  `build_authored_lag_sensors` `:1449` (target match `:1498`-`:1499`).
- `test_authored_factory.py`: the atomic write tests `:502`, `:523`; the
  compile tests `:350`-`:444`.
- `ops/g7/g7_pipeline_test.py`: `_create_and_run_authored_pipeline`
  `:1279`; the BLD-16 case `:1329`; `main` `:1615`.

Console, `src/`:

- `services/contracts/pipelines.ts`: `Pipeline` `:20`; `PipelineInput`
  `:162`; `PipelineStep` `:254`; `AuthoredDefinition` `:274`.
  `services/contracts/governance.ts`: `QualityRule` `:52`;
  `CreateQualityRuleInput` `:400`; `UpdateQualityRuleInput` `:408`.
- `lib/pipeline-editor.ts`: `EditorState` `:194`; `editorStateFrom` `:215`;
  `removeItem` `:431`; `renameItem` `:446`; `OutputSettings` `:567`;
  `editorProblems` `:611`; `definitionBody` `:789`; `definitionChanged`
  `:861`.
- `features/pipelines/pipeline-editor-panes.tsx`: `StepHeader` `:309`;
  `OutputPane` `:1496`. `features/pipelines/pipeline-editor.tsx`: output
  state `:136`, the save body `:389`.
  `features/pipelines/pipeline-definition.tsx`: the Output table panel `:432`.
  `features/pipelines/pipeline-flow-layout.ts`: the target node `:258`-`:269`.
- `features/governance/quality-rule-create-page.tsx` (severity `:45`, `:160`),
  `quality-rule-edit-dialog.tsx`, `data-quality-columns.tsx`.

## 5. Migration

**`rust/migrations/0154_pipeline_publish_and_column_change.sql`.** `0151` is
the highest on `main`; `0152` is reserved by `data-4-silver-publish` and
`0153` by `bld-14-downstream-reupload`; no remote branch has any `015[2-9]`
file and no plan names `0154` (checked over every `origin/*` on
2026-11-20). Recheck with `ls rust/migrations` and `git ls-tree` over the
open branches before the PR opens; if `0154` is taken, use the next free
number and say so in the handoff.

Why-header (in the file): BLD-17 P1–P11; existing rows get `replace` so
they run as before (P10, `BLD-1-AC9`); `published_tables` is derived from
`published` in one place in the store (as `input_tables` is, `0140`);
`blocks_publish` defaults to false so no existing rule starts blocking.

```sql
ALTER TABLE pipeline_definition
    ADD COLUMN published JSONB NOT NULL DEFAULT '[]'::jsonb,
    ADD COLUMN published_tables TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN on_column_change TEXT NOT NULL DEFAULT 'replace'
        CONSTRAINT pipeline_definition_on_column_change_check
        CHECK (on_column_change IN ('fail', 'add', 'replace'));
ALTER TABLE quality_rule
    ADD COLUMN blocks_publish BOOLEAN NOT NULL DEFAULT false;
```

No index: the tables are small and every reader is tenant- or id-scoped.

## 6. Tasks

One task per commit, in order. **Push after every commit.** Per commit, run
only the scoped checks named in the task; all cargo through the shared
`CARGO_TARGET_DIR` (and `bin/cargo-locked` where the machine has it); never
`cargo clean`. The reviewer opens a draft PR after the first code commit is
pushed, so CI runs the whole suite and the gates.

If the code at any anchor is not what this plan says, **stop and report the
mismatch**; do not improvise a different design.

### S1 (`feat/bld-17-publish-api`, Hy4 flash)

T1 to T8 are Rust and sit together (`lakehouse-store` then `lakehouse-api`,
the migration in the same sitting as its store code). T9 is Python, T10 the
gate.

- **S1-T1. Migration and store shape (D1, D2, D3).**
  - Add section 5's migration.
  - `lakehouse-store/src/pipelines.rs`: new
    `pub struct PublishedStep { step: String, zone: String, table: String }`
    (`Serialize, Deserialize, Clone, Debug, PartialEq, Eq`, camelCase, every
    field `#[serde(default)]` — the compiler checks them and names the path,
    as J2 of part 2) with `fn location(&self) -> String` like
    `PipelineInput::location` (`:249`). New
    `pub enum OnColumnChange { Fail, Add, Replace }` with
    `as_str`/`parse` (`"fail"`, `"add"`, `"replace"`; anything else `None`).
  - `AuthoredDefinition` gains `published: Vec<PublishedStep>` and
    `on_column_change: String`; `PipelineDefinitionSnapshot` gains both with
    `#[serde(default)]` (an old snapshot has none, and its
    `on_column_change` default is `"replace"`: write a
    `fn replace() -> String` for the serde default, not `""`).
  - `Pipeline` and `PipelineRow` gain `published_tables: Vec<String>`
    (wire `publishedTables`); add the column to both `PIPELINE_COLUMNS`
    constants (`:735`, `:747`) and every `query_as` that lists them.
  - `DefinitionColumns` (`:473`) gains `published_tables`;
    `definition_columns` takes the published list as a third argument and
    derives the locations. Callers: create, update, restore.
  - `CreatePipelineInput` gains `published: Vec<PublishedStep>`,
    `on_column_change: String`; `UpdatePipelineInput` gains
    `published: Option<Vec<PublishedStep>>`,
    `on_column_change: Option<String>` (`None` keeps the stored value: use
    `COALESCE($n, published)` in the `UPDATE`, binding `Option<Json<..>>`).
    `restore_pipeline` writes the snapshot's values.
  - `get_definition` reads `published` and `on_column_change`;
    `list_runnable_pipelines` therefore carries them with no change of its
    own.
  - New `pub async fn writers_of(pool, location: &str, except_id: Option<&str>) -> Result<bool, StoreError>`:
    whether any pipeline other than `except_id` has `target = $1` or
    `$1 = ANY(published_tables)`, across all tenants (the `ClickHouse`
    namespace is shared). `# Errors` doc.
  - `governance.rs`: `QualityRule`, its row, `CreateQualityRuleInput` and
    `UpdateQualityRuleInput` (as `Option<bool>`, `None` keeps) gain
    `blocks_publish`; every select of `quality_rule` reads it.

  *Check:* `lakehouse-store/tests/pipelines.rs`:
  `a_published_step_and_the_column_choice_are_stored_and_read_back`,
  `published_tables_are_derived_from_the_published_list`,
  `an_update_without_published_keeps_the_stored_list`,
  `a_snapshot_written_before_0154_restores_no_published_step_and_replace`,
  `a_pipeline_made_before_0154_reads_replace_and_no_published_step`
  (insert a row with SQL omitting the new columns),
  `writers_of_finds_an_output_and_a_published_table_and_skips_itself`.
  `lakehouse-store/tests/governance.rs` (or `quality.rs`, where the rule
  tests live): `a_rule_is_stored_with_blocks_publish_and_an_update_without_it_keeps_it`.
  `cargo fmt --check && cargo clippy -p lakehouse-store --all-targets -- -D warnings && cargo test -p lakehouse-store --test it pipelines:: governance:: quality::`.
  The API crate must still compile: fill the new struct fields at every
  `lakehouse-api` construction site with today's values (empty list,
  `"replace"`/`None`, `false`/`None`) — `cargo check -p lakehouse-api`. T4
  and T7 replace them.

- **S1-T2. The compiler checks the published list (P1, P3, D6).** In
  `pipeline_compile.rs`:
  - `pub const MAX_PUBLISHED: usize = 5;` and the D6 save sentences as
    constants.
  - New `pub fn check_published(inputs, steps, target: (&str, &str), published: &[PublishedStep]) -> Result<(), DefinitionError>`,
    pure, applied by the save path after `validate_and_compile`:
    the count; each `step` names a step exactly and is not the last;
    each `table` passes `valid_name`; each `zone` is not `bronze` and passes
    `Ident`; no entry equals the target or an input location; no two
    entries share a location; no two entries name the same step. Paths
    `published`, `published[i].step`, `published[i].zone`,
    `published[i].table`.
  - New `pub fn compile_published(inputs, steps, published, catalog_db) -> Result<Vec<String>, DefinitionError>`:
    `compile(inputs, steps, Some(&entry.step), catalog_db)` for each, in
    order.

  *Check:* unit tests in `pipeline_compile.rs`'s `mod tests`, one per rule
  (`a_published_entry_naming_the_last_step_is_refused_at_published_0_step`,
  `six_published_steps_are_refused`, `a_published_table_equal_to_an_input_is_refused`,
  `two_entries_with_one_table_are_refused_at_the_second`,
  `a_bronze_published_zone_is_refused`,
  `a_published_table_named_with_two_leading_underscores_is_refused`,
  `compile_published_stops_at_the_named_step` — the text ends with
  ``SELECT * FROM `paid` `` and holds no CTE after it).
  `cargo clippy -p lakehouse-api --all-targets -- -D warnings && cargo test -p lakehouse-api --lib pipeline_compile`.

- **S1-T3. Create and update take the new fields (D3, P3, P10).**
  - `CreatePipelineBody` (`routes/pipelines.rs:3020`) gains
    `#[serde(default)] published: Vec<PublishedStep>` and
    `#[serde(default)] on_column_change: Option<String>`; absent →
    `"add"`. `UpdateBody` (`authored_pipelines.rs:2130`) gains both as
    `Option`.
  - Parse `onColumnChange` with `OnColumnChange::parse`; anything else is a
    400 with D6's sentence.
  - After the definition's own checks, run `check_published` (400 naming the
    path), then `writers_of` for each entry with `except_id` = this
    pipeline (409 with `"published[i].table: another pipeline already writes this table"`),
    then put each published statement through the same save check as the
    output (`save_check_compiled`, `:1443`), so a published step reading a
    table the author may not read is refused at save naming
    `published[i]` — give `save_check_compiled` the path to name rather than
    writing a second check (rule 4).
  - Update with `published` absent keeps the stored list, and the stored
    list is still checked against the new steps (a step renamed or removed
    by a client that does not know `published` must be refused, not leave a
    dangling entry): the route reads the stored list and runs
    `check_published` on it with the new steps. **This is the only way an
    older client can be refused for a field it does not send; the sentence
    names `published[i].step`.**
  - Restore: the snapshot's values, checked the same way.

  *Check:* `lakehouse-api/tests/pipeline_definition_save.rs`:
  `a_pipeline_is_created_with_a_published_step_and_reads_it_back`,
  `a_create_without_on_column_change_stores_add`,
  `an_update_without_the_new_fields_keeps_them`,
  `a_published_table_another_pipeline_writes_is_refused_with_409`,
  `a_published_table_equal_to_the_output_is_refused`,
  `renaming_a_published_step_without_sending_published_is_refused_at_published_0_step`,
  `a_published_step_reading_a_table_the_author_may_not_read_is_refused_at_save`,
  `an_unknown_on_column_change_is_a_400`.
  `cargo test -p lakehouse-api --test it pipeline_definition_save::`.

- **S1-T4. The compile answer carries the published statements (D4).** In
  `compile_pipeline` (`authored_pipelines.rs:2043`): after the output's
  rewrite, `compile_published` then `check_and_rewrite` each as the author,
  with the same refusal mapping (`:2097-2105`). Add `published` (`step`,
  `zone`, `table`, `sql`) and `onColumnChange` to the `json!`. Do not note
  published statements for column lineage (P8). Leave the
  `is_legacy_shape` block (`:2116`) as it is — `data-57-with-lineage`
  removes it (its D9); see section 7.

  *Check:* `lakehouse-api/tests/pipeline_compile_route.rs`:
  `the_compile_answer_lists_each_published_step_rewritten_as_the_author`
  (a masked column of the author is masked in the published `sql`, as the
  existing masking test does for `sql`),
  `a_pipeline_with_no_published_step_answers_an_empty_list_and_its_choice`,
  `a_published_step_that_no_longer_compiles_refuses_the_whole_compile_with_422`.
  `cargo test -p lakehouse-api --test it pipeline_compile_route::`.

- **S1-T5. The column comparison (P9, D6), pure.** New module
  `lakehouse-api/src/pipeline_publish.rs` (`pub mod pipeline_publish;` in
  `lib.rs` next to `pipeline_compile`):
  - `pub const STAGING_PREFIX: &str = "__staging_";` with a doc saying it
    mirrors `authored_factory.py`'s `staging_table` and that a test on each
    side pins the text.
  - `pub struct ColumnChange { removed: Vec<String>, added: Vec<String>, retyped: Vec<String> }`;
    `pub fn column_change(live: &[(String, String)], new: &[(String, String)], mode: OnColumnChange) -> Option<ColumnChange>`:
    `None` when the run may publish. `live` empty (first run) → `None`.
    `Replace` → `None`. `Add` → `Some` only when something is removed or
    retyped (`added` still listed for the sentence). `Fail` → `Some` on any
    difference. Compare as sets by name; a type is compared as the exact
    `system.columns.type` string.
  - `pub fn not_published_columns(location, &ColumnChange) -> String`,
    `not_published_rule(location, rule, value, threshold) -> String`,
    `not_published_unchecked(location, rule) -> String` — D6's text, the
    lists cut at 10 with `and N more`.

  *Check:* unit tests, each a full sentence:
  `a_first_run_publishes_whatever_its_columns`,
  `take_the_new_columns_publishes_a_removed_column`,
  `allow_new_columns_publishes_an_added_column`,
  `allow_new_columns_stops_a_removed_column`,
  `allow_new_columns_stops_a_retyped_column`,
  `stop_the_run_stops_an_added_column`,
  `column_order_alone_is_not_a_change`,
  `the_sentence_lists_ten_names_and_counts_the_rest`,
  `the_staging_prefix_is_the_orchestrators`.
  `cargo test -p lakehouse-api --lib pipeline_publish`.

- **S1-T6. The quality gate (P11, P12, P13).** In `routes/quality.rs`:
  - Split `evaluate` (`:1245`) into a resolver and
    `async fn evaluate_on(state, rule, source: &ReadSource, run_by) -> Result<Verdict, RunError>`
    (the column check, `check_sql`, `record_run`, the incident calls), so
    `evaluate` is `resolve` + `evaluate_on` and nothing else changes.
  - New `pub(crate) async fn check_before_publish(state, table_key: &str, staging: &ReadSource, run_by: &str) -> Result<Option<String>, PublishCheckError>`:
    the rules with `blocks_publish` whose `asset` (trimmed, lower-cased)
    equals `table_key`; each through `evaluate_on` with `per_load` false;
    the first `failed` verdict returns `Some(not_published_rule(..))`; a
    `RunError::NotRunnable` returns `Some(not_published_unchecked(..))`;
    a query failure (`RunError::Failed`) is `PublishCheckError` (→ 503). A
    rule's run is recorded with `run_by` =
    `"pipeline {name}, before publishing"`.
  - `create_quality_rule` (`governance.rs:2056`) and `update_rule`
    (`quality.rs:1423`) accept `blocksPublish`; refuse it (400, D6) when
    the threshold does not parse (`parse_threshold`) or the asset's zone is
    `bronze`. Changing it needs nothing beyond the routes' existing
    `governance:write`.

  *Check:* `lakehouse-api/tests/quality_on_load.rs` (it already mocks
  `ClickHouse` for rule runs) or a new `quality_publish_gate.rs` registered
  in `tests/it.rs`:
  `a_failed_blocking_rule_returns_its_sentence_and_is_recorded_as_run_by_the_pipeline`,
  `a_warning_does_not_block`,
  `a_rule_without_blocks_publish_is_not_run_before_publishing`,
  `a_blocking_rule_whose_column_is_missing_from_the_new_rows_blocks`,
  `a_rule_on_another_table_is_not_run`,
  `marking_a_rule_with_an_unreadable_check_as_blocking_is_refused`,
  `marking_a_bronze_rule_as_blocking_is_refused`.
  `cargo test -p lakehouse-api --test it quality_` (or the new module).

- **S1-T7. The publish check route (D5).** New file
  `routes/pipeline_publish.rs` (`mod pipeline_publish;` in `routes/mod.rs`
  next to `pipeline_preview` `:88`), handler
  `pub async fn publish_check(State, Extension<Principal>, Path<String>, body: Bytes) -> ApiResult<ApiJson<Value>>`:
  - the caller narrowing and the author resolution of `compile_pipeline`
    (`:2048-2075`) — move them into one `pub(crate)` helper both handlers
    call rather than copying (rule 4);
  - body `{ runId }` (non-empty, else 400);
  - the tables: `(target_zone, target_table)` then each published entry;
    for each, `clickhouse_source(&state.clickhouse, zone, STAGING_PREFIX + table)`
    (`None` → `publish:false`, `STAGING_MISSING`) and
    `clickhouse_source(.., zone, table)` (`None` → first run);
    `column_change` with the stored choice → the sentence; then
    `check_before_publish(state, zone_key(location), &staging, ..)`;
  - a `ChError` → `upstream_error` (503); never `?` on it into the
    response (`sec11_guard`).
  - Route: `pipeline_compile_router` (`routes/mod.rs:425`) gains
    `.route("/api/pipelines/runnable/{id}/publish-check", post(pipeline_publish::publish_check))`.
    `POLICY_TABLE` row after the compile row (`policy.rs:638`), same
    `pipeline:read` floor, with the same comment's reason.
    `audit_coverage.rs`: `Coverage::Excluded` with a new reason constant
    (`EX_PUBLISH_CHECK`: "the orchestrator's check before a swap; it changes
    no definition, and each verdict is kept in the rule's run history").
  - `tests/route_auth.rs`: `the_publish_check_route_authorization_is_asserted_both_ways`,
    modelled on `:4706`.

  *Check:* new `lakehouse-api/tests/pipeline_publish_check.rs` (a `mod`
  line in `tests/it.rs`, `use crate::common;`), `ClickHouse` mocked with
  `wiremock` as the existing quality tests do:
  `a_matching_staging_table_with_no_blocking_rule_is_published`,
  `stop_the_run_refuses_an_added_column_with_its_sentence`,
  `a_published_tables_column_change_is_reported_with_its_own_name`,
  `a_failed_blocking_rule_on_the_output_refuses_the_publish`,
  `a_missing_staging_table_refuses_the_publish`,
  `a_clickhouse_failure_is_a_503_without_the_engines_text`,
  `a_user_with_pipeline_read_who_is_not_an_admin_is_refused_with_403`.
  `cargo test -p lakehouse-api --test it pipeline_publish_check:: route_auth::`.

- **S1-T8. A published table is an output (P8) and the assistant (P15).**
  - Store: `lag_reader_predicate` (`:2748`): the first disjunct becomes
    `({candidate_target} = ANY (p.input_tables) OR EXISTS (SELECT 1 FROM pipeline_definition o WHERE o.id = {candidate_id} AND o.published_tables && p.input_tables))`
    — its two callers need no change. `upstream_downstream_pipelines_conn`
    (`:2865`): `(target = ANY($2) OR published_tables && $2 OR id = ANY($3))`.
    `find_target_lag_cycle_conn` (`:3031`): read `published_tables` with
    `target`, and match an upstream when any of its outputs is an input.
  - `routes/lineage.rs:449-479`: after the target's node and edges, the same
    for each `published_tables` entry (`zone_node`, one edge per input,
    label `authored pipeline {name} ({id}), step published`).
  - `routes/quality_load.rs:99`: `pipeline_loaded` returns the target's key
    and each published table's key (`pipeline_table`).
  - Assistant: `create_pipeline_schema` (`registry.rs:1717`) gains
    `published` (array, `maxItems` 5, items `step`, `zone` enum
    `["silver","gold"]`, `table`, all required) and `onColumnChange` (enum
    `fail`, `add`, `replace`), and one sentence in the description: "To let
    other pipelines read an intermediate step, list it under published with
    its own table; onColumnChange says what a run does when the columns
    change (default add: new columns are fine, a removed or retyped column
    stops the run)." `draft_quality_rule_schema` (`:1637`) gains
    `blocksPublish` (boolean, "stop pipelines from publishing this table
    when the check fails") and `tools/governance.rs` forwards it.
    Regenerate `tests/fixtures/tool_schemas.json` the way the snapshot
    test's doc says (do not hand-edit it).

  *Check:* `lakehouse-store/tests/pipelines.rs`:
  `a_downstream_lag_pipeline_reading_a_published_table_is_found_as_downstream`,
  `a_pipeline_whose_published_table_is_read_has_a_reader`,
  `a_cycle_through_a_published_table_is_found`.
  `lakehouse-api/tests/pipeline_lag_check.rs` or `column_lineage_route.rs`'s
  sibling for the table lineage graph:
  `the_lineage_graph_draws_a_published_table_from_every_input`.
  `quality_on_load.rs`: `an_on_load_rule_on_a_published_table_runs_after_the_run`.
  `routes/ai/tools/pipelines.rs` tests (`:1179` style):
  `create_pipeline_accepts_a_published_step_and_the_column_choice`.
  `tool_schemas_snapshot_is_byte_identical` with the new fixture.
  `cargo test -p lakehouse-store --test it pipelines:: && cargo test -p lakehouse-api --test it pipeline_lag quality_on_load column_lineage && cargo test -p lakehouse-api --lib ai::`.

- **S1-T9. The orchestrator builds, checks, then swaps (P6, P13, P14, F4).**
  In `authored_factory.py`:
  - `_compiled_sql` returns a frozen dataclass
    `CompiledRun(sql, reads_iceberg, published: tuple[PublishedSql, ...], on_column_change)`.
    `published` absent → empty (an API older than this change); a
    `published` that is not a list of `{step, zone, table, sql}` strings,
    or a zone/table that fails `_is_safe_ch_identifier` or is `bronze` →
    `AuthoredEnforceRefused` (not retried).
  - `STAGING_PREFIX = "__staging_"` as a module constant, used where
    `_write_clickhouse_table` builds `staging_table` (`:516`); a test pins
    the text (`the_staging_prefix_is_the_apis`).
  - Split `_write_clickhouse_table` into `_build_staging(target, zone, table, sql, reads_iceberg)`
    (the `DROP IF EXISTS`, `CREATE ... LIMIT 0`, `INSERT` of `:516-534`) and
    `_swap_in(target, zone, table, sql, reads_iceberg) -> int` (then
    `_ensure_target_table`, the count, `EXCHANGE`, `DROP`, the count —
    `_ensure_target_table` moves here, after the check, F4). The docstring
    keeps the atomicity argument and adds P6's limit.
  - New `_publish_check(cfg, pipeline_id, run_id) -> str | None` (the reason,
    or `None` to publish): `requests.post(..., timeout=60)`,
    `raise_for_status()` on anything but 200/404/409/422 (a 5xx or a
    transport error propagates and the op retries); 404/409/422 →
    `AuthoredEnforceRefused`; a body that is not `{"publish": bool, "reason"?: str}`
    → `AuthoredEnforceRefused(_ENFORCE_BAD_BODY)`.
  - The op (`:653-678`): build staging for the output, then each published
    table; `_publish_check(cfg, pipeline["id"], context.run_id)`; on a
    reason, drop every staging table it built and raise
    `Failure(description=f"authored pipeline {id!r} not published: {reason}", allow_retries=False)`
    (P14); else `_swap_in` each, output first, and log one
    `AssetMaterialization` per table (`{zone}.{table}`, its `rows`). The op
    returns the output's `rows` as today.

  *Check:* `test_authored_factory.py`, each name a sentence:
  `test_a_published_step_is_built_checked_and_swapped_after_the_output`,
  `test_a_refused_check_swaps_nothing_and_drops_every_staging_table`,
  `test_a_refused_check_fails_without_retry_and_says_why`,
  `test_a_check_the_api_cannot_answer_is_left_retryable`,
  `test_the_first_run_creates_the_live_table_only_after_the_check`,
  `test_a_published_entry_with_an_unsafe_name_is_refused_before_any_sql`,
  `test_an_api_without_published_writes_only_the_output`,
  `test_the_staging_prefix_is_the_apis`. Update the two atomic-write tests
  (`:502`, `:523`) for the split and the check call; say why in the commit
  (rule 2: the statement order changed on purpose, F4). No network: the
  HTTP calls are patched as the existing compile tests patch them.
  `(cd dagster && python -m pytest dispar_orchestrate/test_authored_factory.py -q)`,
  then `python3 ops/lint/check_intra_package_imports.py`.
  Same commit: `_pipeline_outputs(p) -> list[str]` (the target, then each
  `definition.published` location) beside `_pipeline_target` (`:1401`),
  and the lag sensor's match (`:1498-1499`) uses it:
  `test_a_downstream_lag_pipeline_follows_a_published_table`.

- **S1-T10. Gate case (needs the real stack).** In `ops/g7/g7_pipeline_test.py`,
  a new `step_a_published_step_feeds_another_pipeline_and_the_checks_hold_the_table()`
  after the BLD-16 case, called from `main` (`:1615`), using
  `_create_and_run_authored_pipeline` (`:1279`):
  1. Pipeline A: the Bronze orders input, step `paid` (a filter form step),
     step `by_country` (an aggregate); `published` = `paid` →
     `silver.g7_bld17_paid`; output `silver.g7_bld17_by_country`;
     `onColumnChange: "fail"`. Run it. Both tables' row counts equal the
     same `SELECT` run directly (`ch_query`).
  2. Pipeline B: input `silver.g7_bld17_paid`, no step, output
     `silver.g7_bld17_paid_copy`. Run it; same count.
  3. Change A's last step to drop one measure; run. The run fails; its
     failure message (`_run_failure_message`, `:486`) contains
     `columns changed (removed:`; `system.columns` of
     `silver.g7_bld17_by_country` is unchanged; `silver.g7_bld17_paid` is
     unchanged too (P6).
  4. Put the step back; add a blocking rule `rows >= 100000000` on
     `silver.g7_bld17_by_country`; run. Fails with `quality rule`; the
     rule's runs (`GET /api/governance/quality/{id}/runs`) hold a failed
     run by `pipeline …, before publishing`.
  5. Delete the rule and both pipelines; drop the three tables.

  *Check:* `python3 -m py_compile ops/g7/g7_pipeline_test.py` locally; the
  case runs in CI's `g7` job on the draft PR. Quote that job's result in the
  handoff, or write *not verified* with the reason.

### S2 (`feat/bld-17-publish-console`, after S1 merges)

No cargo. Per commit: `bun run typecheck` and the touched tests
(`bun run test <file>`); `bun run lint` on the last commit.

- **S2-T1. Contracts and clients.** `services/contracts/pipelines.ts`:
  `type PublishedStep = { step: string; zone: string; table: string }`;
  `type OnColumnChange = "fail" | "add" | "replace" | string`;
  `AuthoredDefinition` gains `published: PublishedStep[]`,
  `onColumnChange: OnColumnChange`; `Pipeline` gains
  `publishedTables?: string[]`; the create and update bodies gain both
  (optional). `services/contracts/governance.ts`: `QualityRule`,
  `CreateQualityRuleInput`, `UpdateQualityRuleInput` gain
  `blocksPublish` (`boolean`, optional on the inputs). No client change
  beyond passing them through; a client test per contract change
  (`clients/pipelines.test.ts`, `clients/governance.test.ts`).
  *Check:* typecheck; the two client tests.

- **S2-T2. Editor state (P1–P3, P7).** `lib/pipeline-editor.ts`:
  `EditorState` gains `published: { stepId: string; zone: string; table: string }[]`
  (keyed by step id so a rename follows, as `StepRead` does at `:68`) and
  `editorStateFrom` maps stored `step` names to ids. `renameItem` needs no
  change; `removeItem` drops the entry; `moveStep` that makes a published
  step last is a problem, not a silent drop. `editorProblems` adds P1/P3's
  rules on the `published` items (field `"publish-table"`, same sentences as
  the API where the API's are about the same thing). `definitionBody`
  emits `published` with names. `definitionChanged` sees both new fields.
  New `suggestPublishedName(outputTable, stepName)` (P2).
  *Check:* `lib/pipeline-editor.test.ts`: a rename keeps the entry; removing
  the step drops it; moving it last is a problem; the body has names; the
  suggested name is cut to 48.

- **S2-T3. The publish section on a step.** `pipeline-editor-panes.tsx`:
  a `PublishSection` rendered by every step pane except the last (under
  `StepHeader`, `:309`): a switch "Publish this step as a table", then the
  zone (`ZoneField`, the output's list) and the table name; when turned off
  or the step is removed, the note "The table stays as it is and stops being
  refreshed." (P7). `pipeline-editor.tsx` keeps `published` in state and in
  the save body (`:389`).
  *Check:* `pipeline-editor.test.tsx` (or the panes' test): turning it on
  proposes P2's name; the save body carries the entry; the last step shows
  no section.

- **S2-T4. The Output pane (P9, P10, P11, P13).** `OutputPane` (`:1496`):
  replace "Every run rebuilds this table." with the rebuild line plus
  "When the columns change": three radio options with one line each —
  **Stop the run** ("any added, removed or retyped column stops the run;
  the table keeps its last version"), **Allow new columns** ("new columns
  are added; a removed or retyped column stops the run"), **Take the new
  columns** ("the table takes whatever the last step returns; a dashboard
  that reads a removed column breaks"). A new pipeline starts on Allow new
  columns. Below: "Checked before publishing" — the rules with
  `blocksPublish` whose asset is the output or a published table
  (`governanceService` list through `useService`), each with its threshold,
  or "No rule stops this pipeline from publishing." and a link to
  `/governance/data-quality/rules/new`.
  *Check:* a test per option sent in the body; the list shows only blocking
  rules of these tables.

- **S2-T5. The switch on a rule.** `quality-rule-create-page.tsx` and
  `quality-rule-edit-dialog.tsx`: a switch "Stop pipelines from publishing
  this table when this check fails" (off by default), sent as
  `blocksPublish`; the API's refusal shown as the form's error.
  `data-quality-columns.tsx`: a "Blocks publishing" badge on such rules.
  *Check:* the create page test sends it; the edit dialog keeps it.

- **S2-T6. Read view and flow.** `pipeline-definition.tsx` (`:432`): the
  published tables and the column choice. `pipeline-flow-layout.ts`
  (`:258-269`): one extra target node per published table, with an edge
  from its step.
  *Check:* `pipeline-flow-layout.test.ts`: a published step draws a second
  output node; the read view test shows both tables.

## 7. Overlap with open branches

Checked on 2026-11-20: `gh pr list --state open` has no `BLD-17` work;
`git log origin/main --grep='BLD-17\|BLD-1 part 3'` finds only the backlog
entry (`d5feff4fb`). Open PRs #612 (`DATA-6`) and #621 (`SRC-48`) touch no
file below.

- **`bld-11-pipeline-page`** (code, 15 files). Shared:
  `lakehouse-store/src/pipelines.rs` (it adds after `set_status`, `:2072`;
  BLD-17 edits the struct, the column lists, create/update and three lag
  queries — different hunks), `routes/pipelines.rs` (it edits
  `run_finished_event`; **BLD-17 does not touch that file except
  `CreatePipelineBody`/`create`**, which is why the quality follow-up is
  extended in `quality_load.rs`, not in `run_finished_event`),
  `pipeline_compile.rs` (it edits `render_step`; BLD-17 adds new functions
  after `validate_and_compile`), and in the console
  `pipeline-detail-page.tsx`/`pipeline-columns.tsx` (S2 touches neither).
  `pipeline_lag.rs`: BLD-17 does not touch it (the predicate change in the
  store covers its readers). Whichever merges second rebases; the developer
  rebases S1 on `main` before handoff.
- **`bld-14-downstream-reupload`** (plan only). Its plan touches
  `lakehouse-store/src/pipelines.rs` (it says it will not, `:300` of its
  plan — check at rebase), `routes/authored_pipelines.rs`, `policy.rs`,
  `routes/mod.rs`, `audit_coverage.rs`, `tests/route_auth.rs`,
  `authored_factory.py`, `test_authored_factory.py`,
  `contracts/pipelines.ts`, `clients/pipelines.ts`. Its new rows in
  `policy.rs`/`audit_coverage.rs`/`route_auth.rs` sit near the lag-check
  row; BLD-17's sit after the compile row — adjacent, resolved hunk by hunk.
  **Semantic overlap:** its chain uses `_pipeline_target` and
  `_pipeline_sources` (its plan `:43`, `:68`); after S1-T9 it must use
  `_pipeline_outputs`, or a re-upload will not run a pipeline that reads a
  published table. The planner tells the BLD-14 planner; whichever lands
  second carries the change.
- **`data-57-with-lineage`** (code, `sql_lineage.rs` only). No shared file
  on its branch today. Its D9 removes the `is_legacy_shape` gate in
  `compile_pipeline` (`authored_pipelines.rs:2116`), the handler S1-T4
  edits: S1-T4 adds lines before the `json!` and leaves the gate alone, so
  the two hunks are apart. Column lineage of published tables stays out
  (P8) even after `DATA-57`; it is a follow-up.
- `data-4-silver-publish` reserves `0152` and touches no pipeline file.

## 8. Verification for the handoff (the full block)

Run once per slice, before its handoff, for the languages it touched:

```bash
cd rust && cargo fmt --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace
bun run typecheck && bun run lint && bun run test
python3 ops/lint/check_intra_package_imports.py && python3 ops/lint/check_bare_iceberg_count.py
(cd dagster && python -m pytest dispar_orchestrate -q)
git status
```

S1 runs the Rust and Python lines (no `bun`); S2 runs the `bun` line only.
Per the owner's rule of 2026-10-30, CI's cargo run on the draft PR stands in
for the full local `cargo test --workspace`: the developer runs the scoped
commands of each task locally and quotes CI's run for the rest, with counts.
No compose change, so no `docker compose up`. The `g7` case (S1-T10) runs in
CI; quote its job or write *not verified*.

Tests that fail on `main` (the reviewer checks each fails before its task):
the store round trips of S1-T1 (no such columns or fields); every
`check_published`/`compile_published`/`pipeline_publish` unit test (no such
functions); the save, compile and publish-check route tests (no fields, no
route); `the_publish_check_route_authorization_is_asserted_both_ways`; the
quality gate tests (no `blocks_publish`); the lag and lineage tests (a
published table is not an output on `main`); `tool_schemas_snapshot_is_byte_identical`
against the new fixture; the new factory tests (no check call, the live
table created before the insert); S2's tests.

## 9. Before S1 starts (planner, not the developer)

1. Owner signs P1 to P15 (section 2). The planner writes them on the
   feature page as decisions after BLD-16's, updates the spec's part 3 rows
   from "None" to the signed targets, and moves capability 15's limit text
   ("A dashboard that reads a column … breaks") to say it holds only under
   Take the new columns.
2. Acceptance rows `BLD-1-AC22` to `BLD-1-AC29` (after BLD-16's AC15–AC21):
   - AC22: publish a step of a two-input pipeline to Silver, run; another
     pipeline reads that table as an input and runs; its rows equal the step's
     preview run in Query Studio.
   - AC23: the published table appears in the catalog and in table lineage
     with an edge from each input of its pipeline.
   - AC24: as an author for whom column C is masked, publish a step that
     copies C; C is masked in the published table.
   - AC25: Stop the run; remove a column from the last step; run. The run
     fails saying which column; dashboards on the table still work.
   - AC26: Allow new columns; add a column, run (published); remove one, run
     (fails, says which).
   - AC27: mark a rule on the output as blocking with a threshold the data
     fails; run. The run fails naming the rule; the table keeps its last
     rows; the rule's history shows the failed run by the pipeline.
   - AC28: a pipeline made before this change still runs and still takes the
     new columns.
   - AC29: ask the assistant to build AC22's pipeline with the step published;
     the draft has the published entry.
3. Add the F1 to F3 rows to `BACKLOG.md` (F1 under `BLD-15`/`DATA`, F2 and
   F3 under `BLD`), and tell the `BLD-14` planner about `_pipeline_outputs`
   (section 7).
4. Hand S1 to the developer.

## 10. Out of scope

- Reading a published table back inside the same run instead of computing
  the step twice (P5); incremental refresh (`BLD-12`).
- Column lineage for a published table (`DATA-57` and a follow-up).
- A unique output table across pipelines and a name rule for the output
  table (F2, F3); the `gold` database (F1).
- Dropping a published table when it is turned off or its pipeline is
  deleted (P7).
- Blocking rules for ingestion loads (connectors, uploads): only pipeline
  runs consult `blocks_publish`.
- A pre-publish check of anything but columns and quality rules (row-count
  drops are `quality` rules already: `rows >= N`).
- Any change to `run_finished_event`, `pipeline_lag.rs`, the alerts, the
  BI builder, Query Studio, the five-operation grammar or the form steps.
- Python steps (`DEC-1`), dbt (`DEC-2`), the pipeline as text.

## Handoff

## Review
