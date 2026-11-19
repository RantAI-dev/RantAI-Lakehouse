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
  number was `0149`; renumbered to `0151` in review 2 (main took `0149`, open PRs `0150`);
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
3. **Migration.** `rust/migrations/0151_kafka_connectors_stream.sql` with a
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

### Handoff (developer, 2026-11-18)

Commits, in plan order:

- `5e10388b9` fix(connectors): the console saves a Kafka connector as a stream (SRC-44)
- `4bd20717e` fix(api): refuse a Kafka connector saved as a batch, and a stream on any other adapter (SRC-44)
- `e3c505ac4` fix(db): move Kafka connectors saved as a batch to stream (SRC-44)

Notes on the build:

- T1: `ingestModeFor` in `src/lib/connectors/ingest-mode.ts` (+ test), used at the three anchors. The new create-page test (`ConnectorCreatePage, Kafka`) drives the wizard with a `kafka` type and asserts the saved spec has `ingestMode: "stream"`; with the create-page site put back to `"batch"` it fails (checked), so it guards the bug.
- T2: `validate_ingest_mode` and `InvalidIngestMode` in `lakehouse-store::ingest_spec`, called in the ingest-spec PUT handler right after `validate_load_modes`. The test helper at `connectors.rs:5971` builds an `ingestible` with mode `batch` for non-cdc adapters; no test builds a Kafka spec through it, so it was left unchanged. The API test covers both refusals (kafka+batch, sql+stream) and checks the body is not echoed; kafka+stream acceptance is covered by the store unit test and by the existing `connector_repoint` Kafka saves (all pass).
- T3: migration number `0149` is free (`origin/main` ends at `0148`). The `sqlx::test` re-runs the migration's SQL (`include_str!`) over seeded rows, twice, because the migration has already run on the empty test database.

Commands run (foreground unless noted) and results:

- `bun run typecheck`: pass (no errors).
- `bun run lint`: 0 errors, 11 warnings (all pre-existing, none in files I touched).
- `bun run test`: 4561 pass, 1 skip, 1 fail. The failure is `EmailSettingsForm (ADM-7) > never pre-fills the password...` (`src/features/admin/email-settings.test.tsx`); it passes in isolation (5 pass, 0 fail) and is unrelated (timing under full load).
- `bun test src/features/connectors/connector-create-page.test.tsx`: 15 pass, 0 fail. `bun test src/lib/connectors/ingest-mode.test.ts`: 3 pass.
- `cd rust && cargo fmt --check`: clean (rustfmt applied only to the two test files I changed).
- `cargo clippy -p lakehouse-store --all-targets -- -D warnings` and `cargo clippy -p lakehouse-api --all-targets -- -D warnings`: clean.
- `cargo test -p lakehouse-store`: lib 337 pass; integration 762 pass, 0 fail (includes `migration_0149_...` and `a_kafka_connector_is_saved_as_a_stream_and_only_a_kafka_connector_is`).
- `cargo test -p lakehouse-api --test it connector_`: 127 pass, 0 fail (includes the new refusal test).
- All cargo commands used `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-src44-target`.

Not verified:

- `cargo test -p lakehouse-api` as a whole (all test binaries) and `cargo test -p lakehouse-api --lib` (so `tests/route_auth.rs` and `tests/sec11_guard.rs` did not run): compiling the lakehouse-api test binaries ran for over 1.5 hours without finishing on this machine and was stopped. The change adds no route and no response text from an upstream error (the 400 sentence is a fixed string), and `clippy --all-targets` compiled every target clean, but the reviewer should run `cargo test -p lakehouse-api --test sec11_guard --test route_auth` and the lib tests.
- Workspace-wide `cargo clippy --workspace` / `cargo test --workspace`, the Python tests and the compose checks: not run (no Python, compose or other-crate change); left to the reviewer and CI.
- No browser or staging check of the console flow.


## Review

(reviewer appends here)

### Review 1 (planner, 2026-11-18, head 4bf57b8e1)

Diff read against the plan and AGENTS.md: D1–D4 followed, one helper at the
three console sites, fixed 400 text with no echo, targeted migration with a
why-header, `SRC-44` cited at every fix site. Callers checked: the gate
scripts (`ops/g6/g6_linklocal_test.py:229`, `g6_ingest_matrix_test.py:1326`)
already save Kafka as `stream`; no Rust test saves Kafka as `batch` outside
the new refusal tests.

No BLOCKER. No SHOULD-FIX.

Note (not a finding): the assistant's `set_ingest_spec` tool
(`routes/ai/registry.rs:1659`) now gets a 400 for Kafka + `batch`; its
description does not say Kafka uses `stream`. Left for the AI team, outside
this plan's scope.

Open before merge: `cargo test -p lakehouse-api` in full (route_auth,
sec11_guard) is *not verified* locally; CI on the PR must show it green.

### Review 2 (planner, 2026-11-19, PR #593 CI on 10ef7cb8a)

- **BLOCKER 1.** Migration number clash. `main` gained
  `0149_seed_connector_dials_as_app_role.sql` (SEC-78, #588) after this
  branch was cut, so every compose gate fails at `lakehouse-db-migrate`:
  `duplicate key value violates unique constraint "_sqlx_migrations_pkey"`
  (G3, G3a, G6, G8, SRC-3/11/13/17, Gold export). `0150` is taken by six open
  PRs (`0150_dashboard_template_creator.sql`). Fix: merge `origin/main` into
  this branch (no rebase, no force push), rename ours to
  `0151_kafka_connectors_stream.sql`, and update the test that
  `include_str!`s it and its name (`connector_ingest_spec.rs:585-612`). D3 in
  this plan now reads `0151`.
