# SRC-45: a Kafka connector made in the console loads its topic

Backlog: `SRC-45` (Data, P1). Source: staging retest of QA `LH-SRC-6-41`,
2026-11-19, build `staging 81acc21f` (after `SRC-44`, #593). Branch
`fix/src-45-kafka-first-load`, base `origin/main` at `13a254796`. Anchors
verified at that commit.

## What the retest showed

`SRC-44` works: the console's Kafka connector now runs as a stream. But it
still loads no row:

1. **No Bronze target.** The console saves the topic in `dial.topic` and no
   source object (`connector-create-page.tsx` wizard; `specWithTopic` in
   `src/lib/connectors/kafka-topic.ts:19-28` sends `spec.sourceObjects`
   back unchanged, i.e. `[]`). The loader takes the table from
   `source_objects[0]["target"]`
   (`dagster/dispar_orchestrate/ingest_factory.py:1150`, also `:1170`,
   `:1176`), so the first run that reads a message stops with
   `IndexError: list index out of range`, shown as "failed, the failure is
   not classified" (run `7f6e6c92`, staging code-location log).
2. **First run skips the topic.** `KafkaConsumer(...)` at
   `ingest_factory.py:1115` has no `auto_offset_reset`, so kafka-python's
   default `latest` applies: a new group starts at the end of the topic, and
   the first run reads nothing already there (runs `0953f4e5`, `a798210f`,
   `131b0d51`: Succeeded, 0 rows, "Every table was loaded").
3. `specWithTopic` still falls back to `ingestMode ?? "batch"`
   (`kafka-topic.ts:24`), the one site `SRC-44` missed.

The `G6` gate hid (1) and (2): it registers the Kafka connector with
`source_objects=[{"name": topic, "target": "g6_kafka_events"}]` through the
API (`ops/g6/g6_ingest_matrix_test.py:1325`) and produces messages only
while the run is live (`_produce_kafka_fixture`, `:467`).

## Decisions (made; do not change)

- D1. **The server gives a Kafka spec its target.** In `save_ingest_spec`
  (`rust/crates/lakehouse-api/src/routes/connectors.rs:3548`), when
  `adapter == "kafka"` and `dial.topic` is a non-empty string, the stored
  `source_objects` is: the body's own when it already holds exactly one
  object whose `name` equals the topic (kept as sent, so an API caller's
  chosen target stands); otherwise
  `[{"name": <topic>, "target": default_bronze_target(<connector name>, <topic>)}]`
  (`lakehouse-store::ingest_spec::default_bronze_target`, `:2318`, the rule
  the console's `defaultTarget` mirrors). This covers the console wizard,
  the topic picker and the assistant's `set_ingest_spec` in one place. Put
  the pure rule in `lakehouse-store::ingest_spec` next to
  `validate_ingest_mode`, with `# Errors`/docs and a unit test; the route
  only fetches the connector name and calls it.
- D2. **The loader fails closed and readable.** In `run_kafka_stream_batch`,
  before any consume, a spec with no source object carrying a non-empty
  `target` raises a classified error (category `setting_rejected` through
  `ingest_errors.py`, not `other`/unclassified) whose fixed message tells the
  user what to do, e.g. "This Kafka connector has no Bronze table yet: pick
  its topic again on the Ingest tab." No `IndexError` path remains
  (`:1125`, `:1150`, `:1170`, `:1176`).
- D3. **A new consumer group reads the whole topic.** `auto_offset_reset=
  "earliest"` on the ingest consumer (`ingest_factory.py:1115`) only. It
  only matters for a group with no committed offset; a group that has
  committed resumes where it stopped (unchanged). Discovery
  (`discover.py`) and CDC (`cdc_apply.py`) are not touched. Product owner
  approved `earliest` on 2026-11-19 (it is what `LH-SRC-6-41` expects).
- D4. `specWithTopic` uses `ingestModeFor(spec.adapter)` from
  `src/lib/connectors/ingest-mode.ts` (`SRC-44`) as its fallback.
- D5. **The gate saves Kafka the console's way.** In
  `ops/g6/g6_ingest_matrix_test.py`, register `g6-kafka` with
  `source_objects=[]` and assert the table lands at the default target the
  server now gives (compute it the same way, or read it back from
  `GET /api/connectors/{id}/ingest-spec`; do not hard-code a guess). Produce
  part of the fixture BEFORE the run is launched so the gate proves D3
  (messages already in the topic load). Keep the reload step's contract
  (second run adds exactly the uncommitted messages).
- No migration: an existing Kafka row with no source object gets D2's
  message, and picking its topic again saves it through D1.

## Tasks (one commit each, in order)

1. **Store rule + route (Rust).** D1, with a store unit test (empty
   objects → default target; one object named as the topic → kept; topic
   changed → new default) and an API test in
   `rust/crates/lakehouse-api/tests/connector_create_checks.rs` that saves a
   Kafka spec with `sourceObjects: []` and reads back the default target.
   Cite `SRC-45` at the site.
2. **Loader (Python).** D2 and D3 in `ingest_factory.py` (+ the
   classification in `ingest_errors.py` if a new rule is needed). Unit
   tests in `dagster/dispar_orchestrate/test_ingest_factory.py` /
   `test_adapters_kafka.py`: a stream spec with no target fails classified
   with the fixed message before consuming; the consumer is built with
   `auto_offset_reset="earliest"` (inject/monkeypatch `KafkaConsumer`; no
   network in unit tests). Test names are full sentences.
3. **Console (TypeScript).** D4, with a `kafka-topic.test.ts` case for a
   spec with no `ingestMode`.
4. **Gate (Python, `ops/`).** D5.

## Checks (the owner chose CI to save disk; do not build Rust locally)

- Rust: `cd rust && cargo fmt --check` only. Everything else (clippy,
  tests) is *not verified* locally and proven by CI on the PR.
- Python: `(cd dagster && python -m pytest dispar_orchestrate -q -k "kafka or ingest_factory or ingest_errors")`
  and `python3 ops/lint/check_intra_package_imports.py && python3 ops/lint/check_bare_iceberg_count.py`.
- TypeScript: `bun run typecheck && bun run lint` and
  `bun test src/lib/connectors`.
- The gate is proven only in CI (`Rust · G6 ingest matrix (files, APIs, streams)`).

## Out of scope

Seeking to `bronze_meta.ingest_offset` instead of Kafka's committed offset;
a UI to choose `earliest`/`latest`; the assistant tool's description (AI
team); the run summary wording for a 0-row stream run; the QA kit's
retention (`LH-SRC-6-41` notes).

## PR

One PR: `SRC-45 fix(connectors): a Kafka connector made in the console loads its topic`.
Opened by the reviewer.

## Handoff

### Handoff (developer, Sonnet 5.5)

Commits, one per task, in plan order on `fix/src-45-kafka-first-load`:

1. `250994e4d` T1: `kafka_source_objects` in `lakehouse-store::ingest_spec` (pure rule, store unit tests `a_kafka_spec_gets_the_default_target_of_its_topic`, `a_spec_that_is_not_kafka_or_has_no_topic_keeps_its_source_objects`); `save_ingest_spec` calls it for `adapter == "kafka"` after fetching the connector name with `connectors::get_connector`; API test `saving_a_kafka_spec_with_no_source_object_stores_the_default_target_of_its_topic` in `connector_create_checks.rs`.
2. `0e7c3c9c0` T2: `KafkaTargetMissing` (classified `setting_rejected`) raised before the consumer is built; `auto_offset_reset="earliest"`; every `source_objects[0]["target"]` read in `run_kafka_stream_batch` goes through the one checked `target`. Tests in `test_ingest_factory.py` and a case in `test_ingest_errors.py`.
3. `7c8ae4fca` T3: `specWithTopic` falls back to `ingestModeFor(...)`; new case in `kafka-topic.test.ts`.
4. `e51fdc366` T4: gate registers `g6-kafka` with `source_objects=[]`, reads the target back from `GET .../ingest-spec`, produces 3 messages before the connector exists and asserts they are in the Bronze table.

Commands run and results:

- `cd rust && cargo fmt --check`: clean (no output), after T1.
- Python, with a scratch dependency install outside the repo (`pip install --target`, no venv available; `pymysql` and `pyiceberg-core` were not installed there): `python3 -m pytest dispar_orchestrate -q -k "kafka or ingest_factory or ingest_errors"` gave 187 passed, 6 failed, 2410 deselected. The same 6 fail on the base commit (stashed run: 184 passed, 6 failed): 3 in `test_ingest_factory.py` need `pymysql`, 2 in `test_sink_killed_run.py` need `pyiceberg-core`, 1 more of the same family. The 3 extra passes are this change's new tests.
- `python3 ops/lint/check_intra_package_imports.py`: OK. `python3 ops/lint/check_bare_iceberg_count.py`: no bare count found.
- `python3 -m py_compile ops/g6/g6_ingest_matrix_test.py`: ok.
- `bun run typecheck`: clean. `bun run lint`: 0 errors, 11 warnings (all pre-existing, none in touched files). `bun test src/lib/connectors`: 73 pass, 1 skip, 0 fail.

Not verified (left to CI, per the owner's instruction): `cargo clippy`, `cargo test` (the store unit tests and the API test were never compiled or run locally), the full workspace run, and the G6 ingest matrix (`Rust · G6 ingest matrix (files, APIs, streams)`), including whether the `id` column name in the preload count query matches the Bronze table.

Notes for the reviewer: in the API test the connector is created with `fixture(true)` and broker `broker-a:9092`, the same as `connector_repoint.rs`, so no DNS lookup is needed. The route fetches the connector name only for `kafka`, so other adapters pay no extra query.


## Review

(reviewer appends here)

### Review 1 (planner, 2026-11-19, head ffde241de)

Diff read against D1–D5 and AGENTS.md. D1: `kafka_source_objects` is a pure
store fn beside `validate_ingest_mode`, reuses `default_bronze_target`, keeps
an API caller's single object named as the topic; the route fetches the
connector name only for `kafka`. D2: `KafkaTargetMissing` is raised before a
consumer exists, classified `setting_rejected`, fixed text, and every
`source_objects[0]` read now goes through the checked `target`. D3:
`auto_offset_reset="earliest"` on the ingest consumer only. D4 done. D5: the
gate preloads 3 messages before registering, saves `source_objects=[]`, reads
the server's target back, and counts the preloaded ids; no other reference to
the old `g6_kafka_events` remains.

No BLOCKER. No SHOULD-FIX.

Open before merge (*not verified* locally, by the owner's choice of CI):
clippy and `cargo test` for the new store/API tests, and the `G6 ingest
matrix (files, APIs, streams)` gate, including its assumption that the Kafka
Bronze column is named `id`.

### Review 2 (planner, 2026-11-19, PR #597 CI on 2b7094c54)

- **BLOCKER 1.** `Rust · clippy` fails: `clippy::too_many_lines` on
  `save_ingest_spec` (`connectors.rs:3548`, 105/100) after the D1 block was
  added inline. Fix: move the D1 block into a small helper in the same module
  (e.g. `async fn with_kafka_source_objects(state, id, body) -> ApiResult<..>`
  or one that returns the new `source_objects`), called from the handler.
  Do not add `#[allow(clippy::too_many_lines)]` (rule 2: never weaken a lint
  to get green).

### Handoff (review 2 fix)

- BLOCKER 1 (`PR #597 review 2`): the D1 Kafka block moved into a private
  `fill_kafka_source_objects(state, id, &mut body) -> ApiResult<()>` in
  `routes/connectors.rs`; `save_ingest_spec` calls it. No `allow`; behaviour
  unchanged (same connector lookup, same `NotFound`, same
  `kafka_source_objects` call, only for `kafka`).
- `save_ingest_spec` code lines (blank and comment lines excluded, signature
  included): 100 before the change counted 105 by clippy, now about 95 for the
  body (hand count; 11 code lines replaced by 1).
- Ran: `cd rust && cargo fmt --check -- crates/lakehouse-api/src/routes/connectors.rs`
  (clean). *Not verified*: clippy and tests, left to CI by the owner's choice.
