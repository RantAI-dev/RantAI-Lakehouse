# BI follow-ups: export safety, reports and print view, alerts and hygiene

Planner: Claude Opus (hub session). Developer: a non-Opus agent (Sonnet). Base: `origin/main` at
`6247400f` (2026-11-13). The product owner wants the open `BI` and `RPT` follow-ups closed this
week. This plan covers the Todo items that need code: `BI-27`, `BI-28`, `BI-29`, `BI-30`,
`BI-33`, `BI-34`, `BI-35`, `BI-36`, `BI-42` and `BI-43`. The item texts in `docs/core/BACKLOG.md`
are the brief; this plan adds the decisions and the slicing.

Anchors are file names from the backlog rows and a first look at `main`. **The developer
verifies every anchor at `6247400f` before changing code, and stops and reports if one is wrong**
(AGENTS.md, "the plan is wrong against the code").

## Decisions already made

- **D1, `BI-43` (owner, 2026-11-13).** List the person's own alerts for a chart in the chart's
  alert dialog, with remove. The alert action stays visible to anyone who may save an alert. A
  person sees and removes only the alerts they created. Removing someone else's alert still
  needs `alert:write` on the Alerts page.
- **D2, `BI-30`: one CSV owner.** `csv_safe::neutralize_csv` (from #86) is the one rule for
  formula neutralising. Quoting and the BOM move to one shared Rust helper that both
  `csv_export.rs` and `upload_workbook.rs` call. `src/lib/csv.ts` neutralises with the same rule
  as the Rust one, and a shared test vector file proves the two agree.
- **D3, `BI-27` and `BI-34`: bounded exports.**
  - A whole-result export holds one copy of the rows: take them, don't clone.
  - Every export statement carries `max_memory_usage`, a byte cap and a timeout. A cut export
    says so in the file name and the audit row, as `BI-1` already does for its row cap. It is
    never silent.
  - Concurrent downloads are limited per process, the same way as the PDF's two slots. A
    refused download answers 429 with a fixed sentence.
  - The XLSX row writes run in `spawn_blocking`.
  - **Values:** the developer picks them from the existing PDF and `BI-1` settings, states them
    in the handoff, and makes them configurable the same way those are.
- **D4, `BI-28`: honest exports.**
  - Number columns are decided from `result.meta`, so negative `Int64` values are written as
    numbers, not `'-5`.
  - When a tile skipped rows (`RecordsSql.skipped`), the file carries a first-row note in CSV and
    a note sheet in XLSX, saying how many rows were skipped and why.
  - A raw table with a calculated column exports. If one calculated column truly cannot be
    computed in the export path, it is exported empty with a note, never as a 400.
- **D5, `BI-35`: scheduled reports.**
  - A requester cannot approve their own outside send, even with `agent:approve`. The approval
    must come from another person.
  - "Send now" is rate-limited per subscription. One send per subscription per 60 s is the
    default, and it is configurable.
  - A report cut by the 310 s route budget is recorded as failed, with the reason, in the run
    history and the audit.
  - The subscriptions dialog uses `useServiceAction`.
  - Console Delete of a subscription asks for confirmation, with the subscription's name.
- **D6, `BI-36` and `BI-42`: the print view.**
  - The print view (`render-view.tsx`, `/render`) reads uploaded map boundaries through a render
    token route, as the public and embed views do since #246.
  - The render token carries the board's content sites, as the embed frame call does, so image
    and web cards draw in a PDF or a PNG. No wider policy than the board's own sites.
- **D7, `BI-29`: services boundary.** Dashboard feature code reaches clients only through
  `@/services`. `filters/use-filter-values.ts` moves to `useService`. A lint rule
  (`no-restricted-imports` for `@/services/clients/*` under `src/features/`) keeps it that way.
- **D8, `BI-33`: the #199 review leftovers.**
  - The AI-15 case `dash_map_charts_in_separate_tab` gets a second chart in its setup.
  - The XSS test covers the sankey edge, the boxplot outlier and the scatter-label branches.
  - The wrong comment in `chart-option.ts` is corrected.

## Slicing: three PRs, built one after another (the VM is shared and busy)

| PR | Branch | Items | Languages |
|---|---|---|---|
| A | `fix/bi-export-safety` | `BI-27`, `BI-34`, `BI-28`, `BI-30` | Rust, TS |
| B | `fix/bi-reports-render` | `BI-35`, `BI-36`, `BI-42` | Rust, TS |
| C | `fix/bi-alerts-hygiene` | `BI-43`, `BI-29`, `BI-33` | Rust, TS |

- One backlog item per commit, in the order above. The commit messages cite the item IDs.
- PR titles start with the first item ID, for example `BI-27 fix(api): …`.
- PR B branches from `main` after A is opened (not merged), and so does PR C after B.
  Conflicts are resolved hunk by hunk.

## Acceptance checks per item

- `BI-27`, `BI-34`: a test proves an export over the byte cap is cut and labelled. A test proves
  a third concurrent download gets 429. The XLSX write path is in `spawn_blocking` (code
  review). No row clone remains on the whole-result path.
- `BI-28`: tests for a negative `Int64` (written as a number), a skipped-rows note, and a raw
  table with a calculated column exporting with status 200.
- `BI-30`: one Rust helper, and a shared vector test that passes in Rust and in TS.
- `BI-35`: tests for self-approval refused, a second Send now within 60 s refused, and a
  timed-out run recorded as failed. Console: Delete asks first. The dialog uses
  `useServiceAction`.
- `BI-36`, `BI-42`: a render-view test proving the map boundary loads through the token route,
  and a test that the render token's policy contains the board's content sites and nothing else.
- `BI-43`: an API test that a person lists and removes only their own alerts for a chart, and
  is refused for another person's. The console dialog shows "Your alerts on this chart" with
  remove.
- `BI-29`: the lint rule is in, and `bun run lint` is clean.
- `BI-33`: the three tests are extended, and the comment is fixed.

## Verification

Use the AGENTS.md block for the languages each PR touched, run once on the PR's final commit,
in the foreground:
- Rust: `cargo fmt --check`, clippy with `-D warnings` and `cargo test --workspace`, with the
  shared `CARGO_TARGET_DIR`;
- TS: typecheck, lint and tests.

The VM is shared and busy, so build with `nice` and `CARGO_BUILD_JOBS=2`.

## Out of scope

- The rest of `BI-17` (forecasting, clustering) and `BI-18` (reports, templates).
- `BI-3`, `BI-20`.
- `BI-16` close-out (docs only, planner).
- `BI-31` part 3 and `BI-32` (owner checks on a running stack).

## Handoff

(The developer appends one entry per PR: commits, the commands run with counts, the values
chosen for D3 and D5, and anything not verified.)

### Handoff, PR A (developer, 2026-11-14)

Branch `fix/bi-export-safety`, base `13e5675e` (plan commit), merged with `origin/main`
(a merge, no conflicts, includes OPS-32). Not pushed; no PR opened. All commits as
`mtharreqq <72479829+mtharreqq@users.noreply.github.com>`.

| Commit | Item |
| --- | --- |
| `cff1b996` | `BI-27`: 1 GiB byte cap, 280 s engine time limit, a cut file says which limit cut it |
| `1b93dc4f` | `BI-34`: `max_memory_usage` on the row and byte streams (2 GiB), two downloads at a time (third is 429, fixed sentence, scheduled reports never take a slot), XLSX writes and save on the blocking pool in 1,000-row batches |
| `db7b5bb1` | `BI-28`: shown and sort columns go into the relation query (calculated column exports), plain-number strings in numeric columns have no apostrophe, skipped filters are said by file name suffix `-filters-not-applied`, `x-export-note` header, the audit row and a `Note` sheet (CSV/TSV body unchanged) |
| `2bc12368` | `BI-30`: `csv_safe` owns `neutralise`, `quote_field`, `BOM`; `csv_export` and `upload_workbook` call them; shared vectors in `tests/fixtures/csv_neutralise_vectors.json` |
| merge | `origin/main` |

**Values chosen.** D3: byte cap 1 GiB, engine time 280 s, memory 2 GiB (`EXPORT_MEMORY_BYTES`).
D5: 2 download slots (`EXPORT_AT_ONCE`), try-acquire, 429 with `Retry-After` 10 s.

**Decision to confirm (BI-28).** The plan's D4 asked for a first-row note in CSV. The planner's
amendment kept the CSV/TSV body untouched, so the note is outside the body (name, header,
audit, `Note` sheet). Also, `upload_workbook` shares only the quoting and the mark with
`csv_safe`; it does not neutralise, because it keeps the workbook's cell text as data.

**Behaviour change (BI-30).** A text cell like `+1` or `-1` in a dashboard download is now
written unchanged (a plain number is data under `csv_safe` and `src/lib/csv.ts`). The
`csv_export` test that expected `'+1` and `'-1` was updated.

**Commands actually run** (no Rust compiles on the VM from 2026-11-14, owner decision):
- Before the rule, on the BI-27 commit: `cargo clippy -p lakehouse-api --all-targets -- -D warnings` exit 0; `cargo test -p lakehouse-api --lib -- export cut` 163 passed, 0 failed, 1 ignored.
- BI-34 tree: `cargo clippy -p lakehouse-clickhouse -p lakehouse-api --all-targets -- -D warnings` exit 0; `cargo test -p lakehouse-clickhouse` 43 passed, 0 failed.
- BI-28 and BI-30: `cargo fmt --check` clean (also after the merge); `bun test src/lib/csv.test.ts` 24 passed; `bash scripts/ci/check_test_modules.sh` passed after the merge.

**Not verified (CI).**
- Every Rust compile and test of the BI-34 `lakehouse-api` tests (the third download is a 429; a 2,500-row workbook; the byte-cap cut at 1,077 rows), and of all BI-28 and BI-30 code and tests. A clippy run that included BI-28 hit one `too_many_lines` lint in `tests/dashboard_export.rs` (fixed with an `allow`) and was not run again.
- The new integration test `a_calculated_column_exports_and_a_skipped_filter_is_said_outside_the_body` (it mocks one `bi_field` row, formula `[amount] * 2`; a type or formula check on the mock's `String` columns may need adjusting).
- The full per-crate test block, `tests/sec11_guard.rs` (BI-28 adds the `x-export-note` header with a fixed sentence; no error text goes into a response), `cargo test --workspace`, and the TS `typecheck`, `lint` and full `bun run test`.
- Intermediate commits were not compiled one by one. In `db7b5bb1` alone, the new long-integer test expects no apostrophe in a text column, which holds only with `2bc12368`.

**Findings.**
- A full `lakehouse-api` test compile is OOM-killed on this 11 GB VM: `rustc` reached 6.4 GB (no debuginfo) to 8.3 GB (debuginfo) of anon-rss, five kills in all. It only fits with `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_INCREMENTAL=0`, one job and nothing else running. Use CI.
- Query Studio's download keeps its own 5 GB cap and sends no `max_memory_usage` (`max_memory_usage: None` in `routes/query.rs`); `query_with_options` (buffered) does not send it either. Left for a follow-up.
- Already present at `6247400f`: rows stream to an unlinked temp file (no row clone), `EXPORT_TIMEOUT` is 300 s, a raw table over a SQL source carries `max_execution_time = 280`.

## Review

### Review, PR A (planner, Opus, 2026-11-14)

No open `BLOCKER`, no `SHOULD-FIX`. Read against this plan and AGENTS.md:

- **`BI-27`:**
  - The byte cap and the engine time limit are set on the dashboard download path.
  - A cut file says which limit cut it, in the file name, the `x-export-cut-by` header and
    the audit row.
- **`BI-34`:**
  - `max_memory_usage` is on the row and byte streams.
  - The two export slots are try-acquire with a fixed 429 sentence (`EXPORT_BUSY`), and
    scheduled reports take no slot.
  - XLSX rows are written on the blocking pool in batches.
- **`BI-28`:**
  - The shown and sort columns go into `relation_query`.
  - A plain-number string in a numeric column is written as a number.
  - The "filters not applied" note is a constant, outside the CSV or TSV body.
- **`BI-30`:**
  - `csv_safe` owns the rule, the quoting and the BOM.
  - The plain-number exemption matches decisions D7/D8 and `src/lib/csv.ts`. `-1+1` still
    gets the apostrophe; `-5` stays a number.
  - `upload_workbook` shares only quoting and the BOM. That is right: it is data being
    loaded, not a file leaving.
- **Security:** no route line puts error text in a response, and the new header and note
  texts are fixed constants.

**Verification.** Owner decision 2026-11-14: no Rust compiles on the shared VM, so the
review's re-run is CI on PR #527.
- CI run on `febc03f6`: three of the developer's new tests failed. The fixes are in
  `c709e869`:
  - one test read the wrong part of the XLSX;
  - one byte count left out the header's 10 bytes;
  - one mock typed a numeric column as `String`.
- CI on `c709e869`: 45 checks pass, 2 are skipped (Dagster, unchanged).
  - Partition 2/3 first failed on `upload_routes::two_simultaneous_ingests_of_one_upload_launch_once`
    (`left: 1, right: 2`). It is a timing test outside this PR's paths. It passed on the
    re-run with no change, and is recorded under `REL-27`.
- Run locally by the planner: `cargo fmt --check`, `scripts/ci/check_test_modules.sh`,
  `bun run typecheck`, `bun run lint` (0 errors), `bun test src/lib/csv.test.ts` (24 pass).

**Open points carried to the backlog, not this PR:**
- the Query Studio download has no `max_memory_usage`;
- the buffered `query_with_options` does not send it either.
