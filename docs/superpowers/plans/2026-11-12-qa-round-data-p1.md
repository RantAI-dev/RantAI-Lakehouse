# QA round, Data (P1): uploads into an older table, decimals in open format, the publish card after a failure

Fixes for three backlog rows opened by the QA round on staging (2026-11-02 to 11-12):
`DATA-47` (under `DATA-35`, case `LH-DATA-35-03`), `DATA-48` (under `DATA-1`, case
`LH-DATA-1-51`) and `DATA-49` (under `DATA-1`, cases `LH-DATA-1-01` and `LH-DATA-12-01` F1).

Base: `main` at `512f6e37c` (every anchor below was read there; the backlog rows were read at
`657c2502`, and none of the anchored files changed between the two). Branch:
`fix/qa-round-data-p1`. One pull request for the whole slice, opened as a draft by the planner
as soon as the Rust tasks are pushed, so CI runs the Rust tests this machine cannot.

Planner and reviewer: Claude Opus. Developer: another agent (not Opus). The developer pushes
after every commit.

The owning features exist and are signed (`docs/core/features/upload-folders.md`,
`docs/core/features/upload-file.md`, `docs/core/features/gold-publish-per-mart.md`; there is no
`docs/core/specs/data-1.md`). Each task closes a gap against a line already on those pages.
The decisions below were made by the planner on the owner's brief; the text on screen is in
D17, and T13 names the two feature-page lines that change.

What QA saw:

- `LH-DATA-35-03`: after 105 uploads, a file loaded into an older uploaded table got no
  "Replace its rows / Add to its rows" choice (Replace was used) and "No folder", although
  `GET /api/uploads?q=<name>` finds the older upload.
- `LH-DATA-1-51`: Publish now refused with `column "amount" has unsupported ClickHouse type
  "Decimal(12, 2)": no Iceberg mapping`.
- `LH-DATA-1-01`: the publish history showed the engine's own error with its version, and the
  catalog's HTTP answer with its headers; the status stayed "Up to date" during five failed
  runs; `GET /api/gold/export/{mart}` answered 500 for a mart never published; Publish now on
  an unchanged table added a full copy; "Copies kept 1" beside two copies; times with no zone.

## 1. Decisions

- **D1. One task per commit, in the order of section 2.** Each commit body cites its QA case
  ID and backlog ID. Each fix site carries a short comment naming the case (rule 13), for
  example `// QA LH-DATA-35-03 (DATA-47): …`.
- **D2. No new helper where one exists (rule 4).**
  - Table step: `isUploadedTable` (`src/lib/uploads.ts:223`) and `folderOfTable`
    (`src/lib/upload-folders.ts:86`) stay as they are; only the list they are given changes.
  - Debounce: the pattern of `useColumnChoices`
    (`src/features/connectors/use-column-choices.ts:18`, `:71-99`: a timer, an
    `AbortController`, `toServiceError`, and "aborted" ignored).
  - Classification: `upstream_error::report`, `report_ch`, `ch_error`, `internal_error` and the
    contexts `DATABASE`, `CATALOG` (`rust/crates/lakehouse-api/src/upstream_error.rs:102`,
    `:116`, `:304`, `:330`, `:372`, `:379`).
  - "The table is not there": the kind test `rest::map_catalog_error` already uses
    (`rust/crates/lakehouse-iceberg/src/rest.rs:59-65`, `ErrorKind::TableNotFound`).
  - A waited ClickHouse mutation: the shape of `forget_runs`
    (`rust/crates/lakehouse-api/src/routes/quality.rs:1067-1075`, `SETTINGS mutations_sync = 1`).
  - Milliseconds to RFC 3339: `millis_to_rfc3339` (`routes/gold.rs:587`). Do not add a third
    copy (`gold_export_history.rs:495` is already a second one).

### DATA-47, the Table step

- **D3. An exact `table=` filter on `GET /api/uploads`; no new route.** The store query is
  already bound to the caller's tenant (`tenant_id = $1`,
  `rust/crates/lakehouse-store/src/uploads.rs:674`) and the route already needs
  `connector:manage` (`rust/crates/lakehouse-api/src/policy.rs:1096`), so an exact match on
  `bronze_table` in that one statement answers "which uploads of my tenant loaded into this
  table", newest first, with each upload's `folderId`. `POLICY_TABLE` and
  `tests/route_auth.rs` do not change (same method and path). The filter narrows the rows
  before the newest-100 window is cut, as `folderId` and `q` do. `q` is not used for this:
  it is a contains-match on two columns, so `stock` would also bring `stock_2026`.
- **D4. The page asks about the named table and waits for the answer.** The lookup has four
  states: no valid name, checking, known, failed.
  - Known: Replace / Add is offered when `isUploadedTable(name, answer)` is true, and the
    folder offered is `folderOfTable(name, answer)`. This holds for any earlier upload of the
    table, however old.
  - Checking: Next (on the Table step) and Load (on the Review step) are off, with one line
    saying what is being checked.
  - Failed: Next and Load stay off, the page says why, and a "Check again" button asks again.
    The page never falls back to Replace on a failed lookup (fail closed, principle 3).
  - Replace stays pre-selected when the choice is shown (`upload-file.md` decision 5: "Replace
    by default; adding is a choice"). That is a visible choice, not a silent one.
- **D5. Latency.** A name the page set itself (the suggestion from the file name, or the
  table of an upload opened with `?id=`) is asked at once, so the answer is normally there
  before the person reaches the Table step. A name the person types is asked 400 ms after
  the last key (the same pause as the column check), and a request for a name that is no
  longer in the field is aborted. An answer is used only for the name it was asked for.
- **D6. What stays as it is.** The page still reads the unfiltered list once: the run view
  of a page opened with `?id=` uses it to name the files loaded together
  (`src/features/connectors/upload-file-page.tsx:193-197`). Two limits remain and are listed
  in section 4: a table whose upload rows were all deleted, and a table whose newest 100
  uploads all failed.

### DATA-49, the publish card after a failure

- **D7. A failed publish is classified once, before it is stored, and the row and the
  response carry the same sentence and the same reference.** The route converts
  `GoldExportError` to `ApiError` first (`From<GoldExportError> for ApiError`,
  `routes/gold.rs:81-112`, which logs the raw text under a reference), then stores
  `ApiError`'s text. The `Display` of `GoldExportError` is for the log only. Per variant:

  | Variant (`gold_export.rs:90-158`) | Stored and returned | Status |
  | --- | --- | --- |
  | `ClickHouse` | `The database request failed. Reference: <id>` or `The database is unavailable. Reference: <id>` (`DATABASE`) | 422 / 503, as today |
  | `Iceberg` | `The catalog request failed. Reference: <id>` (`CATALOG`) | 500, as today |
  | `Batch` | new: `A value in this table could not be written in open format. Reference: <id>` | 422, as today |
  | `UnsupportedColumn`, `RowCapExceeded`, `PolicyRefused` | this repository's own sentence, unchanged | 422 |
  | `PolicyTableRefused`, `TenantGateRefused` | the gate's own sentence, unchanged | the gate's |

  `Batch` is classified because its text can hold a cell value and the text of the Arrow or
  Iceberg library (`gold_export.rs:273-322`, `:772`, `:821`).
- **D8. Text already stored is replaced, in ClickHouse, not by a `rust/migrations` file.**
  The owner's brief says "a migration, as `0127` did for query history". The history table
  is `console.gold_export_run`, a ClickHouse table whose single owner is
  `gold_export_history::ensure_gold_export_run_table`
  (`rust/crates/lakehouse-api/src/gold_export_history.rs:97-130`); migration `0117`'s header
  says the same (`console` is not a Postgres schema). So the rewrite is the same targeted
  `UPDATE … WHERE`, written as one ClickHouse `ALTER TABLE … UPDATE` that the owner function
  runs once per process, after it has ensured the table. No file is added under
  `rust/migrations/` (highest today: `0136`).
  - Rows touched: `status = 'failed'` and `error` starting with one of the three prefixes
    this code wrote in front of upstream text: `reading Gold mart from ClickHouse failed: `,
    `Iceberg export failed: `, `failed to build export batch: `.
  - New text: `The database request failed.`, `The catalog request failed.`,
    `A value in this table could not be written in open format.`; the exact old text
    `reading Gold mart from ClickHouse failed: fetch failed` (this code's own fixed words for
    an unreachable engine) becomes `The database is unavailable.`. No reference is added:
    those rows never had one, and inventing one would be fabricating (principle 2).
  - Rows not touched: every other text (the row cap, an unsupported column, a policy or gate
    refusal) was written by this repository and stays.
  - Written to be run again: after the rewrite no row matches, and no mutation is issued
    when a count of matching rows is zero.
  - The response does not depend on the rewrite: the read path maps the same three shapes to
    the same sentences before a row leaves the API. If the rewrite fails it is logged and
    tried again at the next start; the list still never shows the raw text.
- **D9. Publication states.** The server adds `lastFailure` to the publication body: `null`
  unless the newest run of the mart (any status) is `failed`; then
  `{ "failedAt": <RFC 3339 or null>, "reason": <the stored sentence> }`. It is computed in
  `publication_body` (`routes/gold.rs:865-912`). The label is computed in `freshnessLine`
  (`src/lib/gold-freshness.ts:10-15`), in this order:
  1. `lastFailure` is set: **"Last publish failed"**.
  2. `lastExportedAt` is null: "Never published".
  3. `changedSinceExport` is null: "Not measured".
  4. `changedSinceExport` is true: "Out of date".
  5. otherwise: "Up to date".

  In state 1 the card shows the failure's time and reason, and says when the last successful
  publish was (or that nothing has been published yet). The "Last published" and "Snapshot"
  rows stay, as today. `lastExportedAt` and `changedSinceExport` keep their meaning (they
  describe the last successful publish).
- **D10. A failed newest run is never skipped by `?ifChanged=true`.** Today the scheduler's
  skip compares the mart with the last *successful* run only, so after a failure on an
  unchanged mart every scheduled run is skipped and the card would say "Last publish failed"
  until someone presses Publish now. A run that failed after it had appended some batches
  also leaves a partial newest copy. `if_changed_skip_reason` already fails toward
  publishing on any doubt (`routes/gold.rs:129-152`); a failed newest run is such a doubt.
- **D11. A mart never published answers 200 with `published: false`.**
  `GET /api/gold/export/{mart}` answers
  `{ namespace, table, published: false, formatVersion: null, rowsInIceberg: null,
  snapshotId: null, exportedAt: null }` when the catalog says the table does not exist, and
  the body it answers today plus `published: true` otherwise. 200 and not 404: both console
  callers treat "no export" as a normal state (`open-format-card.tsx:143-150`,
  `gold-exports-page.tsx:87-93`), the consumers route already answers a missing capability
  with 200 and `supported: false` (`routes/gold.rs:728-734`), and a 404 would be a failed
  request in the browser for a state that is not an error. Any other catalog failure is
  still the classified 500. Once "never published" is typed, the console stops reading
  every failure of this route as "Never exported".
- **D12. Smaller items of the row.** Included: "Copies kept" (D17, T12): the number is the
  count at the last trim, and no trim follows a manual publish
  (`dagster/dispar_orchestrate/gold_export.py:959`, `:778-799`), so the card must say when it
  was counted. Not included, with the reason, in section 4: Publish now on an unchanged
  mart, and the time zone.

### DATA-48, decimals in open format

- **D13. `Decimal(P, S)` maps to Iceberg `decimal(P, S)`, same precision and scale.** The
  workspace pins `iceberg` and `iceberg-catalog-rest` at `=0.10.1` with `arrow-array` 58.4.1
  and `parquet` 58.4.1 (`rust/crates/lakehouse-iceberg/Cargo.toml:15-23`, `rust/Cargo.lock`).
  Read in that crate's source: `PrimitiveType::Decimal { precision, scale }` exists, the
  limit is 38 digits (`MAX_DECIMAL_PRECISION`), the Arrow type is `Decimal128(P, S)`, and the
  Parquet writer stores it as `INT32` (P up to 9), `INT64` (P up to 18) or a fixed-length
  byte array (above), which is what the Iceberg format asks for. The export's writer
  (`GoldTable::append`, `rust/crates/lakehouse-iceberg/src/catalog.rs:595-640`) takes any
  Arrow batch that matches the table's schema, so nothing changes in `lakehouse-iceberg`
  for this.
- **D14. The value is read as text and converted exactly.** In the export's `FORMAT JSON`
  ClickHouse writes a decimal as a JSON number by default, and `serde_json` here has no
  arbitrary precision (`rust/Cargo.toml:29`), so a number would pass through a float. The
  batch query therefore selects a decimal column as `toString(col) AS col`, the way it
  already selects a `DateTime` through `toString(toTimeZone(col, 'UTC'))`
  (`gold_export.rs:419-427`). The conversion of the text to the unscaled integer the writer
  takes:
  1. An optional leading `-`. Nothing else before the digits (no `+`, no spaces).
  2. Digits, then optionally `.` and digits. Nothing else (no exponent, no separators).
  3. The fraction may be shorter than `S` (ClickHouse drops trailing zeros by default): pad
     it on the right with `0` to exactly `S` digits. A fraction longer than `S` is an error.
  4. Unscaled value = the integer digits followed by the padded fraction, read as `i128`;
     negate it if there was a `-`. More than `P` digits (after leading zeros) is an error.
  5. The Arrow array is `Decimal128Array` of those `i128` values with
     `with_precision_and_scale(P, S)`; a JSON `null` or a missing cell is a null.

  Anything that is not such a text (a JSON number included) fails the run with the D7
  `Batch` sentence. Nothing is rounded or coerced (rule 14).
- **D15. Edge cases.**
  - Negative values, including between -1 and 0 (`-0.01`: the sign is read before the digits,
    so it is not lost when the integer part is `0`).
  - Scale 0 (`Decimal(9, 0)`: the text has no `.`).
  - `Nullable(Decimal(..))`: an optional Iceberg field; `toString` of a NULL is NULL.
  - Aliases, if ClickHouse ever reports them: `Decimal32(S)` is P 9, `Decimal64(S)` P 18,
    `Decimal128(S)` P 38, `Decimal256(S)` P 76; `Decimal(P)` is scale 0.
  - Precision above 38 (`Decimal256`, or `Decimal(39..76, S)`): refused before anything is
    read or written, with `UnsupportedColumn`, whose sentence names the column and its type:
    `column "<name>" has unsupported ClickHouse type "Decimal(76, 10)": open format (Iceberg)
    holds decimals of at most 38 digits, and this column has 76`.
- **D16. Proof that the exported values equal the mart's.** The CI gate "Rust · Gold export
  to Iceberg (ADR 0010)" (`ops/gold_export/gold_export_test.py`, job at
  `.github/workflows/ci.yml:3447`) gets three decimal columns, one per Parquet storage form,
  and compares every value a third-party client (`pyiceberg`) reads from the published table
  with what ClickHouse holds (T8). The unit tests of T7 prove the conversion itself.

### Text and assistant

- **D17. Text on screen (English, as the console).**
  - Table step and Review step, while checking:
    **"Checking whether an earlier upload created `<table>`…"**
  - Table step and Review step, lookup failed: **"Could not check whether an earlier upload
    created `<table>`, so Load is off: a load now could replace its rows without asking."**
    followed by the API's own sentence, and a **"Check again"** button.
  - Card status: **"Last publish failed"**. Under it: **"Failed `<time>`: `<reason>`"**
    (without the time: "Failed: `<reason>`"), then **"Last successful publish: `<time>`."** or
    **"Nothing has been published yet."**
  - Card, read-back failed: the "Last published" row reads **"Could not be read"**.
  - Card, trim row: the label **"Copies kept at last trim"**, the value
    **"`<n>` (counted `<time>`; oldest `<time>`)"**.
- **D18. Assistant.** No dashboard (BI) behaviour changes, so there is no tool or schema
  change and `tests/fixtures/tool_schemas.json` is not regenerated. One result changes: the
  assistant's `get_gold_export` tool reads the same function as the route, so for a mart
  never published it now answers `published: false` instead of an error (T6). Its arguments
  are unchanged.

## 2. Tasks

Rust first (T1 to T7), then the gate (T8), then the console (T9 to T12), then the two
feature pages (T13).

### T1. The store lists one table's uploads by its exact name (LH-DATA-35-03; DATA-47)

- Anchors: `rust/crates/lakehouse-store/src/uploads.rs:638-644` (`ListFilter`), `:646-693`
  (`list_filtered`; the statement at `:674-677`, the binds at `:680-687`);
  `rust/crates/lakehouse-store/tests/upload_folders.rs:75-80` (`show`), `:428-438`.
- Change:
  - Add to `ListFilter` the field `pub table: Option<&'a str>`, documented: "Exactly this raw
    table name. Bound as a value and compared with `=`, so `%` and `_` are ordinary
    characters and the case must match."
  - In the statement add, before `ORDER BY`: `AND ($6::text IS NULL OR bronze_table = $6)`,
    and `.bind(filter.table)` after `.bind(pattern)`.
  - Add the sentence about the table filter to the doc comments of `ListFilter` and
    `list_filtered`.
  - The two struct literals in `tests/upload_folders.rs` (`:76`, `:432`) get `table: None`.
- Check: a new `sqlx::test` in `rust/crates/lakehouse-store/tests/upload_folders.rs`,
  `the_list_narrows_to_one_tables_uploads_by_its_exact_name_before_the_window_is_cut`. Seed
  with the file's `add` helper: a tenant with uploads `up-1` (table `stock`), `up-2` (table
  `stock_2026`), `up-3` (table `stoxk`), then `up-5` and `up-6` (no table); another tenant
  with `up-4` (table `stock`). Set `bronze_table` with a direct
  `UPDATE file_upload SET bronze_table = $1 WHERE id = $2`. Assert:
  `table: Some("stock")` gives `["up-1"]`; `Some("sto_k")` and `Some("STOCK")` give nothing;
  the other tenant with `Some("stock")` gets `["up-4"]`; with `limit` 1 the unfiltered list
  is `["up-6"]` while `table: Some("stock")` with `limit` 1 is still `["up-1"]`.

### T2. `GET /api/uploads?table=<name>` (LH-DATA-35-03; DATA-47)

- Anchors: `rust/crates/lakehouse-api/src/routes/uploads.rs:1608-1617` (the doc comment of
  `list`), `:1638-1647` (the filter is built and used), `:1656-1664` (`ListQuery`).
- Change: `ListQuery` gains `table: Option<String>` ("Exactly this raw table name; absent or
  empty: every table"). In `list`, trim it and drop an empty value the way `q` is handled
  at `:1638`, and pass it as `ListFilter { folder, text, table }`. Add `table=<name>` to the
  doc comment of `list`. Nothing else: the tenant still comes from `tenant_scope::resolve`
  (`:1631`), the route's entry in `POLICY_TABLE` is unchanged.
- Check: a new test in `rust/crates/lakehouse-api/tests/upload_folder_routes.rs`, next to
  `the_upload_list_can_be_narrowed_by_folder_and_text` (`:553`) and with its helpers
  (`Stack::start`, `stack.seed`, `stack.send`):
  `the_upload_list_can_be_narrowed_to_one_table_by_its_exact_name`. Seed `up-1` with table
  `stock_raw`, `up-2` with `stock_raw_2026`, `up-3` with none, and `up-r` for the other
  tenant with `stock_raw`. Assert `GET /api/uploads?table=stock_raw` lists `["up-1"]` for
  the first tenant's user and `["up-r"]` for the other's, `?table=stock` lists nothing, and
  `?table=` (empty) is the whole list. Not run locally (CI).

### T3. A failed publish stores the classified sentence (LH-DATA-1-01, LH-DATA-12-01 F1; DATA-49)

- Anchors: `rust/crates/lakehouse-api/src/upstream_error.rs:115-120` (`CATALOG`);
  `rust/crates/lakehouse-api/src/routes/gold.rs:81-112` (the `From` impl), `:493-528` (the
  export runs, the history is recorded, the error is converted), `:540-580`
  (`record_export_history`; the raw text is taken at `:559` and stored at `:570`);
  `rust/crates/lakehouse-api/src/gold_export.rs:91-93`, `:105-111` (the variants' docs);
  `rust/crates/lakehouse-api/src/gold_export_history.rs:153-156` (the comment that says
  "never raw"); `rust/crates/lakehouse-api/tests/sec11_guard.rs:216-220`.
- Change:
  1. `upstream_error.rs`: after `CATALOG`, add
     `pub const PUBLISH_BATCH: Context = Context::new("gold publish batch", "A value in this
     table could not be written in open format.", "A value in this table could not be
     written in open format.");` with a doc comment naming the QA case.
  2. `routes/gold.rs` `From` impl: take `GoldExportError::Batch` out of the shared arm and
     give it its own: `GoldExportError::Batch(detail) => upstream_error::report(
     &upstream_error::PUBLISH_BATCH, upstream_error::Class::Failed, &detail)
     .into_api_error(upstream_error::FailedAs::Unprocessable)`. Leave the line
     `| GoldExportError::PolicyRefused(_) => Self::Unprocessable(err.to_string()),` exactly as
     it is: the SEC-11 guard's allowlist matches that text, and no entry is added to it.
  3. `export` (`:493-528`): right after `finished_at`, convert once:
     `let export_result = export_result.map_err(ApiError::from);` then call
     `record_export_history` with it, then return `export_result?` in place of the
     `map_err` at `:528`. One conversion means one log line and one reference.
  4. `record_export_history`: its last parameter becomes
     `&Result<gold_export::GoldExportResult, ApiError>`. Take the stored text from a new
     private function `stored_error(result: &Result<gold_export::GoldExportResult, ApiError>)
     -> Option<String>` (the `ApiError`'s `to_string()`), so the type itself keeps a
     `GoldExportError` from being stored. Say in its doc comment that this is the sentence
     the response carries.
  5. `gold_export_history.rs:153-156`: replace the comment with what is now true: the text
     is the response's own sentence, already classified by `routes::gold` (fixed words and a
     `Reference:` for an upstream failure, this repository's sentence for a refusal); it is
     never the `Display` of `GoldExportError`, which holds upstream text for the log.
  6. `gold_export.rs`: add one line to the docs of the `ClickHouse`, `Batch` and `Iceberg`
     variants: their `Display` is for the log, never for a response or a stored row.
- Check:
  - Unit tests in the `tests` module of `routes/gold.rs`:
    `a_batch_failure_answers_a_fixed_sentence_with_a_reference_and_never_the_value` (an
    `ApiError::from(GoldExportError::Batch("amount: \"12,5\" is not a valid decimal".into()))`
    has status 422, its text starts with `A value in this table could not be written in open
    format. Reference: ` and does not contain `12,5`; fails today) and
    `the_stored_error_of_a_failed_publish_is_the_classified_sentence` (`stored_error` of an
    `Err(ApiError::from(GoldExportError::ClickHouse(ChError::Server("Code: 60. DB::Exception:
    Unknown table (version 26.8.1.1)".into()))))` starts with `The database request failed.
    Reference: ` and contains neither `Code: 60` nor `26.8`).
  - Integration test in `rust/crates/lakehouse-api/tests/gold_publication.rs`:
    `a_failed_publish_stores_the_sentence_the_response_carried_and_no_catalog_text`. It needs
    the export to get past `read_catalog_token`: add beside `spin_up_with_clickhouse`
    (`:90-104`) a helper that also sets `LAKEKEEPER_GOLD_EXPORT_TOKEN_FILE` to a `tempfile`
    holding any text. Seed a mart with `seed_mart`, `POST /api/gold/export/<mart>` with a
    `gold:export` session. The catalog of the harness is unreachable, so assert: status 500;
    `body["error"]` starts with `The catalog request failed. Reference: `; the newest row of
    `GET /api/gold/exports?mart=<mart>` has `status` `failed` and an `error` equal to
    `body["error"]`. Fails today (the row holds the catalog client's text). Not run locally
    (CI). If the export stops somewhere else in the harness, report what it answered; do not
    change the assertion to fit.

### T4. Raw text already stored is replaced, and is never listed (LH-DATA-1-01; DATA-49)

- Anchors: `rust/crates/lakehouse-api/src/gold_export_history.rs:104-130`
  (`ensure_gold_export_run_table_uncached`), `:172-178` (`SqlLiteral` helpers), `:273-300`
  (`row_to_export_run`; the stored text is returned at `:290-294`);
  `rust/crates/lakehouse-api/src/routes/quality.rs:1067-1075` (the mutation shape).
- Change, all in `gold_export_history.rs`:
  1. Three `const`s for the prefixes of D8 and one for the exact unreachable text, with a
     doc comment saying they are this code's own old `Display` prefixes.
  2. `fn public_error(stored: &str) -> &str`: the exact unreachable text returns
     `upstream_error::DATABASE.unavailable`; the ClickHouse prefix returns
     `upstream_error::DATABASE.failed`; the Iceberg prefix returns
     `upstream_error::CATALOG.failed`; the batch prefix returns
     `upstream_error::PUBLISH_BATCH.failed`; anything else is returned as it is.
  3. `row_to_export_run`: pass the stored text through `public_error` before it is copied.
  4. `fn raw_error_predicate() -> String` (the three `startsWith(error, <literal>)` joined by
     `OR`, literals through `SqlLiteral`), used by both statements below so they cannot
     drift:
     - count: `SELECT toString(count()) AS n FROM console.gold_export_run WHERE status =
       'failed' AND (<predicate>)`;
     - rewrite: `ALTER TABLE console.gold_export_run UPDATE error = CAST(multiIf(error =
       <unreachable>, <unavailable sentence>, startsWith(error, <clickhouse prefix>),
       <database sentence>, startsWith(error, <iceberg prefix>), <catalog sentence>, <batch
       sentence>) AS Nullable(String)) WHERE status = 'failed' AND (<predicate>) SETTINGS
       mutations_sync = 1`.
  5. `pub async fn rewrite_raw_errors(ch: &ChClient) -> Result<u64, ChError>`: runs the
     count; returns `Ok(0)` without a mutation when it is zero; otherwise runs the rewrite
     and returns the count. `# Errors` section as the file's other functions.
  6. `ensure_gold_export_run_table_uncached`: after the `ADD COLUMN`, call
     `rewrite_raw_errors`; on `Err`, `tracing::error!` and carry on (comment: the read path
     already hides the text, and the next start tries again; failing here would take the
     whole history down for a clean-up).
- Check:
  - Unit tests in the file's `tests` module:
    `public_error_replaces_each_raw_shape_and_keeps_this_repositorys_own_sentences` (the four
    raw shapes, plus a row-cap sentence and an already classified
    `The catalog request failed. Reference: abc` that come back unchanged);
    `row_to_export_run_never_returns_a_stored_upstream_text` (a row whose `error` is
    `Iceberg export failed: iceberg catalog operation failed: status: 401 Unauthorized,
    headers: {…}` comes out as `The catalog request failed.`; fails today);
    `the_rewrite_statement_touches_only_failed_rows_in_the_three_raw_shapes` (the SQL contains
    `status = 'failed'`, the three prefixes, and `mutations_sync = 1`).
  - Integration test in `tests/gold_publication.rs`:
    `stored_upstream_text_is_rewritten_at_rest_and_never_listed`. Seed, with
    `gold_export_history::record_export_run`, three failed rows with the three raw shapes,
    one failed row with a row-cap sentence and one success, each a different second. Assert
    the list route shows the fixed sentences; call `rewrite_raw_errors` and assert it
    returns 3; read `error` straight from ClickHouse and assert the three fixed sentences,
    the row-cap sentence unchanged; call it again and assert 0. Not run locally (CI). The
    planner could not run ClickHouse: if 26.8 refuses the statement, report its answer.

### T5. The publication says when the last run failed, and a failed run is not skipped (LH-DATA-1-01; DATA-49)

- Anchors: `rust/crates/lakehouse-api/src/gold_export_history.rs:232-241`
  (`last_success_sql`), `:317-354` (`last_success_export`);
  `rust/crates/lakehouse-api/src/routes/gold.rs:53-60` (module doc), `:129-152`
  (`if_changed_skip_reason`), `:162-180` (`unchanged_since_last_export_reason`), `:353-367`
  (the Skip section of `export`'s doc), `:865-912` (`publication_body`), `:973-981` (the doc
  of `publication`), `:1348-1390` (the five skip tests).
- Change:
  1. `gold_export_history.rs`: `last_run_sql(mart)`, the mirror of `last_success_sql` without
     the status filter: `SELECT status, error, toString(toUnixTimestamp64Milli(started_at)) AS
     started_ms FROM console.gold_export_run WHERE mart = <literal> ORDER BY started_at DESC,
     finished_at DESC LIMIT 1 FORMAT JSON`. A struct `LastExportRun { status: String,
     started_at_ms: Option<i64>, error: Option<String> }` and
     `pub async fn last_export_run(ch, mart) -> Result<Option<LastExportRun>, ChError>`, the
     mirror of `last_success_export`; `error` goes through `public_error` (T4).
  2. `routes/gold.rs`: `if_changed_skip_reason` gains a fourth parameter
     `newest_run_failed: bool` and returns `None` at once when it is true. Its doc comment
     gains the rule and the reason (D10).
  3. `unchanged_since_last_export_reason`: read `last_export_run` first; when its status is
     `failed`, return `Ok(None)` without the other read; otherwise as today, passing `false`.
     A failed read stays a classified error, as the existing read does.
  4. A pure function `last_failure(newest: Option<&gold_export_history::LastExportRun>) ->
     Value`: `Value::Null` unless the status is `failed`; then `{"failedAt": <RFC 3339 through
     millis_to_rfc3339, or null>, "reason": <error, or "The publish failed. No reason was
     recorded." when the row has none>}`.
  5. `publication_body`: read `last_export_run` (classified like its neighbours) and add
     `"lastFailure": last_failure(..)` to the body.
  6. Update the three doc comments named in the anchors so they describe the new field and
     the new skip rule.
- Check:
  - Unit tests in `routes/gold.rs`: the five existing skip tests pass `false` as the new
    argument (the assertions do not change); new
    `a_failed_newest_run_is_never_skipped_even_when_the_fingerprint_matches`;
    `last_failure_is_null_when_the_newest_run_succeeded_or_there_is_none`;
    `last_failure_carries_the_time_and_the_stored_sentence_when_the_newest_run_failed`.
    A unit test in `gold_export_history.rs`:
    `last_run_sql_takes_the_newest_row_of_any_status`.
  - Integration tests in `tests/gold_publication.rs` (CI):
    `publication_detail_reports_the_last_failure_until_a_later_run_succeeds` (success an
    hour ago, failure a minute ago: `lastFailure.reason` is the stored sentence and
    `lastExportedAt` is still the success; add a success now: `lastFailure` is null);
    `if_changed_does_not_skip_after_a_failed_run` (a success with the mart's current
    fingerprint, then a failed row: `POST …?ifChanged=true` is not `skipped`; it stops at
    the token file with 503, as the neighbouring tests at `:563-632` do).

### T6. A mart never published answers "not published", not 500 (LH-DATA-1-01; DATA-49)

- Anchors: `rust/crates/lakehouse-iceberg/src/catalog.rs:345-358` (`load_gold_table`);
  `rust/crates/lakehouse-iceberg/src/rest.rs:498` (`wiremock_tests`), `:655-673` (`connect`),
  `:797-817` (the 404 test to copy from); `rust/crates/lakehouse-api/src/gold_export.rs:915-958`
  (`GoldReadBack`, `read_back_row_count`); `rust/crates/lakehouse-api/src/routes/gold.rs:596-644`
  (`read_back`); `rust/crates/lakehouse-api/src/routes/ai/tools/gold.rs:149-171`
  (`get_gold_export`).
- Change:
  1. `catalog.rs`: add `pub async fn load_gold_table_if_exists(&self, mart_name: &str) ->
     Result<Option<GoldTable>, IcebergError>`: the body of `load_gold_table`, but a
     `load_table` error whose `kind()` is `iceberg::ErrorKind::TableNotFound` is `Ok(None)`
     (the REST client gives that kind for the catalog's 404). Every other error is returned
     as today (import `iceberg::ErrorKind`; test the kind before the error is converted to
     `IcebergError`, which keeps only its text). `load_gold_table` and
     `create_or_load_gold_table` are not changed.
  2. `gold_export.rs`: `read_back_row_count` returns `Result<Option<GoldReadBack>,
     GoldExportError>`: `Ok(None)` when `load_gold_table_if_exists` says none. Update its doc
     and `# Errors`.
  3. `routes/gold.rs`: a `pub(crate) fn read_back_body(mart: &str, readback:
     Option<&gold_export::GoldReadBack>) -> Value` that builds the two bodies of D11 (the
     published one is today's body at `:636-643` plus `"published": true`). `read_back` uses
     it. Update the doc of `read_back`: the "500 if the table does not exist yet" is gone.
  4. `routes/ai/tools/gold.rs`: `get_gold_export` returns
     `crate::routes::gold::read_back_body(..)` for `Ok`, so the tool and the route cannot
     drift. The `Err` arm is unchanged.
- Check:
  - `rest.rs`, in `wiremock_tests`, with its `connect`:
    `a_gold_table_the_catalog_does_not_have_loads_as_none` (a 404 on
    `/v1/namespaces/gold/tables/never` gives `Ok(None)`) and
    `a_catalog_that_refuses_the_caller_is_still_an_error_for_a_gold_table` (a 401 on the same
    path gives `Err`).
  - Unit tests in `routes/gold.rs`:
    `the_read_back_of_a_mart_never_published_says_so_and_invents_no_number` (`published` is
    false; `formatVersion`, `rowsInIceberg`, `snapshotId`, `exportedAt` are null) and
    `the_read_back_of_a_published_mart_keeps_its_fields_and_says_published`.

### T7. A decimal column is published as an Iceberg decimal (LH-DATA-1-51; DATA-48)

- Anchors, all in `rust/crates/lakehouse-api/src/gold_export.rs`: `:72-75` (Arrow imports),
  `:162-176` (`ColumnPlan`, `unwrap_nullable`), `:178-209` (`ch_type_to_primitive` and its doc,
  which lists `Decimal` as rejected), `:260-265` (`cell_as_str`), `:389-427`
  (`select_projection` and its doc), `:546-598` (`build_array`; the fallback arm at `:592-596`),
  `:996-1007` (the test helpers `row_with`, `ch_column`).
- Change:
  1. `const MAX_ICEBERG_DECIMAL_PRECISION: u32 = 38;` and
     `fn ch_decimal(base: &str) -> Option<Result<(u32, u32), String>>`: `None` when `base` is
     not a decimal name; otherwise the precision and scale read from the spellings of D15
     (spaces around the numbers trimmed), or `Some(Err(reason))`. Reasons: a number that does
     not parse, a precision of 0, or a scale above the precision: `its precision and scale
     could not be read`; a precision above 38: `open format (Iceberg) holds decimals of at
     most 38 digits, and this column has <P>`.
  2. `ch_type_to_primitive`: before the `match`, when `ch_decimal(base)` is `Some`, return
     its error or `Ok((PrimitiveType::Decimal { precision, scale }, nullable))`. Rewrite the
     doc comment: decimals are mapped; `Decimal` leaves the list of rejected types.
  3. `select_projection`: a decimal column is `toString(<quoted>) AS <quoted>`. Add the
     reason to the doc comment (D14).
  4. `fn decimal_text_to_unscaled(text: &str, precision: u32, scale: u32) -> Result<i128,
     String>`: the five steps of D14, with a doc comment that lists them. No float anywhere.
  5. `fn cell_as_decimal(row, name, precision, scale) -> Result<Option<i128>,
     GoldExportError>`: `None` or JSON null is `Ok(None)`; a JSON string goes through
     `decimal_text_to_unscaled`; anything else (a JSON number included) is a
     `GoldExportError::Batch` naming the column.
  6. `build_array`: an arm for `PrimitiveType::Decimal { precision, scale }` that collects the
     cells and builds `Decimal128Array::from(values).with_precision_and_scale(..)` with
     `u8::try_from(precision)` and `i8::try_from(scale)`; every error becomes a
     `GoldExportError::Batch` (no `unwrap`).
  The schema needs nothing more: `plan_to_iceberg_fields` (`:247-258`) already wraps
  whatever `PrimitiveType` the plan holds.
- Check, unit tests in the file's `tests` module (each fails today, where
  `ch_type_to_primitive("Decimal(12, 2)")` is an error):
  - `a_clickhouse_decimal_maps_to_an_iceberg_decimal_of_the_same_precision_and_scale`
    (`Decimal(12, 2)`, `Decimal(38, 10)`, `Decimal(9, 0)`, `Decimal(10)`).
  - `a_nullable_decimal_is_an_optional_decimal` (through `plan_columns` and
    `plan_to_iceberg_fields`: the field is optional and its type is `decimal(12, 2)`).
  - `the_decimal_aliases_map_to_their_fixed_precisions` (`Decimal32(2)`, `Decimal64(4)`,
    `Decimal128(10)`).
  - `a_decimal_wider_than_38_digits_is_refused_and_the_sentence_names_the_column`
    (`plan_columns` on a column `big` of `Decimal(76, 10)` and on `Decimal256(10)`: the
    error's text is the sentence of D15, with `big` in it).
  - `select_projection_reads_a_decimal_as_text` (plain and `Nullable`).
  - `decimal_text_becomes_the_exact_unscaled_integer`: `("12.5", 12, 2)` is 1250;
    `("12.50", 12, 2)` 1250; `("-0.01", 12, 2)` -1; `("-0.5", 3, 1)` -5; `("0", 12, 2)` 0;
    `("123", 9, 0)` 123; `("-1234567890123456789012345678.0123456789", 38, 10)` is
    `-12345678901234567890123456780123456789_i128`.
  - `decimal_text_that_is_not_an_exact_decimal_is_refused`: `"1.234"` at scale 2, `"1e5"`,
    `"+1"`, `""`, `"abc"`, `"1,5"`, `" 1"`, and 13 digits at precision 12.
  - `a_decimal_cell_that_arrives_as_a_json_number_is_refused`.
  - `build_array_makes_a_decimal128_column_with_the_columns_precision_and_scale_and_keeps_nulls`
    (three rows `"12.50"`, null, `"-0.01"` for `decimal(12, 2)`: the array's data type is
    `Decimal128(12, 2)`, its values 1250, null, -1).

### T8. The Gold export gate carries decimal columns and compares their values (LH-DATA-1-51; DATA-48)

- Anchors: `ops/gold_export/gold_export_test.py:172-204` (`step_seed_gold_mart`), `:412-428`
  (`step_outside_tool_reads_the_published_table`), `:537-538`, `:603-605` (later inserts that
  name four columns), `:693-711` (`main`).
- Change:
  - `step_seed_gold_mart`: the table gains `units Decimal(9, 0)`, `net_amount Decimal(12, 2)`
    and `fee Nullable(Decimal(38, 10))`. Row `i` takes entry `i % 7` of three fixed lists
    (module constants), written into the `INSERT` as `toDecimal32('<text>', 0)`,
    `toDecimal64('<text>', 2)` and `toDecimal128('<text>', 10)` or `NULL`, so no value passes
    through a float:
    - `units`: `-999999999`, `0`, `1`, `999999999`, `-1`, `42`, `7`
    - `net_amount`: `-1234567890.12`, `0`, `0.01`, `-0.01`, `9999999999.99`, `12.50`, `-0.50`
    - `fee`: `NULL`, `-1234567890123456789012345678.0123456789`, `0.0000000001`,
      `9999999999999999999999999999.9999999999`, `0`, `-0.0000000001`, `1`
  - The later inserts keep their four columns (the new columns take their defaults); do not
    change them.
  - `step_outside_tool_reads_the_published_table`: keep its one `load_table` and its one
    scan (the consumers step counts exactly one read), and from that same Arrow table also
    assert: the types of the three columns are `pa.decimal128(9, 0)`, `pa.decimal128(12, 2)`,
    `pa.decimal128(38, 10)`; and, keyed by `region`, every `(units, net_amount, fee)` equals
    what ClickHouse holds, read with `SELECT region, toString(units), toString(net_amount),
    toString(fee) FROM <mart> ORDER BY region FORMAT TabSeparated` and parsed with
    `decimal.Decimal` (`\N` is `None`). Raise `GoldExportFailure` naming the first region
    that differs and both values.
  - Add the decimal proof to the module docstring's step 5.
- Check: the gate itself, in CI's job "Rust · Gold export to Iceberg (ADR 0010)". It fails
  on today's API (the export refuses the mart at the first decimal column). Locally only
  `python3 -m py_compile ops/gold_export/gold_export_test.py`; the run is *not verified
  locally, CI*.

### T9. The Table step asks the server about the named table (LH-DATA-35-03; DATA-47)

- Anchors: `src/services/contracts/uploads.ts:232-241` (`UploadListFilter`);
  `src/services/clients/uploads.ts:98-105` (`list`);
  `src/features/connectors/use-column-choices.ts:18`, `:71-99` (the pattern);
  `src/features/connectors/upload-file-page.tsx:65` (the wizard's props), `:72-74`
  (`tableName`), `:86-89` (the list and its comment), `:111-118` (`offersMode`, `tableFolder`),
  `:129-143` (`reset`), `:145-159` (`changeStep`, where the name is suggested), `:224-236`
  (`canProceed`), `:238-255` (`notices`), `:288-292` (the Table step's props), `:334`
  (`UploadFilePage`'s props); `src/lib/uploads.ts:213-225` (the doc of `isUploadedTable`);
  `src/features/connectors/upload-file-page.test.tsx:150-161` (`stubFetch`), `:970-1041` (the
  Table step tests).
- Change:
  1. Contract and client: `UploadListFilter` gains `table?: string` ("Exactly this raw table
     name: the uploads loaded into it, newest first"). `list` sends it as `table` when it is
     set and not empty.
  2. New file `src/features/connectors/use-table-uploads.ts` (`"use client"` first line):
     `TABLE_LOOKUP_DELAY_MS = 400`; the type `TableUploadsState` = `{ status: "idle" }` |
     `{ status: "checking" }` | `{ status: "known"; uploads: readonly Upload[] }` |
     `{ status: "error"; error: ServiceError }`; and
     `useTableUploads(table: string | null, delayMs: number)` returning `{ state, retry }`.
     `table === null` is `idle`. Otherwise it waits `delayMs`, calls
     `uploadService.list(signal, { table })`, and keeps the answer together with the name it
     asked; while the kept name is not the current `table` the state is `checking`. An
     aborted request sets nothing. `retry` asks again for the same name. Write the hook
     after `useColumnChoices`'s effect; say in its doc comment why it is not `useService`
     (the pause and the abort per key).
  3. `upload-file-page.tsx`:
     - A state `nameTyped` (false at start; false again in `reset` and where `changeStep`
       sets the suggested name; true in the Table step's `onTableName`).
     - `UploadFilePage`, `ExistingUpload` and `UploadWizard` take an optional
       `lookupDelayMs` (tests), passed down like `pollMs`.
     - `const lookup = useTableUploads(run === null && tableName !== "" &&
       tableNameProblem(tableName) === null ? tableName : null, nameTyped ? (lookupDelayMs ??
       TABLE_LOOKUP_DELAY_MS) : 0)`.
     - `offersMode` is `lookup.state.status === "known" && isUploadedTable(tableName,
       lookup.state.uploads)`; `tableFolder` reads `folderOfTable(tableName,
       lookup.state.uploads)` under the same condition.
     - `canProceed`: the Table branch and the Review branch also need
       `lookup.state.status === "known"`.
     - `notices`: on the Table and Review steps, while `checking` show the first sentence of
       D17 with `UploadNotice` (`./upload-parts`); on `error` show the second sentence, the
       error's message, and a "Check again" button calling `lookup.retry`, in the bordered
       block the failed-load notice uses (`:249-252`) with `role="alert"`.
     - Rewrite the comment at `:86-88`: the list is now read only for the run view
       (`:193-197`); the choice comes from the lookup, and a lookup that fails blocks Load.
  4. `src/lib/uploads.ts:213-225`: the doc of `isUploadedTable` says the list it is given is
     the named table's own uploads, asked from the API by name (the sentence "The API has
     no read for who owns a name" is no longer true). Keep the sentence about a deleted
     upload: that limit remains.
- Check, in `src/features/connectors/upload-file-page.test.tsx`:
  - Extend `stubFetch`: a `GET /api/uploads` with a `table` parameter answers
    `routes.tableUploads(table)` when the test gives one, and otherwise `routes.list`
    filtered to `bronzeTable === table` (what the API does). Existing tests keep their
    meaning. Where an existing test pressed Next or Load right after typing a name, it now
    waits for the button (`waitFor`); render with `lookupDelayMs={0}` where the pause would
    only slow the test. Do not remove or loosen an assertion (rule 2).
  - New tests in the "Table step" block:
    `offers Replace or Add and the table's folder for a table whose upload is not in the
    newest list` (the unfiltered list is empty; the lookup for `stock` answers an ingested
    upload in folder `uf-q1`: both radios are there, the folder field shows that folder, and
    one request went to `/api/uploads?table=stock`; fails today);
    `says the check failed, keeps Next off, and asks again on Check again` (the lookup
    answers 503 once, then the upload: the D17 sentence and the API's sentence show, Next is
    disabled, and after "Check again" the radios appear; fails today);
    `keeps Next off and says it is checking until the table check answers` (a lookup that
    answers later);
    `keeps Load off on the Review step while the table check has not answered`;
    `asks about a typed name once, after the pause, and uses only the answer for the name in
    the field` (three quick changes with `lookupDelayMs={30}`: one request, for the last
    name).
  - `src/services/clients/uploads.test.ts`: `list sends the table filter as table=<name> and
    nothing when it is empty`.

### T10. The card says "Last publish failed" (LH-DATA-1-01; DATA-49)

- Anchors: `src/services/contracts/gold.ts:125-134` (`GoldPublication`);
  `src/lib/gold-freshness.ts:10-20`; `src/features/catalog/open-format-card.tsx:77-81`
  (`stateLabelClass`), `:216-229` (the status line and the "off" note);
  `src/lib/gold-freshness.test.ts:5-14` (`basePublication`).
- Change:
  1. Contract: `export type GoldPublicationFailure = { failedAt: string | null; reason:
     string }` and `lastFailure: GoldPublicationFailure | null` on `GoldPublication` (required:
     the API always sends it after T5). Add `lastFailure: null` to every fixture the
     typecheck names.
  2. `freshnessLine`: first line `if (pub.lastFailure) return "Last publish failed"`; the
     rest as today. Update the module comment with the order of D9.
  3. Card: `stateLabelClass` returns `font-medium text-destructive` for "Last publish
     failed". Under the status line, when `p.lastFailure` is set, two lines with
     `data-testid="publication-failure"` on the first: the D17 failure sentence (time through
     `formatDateTime`, in `text-xs text-destructive`) and the D17 "Last successful publish"
     or "Nothing has been published yet." sentence (in `text-xs text-muted-foreground`).
- Check:
  - `src/lib/gold-freshness.test.ts`:
    `reports Last publish failed when the newest run failed, even if the last success matches
    the table` (`changedSinceExport: false`, `lastFailure` set; fails today: "Up to date");
    `reports Last publish failed for a mart whose only run failed`;
    `keeps the four other lines when lastFailure is null`.
  - `src/features/catalog/open-format-card.test.tsx`:
    `shows Last publish failed with the time, the reason and the last successful publish`;
    `says nothing has been published yet when the only run failed`.

### T11. The console reads "not published" from the answer, not from a failure (LH-DATA-1-01; DATA-49)

- Anchors: `src/services/contracts/gold.ts:26-40` (`GoldReadBack` and its doc);
  `src/features/gold/gold-exports-page.tsx:79-93` (the doc and the catch-all), `:111-124`
  (the two cells); `src/features/catalog/open-format-card.tsx:143-150` (the catch-all), `:178`
  (`showHistory`), `:235-244` (the rows).
- Change:
  1. Contract: `GoldReadBack` gains `published: boolean`; `formatVersion` and `rowsInIceberg`
     become `number | null`. Rewrite the doc: the route answers 200 with `published: false`
     for a mart never published; a failure is a failure.
  2. `gold-exports-page.tsx`: remove the `try`/`catch` (the state is
     `useService<GoldReadBack>`). Last-export cell: loading "…"; error
     `<span className="text-destructive">—</span>` like the neighbouring cells; `published`
     false or no `exportedAt`: "Never exported"; otherwise the time. Snapshot cell: the
     value only when `published`. Rewrite the doc comment at `:79-85`.
  3. `open-format-card.tsx`: keep `if (!mayReadHistory) return null`, remove the
     `try`/`catch`. `showHistory` also needs `lastExport.data.published`. When `hasHistory`
     and the read-back is in error, the "Last published" row shows "Could not be read" in
     `text-destructive` with the error's message as `title`.
- Check:
  - `src/features/gold/gold-exports-page.test.tsx`:
    `says Never exported for a mart the API reports as not published`;
    `shows a dash, not Never exported, when the read-back fails` (fails today).
  - `src/features/catalog/open-format-card.test.tsx`:
    `shows no last-published rows for a mart the API reports as not published`;
    `says the published copy could not be read when the read-back fails`.
  - Tests whose mock answered 500 for a mart never published now answer the D11 body; say
    in the commit that the API changed, not the assertion (rule 2).

### T12. "Copies kept" says when it was counted (LH-DATA-1-01; DATA-49)

- Anchor: `src/features/catalog/open-format-card.tsx:245-253`;
  `src/features/catalog/open-format-card.test.tsx:348` (the test of this row).
- Change: the label is "Copies kept at last trim"; the value is the D17 text: the number,
  then in brackets `counted <formatDateTime(p.trim.finishedAt)>` when `finishedAt` is not
  empty, and `oldest <time>` when there is one, joined by `; ` (no brackets when neither).
  A comment says why: the number is the trim's, and a publish after it is not counted.
- Check: update the test at `:348` to the new label and value (the label changes by D17;
  say so in the commit) and add
  `does not show a counted time when the trim recorded none`.

### T13. The two feature pages say what is now true (DATA-47, DATA-48, DATA-49)

Edit only these lines.

- `docs/core/features/gold-publish-per-mart.md:26-27`, item 3 becomes:
  `3. See on the table's page: last published time, the snapshot ID, and one of "Up to
  date", "Out of date", "Not measured", "Never published", or "Last publish failed" (with
  the time and the reason) when the most recent publish failed.`
- Same file, under "Limits to tell a customer", after the bullet that ends "An operator can
  raise the limit." (`:67-68`), add:
  `- A decimal column is published as a decimal of the same precision and scale. A decimal
  wider than 38 digits is refused, and the refusal names the column.`
- `docs/core/features/upload-folders.md:65`, the last sentence ("The Table step offers the
  folder of the newest listed upload of the table, so it can differ from the remembered one
  after files were moved.") becomes: `The Table step asks for the table's own uploads by its
  name and offers the folder of the newest one, however old, so it can differ from the
  remembered one after files were moved.`

No acceptance row changes. Row 10 of `gold-publish-per-mart.md` ("status does not say up to
date") is the row T5 and T10 make true.

## 3. Verification

This build machine has 11 GB of RAM and little disk. Run cargo only through
`/home/shiro/rantai/bin/cargo-locked`, from the worktree's `rust/` directory, never
`cargo clean`, never two cargo commands at once.

Per commit, only for what the commit touched:

- Rust: `cd rust && cargo fmt --check`, then
  `/home/shiro/rantai/bin/cargo-locked check -p <crate> -j 2` for each crate touched
  (`lakehouse-store`, `lakehouse-iceberg`, `lakehouse-api`). T1 also changes files under
  `rust/crates/lakehouse-store/tests/`: add `--tests` to that one check.
- Rust tests that may run here: `cargo-locked test -p lakehouse-iceberg --lib wiremock_tests
  -j 2` (T6) and, only as `--lib` with a filter and only when nothing else heavy is running,
  `cargo-locked test -p lakehouse-api --lib <test name> -j 2` for the unit tests of T3 to T7.
  Never `cargo test -p lakehouse-api` without `--lib`. If a `--lib` run is not made, write
  *not verified locally, CI* for those tests.
- Left to CI, and listed in the handoff as *not verified locally, CI*: `cargo clippy`, every
  test under `rust/crates/lakehouse-api/tests/` (`upload_folder_routes.rs`,
  `gold_publication.rs`, `sec11_guard.rs`, `route_auth.rs`), the `sqlx::test` of T1 unless a
  Postgres is reachable, and the Gold export gate (T8).
- TypeScript: `bun run typecheck`, `bun run lint`, and `bun test` on the touched test files:
  `src/features/connectors/upload-file-page.test.tsx`, `src/services/clients/uploads.test.ts`,
  `src/lib/gold-freshness.test.ts`, `src/features/catalog/open-format-card.test.tsx`,
  `src/features/gold/gold-exports-page.test.tsx`, `src/lib/uploads.test.ts`,
  `src/lib/upload-folders.test.ts`.
- Python: `python3 -m py_compile ops/gold_export/gold_export_test.py`. Nothing under
  `dagster/` changes, so no pytest.
- Always: `python3 ops/lint/check_bare_iceberg_count.py && python3
  ops/lint/check_intra_package_imports.py`.

Before handoff, once, in the foreground, on the final commit:

```bash
cd rust && cargo fmt --check
/home/shiro/rantai/bin/cargo-locked check -p lakehouse-store -p lakehouse-iceberg -p lakehouse-api -j 2
bun run typecheck && bun run lint && bun run test
python3 ops/lint/check_bare_iceberg_count.py && python3 ops/lint/check_intra_package_imports.py
git status   # nothing generated staged
```

No compose file changes, so no `docker compose` check. Do not run prettier. The handoff
quotes each command with its counts, names every Rust test added or changed so it can be
found in the CI log, and says *not verified* for anything not run (rule 7).

What the planner could not verify, and CI must: what ClickHouse 26.8 prints for
`toString` of a decimal (the parser of D14 takes the text with or without trailing zeros);
that the `ALTER TABLE … UPDATE` of T4 is accepted as written; that the export in the test
harness of T3 fails at the catalog and not earlier; that `pyiceberg` reads the three
decimal storage forms. If any of these answers differently, the developer reports the
answer and stops; the plan is corrected, not the assertion.

## 3a. Change of machine (planner, 2026-11-12)

The developer works on the ARM64 build machine, in the checkout
`/home/rismana/rantai/lakehouse-agents-1` (20 CPUs, 121 GB of RAM), not on the machine
section 3 describes. Section 3's list of what to run stays; how to run it changes:

- Before the first cargo command of a shell: `export CARGO_TARGET_DIR=/home/rismana/rantai/cargo-target-1 CARGO_BUILD_JOBS=6`.
  There is no `/home/shiro/rantai/bin/cargo-locked` here: where section 3 writes
  `/home/shiro/rantai/bin/cargo-locked <args> -j 2`, run `cargo <args>`.
- Per Rust commit, for each crate the commit touched: `cargo fmt --check`,
  `cargo clippy -p <crate> --all-targets -- -D warnings`, and the unit tests the task names,
  as `cargo test -p <crate> --lib <test name>`. These are no longer left to CI.
- An integration test file under `rust/crates/lakehouse-api/tests/` that a task adds to or
  changes is run as `cargo test -p lakehouse-api --test <file>` (Docker is running here). Never
  a bare `cargo test -p lakehouse-api <filter>`. If a container image has no ARM64 build or a
  test cannot start for a reason of the machine, write *not verified locally, CI* with the
  error's first line; do not change the test.
- Before the first TypeScript check: `bun install --frozen-lockfile`.
- CI on the pull request (x86_64) is still the final word. The handoff quotes each command
  with its counts and says *not verified* for anything not run (rule 7).
- The machine is shared with another project: start no container you do not remove, stop no
  process you did not start, and work only under `/home/rismana/rantai/`.

## 4. Out of scope

- **Publish now on an unchanged mart adds a full copy.** This is how the feature was
  planned: the manual trigger sends no `ifChanged`
  (`docs/superpowers/plans/2026-10-02-gold-publish-per-mart.md`, "The console's manual trigger
  does not pass `ifChanged`"; `routes/gold.rs:120-126`), and the card says "Publishing adds
  one full copy each time". Changing it is a decision, not a bug fix (question 1).
- **Times with no zone.** Read at this commit, the card formats every time with
  `formatDateTime` (`src/lib/format.ts:94`), which uses the browser's zone and names none, so
  the code does not show UTC by itself; the finding was not reproduced by reading. Naming
  the zone means changing `formatDateTime`, which 34 files use (question 2).
- **A live count of copies.** The card shows the count of the last trim (T12 says so). A
  count on every page load would scan the published table.
- **A partial newest copy after a run that failed between batches.** The export appends
  batch by batch and cannot take a batch back (`gold_export.rs:646-650`). D10 makes the next
  scheduled run publish again; removing the partial copy is not in this slice.
- **A table whose upload rows were all deleted** still gets no Replace / Add choice (the
  documented limit of `isUploadedTable`). Answering it needs the table's claim
  (`uploads::table_claim`) and a check that the table exists, which is a new route
  (question 3).
- **A table whose newest 100 uploads all failed to load** while an older one succeeded gets
  no choice: the `table=` list is cut at the same 100 as every list.
- **The assistant's `export_gold_mart` writes no history row**
  (`routes/ai/tools/gold.rs:116-147`), so a publish started from the chat does not appear in
  the card's history or status. Not a QA finding; noted for the backlog.
- **The gold-exports page asks for the read-back and the run list without checking
  `canEdit`** (`gold-exports-page.tsx:87-98`); a person without `gold:export` sees dashes
  there. The card was fixed for this in part A; the page is a separate item.
- **A mart that gains a decimal column after it was first published.** The published
  table keeps the schema of its first publish (`catalog.rs:360-377`); schema change is not
  part of this slice.
- **A mask on a decimal column that turns it into text.** The run fails with the D7 `Batch`
  sentence, as a mask on an integer column does today.
- Everything else in the QA round that is not on rows `DATA-47`, `DATA-48`, `DATA-49`.

### Open questions for the owner

None of these blocks the plan.

1. Should "Publish now" skip an unchanged mart (send `ifChanged=true`, and say "Nothing to
   publish: the table has not changed since `<time>`"), while still publishing after a
   failed run? The planner's proposal is yes; it is one console change and one sentence.
2. Should every time in the console name its zone (for example "06 Nov 2026, 10:41
   GMT+7")? If yes it is a console-wide change with its own backlog item.
3. Should a table whose uploads were all deleted still offer Replace / Add, from the
   table's claim? If yes it needs a small route and its own backlog item.

## 5. Handoff

## 6. Review
