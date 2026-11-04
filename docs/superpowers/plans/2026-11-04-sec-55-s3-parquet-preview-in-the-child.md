# SEC-55: an S3 connector's Parquet preview is read by the child, from the file's tail

Backlog `SEC-55` (P1, Administration & Security). Planner: Claude Opus, 2026-11-04.
Anchors verified at `main` `6e5be45e`. Base branch: this one,
`fix/sec-55-s3-parquet-preview-in-the-child`.

## 0. What is wrong, with evidence

- `connector_listing.rs:1000` `preview_s3` previews a Parquet object in an S3 files
  connector by fetching the WHOLE object (`get_range(&path, 0..size)`, `:1020`, up to
  `PARQUET_MAX_BYTES` = 64 MiB, `:88`) and decoding its footer IN THE API PROCESS with
  `upload_parquet::inspect(&bytes)` (`:1022`).
- `SEC-22` showed what that does: `parquet` reserves list space from counts the footer
  declares before any check runs, so a crafted footer (`ops/fixtures/uploads/hostile/huge_list_footer.parquet`)
  aborts the process that reads it. For an upload that process is the child
  (`routes/uploads.rs:824` `parquet_footer`, `upload_convert.rs:765`
  `Operation::InspectParquet`). For an S3 preview it is the API, for every tenant.
- Who can trigger it: anyone allowed to preview a files connector, against any bucket the
  connector reads, including one an outsider can write to.

## 1. Decisions

- **D1. The child reads it, as uploads do (`SEC-22` P1).** `preview_s3`'s Parquet branch
  fetches only the file's tail, `state.conversion_runner.parquet_tail_bytes()` bytes (or the
  whole file when it is shorter), with one `get_range(size - tail..size)`, and asks the
  runner `answer(Operation::InspectParquet { size }, source)` where the source holds just
  those bytes (`Source::Parts { bytes: tail, parts: vec![Part covering 0..tail.len()] }`, or
  `Source::Bytes(tail)` if that is what `InspectParquet` expects; read `parquet_reader` in
  `upload_convert.rs` and use whichever the upload path uses for a tail it already holds).
  The API never calls `upload_parquet::inspect` / `inspect_parts` on connector bytes again.
- **D2. The 64 MiB refusal goes.** It existed because the whole file was read. Only the tail
  moves now, so a Parquet object of any size previews, as an upload of up to 2 GiB does.
  Delete `PARQUET_MAX_BYTES` and its doc comment; keep `size == 0` → `Failed`.
- **D3. One reader of the child's answer (rule 4).** The upload path turns the child's
  answer into columns with `parquet_of_answer` (`routes/uploads.rs`). Do not write a second
  one: move it (or the part of it that yields `ParquetInfo`) to `upload_convert.rs` or
  `upload_parquet.rs` as `pub(crate)` and call it from both places.
- **D4. Failures keep the listing's fixed words (`SRC-13` D10, principle 4).**
  - the child refused the file, was killed, or ran out of time → `ErrorClass::Failed`;
  - every conversion slot is taken (`ConversionError::Busy`) → `ErrorClass::Failed`, and a
    `tracing::warn!` that says the conversion slots were full (no file name, no key);
  - the child could not be started → `ErrorClass::Failed`, logged at `error`;
  - the S3 read fails → the existing `class_of_object_store` mapping, unchanged.
  No upstream or child text reaches the answer.
- **D5. The runner reaches the listing by argument.** `dispatch`
  (`connector_listing.rs:1103`) gains `runner: &ConversionRunner`, passed from
  `routes/connector_discovery.rs:183` as `&state.conversion_runner`, and down to
  `files_answer` → `preview_s3`. No global, no second runner.
- **D6. CSV is unchanged.** Its head is text read by `csv_preview`; nothing in it trusts a
  declared count.

## 2. Anchors (at `6e5be45e`)

| What | Where |
| --- | --- |
| The in-process read | `rust/crates/lakehouse-api/src/connector_listing.rs:1000-1040` (`preview_s3`), `:86-88` (`PARQUET_MAX_BYTES`) |
| Dispatch and its caller | `connector_listing.rs:1103` (`dispatch`), `routes/connector_discovery.rs:183` |
| The upload path to copy | `routes/uploads.rs:768-833` (`ParquetFile`, `parquet_footer`), `parquet_of_answer` in the same file |
| The runner | `upload_convert.rs:1303` (`parquet_tail_bytes`), `:1334` (`answer`), `:1026-1056` (`Source`), `:765` (`InspectParquet` in the child) |
| Hostile fixtures | `ops/fixtures/uploads/hostile/huge_footer.parquet`, `huge_list_footer.parquet` (README there) |
| The upload tests to mirror | `tests/upload_routes.rs:8058`, `:8082` |
| Discovery route tests | `tests/connector_discovery_routes.rs` |
| Error words | `lakehouse-store/src/connector_discovery.rs:176` (`ErrorClass`) |

## 3. Tasks (one commit each, in order; push after each)

**T1. The child reads the preview (D1–D6).** Change `preview_s3`, `files_answer` and
`dispatch` as above; move `parquet_of_answer` (D3). Module doc of `connector_listing.rs`:
replace the "read whole" sentence with one that says the tail goes to the child and why
(`SEC-55`, `SEC-22` P1).
- *Accept:* `cargo clippy -p lakehouse-api --all-targets -- -D warnings`;
  `cargo test -p lakehouse-api --lib connector_listing` and `--test upload_routes parquet`
  (the upload path must not change).

**T2. Tests.** In `tests/connector_discovery_routes.rs` (or a new
`tests/connector_s3_preview.rs` if the fixture there cannot host it):
- an S3 files connector whose object store is a local stub serving
  `huge_list_footer.parquet` → the preview answers `failed`, and a SECOND request to the API
  (any route, e.g. `/api/health`) still answers 200: the API did not die;
- a valid Parquet object (any small one from `ops/fixtures/uploads/`) → the preview lists its
  columns and types;
- the stub receives a range request for the tail only, never `0..size` (assert on the
  requests it saw);
- a Parquet object larger than 64 MiB (the stub reports the size; it only has to serve the
  tail) previews (D2).
The stub is a `wiremock` server speaking enough of S3 for `object_store` (HEAD and ranged
GET); if `object_store` needs more, stop and report. The connector's host must pass the
internal-host check in the test the way the other discovery tests make a local host
allowed; do not weaken the check itself.
- *Accept:* the new tests pass; `cargo test -p lakehouse-api --test connector_discovery_routes`.

**T3. Docs and changelog.** CHANGELOG `## [Unreleased]` → `### Security`: one entry naming
`SEC-55`, what could happen (a crafted Parquet file in a bucket a files connector reads could
end the API for every tenant when previewed), and what changed (the preview reads the tail in
the child; the 64 MiB limit is gone). `docs/core/features/` page for connectors / files, if it
names the 64 MiB preview limit: update it.

## 4. Mutation check (in the handoff)

Temporarily put back the in-process `upload_parquet::inspect(&bytes)` call in `preview_s3`
and run T2's hostile test: it must fail (the test process aborts or the second request gets
no answer). Restore, rerun, quote both results.

## 5. Out of scope

- Moving the CSV preview to the child (D6).
- Any other connector type's preview.
- Changing the child's limits (`UPLOAD_*` settings).

## 6. Verification before handoff

```bash
cd rust && cargo fmt --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace
git status
```
No TypeScript, Python or compose change is expected; if one happens, run its block too.

## 7. Handoff (developer appends below)

## 8. Review (planner appends below)
