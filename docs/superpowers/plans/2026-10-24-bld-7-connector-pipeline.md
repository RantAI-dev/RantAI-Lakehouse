# BLD-7, BLD-8, BLD-9: a pipeline made from a connector runs

| | |
| --- | --- |
| Planner and reviewer | Claude Opus (`AGENTS.md`). Developer: a non-Opus agent |
| Date | 2026-10-24 |
| Backlog | `BLD-7` (P1), `BLD-8` (P1), `BLD-9` (P2), module Build. Decision `DEC-15` |
| Source | QA report of 2026-10-23: a pipeline created from the connector page's *Create pipeline* is saved as ready and every run fails in about 5 s |
| Base | `origin/main` at `563bb706`; anchors read there |
| Branch | `fix/bld-7-connector-pipeline`, worktree `/home/shiro/rantai/wt-bld7` |

`A` = `rust/crates/lakehouse-api/src`, `D` = `dagster/dispar_orchestrate`,
`P` = `src/features/pipelines`.

## What is wrong today

- `src/features/connectors/connector-detail-page.tsx:330` links to
  `/pipelines/create?connectorId=<id>`; `P/pipeline-create-page.tsx:129-130`
  fills `connectorId` from the URL.
- `A/routes/pipelines.rs:3165-3171` (`create`) only checks the connector is
  in the caller's tenant.
- `D/authored_factory.py:701-703` refuses every run whose definition has a
  `connectorId` (`CONNECTOR_SOURCE_UNSUPPORTED_REASON`, line 270), before
  it even looks at `sourceZone`/`sourceTable`.
- But `sourceZone` and `sourceTable` are required on every pipeline
  (`CreatePipelineBody`), so a pipeline with a connector already names the
  table it reads. In the QA case it was `bronze.northwind_orders`, the
  table that connector's ingest wrote, and `BLD-3` (#155) already reads
  Bronze. Only the guard stops it.

## Decisions (planner, product owner agreed 2026-10-24)

1. **`DEC-15`:** no direct read from a source into Silver. A pipeline's
   connector means *where its data came from*, never a second read path.
   No adapter, no secret resolution, no bulk load is built here.
2. **The rule, one place in the API.** A pipeline with a connector must
   read from `bronze`, and `sourceTable` must be one of that connector's
   ingest targets (the `target` of an entry in the connector's
   `source_objects`, `lakehouse-store` `connectors.rs`, see the
   `text(o, "target")` reader near line 1484; reuse it, do not write a
   second parser, rule 4). Otherwise 400 with one fixed sentence that says
   what to pick, e.g. "A pipeline with a connector reads one of that
   connector's Bronze tables: choose `bronze` and one of: <targets>.
   To read another table, choose No connector." Applied on create (T1)
   and, after T4, on update. A connector with no ingest targets yet gets
   "This connector has not loaded any table yet; run its ingest first."
3. **The run.** Remove the blanket guard. A run with a connector reads its
   `sourceZone.sourceTable` exactly as any Bronze pipeline does (`BLD-3`
   path, masking and the author's policy unchanged). Keep a narrow guard
   for rows stored before this change that break the rule: connector set
   and `sourceZone != "bronze"` fails with a new fixed reason that says to
   edit the pipeline (rule 2 of `AGENTS.md`: honest failure, no silent
   read). Rewrite the constant and its comment; `WS4 item E3` wording goes.
4. **The trigger.** A pipeline created from a connector page runs after
   that table's load: the form adds `ingest_job` to `dependsOn` by default
   (switch shown and on, user may turn it off). `BLD-4`'s per-table
   sensor already filters `ingest_job` by the pipeline's own Bronze table
   (`D/authored_factory.py` `_latest_record_for_ingest_table`), and the
   API already refuses `ingest_job` for a non-Bronze source
   (`A/routes/authored_pipelines.rs` around line 490-512). Reuse both;
   do not add a new sensor.
5. **The form (connector page entry).** With `?connectorId=`: source zone
   is fixed to `bronze` (shown, not editable), *Source table* is a picker
   of that connector's ingest targets (from the connector the page already
   loads via `connectorService`, not a new endpoint unless none returns
   `source_objects`; if a new read is needed, stop and report), and the
   "run after load" switch from decision 4 is on. With no connector, the
   form is as today. The picker's empty state says to run the ingest
   first and links to the connector's Ingest tab.
6. **`BLD-8`: the run's real state and reason.** The pipeline page must
   not show "Running" for a run that has ended. Find where the page gets
   the state (`P/pipeline-detail-page.tsx` around line 511, and the API
   route behind it) and show the latest run's real status. For a failed
   run, show the failure reason the API already records or can read from
   Dagster, as a fixed classified text: our own `Failure` descriptions
   (`config rejected: …`, `refused: …`) are ours and may be shown; any
   other upstream text is not shown (principle 4), the page says "The run
   failed; see the run log" with a link to the run instead. If the API
   holds no failure text today and adding it needs a migration, stop and
   report before writing one.
7. **`BLD-9`: change or clear the connector.** `PUT /api/pipelines/{id}`
   accepts `connectorId` (absent = unchanged, `null` or empty = clear),
   with decision 2's check. The Edit form shows the connector picker
   instead of the read-only box (`P/pipeline-create-page.tsx:344`), with
   "No connector" as an option.
8. **Assistant.** `A/routes/ai/tools/pipelines.rs:1205` sends
   `connector_id: None`; leave it (the assistant does not set a connector
   today). Say so in the handoff.

## Tasks (one commit each, in this order)

- **T1 API rule on create.** Decision 2 in `create`; a helper shared with
  T4. Tests in `A/routes/pipelines.rs` next to the existing
  `connectorId` create tests (around lines 5541 and 5580): accepted with
  `bronze` + a target of that connector; 400 with the fixed sentence for a
  non-Bronze zone, for a table not among the targets, and for a connector
  with no targets; the tenant check still runs first (unchanged
  behaviour, unchanged test). Each new test fails without the change.
- **T2 The run.** Decision 3 in `D/authored_factory.py`. Unit tests in the
  existing authored factory tests: a definition with a connector and a
  Bronze source reaches the source resolution and SQL build (assert on the
  built SQL or the resolved source, no network); a connector with a
  non-Bronze source fails with the new reason; the old reason string is
  gone (`grep -rn "WS4 item E3" dagster` empty). Test names are full
  sentences.
- **T3 The form.** Decisions 4 and 5 in `P/pipeline-create-page.tsx` (and
  the connector page link only if it needs a change). Console tests: with
  `?connectorId=` the zone is `bronze` and fixed, the table picker lists
  only that connector's targets, the run-after-load switch is on and its
  `dependsOn` reaches the create call; with no connector nothing changes.
- **T4 `BLD-9`.** Decision 7, API and form, with tests (set, change,
  clear, refused by the T1 rule).
- **T5 `BLD-8`.** Decision 6, with tests for an ended run never showing
  "Running" and for the reason shown or withheld.
- **T6 Docs.** The pipeline feature page in `docs/core/features/` (find it
  by grepping for "Create pipeline" or "authored pipeline") says what a
  connector on a pipeline means and that direct source reads are not
  supported (`DEC-15`). Do not edit `docs/core/BACKLOG.md` status/PR; the
  planner does.

## Verification before handoff

Scoped per commit (`AGENTS.md`, "Keep build time down"). Before the
handoff, the full block for Rust, TypeScript and Python:
`cargo fmt --check`, workspace clippy, `cargo test --workspace`,
`bun run typecheck`, `bun run lint`, `bun test src`, the two
`ops/lint` scripts, `(cd dagster && python -m pytest dispar_orchestrate -q)`,
`git status`.

## Out of scope

Direct source reads (`DEC-15`), column drop or hash at ingest (`SRC-33`),
the FBIC switch (`BLD-10`), any new sensor, the assistant's pipeline tool.

## Handoff

(developer appends here)

## Review

(planner appends here)
