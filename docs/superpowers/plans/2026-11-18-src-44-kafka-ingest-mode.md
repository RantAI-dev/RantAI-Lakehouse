# SRC-44: Kafka connectors run as a stream

Backlog: `SRC-44` (Data, P1). Source: QA `LH-SRC-6-41` (Fail, Major,
2026-11-18). Branch `fix/kafka-connector`, base `origin/main` at `37d16203f`.
Anchors verified at that commit.

## The bug

The console saves every connector that is not CDC with
`ingestMode: "batch"`:

- `src/features/connectors/connector-create-page.tsx:155`
  `ingestMode: adapter === "cdc" ? "cdc" : "batch"`
- `src/features/connectors/connector-edit-page.tsx:241`
  `spec.ingestMode ?? (adapter === "cdc" ? "cdc" : "batch")`
- `src/features/connectors/connector-ingest-panel.tsx:484`
  `spec.ingestMode ?? "batch"`

The loader runs a `kafka` connector only as `stream`
(`dagster/dispar_orchestrate/ingest_factory.py:74`, "A `kafka`-adapter
connector's `ingest_mode` is `"stream"`, never `"batch"`"). On the batch path
there is no dispatch for `kafka`, so every run stops with `UnknownAdapter`,
shown to the user as "a setting was rejected". The API stores whatever mode
it is sent (`rust/crates/lakehouse-api/src/routes/connectors.rs:3161`
`IngestSpecBody`, stored at `:3638`); the database CHECK
(`rust/migrations/0043_ingest_tier2_adapters.sql:62`) admits `batch`, `cdc`
and `stream` for any adapter.

## Decisions (made; do not change)

- D1. One helper decides the default mode from the adapter: `kafka` →
  `stream`, `cdc` → `cdc`, anything else → `batch`. All three console sites
  use it; no site keeps its own copy (AGENTS.md rule 4).
- D2. The API fails closed: `PUT /api/connectors/{id}/ingest-spec` refuses
  `adapter = "kafka"` with a mode other than `stream`, and `stream` with any
  adapter other than `kafka`, with **400** and a fixed message (no echo of
  the body). `cdc` pairing rules are out of scope (unchanged).
- D3. Rows already saved wrong are repaired by a migration:
  `UPDATE connector SET ingest_mode = 'stream' WHERE adapter = 'kafka' AND
  ingest_mode = 'batch';` (targeted `AND col = '<old>'` form). Next free
  number is `0149` (latest on `main` is `0148_usage_rate_and_permission.sql`);
  re-check before using it.
- D4. No change to the loader (`dagster/`): it is right.

## Tasks (one commit each, in order)

1. **Console helper.** Add `src/lib/connectors/ingest-mode.ts` exporting
   `ingestModeFor(adapter: string | undefined)` returning
   `"cdc" | "stream" | "batch"`, with a doc comment saying why (cite
   `ingest_factory.py` and `SRC-44`). Use it at the three anchors
   (`spec.ingestMode ?? ingestModeFor(adapter)` where there is a fallback).
   Add `src/lib/connectors/ingest-mode.test.ts` (kafka → stream, cdc → cdc,
   sql/rest/undefined → batch). Add or adjust a create-page test proving a
   Kafka connector is saved with `ingestMode: "stream"`.
   Check: `bun run typecheck && bun run lint && bun run test` (scoped runs
   while iterating are fine). Never run prettier; match file style by hand.
2. **API refusal.** In the ingest-spec PUT handler, next to the existing
   `validate_load_modes` call (`connectors.rs:3568`), refuse the mismatched
   pairs per D2. Put the rule where it belongs (a small pure fn, in
   `lakehouse-store::ingest_spec` beside `validate_load_modes` if that fits
   the existing pattern), with `# Errors` docs. Tests for both directions
   (kafka+batch → 400, sql+stream → 400) and kafka+stream accepted. Fix the
   test helper at `connectors.rs:5971` if any test builds a Kafka spec.
   Check: `cd rust && cargo fmt --check && cargo clippy -p <crate> --all-targets -- -D warnings && cargo test -p <crate>`.
3. **Migration.** `rust/migrations/0149_kafka_connectors_stream.sql` with a
   why-header (the bug, `SRC-44`, `LH-SRC-6-41`) and the D3 `UPDATE`. Same
   sitting as task 2 (sqlx embeds migrations). Add a `sqlx::test` if the
   crate has a pattern for migration data fixes; otherwise say so in the
   handoff.

Every commit body cites `SRC-44` / `LH-SRC-6-41`; the fix sites cite
`SRC-44` (rule 13).

## Out of scope

Loader changes; CDC mode rules; the assistant (no BI surface); any UI
redesign; `.env.staging` and `AGENTS.md` local edits in the working tree
(never stage them).

## PR

One PR: `SRC-44 fix(connectors): run Kafka connectors as a stream`. Opened by
the reviewer, not the developer.

## Handoff

(developer appends here)

## Review

(reviewer appends here)
