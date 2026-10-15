# `BLD-1` slice 1: a pipeline reads as its author

Planner and reviewer: Claude Opus. Developer: a non-Opus agent (`AGENTS.md`).
Base: `origin/main` at `ead1a871`. Branch: `feat/bld-1-run-as-author`.
One PR slice. Spec: `docs/core/specs/bld-1.md`; feature page:
`docs/core/features/pipeline-joins-and-sql.md` (decision 3, product owner,
2026-10-14). This slice also closes `GOV-4` for pipelines.

`BLD-1` is built in three slices, in this order, each its own plan and PR:

1. **This plan.** Every pipeline, existing and new, reads as its author.
2. Joins and aggregations (multiple inputs, `join`, `group`, post-group
   filter), built on slice 1's enforcement path.
3. The SQL step, built on the same path.

## 1. The problem

A pipeline runs as the orchestrator's ClickHouse user and reads every table
unmasked. `authored_factory._op_for_pipeline` builds a `SELECT` and runs it
directly; nothing applies the masking and row-filter rewrite
(`sql_rewrite::enforce`) that Query Studio, dashboards, alerts and the Gold
export apply. So anyone with `pipeline:write` can copy a masked column into
a Silver table in clear (`GOV-4`). Decision 3 of the feature page: a pipeline
reads as its author, through the same rewrite as the author's own queries.

Facts read from `main` at `ead1a871`:

- `pipeline_definition.owner` is free text, not a user. The only user id is
  `pipeline_definition_version.changed_by` (`lakehouse-store/src/pipelines.rs`,
  `PipelineVersionMeta`). Pipelines created before version history have a
  `baseline` version with `changed_by = NULL` (migration `0071`).
- `sql_rewrite::enforce(sql, dialect, principal_roles, placeholders,
  obligations_source, views_catalog)` takes role names and placeholder
  values; `policy_engine` builds the obligations source with
  `with_iceberg_db(state.config.iceberg_query_db)`, so a governed Bronze
  table is recognised under the API's Iceberg query database name
  (`ICEBERG_QUERY_DB`, `icecat_api` in compose), not under another job's.
- The orchestrator already authenticates to the API with
  `PIPELINE_RUN_TOKEN`; `authored_pipelines::runnable` is the precedent for
  an orchestrator-only route (403 to anyone else).

## 2. Decisions (planner)

- **D1. Who the author is: a new column `pipeline_definition.run_as UUID`**
  (references the user table, `ON DELETE SET NULL`). Set to the creating
  principal's user id on create; unchanged by update and restore (editing
  someone's pipeline does not change who it reads as). Backfill: from the
  `created` version's `changed_by` where one exists; otherwise NULL.
  Migration: the next free number when the PR is opened, at least `0073`;
  check every open branch first (several hold `0070` to `0072`). Why-header,
  additive, never edited once applied.
- **D2. No author, no run (fail closed).** A pipeline whose `run_as` is NULL,
  or names a user that is deactivated or gone, does not run: the op fails,
  not retried, with the fixed reason `"this pipeline has no author to read
  as; an administrator must assign one"`. A Platform Admin can set `run_as`
  through `PUT /api/pipelines/{id}/run-as` (`{ "userId": "<uuid>" }`, the
  user must be in the pipeline's tenant), audited like other pipeline
  writes, in `POLICY_TABLE`, asserted both ways in `tests/route_auth.rs`.
  The console shows "Reads as: <name>" on the pipeline page and, when unset,
  the reason and (for a Platform Admin) a control to assign it.
- **D3. One rewriter, in the API.** A new orchestrator-only route
  `POST /api/pipelines/runnable/{id}/enforce`, body `{ "sql": "<SELECT>" }`,
  same authentication rule as `runnable`. It loads the pipeline, resolves
  `run_as` to that user's current role names and placeholder values (the
  same way a session principal gets them; find the loader by symbol, do not
  write a second one), runs `sql_rewrite::enforce` with the same dialect,
  obligations source and views catalog Query Studio uses, and returns
  `{ "sql": "<rewritten>" }`. A refusal returns 422 with
  `policy_engine::enforcement_error_message` (a fixed string), never the
  rewriter's or ClickHouse's text. D2's cases return 409 with D2's message.
- **D4. Dagster asks before it runs.** `_op_for_pipeline` builds the
  `SELECT` as today, sends it to D3, and runs only the returned SQL, in the
  `CREATE TABLE ... AS`, staging `INSERT` and every other statement that
  reads the source. A 409 or 422 fails the op without retry, with the fixed
  message; a transport failure is retryable, as today.
- **D5. Bronze is named the way the rewriter recognises it.** For a Bronze
  source the op names the table in the API's Iceberg query database
  (`ICEBERG_QUERY_DB`, passed to `dagster-code-location` in compose), so
  `enforce` sees the governed table it would see in Query Studio. This
  replaces the `icecat_authored` database `BLD-3` added for authored jobs;
  remove it from `authored_factory.py` and say why in the module doc
  (governance must recognise the table). Keep `allow_database_iceberg = 1`.
- **D6. Assistant.** `create_pipeline` creates as the calling user, so
  `run_as` is that user. No schema change.

## 3. Anchors (`main` at `ead1a871`)

- `rust/crates/lakehouse-api/src/sql_rewrite.rs`: `enforce`, `PlaceholderValues`.
- `rust/crates/lakehouse-api/src/policy_engine.rs`: the obligations source
  built `with_iceberg_db`, `enforcement_error_message`.
- `rust/crates/lakehouse-api/src/routes/gold.rs` and `gold_export.rs`: how
  the Gold export calls `enforce` for a principal (the closest precedent).
- `rust/crates/lakehouse-api/src/routes/authored_pipelines.rs`: `runnable`
  (orchestrator-only rule), `update`, `restore_version`, `in_scope`;
  `routes/pipelines.rs::create`.
- `rust/crates/lakehouse-store/src/pipelines.rs`: `Pipeline`,
  `PipelineVersionMeta`, the create path.
- `rust/crates/lakehouse-auth/src/`: how a session principal's role names
  are loaded for a user id.
- `dagster/dispar_orchestrate/authored_factory.py`: `_op_for_pipeline`,
  `_resolve_source`, `_headers`, `PIPELINE_RUN_TOKEN`.
- `docker-compose.yml`: `dagster-code-location` env.
- `ops/g7/g7_pipeline_test.py`, `ops/g8/` (the governance masking gate).

## 4. Tasks (one commit each; Rust tasks together, migration with its code)

- **T1. D1:** migration, store field, create sets it, backfill test (a
  `created` version → `run_as`; a `baseline`-only pipeline → NULL).
- **T2. D2 API half:** `PUT .../run-as`, policy row, route_auth both ways,
  tenant check, audit.
- **T3. D3:** the enforce route, with tests: masked column rewritten for an
  author with a masking obligation; passes through unchanged for an author
  with none; 409 for NULL / deactivated author; 422 with the fixed message
  for a refused construct; 403 for a non-orchestrator caller; another
  tenant's pipeline id → the same 404 as unknown.
- **T4. D4 + D5 in Dagster**, unit tests with the API mocked: the op runs
  the returned SQL, never its own; 409/422 fail without retry; Bronze named
  in `ICEBERG_QUERY_DB`.
- **T5. D2 console half:** "Reads as" on the pipeline page; the unset state
  with the reason; the Platform Admin control. Tests in
  `src/features/pipelines/` only.
- **T6. The gate proves it.** Extend `ops/g8` (governance masking) or `ops/g7`,
  whichever already seeds a masking policy: a pipeline authored by a user
  for whom column C is masked writes C masked into Silver; the same pipeline
  authored by an unmasked user writes it clear; a pipeline with no author
  fails with D2's message. Run the matching CI job's sequence locally from a
  clean project; quote the output.
- **T7. Docs:** `CHANGELOG.md` (a behaviour change customers must know:
  existing pipelines without a recorded author stop running until an admin
  assigns one), module docs. Do not edit `docs/core/`.

## 5. Out of scope

Joins, aggregations, the SQL step (slices 2 and 3). Masking of Gold exports
triggered by the run token with no principal (a separate question). The
assistant's tool schema.

## 6. Verify, do not assume

1. How does Query Studio name a Bronze table in the SQL it sends to
   `enforce`, and does `enforce` govern the same table when named
   `<ICEBERG_QUERY_DB>.\`bronze.t\``? Prove it with a test before T4.
2. Which existing function turns a user id into role names and placeholder
   values? Name it in the handoff.
3. Does `CREATE TABLE ... AS <rewritten SELECT> LIMIT 0` still infer the
   right column types when masked columns are rewritten expressions? Quote a
   `DESCRIBE` of the output.
4. How many pipelines in a fresh compose stack end up with `run_as = NULL`
   after the backfill (the g7 fixture, the seeds)? Every one of them stops
   running: list them and say whether a gate depends on one.

## 7. Handoff

(Developer appends here, task by task.)

## 8. Review

(Reviewer.)
