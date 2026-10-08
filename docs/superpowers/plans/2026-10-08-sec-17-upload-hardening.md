# `SEC-17` Upload hardening: plan

Planner: Claude Opus. Developer: a different agent. Base: `origin/main` at
`cd8e3df`. Branch: `fix/sec-17-upload-hardening`. Spec:
`docs/core/specs/sec-17.md`. Feature page:
`docs/core/features/upload-limits-and-safe-csv.md` (decisions D1–D8; not yet
signed, the defaults are built).

## 1. What is wrong, with anchors (verified at `cd8e3df`)

- **No column cap.** The API's preview takes the header record as it is:
  `rust/crates/lakehouse-api/src/upload_parse.rs:499` (`preview`, `columns =
  records.get(header_row)`). It reads at most `PREVIEW_BYTES` (256 KB,
  `routes/uploads.rs:112`), so the API itself sees at most about 262,000
  cells; the unbounded part is the job. `dagster/dispar_orchestrate/
  file_ingest.py:360` (`parse_file`) refuses a header with no columns and a
  file with more than `MAX_ROWS` rows (`:100`), and has no width check.
  `routes/uploads.rs:1259` (`ingest`) launches the job without reading the
  file.
- **No concurrency limit.** `routes/uploads.rs:512` (`create`) reads a part
  of up to 50 MB into memory for every request it is given. `ingest` marks
  a row `ingesting` and launches a run with no count of the runs already
  going.
- **Formula cells.** `src/lib/csv.ts:15` (`escapeCell`) quotes for RFC 4180
  only. Every console-written CSV goes through `toCsv` in that file
  (`src/lib/table-csv.ts`, `src/features/dashboards/tile-dialogs.tsx:24`).
  `rust/crates/lakehouse-api/src/routes/query.rs:1001` (`download`) returns
  `ClickHouse`'s `FORMAT CSV` bytes untouched.

Already there and to be reused: `ApiError::TooManyRequests { message,
retry_after_secs }` (`rust/crates/lakehouse-core/src/error.rs:97`, rendered
with `Retry-After` in `lakehouse-api/src/error.rs:52`); the fixed failure
reasons shared by the job and the API (`JOB_FAILURE_REASONS`,
`routes/uploads.rs:232`; `FAILURE_REASONS`, `file_ingest.py:139`;
`ops/fixtures/upload_load_failure_reasons.json`, which a test on each side
reads).

## 2. Decisions already made

Feature page D1–D8. In engineering terms:

- E1. One sentence for the cap everywhere: `The file has more than 1,000
  columns.` It is the eighth job failure reason.
- E2. The cap counts the cells of the header record under the reading in use
  (encoding, delimiter, header row). The API counts on the first
  `PREVIEW_BYTES`; a header record cut off by that boundary is counted as
  far as it goes (a cut can only undercount, and 1,001 cells need about
  1 KB, so a header that breaks the cap is always seen). The job counts on
  the whole file and is the authority.
- E3. `GET …/preview` over the cap answers 400 with the sentence. The
  console's Check step shows it and keeps its delimiter, encoding and header
  row controls usable.
- E4. `create` is not given the cap: it has no reading to count under, and a
  wrong guess must not refuse a good file.
- E5. Files being received: counted in the API process (per user and in
  total), taken before the body is read, given back when the handler
  returns, on every path. Loads: counted in Postgres as rows in `ingesting`,
  per `uploaded_by` and in total, in the same transaction that marks the
  row, serialised by a transaction-level advisory lock so two simultaneous
  starts cannot both take the last place.
- E6. Over either limit: `ApiError::TooManyRequests` with `Too many uploads
  are in progress. Try again in a moment.` and `retry_after_secs: 5`.
- E7. `UPLOAD_MAX_CONCURRENT_PER_USER` (default 4) and
  `UPLOAD_MAX_CONCURRENT` (default 16), read once into the config. A value
  that is not a whole number of 1 or more is a start-up error (fail closed),
  not a silent default.
- E8. Formula rule, identical in TypeScript and Rust: a cell whose first
  character is `=`, `+`, `-`, `@`, U+0009 or U+000D gets `'` in front,
  unless the whole cell matches `^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$`.
  The apostrophe goes on before RFC 4180 quoting, so it ends up inside the
  quotes.
- E9. The query download is rewritten in the API, field by field, by a small
  state machine over the bytes `ClickHouse` returned. No new dependency.
  Parquet is returned as it is.
- E10. No migration.

## 3. Tasks

One task per commit, in this order. Rust tasks sit together.

**T1. Column cap in the API.** In `upload_parse.rs`: `pub const
MAX_COLUMNS: usize = 1_000` and a function that returns the header record's
width for a head and a reading without dropping a cut-off last record.
`routes::uploads::preview` answers 400 `TOO_MANY_COLUMNS` over the cap.
`routes::uploads::ingest` reads the head (`store.head_bytes`) under the
requested reading and answers 400 before any claim, mark or launch.
`JOB_FAILURE_REASONS` becomes eight; `ops/fixtures/
upload_load_failure_reasons.json` gets the sentence (the same commit, since
the Rust test reads the file).
*Check:* unit tests in `upload_parse.rs` (1,000 passes, 1,001 fails, a head
of 256 KB of commas fails, a cut-off header is counted); route tests in
`tests/upload_routes.rs` (preview 400; ingest 400 and the orchestrator mock
received no launch; a 1,000-column file previews).

**T2. Concurrency limits in the API.** Config fields per E7. A limiter in
`AppState` for files being received (a guard type that gives the place back
on drop). `create` takes a place first. A store function that counts and
marks in one transaction under an advisory lock, used by `ingest` in place
of `mark_ingesting`; the claim made just before it must be released, as the
existing refusal paths release it, when the limit refuses.
*Check:* unit tests for the limiter (fifth per user refused, another user
admitted, seventeenth in total refused, a dropped guard frees the place);
`sqlx::test` for the store function (four rows `ingesting` for one
`uploaded_by` refuse a fifth, a different `uploaded_by` passes, sixteen in
total refuse the next); a route test that a refused `ingest` answers 429
with `Retry-After` and leaves the row `uploaded` with no table claim;
config tests for the defaults and for a bad value.

**T3. Query download.** A module (for example `csv_safe.rs`) with the E8
rule and the rewriter; `routes::query::download` passes CSV through it.
*Check:* unit tests: quoted and unquoted fields; a quoted field holding
commas, doubled quotes and line breaks; `=`, `+`, `-`, `@`, tab, CR first;
numbers left alone (`-5`, `+3.2e4`, `.5`, `-1+1` is not one); empty fields;
CRLF and LF row ends; the last row with and without a row end; a route test
that a download whose mocked engine answer holds `"=1+1"` returns
`"'=1+1"` and that `format=parquet` bytes are unchanged.

**T4. Other CSV writers in the API.** `gold_export.rs`, `routes/alerts.rs`
and `routes/support.rs` mention CSV. Read each. If one writes a CSV file a
person downloads, pass it through T3's function in this commit; if not,
say what it does in the handoff. No change is also a result; write it down.

**T5. The job.** `file_ingest.py`: `MAX_COLUMNS = 1_000`,
`TOO_MANY_COLUMNS`, added to `FAILURE_REASONS` in the fixture's order,
raised by `parse_file` right after the header is read and before any row
is.
*Check:* tests in `test_file_ingest.py` named as sentences: a header of
1,000 columns is parsed; a header of 1,001 is refused with the sentence
before any row is read; the reasons still equal the fixture.

**T6. Console CSV.** `src/lib/csv.ts`: the E8 rule in `escapeCell`'s place
in the pipeline (apostrophe first, then quoting), for headers and cells.
New comments in English.
*Check:* `src/lib/csv.test.ts` cases mirroring T3's; `table-csv` tests
still pass.

**T7. Console messages.** The Check step shows the 400 sentence from
`preview` and keeps its controls usable; a 429 from `create` or `ingest`
shows the sentence and leaves the form as it was so the person can press
again. Look first: `apiFetch` and the upload client may already carry an
API message through unchanged, in which case this task is tests only.
*Check:* component tests for both.

**T8. Gate and docs.** `ops/g9/upload_test.py`: a step that uploads a
1,001-column file and expects 400 from preview and from ingest. `CHANGELOG.md`
entry. `docs/core/features/upload-file.md`: the limits, in its Limits
section. `.env.example`: the two variables, commented, with their defaults.
`docker-compose.yml`: pass them through with `${X:-}` only if the API
service lists its variables one by one (look first).

## 4. Pull request

One slice, all tasks.

## 5. Out of scope

Everything under "Not included" on the feature page. No change to
`routes/pipelines.rs` or `routes/lineage.rs`. No migration. No new
dependency.

## 6. Verification

Per commit: the scoped checks of `AGENTS.md` ("Keep build time down").
Before handoff, on the final commit: the block in `AGENTS.md` for Rust,
TypeScript and Python.

**On this machine, do not build Rust test binaries**: `cargo test`, `cargo
build` and `cargo check --tests` have crashed the machine. Run `cargo fmt
--check` and `cargo clippy --workspace --all-targets --all-features -- -D
warnings` with `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-catalog-target
CARGO_BUILD_JOBS=2`, and write every Rust test as *not verified* in the
handoff. CI on the pull request runs them.

## 7. Handoff

(The developer writes here.)

## 8. Review

(The planner writes here.)
