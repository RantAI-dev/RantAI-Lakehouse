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

Developer: Claude Sonnet 5.5. Branch `fix/sec-17-upload-hardening`, one slice
(all tasks), eight task commits on top of `1e47938`.

### Commits

| Task | Commit | Subject |
| --- | --- | --- |
| T1 | `e8ac7f8` | fix(api): refuse a file with more than 1,000 columns at preview and ingest |
| T2 | `92c2018` | fix(api): limit uploads in progress per user and in total |
| T3 | `2a8e936` | fix(api): query CSV download writes cells that cannot run a formula |
| T4 | `2121421` | docs(api): no other CSV writer in the API needs the formula rule (empty commit, the finding) |
| T5 | `3636e12` | fix(dagster): the file load refuses a header of more than 1,000 columns |
| T6 | `9ea65bc` | fix(frontend): CSV files the console writes cannot run a formula |
| T7 | `8590342` | fix(frontend): keep the Check step's controls when the file has too many columns |
| T8 | `deaa69b` | docs(ops): gate step, changelog and settings for the SEC-17 upload limits |

### Commands run, with results

All Rust commands with `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-catalog-target CARGO_BUILD_JOBS=2`, in the foreground.

- `cd rust && cargo fmt --check`: clean on the final commit.
- `cd rust && cargo clippy --workspace --all-targets --all-features -- -D warnings`: finished, 0 warnings, on the final commit. (`--all-targets` type-checks the unit and integration tests without linking them.) Also run per Rust commit as `cargo clippy -p lakehouse-api` / `-p lakehouse-store --all-targets -- -D warnings`.
- `bun run typecheck`: clean. `bun run lint`: 0 errors, 6 warnings (all in files this change does not touch, present before).
- `bun run test`: 877 pass, 1 skip, 0 fail, 878 tests in 99 files.
- `python3 ops/lint/check_intra_package_imports.py`: OK. `python3 ops/lint/check_bare_iceberg_count.py`: OK.
- `docker compose --profile '*' config --quiet`: exit 0, no output.
- `git status`: clean apart from the untracked `node_modules` symlink (not staged).

### Not verified, and why

- **Every Rust test** (`cargo test` is forbidden on this machine): the new unit tests in `upload_parse.rs`, `upload_limits.rs`, `csv_safe.rs`, `config.rs`; the `sqlx::test` store tests in `lakehouse-store/tests/uploads.rs`; the route tests in `lakehouse-api/tests/upload_routes.rs` and `query_download.rs`; and every existing Rust test (the changed `ingest` now reads the stored object before the claim). Only compilation (clippy `--all-targets`) is shown.
- `(cd dagster && python -m pytest ...)`: `pytest` and `dagster` are not installed here. The new tests in `test_file_ingest.py` and the changed one (seven to eight reasons) were not run. Only `ast.parse` of both files was checked.
- `ops/g9/upload_test.py` step 8: needs a running stack; only `ast.parse`.
- `docker compose up` for the two new variables (AGENTS.md rule 8): not done; only `config --quiet`. The change is two `${X:-}` lines on the API service.
- Console behaviour in a browser (the Check step with the controls on a first-read 400): component tests only.

### Tests most likely to fail in CI, and why

1. `the_fifth_simultaneous_upload_of_one_user_is_429_with_retry_after` (`upload_routes.rs`): five uploads in `tokio::join!` over a 1.5 s delayed `PUT`; it counts exactly four 201 and one 429, so it depends on all five reaching the handler within 1.5 s. A slow CI database could break the overlap (then it would see five 201). It is the only timing-based test. It also assumes `wiremock` priority 1 beats the default `PUT` mock, as `break_bucket` relies on.
2. `a_fifth_load_of_one_uploader_is_429_and_claims_and_launches_nothing` and `loads_of_another_uploader_do_not_count_against_this_one`: they build the four loads with `seed_loading` (uploaded by `Seeded Uploader`) and ingest a fifth seeded upload as Bayu. They rely on `ingest` counting the upload's recorded `uploaded_by` and not Bayu's name (decision D6). If the store's `COLS` or `seed` change, they fail.
3. `a_header_of_commas_longer_than_the_preview_is_refused`: 300 KiB of commas goes through `sniff` and `check_file` at upload; I read both and expect it accepted as text, but did not run it.
4. `lakehouse-store` tests (`mark_ingesting_within_limits`): first use of `pool.begin()` with the advisory lock for this table; `under_limits` takes `&mut PgConnection` and is called with `&mut tx` (it compiles under clippy).
5. `the_reasons_the_api_knows_are_the_reasons_in_the_shared_fixture`: the fixture has the eighth sentence appended last; the Rust and Python constants are in that order.
6. Python: `test_a_header_of_1001_columns_is_refused_before_any_row_is_read` relies on a header-only file otherwise failing as `NO_ROWS`, which `parse_file` does.

### Mismatches between the plan and the code, and what I chose

1. **"The claim made just before it must be released, as the existing refusal paths release it" (T2).** No existing path releases a table claim: `uploads.rs` (store) states "A claim is NEVER RELEASED", and `ingest` leaves the claim after a conflict or a failed launch. I did not add a release (that would break the module's invariant). Instead `ingest` asks a read-only count (`uploads::loads_under_limits`) before `claim_table`, so the normal refusal leaves no claim and the row `uploaded`; the locked count in `mark_ingesting_within_limits` stays the decision. In the race where a request passes the read and loses the locked count, the claim stands, like a claim after a failed launch. The route test for the 429 asserts no claim for the normal case. **Planner: confirm this is acceptable, or ask for a claim-and-mark in one transaction.**
2. **E9 "rewritten ... by a small state machine" and T3 `format=parquet`:** done as planned; the CSV test also covers the existing happy path (`n\n1\n2\n` is unchanged).
3. **E3/T7 "the Check step shows it and keeps its controls".** Not tests-only: on a first-read 400 the step had only a Retry button (no preview, so no controls). `UploadCheckStep` now renders the controls with an undecided "Detected by the service" option when the failure is `invalid_request` and nothing is shown yet. The client already carried the 429 and 400 sentences and statuses unchanged, so no client change (tests added).
4. **Per-user key for received files.** `create` has no `uploaded_by`; it uses `principal.display_name`, the same text `uploaded_by` records (two users with one display name share a place count; the same holds for loads).
5. **Where the 400 is checked in `ingest`:** after the `ingesting` and tenant checks and before `ensure_table_free` (the first external call), reading the head with `head_bytes`. A missing object now answers 503 from `ingest` where it used to answer from the launch; no existing test seeds an upload without its object.
6. **Python `parse_file`:** the column count is taken on a header list the reader already built, so a 50 MB header of commas still builds that list in the job before it is refused. The API refuses such a file first; the job is the authority for a load started another way.

### T4: what each file does

- `rust/crates/lakehouse-api/src/gold_export.rs`: exports a Gold mart to Iceberg (Parquet files) through `iceberg-rust`. The only "CSV" in it is a test string, `url('h', 'CSV')`, for the read-only SQL gate. Writes no CSV a person downloads. No change.
- `rust/crates/lakehouse-api/src/routes/alerts.rs`: the only "CSV" is a gate test (`url('http://example.com/x.csv', 'CSV')`). No CSV is written. No change.
- `rust/crates/lakehouse-api/src/routes/support.rs`: the only "CSV" is the same kind of gate test string. No change.
- A search for `text/csv`, `FORMAT CSV`, `to_csv` and `.csv` over `rust/crates` finds one writer, `routes/query.rs` `download` (done in T3). The console's writers all go through `toCsv` in `src/lib/csv.ts` (T6).

### Open decisions (the product owner's)

D1 to D8 of the feature page are unsigned; the defaults are built. The page's acceptance checklist is for the product owner on a running console and was not run.

## 8. Review

(The planner writes here.)
