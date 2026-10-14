# SRC-21: load-mode follow-ups from SRC-10

Plan by the planner, 2026-10-14. Base: `main` at the commit this branch
starts from (after #115). Backlog `SRC-21` (Data, P2). The product owner asked
for what #115 left open to become a ticket and be finished.

## Why

PR #115 (`SRC-10`) merged on 2026-10-14 with no open `BLOCKER`. Two things it
left are worth fixing now.

1. **The assistant drops load modes.** `set_ingest_spec`'s schema
   (`rust/crates/lakehouse-api/src/routes/ai/registry.rs:646`,
   `set_ingest_spec_schema`) describes a source object as `name`,
   `incrementalKey` and `target` only. The tool forwards `sourceObjects` as
   given (`routes/ai/tools/ingest.rs`, `set_ingest_spec`), and the store reads
   a missing `loadMode` as `None`. The doc on
   `lakehouse-store/src/ingest_spec.rs:1031` says the job then replaces.
   - When the assistant re-sends a connector's table list (for example to add
     a table), every table set to deduplicate or keep history goes back to
     "replace".
   - The model also cannot set a mode a user asks for.
   - Plan §4 of `2026-10-12-src-10-load-modes.md` asked for the schema to list
     the modes; `tests/fixtures/tool_schemas.json` did not change.
2. **A merge-time change has no test.** In the `SRC-10` × `SRC-16` merge,
   `lift_pause_if_nothing_waits`
   (`lakehouse-store/src/schema_change.rs:1201`) started reading the
   deduplication keys. The reason: a pending removal of a key column, which
   Approve refuses, must not hold the pause. That merge review marked "no
   test covers that pair of tables" as `SHOULD-FIX`.

Not a task: the SQL gate job sits six seconds inside its 20-minute target.
The closing review already decided that the next step added there moves to a
job of its own (`docs/plans/SRC-10-RESULT.md`). Nothing to do until then.

## Decisions

1. **The schema lists the modes.** Each `sourceObjects` item gains:
   - `loadMode`: string, enum `replace`, `append`, `incremental`,
     `replace_dedup`, `incremental_dedup`, `history`. These are the
     `snake_case` names of `LoadMode` in `ingest_spec.rs`. Check the list
     against the enum; if it differs, the enum wins and the plan is wrong:
     stop and report.
   - `primaryKey`: array of strings.

   Each property gets a short description. The tool's description gains one
   sentence: when re-sending a connector's objects, keep each object's
   `loadMode` and `primaryKey` exactly as `get_ingest_spec` returned them.
2. **No server-side merge of missing modes.** `PUT /api/connectors/{id}/ingest-spec`
   keeps its "the body is the whole spec" meaning. Changing that is a
   contract change for the console too, and is out of scope.
3. **The fixture is regenerated the way that file is normally written.** Find
   how: a test that writes it, or a documented command. Do not hand-edit it
   unless that is the documented way.
4. **The store test uses the existing `sqlx::test` set-up in
   `lakehouse-store/tests/`**, next to the other schema-change pause tests.

## Tasks (one commit each)

1. **Assistant schema.**
   - Add `loadMode` and `primaryKey` to `set_ingest_spec_schema`, plus the
     description sentence (decision 1).
   - Regenerate `tests/fixtures/tool_schemas.json` (decision 3).
   - Add a test in the crate's existing tests for the AI ingest tools. It
     calls `set_ingest_spec` with an object carrying
     `loadMode: "incremental_dedup"` and `primaryKey: ["id"]`, then shows that
     both are in the saved spec. If no such test file can reach the tool, put
     it beside the closest existing `set_ingest_spec` test and say where.
   - Check: `cargo test -p lakehouse-api` with a name filter for the new test
     and for the tool-schema fixture test.
2. **Store test.**
   - The connector is paused by the schema-change policy. Table A has a
     pending removal of a column that is A's deduplication key (Approve
     refuses it). Table B has a pending approvable change.
   - Approving B lifts the pause. A stays pending.
   - Also assert the opposite: if B's change is still pending, the pause
     holds.
   - Check: `cargo test -p lakehouse-store` with a name filter. It needs
     Postgres: if none is reachable here, it is *not verified* locally, and CI
     runs it.
3. **Handoff** in this file: commits, the commands run with counts, and what
   was not verified.

## Checks per commit (`AGENTS.md`, Keep build time down)

```bash
cargo fmt --check
cargo clippy -p <crate> --all-targets -- -D warnings
cargo test -p <crate> <filter>
```

Run them with a shared `CARGO_TARGET_DIR` outside the repo. Disk is limited
(about 12 GB free), so build only the two crates.

## Not included

- A server-side merge of omitted modes.
- Console changes.
- Moving the gate step.
- The owner's acceptance run of `features/load-modes.md`.
