# SEC-11 No raw upstream errors in responses — Implementation Plan

**Status:** decisions signed 2026-10-10, not started. Written by the planner
(Claude Opus) for a developer agent, under the role split in `AGENTS.md`.

**Feature page:** `docs/core/features/no-raw-upstream-errors.md`. Spec:
`docs/core/specs/sec-11.md`.

**Base commit:** `9cff2c2` on `main`. Branch `fix/sec-phase-0` (one branch
for roadmap phase 0, the owner's choice; one commit series per item).

**Goal:** no response body carries text that came from `ClickHouse`,
Postgres, the catalog, an LLM provider, a connector driver or any other
upstream. The user gets a fixed message and a reference id; the raw text
goes to the log under that id.

---

## 1. Decisions already made (do not re-ask, do not change)

| Decision | Choice |
| --- | --- |
| Scope | Every handler and tool under `rust/crates/lakehouse-api/src/routes/`, plus what they call to build a response body. |
| What is "upstream" | Any error whose text we did not write: `ChError`, `sqlx`/store errors, `reqwest`, the catalog/REST client, LLM client errors, driver errors (`tiberius`, …), `serde` errors over upstream payloads, I/O. |
| What stays | Messages this code base writes: validation (`BiError::Validation`, `ApiError::BadRequest` with our text), not-found, permission and policy refusals (`enforcement_error_message`), `supported: false` reasons. |
| Shape | One helper, used everywhere, that logs the upstream error at `error` (or `warn` for expected unavailability) with a generated reference id and returns our fixed message plus the id. No per-file copies. |
| Status codes | Unchanged, except where the status was computed from upstream text. |
| Guard | A test that fails when a route builds a response from an upstream error's text (`SEC-11-AC1`), with an explicit, reasoned allowlist for our own messages. |
| Known gap in `AGENTS.md` | The "fourteen Phase-1 handlers" using `ApiError::Internal(err.to_string())` are in scope; when they are gone, the sentence in `AGENTS.md` is corrected by name (principle 5). |

## 2. What exists today (anchors, verified at `9cff2c2`)

- Classifiers already in the tree, each private to its file: `routes/lakehouse.rs` (`classify_rest_error`, `classify_ch_error`, `classify_capacity_ch_error`), `routes/dashboard.rs:856` (`classify_dashboard_ch_error`), `routes/dashboard_sources.rs:93` (`classify_ch_error`), `connector_probe.rs` (`classify_tiberius_error`, `classify_reqwest_error`). Read them first; the new helper generalises them, it does not add a seventh.
- Tile errors: `routes/support.rs` `run_spec_sql` puts the `ClickHouse` message into the tile's `error`; `routes/dashboard.rs` and `routes/embed.rs` return it, and `embed.rs` also returns `err.to_string()` in 500 bodies for the public and embed routes (unauthenticated callers).
- The assistant: `routes/ai/mod.rs` and `routes/ai/tools/*.rs` return `json!({ "error": err.to_string() })` in about thirty places.
- A rough count of candidate sites, by `grep` for `err.to_string()`, `format!("Error: {err}")` and similar outside log macros: about 130 across 28 files under `routes/`, most in `dashboard.rs` (24), `connectors.rs` (19), `ai/mod.rs` (12), `ai/tools/dashboards.rs` (11), `alerts.rs` (9). Some are in tests or already carry our own text; the developer classifies each.
- The console shows a tile's `error` string as given (`src/features/dashboards/tile-body.tsx`).

## 3. Tasks

One commit per task.

### T1 — The helper and the reference id

- Find whether a request or correlation id already exists (middleware, `tracing` span field). If it does, reuse it as the reference id; if not, generate a short random id per reported error. Do not add a dependency for this.
- Add one module (for example `rust/crates/lakehouse-api/src/upstream_error.rs`) with: a function that takes the upstream error (as `&dyn Display` or the concrete types), a short static context ("dashboard tile", "catalog"), logs `error!`/`warn!` with the id and the raw text, and returns a small value holding our fixed message and the id; and conversions to `ApiError` and to the tile/tool JSON shape (`{ "error": "<fixed>", "errorId": "<id>" }`).
- The fixed messages are few and plain: "This chart could not be loaded.", "The database is unavailable.", "The request to <service> failed." Keep unavailability (connection refused, timeout) distinct from failure, as the existing classifiers do, because the status codes differ.
- Fold the existing private classifiers into it where they do the same job; keep any that carry real extra meaning (for example mapping a not-found to 404 naming the table) and say why at the site.

*Accept:* unit tests: the returned message never contains the upstream text (property over a few planted markers); the log line carries the id and the raw text (use the repo's existing way of asserting logs, or a test subscriber); unavailability and failure map to the statuses used today.

### T2 — Tiles, public and embed

`routes/support.rs`, `routes/dashboard.rs`, `routes/embed.rs`, `routes/dashboard_sources.rs`: every tile result and every 4xx/5xx body goes through T1. The public and embed routes are unauthenticated; they are the priority.

*Accept:* route tests with a mock `ClickHouse` returning an error with a planted marker: the marker is absent from the dashboard payload, the public payload, the embed payload and the 500 body; `errorId` is present; our own validation messages (a malformed filter, a refused SQL source) are unchanged.

### T3 — The assistant

`routes/ai/mod.rs` and `routes/ai/tools/*.rs`: tool results and chat errors go through T1. A tool's own refusals (`supported: false`, "permission required", dry-run refusals) are unchanged. LLM provider errors are upstream.

*Accept:* tests per tool family with a planted marker; the existing tool-schema fixture test still passes (regenerate only if the schema itself changes, which it should not).

### T4 — The remaining routes

Everything else under `routes/` the sweep finds: `connectors.rs`, `alerts.rs`, `pipelines.rs`, `identity.rs`, `governance.rs`, `gold.rs`, `query.rs`, `catalog*.rs`, `overview.rs`, `auth.rs`, `agents.rs`, `storage.rs`, `quality.rs`, `knowledge.rs`, `authored_pipelines.rs`, and the files the spec names (`catalog.rs`, `connector_deprovision.rs` if present). For each site decide: upstream (fix) or our own message (leave, and add to the guard's allowlist with the reason).

*Accept:* a marker test for at least one site per file touched.

### T5 — The guard (`SEC-11-AC1`)

A test under `rust/crates/lakehouse-api/tests/` (or `ops/lint/` if a source scan fits there better; look at how `ops/lint/check_bare_iceberg_count.py` and `tests/route_auth.rs` do their jobs and choose the one that can be precise) that scans `src/routes/**` and fails on a response built from an error's text outside the helper. It must be precise enough not to need a blanket allow: an allowlist entry names the file, the pattern and the reason. Prove it bites: the handoff quotes the failure it prints when a violation is added on purpose and then removed.

### T6 — Console

`tile-body.tsx` and the public/embed views show the fixed message and, small and selectable, "Reference: <id>" when `errorId` is present. The assistant's chat shows a tool failure the same way if it renders tool errors. No new dependency. Update the contract type.

*Accept:* typecheck, lint, unit test for the rendering.

### T7 — Docs

`CHANGELOG.md` `[Unreleased]` under Security; correct the "fourteen Phase-1 handlers" sentence in `AGENTS.md` to what is true after this change (zero, or the named remainder).

## 4. PR slicing

Phase 0 ships as one branch. This item is commits T1…T7 on it.

## 5. Out of scope (do not build)

An error browser; changing log format or retention; status-code clean-ups unrelated to upstream text; errors in the Dagster code location or other services.

## 6. Things the developer must verify, not assume

- Whether a request id already exists and reaches the handlers.
- For every candidate site, whether the text is upstream or ours. When in doubt it is upstream.
- That no test was weakened: a test that asserted on upstream text in a response is asserting the bug; change it to assert the fixed message and say so in the handoff.
- That the log line does not itself leak a secret the upstream error may contain (connection strings with passwords): check what `ChError`, the store error and `reqwest::Error` print, and redact if any of them can carry a credential.

## 7. Handoff (developer appends one entry per PR)

### SEC-11 — T1–T7 (developer, 2026-10-10)

Nothing is committed, stashed or pushed (product owner QA first). All edits are in the working tree of `fix/sec-phase-0` at `9cff2c2`; the plan file got only this section 7.

**Files per task** (so the work can be split into commits)

- T1: `rust/crates/lakehouse-api/src/upstream_error.rs` (new), `src/lib.rs`, `src/main.rs` (module registration). `uuid` (already a dependency) makes the reference.
- T2: `routes/support.rs`, `routes/dashboard.rs`, `routes/dashboard_sources.rs`, `routes/dashboard_folders.rs`, `routes/embed.rs`; the `?`-on-`ChError` fallout in `routes/gold.rs`, `routes/catalog_profile.rs`, `routes/query.rs`; `crates/lakehouse-clickhouse/src/lib.rs` (the `From<ChError> for ApiError` impl removed), `src/error.rs` (its test rewritten); `tests/upstream_errors.rs` (new).
- T3: `routes/ai/mod.rs`, `routes/ai/tools/{alerts,dashboards,queries,pipelines,data}.rs` (connectors, gold, governance only got comments/allowlist reasons).
- T4: `routes/{alerts,agents,query,connectors,pipelines,catalog,ops,overview,storage}.rs` (the last four only for the `js_error` -> `upstream_message` rename), `tests/ingest_run.rs` (one assertion changed).
- T5: `tests/sec11_guard.rs` (new).
- T6: `src/components/error-reference.tsx` and `.test.tsx` (new), `src/features/dashboards/{tile-body,embed-view,public-dashboard,dashboard-preview,tile-dialogs,chart-builder}.tsx`, `src/features/copilot/{tool-step,use-copilot,chart-draft-card}.tsx`, `src/services/contracts/dashboards.ts` (`TileFailure`).
- T7: `CHANGELOG.md`, `AGENTS.md`, `docs/CODE-STANDARD.md`.

**Sites.** About 84 call sites now go through `upstream_error::*` and 39 through `upstream_message` (the renamed `js_error`, which now returns the fixed sentence plus a reference). Left as they are, each with a reason in the `ALLOWED` list of `tests/sec11_guard.rs` (44 entries): serde errors about the caller's own request body (about 20), our own validators' messages (`Ident`, transform grammar, secret-value, ingest-spec, debezium), `StoreError`'s Display (fixed "database error"), classification-only reads of the text, and serialising our own `Value`. Already-classified fixed messages elsewhere (governance, lakehouse, health, dashboard `classify_*`) were folded into the helper where they served tiles and dashboards (`dashboard.rs`, `dashboard_sources.rs`, `dashboard_folders.rs`) and left alone otherwise (`routes/governance.rs`, `routes/lakehouse.rs`, `health.rs`): they log and return fixed text but carry no reference id.

**Commands** (all foreground-equivalent, from `rust/` with `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-uiux-target CARGO_BUILD_JOBS=2`)

- `cargo fmt --check`: clean (I ran `cargo fmt -p lakehouse-api -p lakehouse-clickhouse`; the 42 modified files were the same before and after).
- `cargo clippy -p lakehouse-api -p lakehouse-clickhouse --all-targets --all-features --locked -- -D warnings`: clean (after `touch` of both `src/lib.rs`; the last run took 4 s because only test files had changed since the run that compiled the libs and the bin). The one edit after that, `tests/ingest_run.rs` (assertion only), was not re-linted: *not verified* for clippy.
- `cargo test -p lakehouse-api --lib`: 1502 passed, 0 failed, 1 ignored.
- `cargo test -p lakehouse-api --tests --no-fail-fast`: 38 targets, 3350 passed, 0 failed, 4 ignored (includes `route_auth` 30, `security_regressions` 10, `sec11_guard` 3, `upstream_errors` 6, `ingest_run` 8). An earlier run died with "No space left on device" inside the test Postgres container; the rerun passed.
- `cargo test -p lakehouse-clickhouse`: 21 passed.
- `bun run typecheck`: clean. `bun run lint`: 0 errors, 6 warnings, all in files this change did not touch. `bun run test`: 938 pass, 1 skip, 0 fail (939 tests, 103 files).
- Guard bites: a `fn bite_probe(err: &dyn Display) -> String { err.to_string() }` added to `routes/home.rs` failed `no_route_builds_a_response_from_an_upstream_errors_text` with `src/routes/home.rs:35: fn bite_probe(...)`; removed (`git diff` of that file is empty).
- Real `ClickHouse` shape: `docker exec lakehouse-clickhouse-1 clickhouse-client -q "SELECT x FROM serving.no_such_table_for_test"` answers `Code: 60. DB::Exception: ... Unknown table expression identifier '<table>' in scope SELECT ... (UNKNOWN_TABLE)` plus the echoed query. Over HTTP that is a non-2xx with the same body, i.e. `ChError::Server(text)`, which `report_ch` classifies as a failure (logged at `error`, the text, table name and query echo only in the log). The mock in the tests uses the same `Code: N. DB::Exception: ... (version ...)` shape.

**Not verified.** No browser. The dev servers were not started. The sweep test for the embed route reaches only the early "jwt" check on a bad token (it proves no marker, not a failing tile inside a signed embed); the failing-tile path itself is covered by `routes::support` unit tests and the public-link test. `queries` assistant tool against a failing Postgres pool is covered only through `upstream_error::store` unit tests. The `ALLOWED` claims "ours" were checked by reading each site, not by a type-level proof. Disk fell to 8 GB free at the end (`df -h /`), below the 10 GB line you set; I stopped running cargo then.

**Deviations from the plan**

1. `lakehouse-clickhouse` no longer has `From<ChError> for ApiError` (plan did not name it). It was the source of the largest class of leaks (every `?`), and removing it makes the compiler list each site and refuse new ones; the plan's guard test is the second half. Its two unit tests and the one in `error.rs` asserted the leak and were replaced.
2. The Trino exception (`map_trino_error` forwarded `TrinoError::Query` verbatim, documented as safe because the caller wrote the SQL) is removed, so the SQL editor shows a fixed message and a reference for both engines. This reverses a documented decision; one line to restore, noted at the site. The same is true of the editor's `ClickHouse` errors and the cost-estimate error. Product-owner call.
3. `js_error` (about 60 uses) became `upstream_message`, which returns a fixed sentence plus a reference; where a site needed `errorId` as data it uses the typed helpers.
4. Status changes: an unreachable `ClickHouse` is `503` on the routes that now use `ch_error` (was `422`, or `500` in a few); a `lakehouse_bi` error saved through `dashboard.rs` now follows `classify_bi_error` (`503` for a `ClickHouse` failure) where it was `400`; `ChError` failures on chart/board writes keep `400`/`500` through `FailedAs`. Dagster mutation failures still pick 404/409 from the text but no longer return it.
5. `docs/CODE-STANDARD.md` section 1.4 carried the same "fourteen handlers" sentence; updated with `AGENTS.md`.
6. TS tests use `bun:test` with Testing Library, as every test file in the repo does; the brief said `node:test`.
7. `tool-schema` fixture test unchanged and passing; no `POLICY_TABLE` change, no migration, no new route, no new dependency (`Cargo.lock` unchanged).

**Existing tests changed** (each asserted the bug)

- `lakehouse-clickhouse`: `ch_error_converts_to_unprocessable_api_error`, `cancelled_converts_to_internal_api_error` removed (the conversion is gone).
- `error.rs`: `ch_error_converts_through_question_mark_to_422_rejection` -> `ch_error_reaches_the_body_as_a_fixed_message_with_a_reference` (asserted `Code: 47. Unknown identifier: nope` in the body).
- `routes/dashboard.rs` and `routes/dashboard_sources.rs`: the two exact-message asserts ("dashboard query failed", "the SQL source failed to run") now check the fixed sentence followed by `Reference: `.
- `routes/query.rs`: `maps_query_error_to_422_forwarding_the_message_verbatim` -> asserts the fixed message and that `mismatched input` is absent.
- `routes/support.rs`: `js_error_formats_like_string_of_error` -> `upstream_message_never_repeats_the_error_text`.
- `routes/ai/mod.rs`: `an_llm_error_body_never_carries_the_providers_text` gained an `errorId` assertion.
- `tests/ingest_run.rs`: `ingest_run_dagster_launch_failure_is_422_not_200` asserted `"job not found"` (Dagster's message) in the 422 body; now asserts the fixed sentence and the absence of that text.

**What to check on a running system** (checklist rows 2 to 8)

1. Drop or rename a mart a chart reads, open the dashboard in the console, the public link and an embed: "This chart could not be loaded." with a small "Reference: <10 hex>", no table name, no `Code:`.
2. `docker logs <api>` and search for that id: one `ERROR upstream request failed` line with the full `ClickHouse` text (and no `password=` value or URL credentials).
3. Stop `ClickHouse`: "The database is unavailable.", at `WARN` in the log.
4. Send a malformed dashboard body: `body JSON is invalid` as before, 400.
5. Ask the assistant to query a table that does not exist: the tool step shows a fixed sentence and a reference, and the chat's own failure (stop the model URL) ends with "Reference: <id>".
6. SQL editor: a statement with a syntax error now says "The database request failed. Reference: ..." (deviation 2).
7. A role without `dashboard:read`: unchanged permission message.


### SEC-11 — review round 1 (developer, 2026-10-10)

Still nothing committed, stashed or pushed. Dev API not restarted.

**Product owner decision applied (Query Studio shows its author the engine's diagnosis).** Reverses deviation 2 above, for `POST /api/query/run` (`ClickHouse` and `Trino`) and `POST /api/query/estimate` only.

- `upstream_error.rs`: `is_statement_error` (a `Code: N. DB::Exception` body, minus the timeout and connection codes 159/203/209/210/279), `author_diagnosis` (first line, credentials masked, trailing `(version ...)` cut, URLs, IPs and `host:port` replaced by `<host>`; `line 1:5` and `db.table` are kept), and three functions all ending `_for_author`: `ch_error_for_author`, `ch_message_for_author` (estimate) and `query_error_for_author` (`Trino`'s `TrinoError::Query`). The full text is still logged under the reference. Timeout, transport, HTTP-status, non-`Code:` bodies take the old fixed message. `api_error`, now unused, was removed.
- `routes/query.rs`: three call sites, each with a comment citing SEC-11 and the decision.
- `tests/sec11_guard.rs`: any route line calling `*_for_author(` is a hit; one `ALLOWED` entry (`query.rs`, pattern `_for_author(`) covers the file; a new test shows the scan flags it in `dashboard.rs`.
- New `tests/query_author_exception.rs` (6): author sees the planted unknown-identifier text with reference and no version, host or IP (run and estimate); an unreachable engine (`127.0.0.1:1`) is still 503 "The database is unavailable. Reference: ..."; a 502 HTML body is still the fixed message; a public-link tile with the same engine error has no engine text; and leftover 2 below.
- Existing tests changed: `routes::query` `maps_query_error_to_422_with_a_fixed_message_and_a_reference` became `..._with_the_engines_diagnosis_and_a_reference`; the editor assertion in the `upstream_errors.rs` sweep now expects the diagnosis (and no version suffix). Both asserted the behaviour the decision reverses.
- `CHANGELOG.md`: the sentence about the editor replaced by the exception. `docs/core/features/no-raw-upstream-errors.md` NOT edited, per instruction; it needs: Query Studio run and estimate show the author the trimmed engine diagnosis for statement errors (decision 2026-10-10).

**Leftover 2.** `a_failing_tile_inside_a_signed_embed_is_the_fixed_message_with_a_reference`: `EMBED_SECRET` set, token minted with `lakehouse_embed::sign_embed`, a mock `ClickHouse` serves an embed-enabled board and one chart and fails the tile's own query with the planted engine error. `/api/embed/data` answers 200; the tile is `{"error":"This chart could not be loaded.","errorId":...}`; no marker, version or host in the body.

**Leftover 1 and verification** (from `rust/`, `CARGO_BUILD_JOBS=2`, disk 26 GB free at the end, 9-11 GB RAM available):
- `cargo fmt --check`: clean.
- `cargo clippy -p lakehouse-api -p lakehouse-clickhouse --all-targets --all-features --locked -- -D warnings`: clean, after the last edit (including `tests/ingest_run.rs`).
- `cargo test -p lakehouse-api --lib`: 1507 passed, 0 failed, 1 ignored.
- `--test sec11_guard`: 4; `--test upstream_errors`: 6; `--test route_auth`: 30; `--test security_regressions`: 10; `--test ingest_run`: 8; `--test query_author_exception`: 6; all passed (the `upstream_errors` run after the one assertion fix; the other targets ran before it, and it touches a test file only).
- No TypeScript touched, so `bun` not run.
- Not verified: against the running dev API or a real `ClickHouse`; the `<host>` masking is lexical (a bare hostname without a port, such as `ch-1.internal`, is indistinguishable from `db.table` and is kept).

### SEC-11 — review round 2 (developer, 2026-10-10)

Not committed; dev API not restarted. Finding (SHOULD-FIX): the real `ClickHouse` suffix is `(version A.B.C.D (official build))`; the four-part number was masked to `<host>` as if an IPv4 address, so the suffix cut no longer matched and the author saw `(version <host> (official build))`. The mock never had this shape.

Fix, in `author_diagnosis` (`upstream_error.rs`): the trailing `(version ...)` group is now cut BEFORE host masking, matched by parenthesis balance (`closes_at_end`) so the nested `(official build)` goes with it, then trailing whitespace is trimmed. New test `the_real_clickhouse_version_suffix_is_cut_whole` covers `(version 25.8.1.3 (official build))`, `(version 24.3.2 (official build))` and `(version 24.3.2)`: each result ends with `(UNKNOWN_IDENTIFIER)` and contains neither `version`, `<host>` nor `official build`. The planted-host test still passes.

Run (disk 30 GB free, 10 GB RAM available): `cargo fmt --check` clean; `cargo clippy -p lakehouse-api --all-targets --all-features --locked -- -D warnings` clean; `cargo test -p lakehouse-api --lib upstream_error` 17 passed; `--test query_author_exception --test sec11_guard` see the report. Not verified against the real `ClickHouse` here; the shape comes from the coordinator's verbatim sample.

## 8. Review (planner appends findings per PR)

### SEC-11 — T1–T7 and two review rounds (reviewer, 2026-10-10)

Reviewed against the plan and run on a rebuilt API against the dev
`ClickHouse`.

- **Decision taken to the owner (round 1).** The first build hid the
  engine's message in Query Studio as everywhere else, so a misspelt
  column answered only "The database request failed". The owner chose to
  restore the engine's diagnosis for the author in Query Studio only
  (feature page, decision 4).
- **SHOULD-FIX (round 2, fixed).** On the real engine the version suffix
  `(version A.B.C.D (official build))` was not cut: the four-part version
  was masked as a host first, so the cut no longer matched. The mocks did
  not have that shape. Fixed by cutting the suffix before masking.
- **No open BLOCKER.**

Verified by the reviewer on the final tree:

| Command | Result |
| --- | --- |
| `bun run typecheck` / `bun run lint` | clean / 0 errors, 6 warnings (all on `main`) |
| `bun run test` | 938 pass, 1 skip (the skip is on `main`), 0 fail |
| `cargo fmt --check` | clean |
| `cargo clippy -p lakehouse-api -p lakehouse-clickhouse --all-targets --all-features --locked -- -D warnings` | clean |
| `cargo test -p lakehouse-api --lib` | 1508 passed, 1 ignored |
| `--test sec11_guard` / `upstream_errors` / `query_author_exception` / `route_auth` / `security_regressions` | 4 / 6 / 6 / 30 / 10 passed |
| `cargo test -p lakehouse-clickhouse` | 21 passed |

On the running API: a chart over a dropped table returns "This chart could
not be loaded." with an `errorId` on the console payload and on the public
link fetched without a cookie, with no engine text in either; the API log
has one ERROR line carrying that reference and the full text; a malformed
board body is still our own 400. In Query Studio a misspelt column, a
missing table and an unparsable date each return the engine's message
ending at the error code name, then the reference, with no version and no
host; a valid statement runs. The product owner ran the Query Studio check
and opened Dashboards, Catalog, Sources, the assistant and Query Studio on
2026-10-10 and reported them normal.

*Not verified by the reviewer:* `cargo test --workspace` and the full
integration suite (the developer reports 38 targets, 3350 passed before
round 1; not re-run after); a failing tile seen in a browser (checked
through the API only); the assistant reporting a failed tool step in the
chat; an unreachable `ClickHouse` on a running system.

Carried forward:

- Status codes changed where an unreachable engine used to answer 422 or
  500; they now answer 503. Clients that branched on 422 need a look.
- `From<ChError> for ApiError` was removed from `lakehouse-clickhouse`, a
  change to a lower crate's API the plan did not name. It is what makes a
  new `?` on a `ChError` in a route fail to compile.
- The host mask is lexical: a bare hostname without a port is kept.
- In Query Studio the reference id sits inside the sentence; on tiles it
  is its own line. Cosmetic.

