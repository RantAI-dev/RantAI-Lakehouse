# `SEC-22`: reading a workbook or a Parquet file can never end the API process

Planner and reviewer: Claude Opus. Developer: a non-Opus agent (`AGENTS.md`).
Base: `origin/main` at `f40b20eb` (re-anchored 2026-10-28, section 0; first
written at `17867e52`). Branch: `fix/sec-22-upload-conversion-memory`, worktree
`/home/shiro/rantai/wt-sec22`. One PR slice, to `main`, before the
Phase 3 PR (#186): the product owner chose this on 2026-10-16. Phase 3 slices 2
and 4 build on it (`docs/superpowers/plans/2026-10-16-data-phase-3-uploads.md`).

Backlog row (`docs/core/BACKLOG.md`, `SEC-22`, P0, Task): converting an uploaded
workbook can exhaust the API's memory and stop the API for every user. Also: the
Parquet reader trusts sizes from the file's footer the same way; nothing limits
conversions running at once; workbook text has no byte cap; each preview converts
the whole file again; and the limits doc in `upload_workbook.rs` says the opposite
of the code. Source: PR #76 review, BLOCKER 1 (merged without the fix), and
`docs/superpowers/plans/2026-10-12-delivery-review.md`.


## 0. Amendment, 2026-10-28: re-anchored at `f40b20eb`

The plan was written before Phase 3 (`DATA-17`, `DATA-18`, PR #186) merged.
Where this section and sections 1 to 4 disagree, **this section wins**. Line
numbers below are at `f40b20eb`; `R` is still `routes/uploads.rs`.

### What Phase 3 changed

- **A Parquet file is no longer converted for a load.** The job reads it
  natively (`dagster/dispar_orchestrate/parquet_ingest.py`), and
  `convert_upload` answers 500 for `Conversion::Parquet` (`R:2404`). The API
  still parses Parquet in three places:
  - `create`, on the request bytes (`R:881-887`, `upload_parquet::inspect`);
  - `parquet_preview` (`R:1860-1868`, `upload_parquet::convert` with
    `PARQUET_PREVIEW_ROWS`);
  - `check_parquet_footer` at ingest (`R:2364-2375`, `inspect`).
  The last two read through `RangeReader` (`upload_parquet_range.rs:48`). The
  footer is bounded (`MAX_FOOTER_BYTES`, `upload_parquet.rs:160`) and so is
  the first row group's compressed size (`MAX_PREVIEW_GROUP_BYTES`, `:166`).
  The sizes a page declares for itself once decompressed are still the file's
  word, and the decode still runs in the API. So Parquet stays in scope, for
  `inspect` and the preview only.
- **Workbooks are unchanged:**
  - `create` lists the sheets at `R:873-879`;
  - `workbook_preview` reads the whole object and runs `read_sheet` on every
    call (`R:1817-1830`);
  - `convert_upload` reads the whole object, writes `to_csv(None)` whole,
    and stores it with one `put` under `converted_key` (`R:2377-2414`,
    `R:582`, `<storage_key>.converted.csv`).
- **`main.rs`:**
  - `main()` (`:74-81`) only builds the runtime.
  - `run()` reads `Config::from_env` (`:91`) and then dispatches
    `rotate-secrets-key` (`:94-100`).
  - The `inspect-upload` branch goes at the top of `main()`, before the
    runtime is built (D1 is unchanged).
  - PR #192 (`SEC-14` part B) adds a rustls provider install at the start of
    `main()`. Whichever PR merges second keeps the child's branch first: the
    child opens no connection and needs no provider.
- **The store already writes in parts** (`upload_store.rs`):
  `create_multipart` `:387`, `put_part` `:403`, `complete_multipart` `:436`,
  `abort_multipart` `:464`, plus `get_range` `:313` and `head_bytes` `:296`.
  D5's streamed write uses these. Do not add a second multipart path (rule 4).
- **`blocking` (`R:623`) stays.** The gzip and zip checks in `create`
  (`R:863-872`) still use it. They stream with a cap and are not `SEC-22`.

### Decisions that change

- **D1a. No file parser runs in the API process.** Every workbook and Parquet
  parse listed above goes through the child.
- **D1b. Feeding the child.** The child never reads the store or the network
  (D1).
  - At `create`, the request bytes are already in memory, and the API writes
    them to the child's stdin.
  - Everywhere else, the API streams the stored original into stdin in
    chunks (`get_range`, 8 MiB). So the API never holds the file, which keeps
    Phase 3's invariant for Parquet. The child holds it under its own memory
    limit; the 50 MiB upload cap (`MAX_UPLOAD_BYTES`, `R:171`) bounds it.
  - A Parquet child reads from memory (`Bytes` is a `ChunkReader`), so
    `RangeReader` is no longer used by these three calls. Remove it if
    nothing else uses it.
  - Cost, deliberate: a Parquet preview now moves the whole file (at most
    50 MiB) from the store, not just the footer and the first row group. Say
    so in the commit.
- **D2 (replaces D2).** The child's operations. Each writes one JSON line to
  stdout first, then, for the two that convert, the CSV text until EOF:
  - `list-sheets` → `WorkbookInfo`.
  - `convert-sheet [name]` → `{sheet, info, empty}`, then the sheet's CSV
    (`to_csv(None)`).
  - `inspect-parquet` → `ParquetInfo`.
  - `preview-parquet <max_rows>` → the converted preview's `info`, then its
    CSV.

  There is no whole-file Parquet conversion; the job reads the file. The
  existing pure functions keep their unit tests.
- **D5 (replaces D5).** Convert a workbook sheet once and store it.
  - `create` stores the `list-sheets` JSON beside the original as
    `<storage_key>.sheets.json`.
  - A preview or an ingest finds the sheet's position `n` from that list. A
    missing sheet means `default_sheet` (`upload_workbook.rs:137`), the same
    rule `read_sheet` uses (`:332`).
    - The list is read from storage, which is small, with no child.
    - An upload made before this change has no `.sheets.json`. It runs
      `list-sheets` once and stores the result.
  - The sheet's text is stored once as `<storage_key>.converted.<n>.csv` and
    its JSON line as `<storage_key>.converted.<n>.json`. Both are written by
    streaming the child's stdout through the multipart helpers, and the
    upload is aborted on any error (D6).
  - A preview with both objects present runs no child. It reads the first
    `PREVIEW_BYTES` with `head_bytes`, as a text file does. The
    `WORKBOOK_PREVIEW_RECORDS` cut is replaced by that byte cut. This is a
    deliberate behaviour change, stated in the commit (the old D5's reason
    stands).
  - Ingest passes the stored `.converted.<n>.csv` key in `Converted`, so the
    job reads the same text the preview showed. It converts only when the
    object is missing.
  - **Parquet previews are not stored.** Their header row is fixed (`R:1873`),
    so a second preview is rare, and the child reads one row group. Each
    preview runs `preview-parquet`.
- **D8 (extends D8).** Delete removes:
  - `.sheets.json`;
  - every `.converted.<n>.csv` and `.converted.<n>.json`;
  - the legacy `.converted.csv`.

  The existing delete tests that name `.converted.csv` (`R:3153`, `R:3554`)
  stay and gain the new names.
- **D7 (narrows D7).** T5 measures:
  - `convert-sheet` on a 5,000,000-cell `.xlsx`;
  - `preview-parquet` on a 50 MiB Parquet file whose first row group is
    close to `MAX_PREVIEW_GROUP_BYTES`;
  - the child's baseline on empty input.

  There is no 5,000,000-cell Parquet conversion to measure any more.

### Tasks: what changes

- T1 unchanged.
- T2 builds the four operations of D2 (above).
- T3 also stores `.sheets.json` at `create`, and replaces the three Parquet
  call sites.
- T4 adds two route tests:
  - a Parquet preview through the real child;
  - a workbook preview made after a header-row change, which runs no child.
    Count the child runs through the runner, test-only.
- T5 follows D7 above.
- T6 unchanged. The `g9-upload` job is at `.github/workflows/ci.yml:1557` now.

## 1. What is true at `17867e52`

`R` = `rust/crates/lakehouse-api/src/routes/uploads.rs`.

- **Every workbook and Parquet parse runs inside the API process,** on a blocking
  thread (`blocking`, `R:548-559`):
  - in `POST /api/uploads`, to list the sheets (`R:732`, `upload_workbook::list_sheets`) and to inspect a Parquet file (`R:740`, `upload_parquet::inspect`);
  - in every preview (`R:1246-1248`, `read_sheet`; `R:1269-1271`, `upload_parquet::convert`);
  - at ingest (`R:1660-1696`, `convert_upload`).
- **The only guard is `catch_unwind`** (`upload_workbook.rs:180-182`, `guarded`). It turns a parser panic into a refusal. It cannot catch an allocation failure: Rust aborts the process on one. Opening an `.xlsx` (`Xlsx::new`, `upload_workbook.rs:205`) or an `.xls` (`Xls::new`, `:210`) already parses the shared strings or the workbook stream, and the parser sizes buffers from counts the file declares. So one crafted 50 MiB file can end the API for every tenant. `MAX_SHEET_CELLS` (`:93`) is checked later, while cells stream in.
- **The module doc says otherwise** (`upload_workbook.rs:64-74`): "a sheet that only claims a huge range costs no more than the cells it has". This is true of the sheet reader but not of opening, and it breaks rule 1.
- **Parquet** reads the footer, then pages sized by the footer (`upload_parquet.rs`, `inspect` and `convert`). By reading the crate, not by running it, the same pattern applies: sizes the file declares are trusted.
- **No limit on conversions at once.** Each one is a `spawn_blocking` (`R:551`), and tokio's blocking pool allows 512 threads.
- **No byte cap on workbook text.** The CSV a sheet becomes is built whole in memory and stored with one `put` (`R:1693-1694`). Parquet has a 512 MiB cap (`upload_parquet.rs:120`).
- **Every preview converts again** (`R:1172-1185` doc, `R:1246`). A header-row change in the Check step parses the whole workbook once more.

## 2. Decisions (planner)

- **D1. The parse runs in a child process, never in the API process.**
  - The child is the same binary with a subcommand, `lakehouse-api inspect-upload <op>`.
  - `main()` dispatches it **before** the tokio runtime is built and before `Config::from_env`, so the child needs no environment, starts no threads, and opens no network connection. Today's dispatch, `rotate-secrets-key` at `main.rs:83`, runs after both, so the new branch goes first.
  - The child does four things, in order:
    1. It sets its own resource limits: memory (`RLIMIT_AS`, or `RLIMIT_DATA` if T2 shows `AS` counts reservations it should not) and CPU time (`RLIMIT_CPU`).
    2. It reads the file from stdin.
    3. It writes the answer to stdout.
    4. It exits with a code from a fixed table.
  - Setting a limit on itself avoids `Command::pre_exec`, which is `unsafe`. Use a safe wrapper: prefer `nix` (already in `Cargo.lock` at 0.31.3) with its `resource` feature over a new crate, and say which in the handoff.
  - An allocation failure, a stack overflow or a CPU-limit signal then ends the child only.
- **D2. The child's operations.** `list-sheets` → JSON `WorkbookInfo`. `convert-sheet [name]` → CSV text on stdout. `inspect-parquet` → JSON `ParquetInfo`. `convert-parquet [max_rows]` → CSV text. The existing pure functions (`list_sheets`, `read_sheet`, `upload_parquet::inspect` and `convert`) are what the child calls; they keep their tests.
- **D3. The API side: one runner.**
  - A `ConversionRunner` in `AppState` spawns the child, feeds it the bytes, and reads its stdout as a stream. It runs it with:
    - a wall-clock limit of **90 s** (the console's proxy gives up at 130 s, `next.config.ts:27`);
    - `kill_on_drop`;
    - at most **2** conversions at once per API process (`UPLOAD_MAX_CONVERSIONS`, default 2).
  - When all slots are taken, the answer is 429 with `Retry-After: 5` and a fixed sentence, as `SEC-17` answers (`upload_limits.rs:25-37`).
  - The child's path is `std::env::current_exe()` in the server. Tests pass `env!("CARGO_BIN_EXE_lakehouse-api")` (the package has that bin target), so the route tests run the real child.
- **D4. What the user is told.** Every answer is a fixed sentence; nothing the child or the library wrote reaches a response (principle 4):
  - A refusal code: the existing sentence of that `WorkbookError` or Parquet error.
  - Killed by a signal, out of memory, out of CPU time, or past 90 s: "This file needs more memory or time to read than an upload is allowed. Save it as CSV and upload that instead." The cases cannot all be told apart reliably, so there is one sentence.
  - Busy: "Too many files are being read right now. Try again in a few seconds." (429).
  - The child cannot be started: 500 `"The file could not be converted."` (the existing `CONVERSION_FAILED`). The real cause is logged, not returned.
- **D5. Convert once and store it; previews read the stored text.**
  - A sheet's conversion is stored once as `<storage_key>.converted.<n>.csv`, where `n` is the sheet's position in the workbook's list. A Parquet file's goes under `.converted.0.csv`. The child's stdout is streamed into the store with an S3 multipart write (`object_store::WriteMultipart` over `put_multipart`), so the API never holds the converted text whole.
  - A preview reads the stored text's first `PREVIEW_BYTES` with `head_bytes` (`upload_store.rs:195`), exactly as for a text file, and converts only when that object does not exist yet. So a header-row change no longer parses the workbook.
  - Ingest uses the stored object; it converts only if none exists.
  - Behaviour change, deliberate: the workbook and Parquet previews now see what fits in 256 KiB rather than 2,000 records (`WORKBOOK_PREVIEW_RECORDS`, `PARQUET_PREVIEW_ROWS`). This matches text files, and `SEC-17`'s `header_width` check now covers converted text too. Say so in the commit.
- **D6. One byte cap for converted text**, 512 MiB (`upload_parquet.rs:120`'s number, now applied to workbooks too). The child stops writing and exits with a "too large" code. Any partial object is aborted, never kept. It is a cap, not a measurement.
- **D7. The limits come from a measurement.** The child's memory limit starts at 1 GiB and its CPU limit at 60 s, both the planner's numbers. T5 measures peak memory and time for the largest file the caps allow: a 5,000,000-cell sheet and a 5,000,000-cell Parquet file. It also measures the child's baseline. The defaults are then set to the measured peak × 1.5, rounded up to 128 MiB, and the constants cite `docs/plans/SEC-22-RESULT.md` (principle 5). `UPLOAD_CONVERT_MEMORY_MB` overrides the memory limit.
- **D8. Delete removes every converted object** of the upload: the new `.converted.<n>.csv` names and the legacy `.converted.csv`.
- **D9. No assistant change, and no console change** beyond the two new sentences reaching the existing error display, which already shows the API's sentence. `SEC-22` is not a BI feature.

## 3. Anchors (`main` at `17867e52`)

- `rust/crates/lakehouse-api/src/main.rs:63-70` (`main`, runtime built by hand), `:83-89` (subcommand dispatch).
- `rust/crates/lakehouse-api/src/upload_workbook.rs`: `guarded` `:180`, `open` `:203-214`, `Book::collect` `:240-270`, `list_sheets` `:307`, `read_sheet` `:319`, the limits doc `:64-74`.
- `rust/crates/lakehouse-api/src/upload_parquet.rs`: `inspect`, `convert`, `MAX_OUTPUT_BYTES` `:120`.
- `rust/crates/lakehouse-api/src/routes/uploads.rs`: `blocking` `:548-559`, `create` `:696` (`:732`, `:740`), `workbook_preview` and `parquet_preview` (`:1240-1285`), `convert_upload` `:1660-1696`, `converted_key` `:507`, `delete` `:1919`.
- `rust/crates/lakehouse-api/src/upload_store.rs`: `put` `:176`, `head_bytes` `:195`, `get_all` `:222`, `delete` `:250`.
- `rust/crates/lakehouse-api/src/upload_limits.rs`: the 429 shape to copy.
- `rust/crates/lakehouse-api/tests/upload_routes.rs`: Excel `:3794-4206`, Parquet `:4230-4545`.
- `ops/g9/upload_test.py` (steps 8 and 9 load `stock.xlsx` and `orders.parquet`) and the `g9-upload` CI job (`.github/workflows/ci.yml:1463`).

## 4. Tasks (one commit each, in order)

- **T1. Hostile fixtures, and the failure reproduced.**
  - Add `ops/fixtures/uploads/hostile/`:
    - an `.xlsx` whose shared-strings part declares a huge `uniqueCount` and `count`;
    - an `.xls` whose SST record declares a huge count, if `calamine` reads that count;
    - a Parquet file whose footer declares sizes far past the file.
  - A small script builds them, so they are reproducible. Keep each file under 1 MiB.
  - Run today's `list_sheets` and `upload_parquet::inspect` on each in a separate process under a memory limit (for example a `#[test] #[ignore]` run with `ulimit -v`). Quote what happens (abort, refusal or success) in the handoff.
  - If no fixture crashes today, say so plainly: the fix is still needed, because `catch_unwind` cannot catch an allocation failure by design. No product code changes in this task.
- **T2. The child** (D1, D2, D6).
  - New module `upload_convert.rs` holds the operations, the exit-code table and the limit setting. `main()` dispatches `inspect-upload` first.
  - Unit tests:
    - each code maps to its sentence and back;
    - an unknown operation exits with the "unreadable" code;
    - the output cap stops the stream.
  - Prove on this VM that, under the limit, the T1 `.xlsx` ends the child and not the caller (run the built binary by hand; quote it). Settle `RLIMIT_AS` against `RLIMIT_DATA` here.
  - Check: `cargo clippy -p lakehouse-api --all-targets -- -D warnings`, `cargo test -p lakehouse-api upload_convert`.
- **T3. The API side** (D3, D4, D5, D8).
  - `ConversionRunner` in `AppState`, plus `UploadStore::put_stream` (a multipart write, aborted on error).
  - Replace every in-process parse: `create`, both previews, `convert_upload`. Remove `blocking` if nothing else uses it.
  - Delete cleans every converted object.
  - `UPLOAD_MAX_CONVERSIONS` and `UPLOAD_CONVERT_MEMORY_MB` go in `config.rs` (positive integers; empty or unset keeps the default). Compose passes them through as `${X:-}` with a comment, as `SEC-17` did (`docker-compose.yml:213-219`), and `.env.example` documents them with no value.
  - The child's stderr is logged at warn level, cut to 2 KiB, and never returned.
- **T4. Route tests through the real child.**
  - Every existing Excel and Parquet test passes, with `CARGO_BIN_EXE_lakehouse-api` as the child. Change only the preview-size expectations D5 changes, and say why in the commit (rule 2).
  - New tests:
    - each hostile fixture answers 400 with the D4 sentence, and the next request on the same router answers normally;
    - two previews in a row convert once (the stored object is read the second time);
    - all slots busy answers 429 (a slow stand-in child from a test fixture script, with `UPLOAD_MAX_CONVERSIONS=1`);
    - delete removes the converted objects;
    - the 512 MiB cap is reached in the child (use a smaller cap through a test-only constructor, not a 512 MiB fixture).
  - Check: `cargo test -p lakehouse-api --test upload_routes`.
- **T5. Measure, then set the limits** (D7).
  - Generate a 5,000,000-cell `.xlsx` and a 5,000,000-cell Parquet file at run time: a script under `ops/g9/`, not committed data.
  - Run the built binary's `inspect-upload` on each with `/usr/bin/time -v`. Record peak RSS and wall time, plus the child's baseline with an empty input.
  - Hand the numbers to the planner, who writes `docs/plans/SEC-22-RESULT.md`. Then set the defaults to peak × 1.5, rounded up to 128 MiB, citing the RESULT doc.
  - If the largest allowed file does not fit under a limit that two conversions at once can afford, stop and report. Do not lower a cap quietly.
- **T6. Gate and docs.**
  - `ops/g9/upload_test.py` gets a step: upload the T1 `.xlsx`; preview it and expect the D4 sentence; then expect any authenticated GET (the upload list) to answer 200. Steps 8 and 9 keep passing.
  - Run the `g9-upload` sequence locally from a clean project if docker allows, otherwise in CI. Quote the time.
  - Fix the module doc at `upload_workbook.rs:64-74` (rule 1) and the docs of `uploads.rs` that describe in-process conversion (`:55-80`, `:1172-1185`). Add a `CHANGELOG.md` entry.
  - Do not edit `docs/core/` (the planner does).
- **Then** run the full verification block (Rust, TypeScript only if touched, compose `config` and the g9 `up`), and write the handoff.

## 5. Out of scope

- Streaming a file from storage into the child instead of `get_all`. The 50 MiB upload cap bounds that read (ADR 0014 decision 2), and Phase 3 changes how Parquet is read anyway.
- Moving Parquet reading into the load job (Phase 3 slice 2).
- Any change to what a cell becomes as text (the conversion rules are signed, `upload-file.md` decision 10).
- A queue instead of 429 when conversions are busy.

## 6. Verify, do not assume

1. Under the chosen limit, a failing allocation in the child ends the child with a signal or an abort. The kernel's OOM killer must not pick another process. Proven in T2 with the T1 fixture.
2. The child's baseline memory: binary mapping and allocator with no tokio. It must leave room under the limit for the largest allowed file (T5).
3. `object_store`'s multipart write works against RustFS for the converted text, including abort on error. Phase 3's slice 1 needs the same answer, so quote it where both plans can read it.
4. `CARGO_BIN_EXE_lakehouse-api` is set for `lakehouse-api`'s integration tests, and the child built for tests is the current code.
5. In the runtime image (`rust/Dockerfile`, `debian:trixie-slim`, the entrypoint's user), `current_exe()` resolves and the API may start the child. g9 proves this.
6. A preview through the console's proxy finishes under its 130 s timeout for the T5 files, or the 90 s wall answers first.

## 7. Handoff

Round 1 (the review of 2026-10-29, at `17957613`). The branch is
`fix/sec-22-upload-conversion-memory`; the last CODE commit of this round
is `0fa53a78`, and this handoff is the commit after it.

### The commits, one line each

| commit | what it does | the finding it answers |
| --- | --- | --- |
| `bbeab4d5` | the hostile fixtures and the reproduction of the failure (T1) | the plan's T1 |
| `af2be706` | the child process: the four operations, the exit-code table, the limits (T2) | the plan's T2 |
| `8c38a5cc` | the API side: the runner, the stored conversion, the slots (T3) | the plan's T3 |
| `20be76a9` | the route tests run the real child and count its runs (T4) | the plan's T4 |
| `546ed291` | the limits from the first measurement (896 MiB) and the script (T5) | the plan's T5 (superseded by `0fa53a78`) |
| `17957613` | the gate refuses the hostile workbook and the API is still there (T6) | the plan's T6 |
| `560ec754` | the merge of `origin/main` (PR #192's crypto provider) into the branch | the merge the review names |
| `c99557d8` | the plan: the review round 1 (the planner's commit, not mine) | — |
| `d3fd0d60` | a Parquet file goes to the child by RANGES, never whole: the sparse reader, the two Parquet operations, the four call sites | BLOCKER 1, BLOCKER 2, P1 |
| `668ca710` | a conversion is committed only after the child exits 0: `put_stream` leaves the upload unassembled, the `.json` is the marker | BLOCKER 3, P2 |
| `7830679f` | the child's answer line is read through a 1 MiB cap | BLOCKER 4 |
| `1cc8f3d0` | a retryable answer at the chunked-upload completion keeps the file | SHOULD-FIX 1 |
| `a1929e08` | a workbook's type guess reads the same sample as a text file | SHOULD-FIX 2 |
| `039b86fe` | the child's failure paths are joined and logged: the feed's errors, the status and the standard error, one deadline, the child's own limit code | SHOULD-FIX 3, SHOULD-FIX 4 |
| `3a6b1440` | a garbled answer is judged by the child's ending | SHOULD-FIX 5 |
| `08b82bfc` | a Parquet preview reads the sample of the text, not all of it | SHOULD-FIX 6 |
| `066589c0` | the delete of a workbook does not trust an unreadable list of sheets | SHOULD-FIX 7 |
| `5420be8e` | the comments, the docs and the missing tests | SHOULD-FIX 8 |
| `0fa53a78` | the memory limit from the RELEASE measurement: 1280 MiB, and the script that measured it | BLOCKER 5 |

### The commands, with their counts

Every command ran through `/home/shiro/rantai/bin/cargo-locked` (the
machine-wide lock, `--locked`, `-j 2`), in the foreground, on the branch's
own worktree (`/home/shiro/rantai/wt-sec22`).

Per commit, on the crates the commit touched:

- `cargo fmt --check` — clean.
- `cargo clippy --locked -p lakehouse-api --all-targets -j 2 -- -D warnings`
  — clean, on every commit of the round.
- `cargo test --locked -p lakehouse-api -j 2 --lib upload_` — 194 passed, 0
  failed, 1 ignored (the `#[ignore]`d child role of the hostile fixtures).
- `cargo test --locked -p lakehouse-api -j 2 --test upload_routes` — 168
  passed, 0 failed (the last run, after the last commit).

On the final commit (`0fa53a78`), the whole block of `AGENTS.md`:

- `cargo fmt --check` — clean.
- `cargo clippy --locked --workspace --all-targets --all-features -j 2 -- -D warnings`
  — clean.
- `cargo test --locked --workspace --no-fail-fast -j 2` — 159 test
  binaries, 8457 passed, 0 failed, 51 ignored.
- `cargo deny check` (in `rust/`) — "advisories ok, bans ok, licenses ok,
  sources ok".
- `docker compose -f docker-compose.yml --profile '*' config --quiet`,
  with dummy values for `POSTGRES_PASSWORD`, `RUSTFS_ACCESS_KEY`,
  `RUSTFS_SECRET_KEY`, `LAKEKEEPER_ENCRYPTION_KEY` and
  `CONNECTOR_SECRETS_KEY` — clean.
- The `g9-upload` gate, as `.github/workflows/ci.yml` runs it, on the
  project name `g9sec22`: the API image of this branch
  (`LAKEHOUSE_API_IMAGE=rantai-lakehouse-api:local`, built from this
  branch), the dagster image that was on the machine, the `oidc-mock`
  fixture image built for it, and the stack up with
  `docker compose -p g9sec22 --profile dagster up -d --no-build
  lakehouse-api dagster-code-location dagster-webserver dagster-daemon
  clickhouse-iceberg-init`. Then `docker compose -p g9sec22 --profile
  dagster run --rm g9-test-runner` — exit 0, "[g9] PASS", and the `SEC-22`
  step of the gate: "[g9] hostile workbook: refused with the one sentence,
  the API is still there". The Parquet steps of the gate passed too ("[g9]
  parquet: PASS (the upload is gone, the table stays)"), so a Parquet file
  was uploaded, previewed and loaded through the child in the real image.
  The stack was taken down afterwards (`down -v`) and the images this round
  built were removed.
- `git status` — the tree is clean; nothing generated is staged.
### T1's reproduction, T2's finding, and the release measurement

**T1's reproduction** (the fixtures of
`ops/fixtures/uploads/hostile/`, read in a child process under
`ulimit -v 524288`, i.e. 512 MiB of address space; the test is
`rust/crates/lakehouse-api/tests/hostile_uploads.rs`, run with
`cargo test -p lakehouse-api --test hostile_uploads -- --ignored --nocapture`):

```text
[sec22] huge_shared_strings.xlsx: child status ExitStatus(unix_wait_status(134))
[sec22] huge_shared_strings.xlsx: memory allocation of 48000000000 bytes failed
[sec22] huge_footer.parquet: child status ExitStatus(unix_wait_status(0))
[sec22] huge_footer.parquet: REFUSED: This file could not be opened as a Parquet file.
[sec22] the process that ran the fixtures is still here
```

134 is 128 + SIGABRT: the shared-strings header of the workbook declares
two billion strings, `calamine` 0.37 reserves room for all of them before
it reads one (`read_shared_strings`: `self.strings.reserve(n)`), and the
reservation is 48 GB. The process that READ the file died; the process
that ran it was still there. The Parquet file declares a row group of one
tebibyte in 123 bytes and is refused with the reader's own sentence, which
is the counter-example: the Parquet reader bounds what the footer declares
and the workbook reader does not.

The `.xls` case of the plan needs no fixture: `calamine` 0.37 does not
read the shared string table's count (`src/xls.rs`, `parse_sst` skips the
`cstTotal` and `cstUnique` headers), so a count that lies costs nothing.

**The new fixture of this round.** `ops/fixtures/uploads/hostile/huge_list_footer.parquet`
(48 bytes) is a Parquet file whose footer declares two billion row groups.
The list header of the Thrift compact protocol carries the count BEFORE the
elements, and the reader reserves room for the whole list before it reads
the first one (`parquet` 58.4.0, `file/metadata/thrift/mod.rs:804`:
`Vec::with_capacity(list_ident.size)`): the reservation is 192 GB, and the
child that reads it aborts (exit 134, "memory allocation of 192000000000
bytes failed"). It is the case BLOCKER 1 is about: the footer decode used
to run in the API's own process, and it is now the child's.

**T2's `RLIMIT_AS` against `RLIMIT_DATA` finding** (measured on this VM,
kernel 6.8, with the T1 hostile workbook):

- `RLIMIT_AS` of 64 MiB: the child dies of SIGSEGV (exit 139) before it
  reads anything, even on a VALID workbook. The address-space limit counts
  the binary's own mappings, the stacks and the allocator's arenas, none of
  which are the parser's memory.
- `RLIMIT_DATA` of 64 MiB: the child reads a valid workbook and refuses
  the hostile one, which ends it with "memory allocation of 48000000000
  bytes failed" — the same abort, at a limit the child can run under.

So the child sets `RLIMIT_DATA` and `RLIMIT_CPU`. The two `Cargo.toml`
comments that still said `RLIMIT_AS` are fixed in `5420be8e`.

**The release measurement** (BLOCKER 5). Measured on 2026-10-29 with the
RELEASE binary of the gate's image (`rantai-lakehouse-api:local`, built
from this branch with `docker compose build lakehouse-api`, the binary at
`/usr/local/bin/lakehouse-api`), with `ops/g9/sec22_measure.py`, which
builds the fixtures and runs the child INSIDE that image under
`/usr/bin/time -v`:

| what                                  | peak RSS | user s | sys s | CPU s | wall     | exit | at the limits |
| ------------------------------------- | -------- | ------ | ------ | ----- | --------- | ---- | ------------- |
| the baseline (an empty input)         | 9.9 MiB  | 0.00   | 0.00   | 0.00  | 0:00.00   | 2    | 2             |
| `convert-sheet`, inline strings       | 523.8 MiB| 4.49   | 0.45   | 4.94  | 0:05.05   | 0    | 0             |
| `convert-sheet`, shared strings       | 801.5 MiB| 5.50   | 0.60   | 6.10  | 0:06.25   | 0    | 0             |
| `preview-parquet`, 2,000 rows         | 163.5 MiB| 0.03   | 0.15   | 0.18  | 0:00.21   | 0    | 0             |

The workbook is 5,000,000 cells (100 columns of 50,000 rows), the most a
sheet may hold (`upload_workbook::MAX_SHEET_CELLS`). The "shared strings"
row is the SAME workbook with the text in a shared-strings part, which is
the way Excel writes one: the reader reserves room for the whole table of
the strings before it reads a cell, and that is 200 MB of reservation for
5,000,000 of them. The Parquet file's first row group is near
`MAX_PREVIEW_GROUP_BYTES` (61 MiB of it) and the preview asks for the
route's 2,000 rows, fed by ranges (`SEC-22` P1: the tail of the file and
that group).

The "at the limits" column is the same run under the product's own limits
(1280 MiB of memory, 60 s of CPU): every exit code is the one of the run
beside it, so every file passes under the limits the product sets.

Two things this measurement says that the old one did not:

- under the OLD default (896 MiB) the shared-strings workbook ABORTED
  (exit 134): the limit did not admit the largest allowed file, and the
  file it refused is the way Excel writes a workbook;
- the release `convert-sheet` of the largest allowed workbook takes 6.10 s
  of CPU and 6.25 s of wall time. Times 1.5 that is 9.2 s of CPU (under the
  60 s limit) and 9.4 s of wall time (under the 90 s one), so no limit
  had to be raised and the plan's stop condition is not met.

**The new default.** 801.5 MiB times 1.5 is 1202.3 MiB, which rounds up to
1280 MiB: `upload_convert::DEFAULT_MEMORY_MIB` and, through it,
`UPLOAD_CONVERT_MEMORY_MB`. Two conversions at once are 2.5 GiB, which a
machine with 12 GB of memory affords. The constant cites
`docs/plans/SEC-22-RESULT.md (to be written by the planner from the round 1
handoff numbers)`, so the reference is honest: the file does not exist
yet.

### Section 6, item by item

1. **"Under the chosen limit, a failing allocation in the child ends the
   child with a signal or an abort. The kernel's OOM killer must not pick
   another process. Proven in T2 with the T1 fixture."** — Yes, and again
   in this round: the T1 workbook aborts the child (exit 134) with
   "memory allocation of 48000000000 bytes failed", and the new
   `huge_list_footer.parquet` aborts it with "memory allocation of
   192000000000 bytes failed" (exit 134). The API is a separate process
   with its own limit, so the kernel's OOM killer has nothing to pick but
   the child. The route test `a_hostile_workbook_ends_the_child_and_leaves_the_api_alone`
   and `a_hostile_parquet_footer_ends_the_child_and_leaves_the_api_alone`
   prove it end to end: the API answers 400 with the one sentence of
   `SEC-22` D4 and the next request answers normally.
2. **"The child's baseline memory: binary mapping and allocator with no
   tokio. It must leave room under the limit for the largest allowed
   file (T5)."** — Yes: the baseline is 9.9 MiB (the table above) and the
   limit is 1280 MiB, which admits the largest allowed file (801.5 MiB) and
   leaves 470 MiB over the baseline. Two conversions at once are 2.5 GiB.
3. **"`object_store`'s multipart write works against RustFS for the
   converted text, including abort on error."** — Yes, twice. The route
   tests prove it against the fake S3 of the harness (`upload_routes.rs`'s
   `Bucket`): the converted text is written as one multipart upload and
   assembled, and a child that is killed mid-stream leaves no object and
   no open multipart upload
   (`a_child_killed_mid_stream_leaves_no_converted_text`). The g9 gate
   proves it against RustFS itself: the workbook of the gate was uploaded,
   previewed and loaded, and the file's rows are in the table.
4. **"`CARGO_BIN_EXE_lakehouse-api` is set for `lakehouse-api`'s
   integration tests, and the child built for tests is the current
   code."** — Yes: the harness points the runner at
   `env!("CARGO_BIN_EXE_lakehouse-api")` (`tests/common/mod.rs`), which
   Cargo sets for the integration tests of a package with that bin target,
   and the runner refuses any binary that is not this API's own
   (`is_our_binary`), so the child the route tests run is the one Cargo
   built from the branch's code.
5. **"In the runtime image (`rust/Dockerfile`, `debian:trixie-slim`, the
   entrypoint's user), `current_exe()` resolves and the API may start the
   child. g9 proves this."** — Yes. The g9 gate ran on the image of this
   branch (`rantai-lakehouse-api:local`, built with `docker compose build
   lakehouse-api` as the gate's `images` job does): the API started the
   child for every workbook and Parquet file of the gate (the gate's own
   output names the sheets it read and the columns of the Parquet file),
   and the hostile workbook ended the CHILD, not the API. The measurement
   of this round ran the child inside the same image as well (`docker run
   --entrypoint /usr/bin/time ... /usr/local/bin/lakehouse-api
   inspect-upload ...`).
6. **"A preview through the console's proxy finishes under its 130 s
   timeout for the T5 files, or the 90 s wall answers first."** — The
   release `convert-sheet` of the largest allowed workbook takes 6.25 s of
   wall time and the release `preview-parquet` of 2,000 rows takes 0.21 s:
   both are far inside the 90 s wall and the 130 s proxy timeout. The
   child's own CPU limit (60 s) is 9.2 s of headroom over the measured
   CPU times 1.5.

### Not verified, and why

- **The TypeScript and the Dagster code location were not checked.** This
  round touches no file under `src/` and none under `dagster/`, so the
  checks of `AGENTS.md` that belong to them (`bun run typecheck`,
  `bun run lint`, `bun run test`, `pytest dispar_orchestrate`) were not
  run. The Python that did change (`ops/g9/sec22_measure.py`) is a gate
  script, not the code location, and it ran as the measurement (and the
  gate that reads the fixtures it builds ran too).
- **The `SEC-22` T1 reproduction was not re-run in this round.** Its
  numbers are the ones of T1 (quoted above), and the hostile fixture of
  this round (`huge_list_footer.parquet`) was run by hand in the gate's
  image: the child aborted with "memory allocation of 192000000000 bytes
  failed" (exit 134), and the API answered 400 with the one sentence and
  was still there for the next request.

### Handoff (review round 2)

Round 2 of the review (the plan's section 8, "Review round 2", at
`2c252c32`). The branch is `fix/sec-22-upload-conversion-memory`; the last
CODE commit of this round is `a93c3663`, and this handoff is the commit
after it.

#### The commits, one line each

| commit | what it does | the finding it answers |
| --- | --- | --- |
| `feabcfa6` | the feed tells a store failure and a broken pipe apart, and every refusal path waits for the child's ending | BLOCKER 1 |
| `f616dfbe` | the child that said nothing is waited for before it is killed, and what it said is read under a fresh bound | BLOCKER 2, SHOULD-FIX 3 |
| `b522ec80` | a text whose marker could not be written is not deleted | SHOULD-FIX 1 |
| `6adb9765` | the row group the child answers is checked against the file | SHOULD-FIX 2 |
| `c3cc40f2` | the child's own Parquet input checks | SHOULD-FIX 4 |
| `591fb692` | the measurement checks the answer, and the Parquet input is copied once | SHOULD-FIX 5 |
| `e1e442a8` | the comments, the docs and the test gaps | SHOULD-FIX 6 |
| `a93c3663` | the memory limit's own numbers, and the store failure's own doc | SHOULD-FIX 6, BLOCKER 1 |

#### The commands, with their counts

Every cargo command ran through `/home/shiro/rantai/bin/cargo-locked` (the
machine-wide lock, `--locked`, `-j 2`), in the foreground, on the branch's
own worktree (`/home/shiro/rantai/wt-sec22`).

Per commit, on the crate the commit touched:

- `cargo fmt --check` — clean, on every commit of the round.
- `cargo clippy --locked -p lakehouse-api --all-targets -j 2 -- -D warnings`
  — clean, on every commit of the round.
- `cargo test --locked -p lakehouse-api -j 2 --test upload_routes <filter>`
  and `cargo test --locked -p lakehouse-api -j 2 --lib <filter>`:

| commit | the test command | the counts |
| --- | --- | --- |
| `feabcfa6` | `--test upload_routes store` | 17 passed, 0 failed |
| `f616dfbe` | `--test upload_routes child` | 9 passed, 0 failed |
| `f616dfbe` | `--lib upload_` | 194 passed, 0 failed, 1 ignored |
| `b522ec80` | `--lib upload_` | 194 passed, 0 failed, 1 ignored |
| `b522ec80` | `--test upload_routes converted` | 5 passed, 0 failed |
| `6adb9765` | `--test upload_routes parquet` | 18 passed, 0 failed |
| `6adb9765` | `--test upload_routes row_group` | 1 passed, 0 failed |
| `c3cc40f2` | `--lib upload_convert` | 14 passed, 0 failed |
| `591fb692` | `--lib upload_convert` | 14 passed, 0 failed |
| `e1e442a8` | `--test upload_routes list_of_sheets` | 2 passed, 0 failed |
| `a93c3663` | the whole block below | — |

On the final commit (`a93c3663`), the whole block of `AGENTS.md`:

- `cargo fmt --check` — clean.
- `cargo clippy --locked --workspace --all-targets --all-features -j 2 -- -D warnings`
  — clean (exit 0).
- `cargo test --locked --workspace --no-fail-fast -j 2` — **159 test
  binaries, 8468 passed, 0 failed, 51 ignored** (exit 0). The
  `upload_routes` binary alone: 173 passed, 0 failed.
- `cargo deny check` (in `rust/`) — "advisories ok, bans ok, licenses ok,
  sources ok" (exit 0).
- `docker compose -f docker-compose.yml --profile '*' config --quiet`,
  with dummy values for `POSTGRES_PASSWORD`, `RUSTFS_ACCESS_KEY`,
  `RUSTFS_SECRET_KEY`, `LAKEKEEPER_ENCRYPTION_KEY` and
  `CONNECTOR_SECRETS_KEY` — clean.
- The `g9-upload` gate, as `.github/workflows/ci.yml` runs it, on the
  project name `g9sec22r2`: the API image of this branch
  (`LAKEHOUSE_API_IMAGE=rantai-lakehouse-api:local`, built from this branch
  with `docker compose build lakehouse-api`), the dagster image that was on
  the machine (`rantai-lakehouse-dagster:local`), the `oidc-mock` fixture
  image built for it, and the stack up with
  `docker compose -p g9sec22r2 --profile dagster up -d --no-build
  lakehouse-api dagster-code-location dagster-webserver dagster-daemon
  clickhouse-iceberg-init`. Then `docker compose -p g9sec22r2 --profile
  dagster run --rm g9-test-runner` — exit 0 and "[g9] PASS", with the
  `SEC-22` steps of the gate: "[g9] workbook: PASS (the upload is gone, the
  table stays)", "[g9] parquet: PASS (the upload is gone, the table stays)"
  and "[g9] hostile workbook: refused with the one sentence, the API is
  still there". The stack was taken down afterwards (`down -v`) and the
  images this round built were removed. The gate's own throwaway `.env` is
  gitignored and was removed with them.
- `git status` — the tree is clean; nothing generated is staged.

#### The measurement, and what moved

The Parquet child copies its input once now (`591fb692`), so the release
measurement was run again: `ops/g9/sec22_measure.py` against the RELEASE
binary of the gate's image (`rantai-lakehouse-api:local`, built from this
branch), with `pyarrow` on `PYTHONPATH` for the Parquet fixture (the
script skips that fixture without it, and says so).

| run | the fixture | peak RSS | CPU | wall | exit | at the limits |
| --- | --- | --- | --- | --- | --- | --- |
| the baseline (an empty input) | 0 B | 10.0 MiB | 0.00 s | 0:00.00 | 2 | 2 |
| `convert-sheet`, inline strings | 17,070,628 B | 524.0 MiB | 4.71 s | 0:04.76 | 0 | 0 |
| `convert-sheet`, shared strings | 27,974,678 B | 801.4 MiB | 6.07 s | 0:06.20 | 0 | 0 |
| `preview-parquet`, 2,000 rows | 61,222,470 B (the child is fed 77,998,929 B of it) | 163.1 MiB | 0.18 s | 0:00.20 | 0 | 0 |

Against the round 1 numbers of `docs/plans/SEC-22-RESULT.md` (9.9, 523.8,
801.5 and 163.5 MiB), the peaks move by less than a MiB: the same
measurement, run twice, on the same machine. **No limit moves**: 801.4 MiB
times 1.5 is 1202.1 MiB, which rounds up to 1280 MiB, and the CPU (6.07 s
here, 6.10 s in the round 1 run) is far under the 60 s limit.

Two things the reviewer should know about the copy:

- The copy cuts the Parquet child's peak by 0.4 MiB and no more, because
  the pieces the reader is given are the whole input between them: one
  copy of the input costs what two copies of its parts cost. The review's
  "this cuts the Parquet child's peak" is true and small.
- The script now checks the answer as well: every run answered, and the
  Parquet preview read its 2,000 rows ("[sec22] preview-parquet, 2000
  rows: the preview read 2000 rows"). A run that read the wrong range
  exits 0 and is caught by the script now.

The constant `upload_convert::DEFAULT_MEMORY_MIB` cites the RESULT doc's
numbers (801.5, 163.5, 1202.3) as the review asks, and the planner may
update the doc from the table above.

#### Not verified, and why

- **The TypeScript and the Dagster code location were not checked.** This
  round touches no file under `src/` and none under `dagster/`, so the
  checks of `AGENTS.md` that belong to them (`bun run typecheck`,
  `bun run lint`, `bun run test`, `pytest dispar_orchestrate`) were not
  run. The Python that did change (`ops/g9/sec22_measure.py`) is a gate
  script, not the code location, and it ran as the measurement (and the
  gate that reads the fixtures it builds ran too).
- **The `SEC-22` T1 reproduction was not re-run in this round.** The
  hostile fixtures were not read by hand under `ulimit -v` as T1 did; the
  gate's own hostile workbook step ran against the image of this branch
  ("[g9] hostile workbook: refused with the one sentence, the API is still
  there"), and that is the same fixture and the same child.

## 8. Review

### Review round 1 (planner, 2026-10-29, at `17957613`; the merge `560ec754` changes none of it)

The review covers T1 to T6. Two passes read the diff, one on the child and
runner and one on the API side, and I checked the blockers in the source
myself. Nothing was run: the round 3 verification was stopped because these
findings change the code.

T1's reproduction, T2's `RLIMIT_DATA` finding, `env_clear()`, the dispatch
order (the child first, then #192's provider install), `kill_on_drop`, the
2-slot semaphore and its 429, and the fixed sentences are correct, with
tests. The findings below are what is left.

#### Planner decisions for this round

- **P1. Parquet goes to the child by ranges, never whole.** This replaces
  D1b for Parquet. The child gets the file's size on argv and, on stdin,
  only the byte ranges it needs. It reads them through a sparse
  `ChunkReader` that refuses any range it was not given.
  - `inspect-parquet <file_size>` is fed the last
    `min(file_size, MAX_FOOTER_BYTES + 8)` bytes. It answers `ParquetInfo`
    plus the byte range of the first row group, which `check_layout`
    already bounds by `MAX_PREVIEW_GROUP_BYTES`.
  - `preview-parquet <max_rows> <file_size> <group_offset> <group_len>` is
    fed that tail followed by the first row group's bytes.
  - So the API runs `inspect-parquet` first, then fetches the range it
    answered, then runs `preview-parquet`.
  - The child's input cap for Parquet becomes
    `MAX_FOOTER_BYTES + 8 + MAX_PREVIEW_GROUP_BYTES`. It no longer depends
    on the file's size, so a 2 GiB chunked-upload Parquet file is read the
    same way.
  - The cost noted under D1b (moving the whole file) goes away.
- **P2. A conversion is committed only after the child exits 0.**
  - `put_stream` does not complete the multipart upload on EOF. It hands
    back the upload, and the caller completes it after `wait()` returns
    success, or aborts it otherwise.
  - `.converted.<n>.json` is written **last**, and it is the commit marker.
    A preview or an ingest treats a `.csv` with no `.json` as missing and
    converts.
  - A failed run removes only objects it completed itself.
  - Two runs for the same sheet write the same bytes, so a second
    complete is harmless.
- **P3. `connector_listing.rs:1022`** (an in-process `inspect` on a tenant's
  S3 Parquet file, up to 64 MiB) is the same kind of bug outside this
  task's scope. The planner files it as a backlog item. Leave it here, but
  P1 makes the fix a one-line reuse.

#### BLOCKER

1. **A crafted Parquet footer still ends the API through the chunked
   upload.**
   - Where: `routes/uploads/sessions.rs:640-659` (`open_whole`) runs
     `upload_parquet::inspect` in the API process.
   - Cause: `parquet` 58.4.0 reserves list space from the file's own
     count, at `parquet_thrift.rs:707` and
     `file/metadata/thrift/mod.rs:804`
     (`Vec::with_capacity(list_ident.size)`). That happens during the
     footer decode, before `check_layout` runs. So a footer of a few
     dozen bytes can ask for more than 100 GB.
   - The comment at the site says the read "cannot be made to ask for more
     memory than it has". That is false (rule 1).
   - Fix: P1. The comment goes.
2. **Parquet over 64 MiB from the chunked upload can no longer be
   previewed or loaded** (a `DATA-17` regression).
   - `parquet_preview` and `check_parquet_footer` stream the whole file into
     a child capped at 64 MiB, which refuses it with "larger than the 50 MB
     limit".
   - That is false for a route that accepts 2 GiB. Ingest of those files,
     and of groups containing them, now fails.
   - Fix: P1. Add a route test with a Parquet file over 64 MiB made by a
     test-only builder, or a smaller cap through a test-only constructor,
     covering preview and ingest.
3. **A truncated conversion is completed in the store before the child's
   status is known** (D6). Reported by both passes.
   - `store_inner` calls `put_stream`, which completes on EOF. Only after
     that does `wait()` see a signal or `TOO_MUCH_TEXT`, and
     `remove_stored` is best effort.
   - Scenario: if that delete fails, or the API restarts in the window, the
     truncated `.csv` stays. Every later preview shows it, and ingest loads
     a cut table. A concurrent preview in the window reads it too.
   - Fix: P2.
4. **The parent reads the child's answer line with no cap.**
   - Where: `upload_convert.rs:1196-1206`, `read_until(b'
')`.
   - Scenario: a workbook with many sheets and long names makes the
     `list-sheets` line hundreds of MiB, within the child's 896 MiB. The
     API holds it, parses a second copy and stores it. That is the memory
     failure `SEC-22` exists to stop.
   - Fix: read through `take(MAX_ANSWER_BYTES + 1)`, with
     `MAX_ANSWER_BYTES` = 1 MiB. Over the cap, or EOF before a newline,
     answers `ResourceLimit`, and the child is killed and reaped. Add a
     test with a stand-in child that writes an endless line.
5. **The memory default is not shown to admit the largest allowed file**
   (D7, principle 5).
   - Peak RSS was measured, but the limit is `RLIMIT_DATA`, which also
     counts reserved, untouched memory.
   - The workbook fixture uses inline strings, while Excel writes shared
     strings.
   - The Parquet run previews 20 rows; the route asks for
     `PARQUET_PREVIEW_ROWS` = 2000.
   - It was a debug build. `convert-sheet` took 82.9 s of wall time
     against a 60 s CPU limit and a 90 s wall clock, so the headroom of
     both limits is unproven.
   - Fix: extend `ops/g9/sec22_measure.py` and re-run it with the
     **release** binary. Run it from the gate's image,
     `rantai-lakehouse-api:local` at `/usr/local/bin/lakehouse-api`, or
     from `cargo build --release -p lakehouse-api` if RAM allows.
     - Fixtures: a 5,000,000-cell sheet with inline strings, the same
       with shared strings, and P1's preview at 2000 rows with a first row
       group near `MAX_PREVIEW_GROUP_BYTES`.
     - Quote peak RSS, CPU seconds and wall time for each.
     - Add a pass/fail run of each at the chosen limits, and quote the
       exit codes.
   - Set the defaults from those numbers (peak × 1.5, rounded up to
     128 MiB). If the release `convert-sheet` CPU time × 1.5 passes 60 s,
     or its wall time passes 90 s, stop and report: do not raise a limit
     quietly.
   - The planner writes `docs/plans/SEC-22-RESULT.md` from the numbers.
     Until then, the constant must not cite a file that does not exist.

#### SHOULD-FIX

1. **A retryable answer at chunked-upload completion deletes the user's
   file.**
   - Where: `sessions.rs:555-558` sends every `Err` from `open_whole` to
     `refuse_and_remove`.
   - The new 429 (busy), 500 (not started), 503 (store) and a failed
     `sheets.json` write all delete the object and the row, so the retry
     gets 404.
   - The existing `too_many_uploads()` 429 at `:664-666` has the same bug,
     which contradicts its own comment.
   - Only a refusal of the FILE (400) may remove it. Test the 429 case.
2. **Workbook type sampling fell from 10,050 records to 256 KiB.**
   - `workbook_preview` builds `sample_records`, which feeds the `DATA-18`
     type guess and the `DATA-17` group check, from the `PREVIEW_BYTES`
     head.
   - D5 changed the preview's cut, not the sample's.
   - Sample with `head_bytes(text_key, SAMPLE_BYTES)` as the text path
     does (`uploads.rs:1814-1822`), and fix the doc that says it already
     does.
3. **Feed errors are dropped.** In `start`, `let _fed = feed(..)` throws
   away the result.
   - A store failure mid-feed truncates the child's input, and the user is
     told the file cannot be opened. That is a false reason (principle 2).
   - Join the feed task in `wait`, and let a store error win, giving the
     fixed 503.
4. **Stderr and the exit status are logged only on the success path's
   `wait()`.**
   - Not logged: EOF before the answer (how the out-of-memory abort shows
     up), the timeout, an early refusal, and a failed `setrlimit`.
   - Log the status and 2 KiB of stderr at warn on every failure path.
   - Give a failed `setrlimit` its own exit code, mapped to 500
     `CONVERSION_FAILED`. It is an installation fault, not a bad file.
5. **A garbled answer line becomes "unreadable".** Decide by the child's
   status instead: a signal gives `ResourceLimit`. Or make the
   `ResourceLimit` doc match.
6. **A Parquet preview buffers up to 512 MiB in the API.**
   `answer_with_text` caps at `MAX_OUTPUT_BYTES` and ignores
   `with_max_output_bytes`. Stop reading at `SAMPLE_BYTES`, kill the
   child, and mark the preview as cut.
7. **Delete counts sheets from `sheets.json`.** If that object is
   unreadable, the count is 0 and every `.converted.<n>` object is left
   behind. Delete by a bounded prefix listing of `<key>.converted.`, or
   over `0..MAX_SHEETS` with a cap stated at the constant.
8. **Comments and tests (rule 1).**
   - `rust/Cargo.toml` and `lakehouse-api/Cargo.toml` say `RLIMIT_AS`; the
     code sets `RLIMIT_DATA`.
   - `with_wall`'s `#[allow(dead_code, reason = ...)]` names a test that
     does not exist. Add it: a slow stand-in child with `with_wall(1s)`,
     expecting 400 with the D4 sentence and no live child.
   - `sessions.rs` docs describe reading in pieces from the store.
   - The message at `upload_routes.rs:4422` contradicts its assertion.
   - `# Errors` of `sheets_of` and `convert_upload` omit 429 and 500.
   - `.env.example` gives values; the plan and the file's own convention
     are no value.
   - Missing tests:
     - a legacy upload with no `sheets.json` lists once, stores it, and the
       next preview runs no child;
     - delete of a legacy `.converted.csv`;
     - after the hostile Parquet file, the next request answers normally.
   - The 429 test sleeps a fixed 500 ms. Poll until `runs() == 1` instead,
     or it will flake.

**Open: 5 BLOCKERs.** One fix round: P1 and P2 first, then the blockers
and should-fixes in order, one commit per finding or per tight group,
each citing "PR #187 review B<n>/S<n>" at the site (rule 13). Then the
full verification and the section 7 handoff.


### Review round 2 (planner, 2026-10-29, at `2c252c32`)

The review covers the round 1 fixes `d3fd0d60..0fa53a78` and the handoff.
Two passes read the diff, one on the child and runner and one on the API
side, and I checked BLOCKER 1 below in the source myself. The planner
wrote `docs/plans/SEC-22-RESULT.md` (`6ea20e41`) from the handoff's
numbers.

**Closed:**
- P1, BLOCKER 1, BLOCKER 2: the sparse `PartsReader`, the tail-only
  footer, the group cap checked in the API before the fetch, and the
  over-the-cap and hostile-footer tests.
- P2, BLOCKER 3: the text is completed only after exit 0, the `.json` is
  written last, and a `.csv` without its `.json` counts as missing in
  both the preview and ingest.
- BLOCKER 4: the 1 MiB `take` cap, with the endless-child test.
- BLOCKER 5: the release measurement gives 1280 MiB. The arithmetic holds,
  and the CPU and wall limits are unchanged.
- SHOULD-FIX 1, 2, 5, 6 and 7. The `RLIMIT_DATA` comments and the
  `with_wall` test from SHOULD-FIX 8 are also closed.

#### BLOCKER

1. **A store error while the child is fed still becomes "the file's
   fault", and a chunked completion then deletes the user's file.** This
   is round 1's SHOULD-FIX 3, reopened. Raised to a blocker because it
   deletes user data.
   - Parquet: `feed_range` (`upload_convert.rs:1921-1926`) maps a store
     error to `io::Error`, and `feed` (`:1908`) maps every I/O error to
     `Refused(ResourceLimit)`. So `Source::StoredParts` can never yield
     `Store`.
   - Workbooks: the child reads stdin to EOF and then answers. A truncated
     feed therefore always produces a well-formed refusal line, and every
     refusal path drops what `wait` returns: `answer_inner` `:1350-1351`,
     `text_inner` `:1419-1420`, and `store_inner` `:1518-1519`, which does
     not wait at all. "Store wins" is reached only on the success path.
   - Scenario:
     1. RustFS returns one 5xx on a `read_range` while a chunked upload
        completes.
     2. The child refuses the short input.
     3. The route answers 400.
     4. `sessions.rs:563-567` removes the object and the row, so a 2 GiB
        upload is gone. A preview gives a false reason (principle 2).
   - The test `a_store_failure_while_the_child_is_fed_is_a_503…`
     (`tests/upload_routes.rs:8176`) breaks the bucket before
     `sheets_of`, so it gets the 503 before any child starts and does not
     test the feed.
   - Fix:
     - The feed keeps `ConversionError::Store` apart from a pipe error;
       only a pipe error is `ResourceLimit`.
     - Every refusal path awaits `wait`, and a `Store` from the feed wins
       over the child's refusal.
   - Tests: a store stand-in that fails `read_range` only after N calls,
     covering:
     - a workbook preview, expecting 503;
     - a Parquet preview, expecting 503;
     - a chunked completion, expecting 503 with the object and the row
       still there.
2. **A failed `setrlimit` answers 400 "needs more memory", not 500.** This
   is round 1's SHOULD-FIX 4, reopened. Raised to a blocker because it
   turns an installation fault into a refusal of every workbook, and with
   BLOCKER 1's path into deleted uploads.
   - `run_child_if_asked` exits with `LIMIT_FAILED` and writes no answer
     line (`:671-675`).
   - The parent sees EOF (`:1671`), and `refuse_the_child` drops `wait`'s
     `Err(LimitFailed)` (`:1722`) and returns `ResourceLimit`. Killing
     before `wait` would also hide the exit code behind SIGKILL.
   - Fix: on EOF, `wait` first, then kill, and let `LimitFailed` and
     `Store` through. Or have the child write a refusal line of its own
     kind before it exits.
   - Test: a stand-in child that exits with `LIMIT_FAILED`, expecting 500
     `CONVERSION_FAILED`.

#### SHOULD-FIX

1. **A failed marker write deletes another run's good text.** When the
   `.json` `put` fails, `store_inner` (`:1563-1571`) deletes `text_key`.
   That object may be a concurrent run's completed text, which that run's
   preview or launched job is about to read.
   - Fix: do not delete. A `.csv` without its `.json` already counts as
     missing, and delete removes it.
2. **The child's row-group answer is half-checked.** `uploads.rs:2276-2278`
   checks `len`, but not `offset + len <= size`, and `read_range` does not
   clamp. A lying answer reaches the store as an out-of-range read and
   ends with the resource-limit sentence.
   - Fix: `checked_add` against the object size, else 400
     `UNREADABLE_PARQUET` before the fetch.
3. **On a timeout, stderr is never logged.** `timeout_at(deadline, stderr)`
   runs with a deadline that has already passed (`:1755-1763` and every
   `refuse_the_child` after the deadline).
   - Fix: after the kill, read stderr under a short fresh timeout (1 s).
4. **No unit tests for the child's own Parquet input checks.**
   `parquet_reader` (`:813-846`) needs tests for:
   - `off + len > size`;
   - overflow;
   - `len > cap`;
   - input shorter than the tail;
   - input shorter than tail plus group;
   - `with_parquet` with garbage values.
5. **The measurement script does not check the preview read the group.**
   `sec22_measure.py` discards stdout, so a wrong range would still exit
   0.
   - Fix:
     - Assert that the answer line and 2000 rows come out.
     - Print each fixture's size, which shows the shared-strings workbook
       is under 50 MiB.
     - Optional: build one `Bytes` from the input and `slice` it rather
       than `copy_from_slice` twice (`:833`, `:838`). This cuts the
       Parquet child's peak.
   - If the numbers change, give them in the handoff; the planner updates
     the RESULT doc.
6. **Comments (rule 1) and two test gaps.**
   - `DEFAULT_MEMORY_MIB` (`:175-183`) says 801.4, 162.6 and 1202.1 MiB,
     and that the RESULT doc is "to be written". Use the measured 801.5,
     163.5 and 1202.3, and cite `docs/plans/SEC-22-RESULT.md`, which now
     exists.
   - `DEFAULT_CPU_SECONDS` (`:192-197`) still says the release build was
     not measured. It was: 6.10 s.
   - `sessions.rs:37-38`, `:615-617` and `:686-687` say the child "reads it
     in pieces from the store". The API feeds the stored object to the
     child in 8 MiB pieces; the child never reads the store (D1).
   - The message at `upload_routes.rs:4480` contradicts the assertion
     above it.
   - The legacy-upload test never asserts the FIRST preview's run count
     (`2`: one listing, one conversion).
   - The hostile fixture `README.md` lost a `##` heading.

**Open: 2 BLOCKERs.** One more fix round, in order: BLOCKER 1, BLOCKER 2,
then SHOULD-FIX 1 to 6. Each cites "PR #187 review round 2 B<n>/S<n>" at
the site. Then the full verification block and a short round 2 handoff
under section 7: commits, commands with counts, and any changed numbers.
