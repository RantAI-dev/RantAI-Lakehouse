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

(Developer: commits, the exact commands with their counts, and what was not
verified, with the reason.)

## 8. Review

(Planner: findings tagged `BLOCKER` or `SHOULD-FIX`.)
