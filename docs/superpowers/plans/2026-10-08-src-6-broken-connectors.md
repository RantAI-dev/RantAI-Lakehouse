# `SRC-6` Fix the broken connectors — Implementation Plan

> **Migration numbers, 2026-10-10.** `main` took `0061` after this plan was
> written. This branch's migrations were renumbered in `36f2a2a`, before any
> was applied: `0061`→`0062` (`SRC-6`), `0062`→`0063` (`SRC-7`),
> `0063`→`0064` and `0064`→`0065` (`SRC-8`). Numbers below are the old ones
> where they record what was done at the time.

**Status:** ready to build. Decisions D1–D7 on the feature page were signed
by the product owner on 2026-10-08, all as proposed. Slice A starts now;
slice B waits for PR #82 and PR #85.
Written 2026-10-08 by the planner (Claude Opus) for a developer agent, under
the role split in `AGENTS.md`.

**Spec:** `docs/core/specs/src-6.md`. Backlog `SRC-6`.
**Feature page:** `docs/core/features/connectors-that-work.md`. Decision
numbers below (D1–D7) are its rows.

**Why.** Three connector types in the picker cannot be tested or edited, and
the Sources header promises features that do not exist.

---

## 1. Slices and bases

| Slice | Content | Base | Branch |
| --- | --- | --- | --- |
| A | Google Sheets shown as unavailable, picker, header, object-storage form | `origin/main` at `339a101` | `feat/phase-1-connector-reliability` |
| B | Object-storage test and key change, Oracle and the other untestable types, the gate step | the same branch, after `main` with PR #82 (`SEC-15`) and PR #85 (`SEC-14`) is merged into it | `feat/phase-1-connector-reliability` |

Slice B waits because #82 and #85 change `connector_probe.rs` by about 830
lines and `routes/connectors.rs` by about 600. **Waits for `SEC-15` and
`SEC-14` part A.** D7 is signed: slice B is not stacked on the security branches.

## 2. What exists today

Anchors for slice A are verified at `origin/main` `cd8e3df`. Anchors for
slice B were re-verified at `cacc1e4` (this branch after `main` `715d87b`, which holds PR #82 and PR #85, was merged in), the code
slice B will sit on. Re-find by symbol if a line has moved.

| # | Spec row | Code | Verdict |
| --- | --- | --- | --- |
| F1 | Object storage: wizard and test disagree on the host | `src/features/connectors/connector-form-parts.tsx:51` sends `host` = the bucket name. `connector_probe.rs:1331` (`probe_s3`) parses `info.host` as `<endpoint>|<bucket>` (`parse_s3_host`, `:1221`) and never reads `info.dial`, which holds `endpoint` and `bucket` (`FilesDial`, `lakehouse-store/src/ingest_spec.rs:146`). | Every wizard-made connector fails its test as "misconfigured". Only the seeded `conn-s3-warehouse` passes, because its `host` has the old shape. |
| F2 | Object storage: keys cannot be changed | `routes/connectors.rs:1704` (`set_credential`) refuses when the probe is `supported && !ok`. F1 makes that always true. | Follows from F1. |
| F3 | Oracle: credential change refused with 422; every test reads as failed | `connector_probe.rs:380` returns `Outcome::misconfigured` for `SqlDriver::Oracle`, which is `supported: true, ok: false`. Same refusal at `routes/connectors.rs:1704`. `record_test_result` then marks the connector unhealthy. | Confirmed. The API has no Oracle client; Oracle is dialled only by `dagster/dispar_orchestrate/adapters/oracle.py`. |
| F4 | *Not in the spec* | `connector_probe.rs:238` (`probe`) has no arm for `mongodb`, `kafka` or `sftp`; they fall to `probe_by_kind` (`:264`) and `Outcome::unsupported` (`:167`), whose text says "Supported today: PostgreSQL, S3-compatible object storage". That list is wrong: MySQL, SQL Server and REST are tested too. | Honest outcome, stale message. D2. |
| F5 | Google Sheets listed as supported, works nowhere | `rust/migrations/0035_connector_type.sql` seeds `('Google Sheets', 'sheets', true, NULL)`. `probe_sheets` (`connector_probe.rs:815`) and `adapters/sheets.py:104` always answer unsupported. | Confirmed. |
| F6 | *Not in the spec* | `src/features/connectors/connector-type-picker.tsx:163-174` tells the user Sheets "can be configured and tested"; the test answers unsupported. A "Test only" badge at `:231`. | Wrong text. Goes away with F5. |
| F7 | Sources header promises SaaS and federation | `src/features/connectors/connectors-page.tsx:143`. | Confirmed. |
| F8 | *Not in the spec* | `src/features/connectors/dial-forms/files-dial-form.tsx:38` offers protocol "SFTP". `adapters/files.py` reads S3 only; SFTP is its own type (`0043_ingest_tier2_adapters.sql`). | A choice that never loads data. D4. |
| F9 | *Not in the spec* | `connector_discover.rs:20-30` says MySQL and SQL Server discovery is "exercised by the `ops/g6` gate". `grep -i discover ops/g6/` finds nothing. | False comment (rule 1). Corrected in task B4; the missing test is `SRC-9`. |

## 3. Decisions already made by the planner

- **The test reads the dial.** For a connector with `adapter = 'files'` the
  test takes `endpoint` and `bucket` from `dial`. `connector.host` stays a
  display value. A row with no adapter keeps the old `host` parse, so
  nothing seeded or legacy changes.
- **"Cannot be tested" is `supported: false`, never `misconfigured`.** That
  is what makes `set_credential` save with `verified: false`
  (`routes/connectors.rs:1669` doc comment already describes this for
  Kafka, SFTP, `MongoDB` and Oracle) and what keeps `record_test_result`
  from touching `health`.
- **The reason a type is unavailable comes from the API.** A nullable
  `connector_type.unsupported_reason` column, shown on the picker tile.
  Google Sheets keeps `adapter = 'sheets'` so existing Sheets connectors
  still open; the "unsupported rows have no adapter" sentence in
  `connector_type.rs` is corrected.
- **No new dependency.** No Oracle, MongoDB, Kafka or SFTP client is added
  to the API.

## 4. Tasks

One task per commit, in this order. Cite `SRC-6` and the finding (`F1`…)
at each fix site and in the commit body (rule 13).

### Slice A (`feat/phase-1-connector-reliability`)

**A1. Migration: Google Sheets is not supported, and says why.**
`rust/migrations/NNNN_connector_type_unsupported_reason.sql`, the next free
number (0061 at `cd8e3df`; check again when branching). Why-header. Adds
`unsupported_reason TEXT`. Then a targeted update:
`UPDATE connector_type SET supported = false, unsupported_reason = '<text>'
WHERE name = 'Google Sheets' AND supported = true AND adapter = 'sheets'`.
Text: "No verified Google sign-in exists in this build, so a Google Sheets
connector cannot be tested or loaded." `ConnectorType`
(`lakehouse-store/src/connector_type.rs`) gains
`unsupported_reason: Option<String>`; the query selects it; the module
comment is corrected. `ConnectorType` in
`src/services/contracts/connectors.ts:553` gains
`unsupportedReason: string | null`; every fixture that builds one is updated.
*Check:* a store test `google_sheets_is_listed_as_unsupported_with_a_reason`
and one asserting every `supported = false` row other than Sheets has a null
reason. `cargo fmt --check`, `cargo clippy -p lakehouse-store --all-targets
-- -D warnings`, `bun run typecheck`.

**A2. Picker shows the reason and drops the wrong banner.**
`connector-type-picker.tsx`: a disabled tile shows `unsupportedReason` as its
line of text when present, "Not available yet" otherwise, and uses it as the
`title`. Remove the amber paragraph (`:163-174`) and the "Test only" badge
(`:231`). *Check:* tests in `connector-type-picker.test.tsx`: a Sheets tile is
disabled and shows its reason; a type with no reason shows "Not available
yet"; the banner text is gone.

**A3. Sources header.** `connectors-page.tsx:143`, text from D6.
*Check:* `connectors-page.test.tsx` asserts the new text and that "SaaS" and
"federation" are absent.

**A4. Object-storage form.** Remove the "SFTP" option from
`files-dial-form.tsx`; the protocol select keeps one value, so replace it with
plain text "S3-compatible". Label the endpoint field "Endpoint" with the hint
"Needed to test the connection. Leave empty for AWS S3." (D3). *Check:*
`dial-forms.test.tsx`.

**A5. Docs (planner, same branch).** `docs/core/specs/src-6.md`: add the
rows for F4 and F8, status. `CHANGELOG.md`.

### Slice B (`feat/phase-1-connector-reliability`)

**B1. Object-storage test reads the dial (F1, F2).** In `connector_probe.rs`
split `probe_s3` into the part that finds the target and the part that dials.
`Some("files")` in `probe` parses `info.dial` with `Dial::parse("files", …)`:
- protocol `s3`, endpoint set: dial that endpoint and bucket, through the same
  `resolve_checked`, `url_names_target` and pinned client the function uses
  today. No second S3 client.
- protocol `s3`, no endpoint: `supported: false`, "This connector has no
  endpoint, so it cannot be tested from the console. Set an endpoint to test
  it." (D3).
- protocol `sftp`: `supported: false`, naming the SFTP connector type.
- dial missing or invalid: `misconfigured`, as `probe_dial` does.
`probe_by_kind` keeps calling the `host` parse for rows with no adapter.
*Check:* unit tests beside the existing S3 ones: a `files` connector whose
`host` is a bare bucket name and whose dial names an unreachable endpoint
reports a real elapsed time and a failure, not "host must be shaped"; a
loopback endpoint in the dial is blocked; no endpoint is `supported: false`
with no latency; protocol `sftp` is `supported: false`; a row with no adapter
and the old `host` shape still reaches the dial. Remove the "For a `files`
connector the candidate probe reads `connector.host`" limit from the `SEC-14`
handoff's list by name in this plan's handoff, since it no longer holds.

**B2. Oracle and the orchestrator-only types (F3, F4).** One constructor,
`Outcome::not_testable_here(kind)`: `supported: false`, no latency, "A {kind}
connector cannot be tested from the console: it connects from the
orchestrator. A load reports whether the connection worked." `probe_dial`'s
`SqlDriver::Oracle` arm returns it. `probe` gains explicit arms for
`mongodb`, `kafka` and `sftp` that return it. `Outcome::unsupported` drops
the "Supported today" list. Update the module comment (`:10-37`), and the doc
comments of `test_connection` and `set_credential`, to what is now true.
*Check:* unit tests: each of the four is `supported: false`, `ok: false`,
no latency, and the message names no other product. A route test in
`tests/`: `PUT /api/connectors/{id}/credential` on an Oracle connector
answers 200 with `verified: false` and the stored file holds the new value;
`POST …/test` on it answers `supported: false` and leaves `health` unchanged.

**B3. Gate: object storage is tested and its keys changed.**
`ops/g6/g6_ingest_matrix_test.py`, for the existing `g6-files` connector
after its ingest spec is saved: `POST …/test` must answer `supported: true,
ok: true`; `PUT …/credential` with a wrong secret key must answer 422 and a
second `POST …/test` must still pass; `PUT …/credential` with the right pair
must answer `verified: true`. This is the proof for acceptance rows 1–3.
*Check:* the g6 job in CI.

**B4. Comment correction (F9).** `connector_discover.rs:20-30`: say that
MySQL and SQL Server discovery has no live test and that `SRC-9` owns it.
No behaviour change.

**B5. Docs (planner, same branch).** `CHANGELOG.md`, `docs/OPERATIONS.md` if
it describes the connection test, and a new backlog item for a connection
test that runs in the orchestrator (D1).

## 5. Build limits on this machine

- Use the shared target dir and two jobs:
  `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-catalog-target CARGO_BUILD_JOBS=2`.
  Check `df -h /` before each cargo command.
- Do not build test binaries here (`cargo test` has crashed this VM). Run
  `cargo fmt --check` and `cargo clippy … --all-targets`, which type-check the
  tests. Write every Rust test as *not verified* in the handoff; its first run
  is CI's.
- Slice A's verification block: the Rust line scoped as above, the TypeScript
  line in full. Slice B: the Rust line, the two Python lint lines.

## 6. Out of scope

- A live test for Oracle, MongoDB, Kafka, SFTP; testing AWS S3 with no
  endpoint; making Google Sheets work.
- Discovery (`SRC-13`), gates for untested types (`SRC-9`), credential
  encryption (`SRC-12`).
- Any change to what `SEC-14` and `SEC-15` decided: re-point rules, address
  pinning, redirects, TLS.

## Handoff

*(developer, per slice)*

### Slice A (developer, Sonnet 5.5)

Commits on `feat/phase-1-connector-reliability`, base `339a101`: A1 `3966e09`, A2 `41a02d8`, A3 `ec6dbee`, A4 `564506f`.

Commands, all in `/home/hv/lakehouse-src6`:
- `df -h /`: 65 GB free.
- `cd rust && cargo fmt --check`: clean.
- `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-src6-target CARGO_BUILD_JOBS=2 cargo clippy -p lakehouse-store --all-targets -- -D warnings`: finished, no warnings. `lakehouse-api` not touched, so no clippy for it.
- `bun install --frozen-lockfile`: 772 packages.
- `bun run typecheck`: clean. `bun run lint`: 0 errors, 6 warnings (unused vars, none in files I touched).
- `bun run test`: second and third runs 871/872 pass, 1 skip, 0 fail. The first run of the same tree reported 871 pass, 1 fail; I did not capture which test, and it did not recur in two reruns. Treat as a possible flake.
- Scoped: `bun test src/features/connectors`: 154 pass, 0 fail.

Existing assertions changed:
- `lakehouse-store/tests/connector_type.rs::unsupported_rows_have_no_adapter` skips the row named "Google Sheets": it now has `supported = false` and `adapter = 'sheets'` by plan decision.
- TS fixtures that build `ConnectorType` gained `unsupportedReason: null` (connectors.test.ts, connector-create-page.test.tsx, connector-edit-page.test.tsx, connector-type-picker.test.tsx) and the fallback in `connector-edit-page.tsx:155`.

Added tests: store `google_sheets_is_listed_as_unsupported_with_a_reason`, `only_google_sheets_has_an_unsupported_reason`; picker (3), connectors page (1), dial forms (3).

Not verified: the two new store tests, the changed store test, and migration 0061 running on Postgres (clippy type-checks them only; `cargo test` forbidden on this VM). No JSON schema snapshot or AI fixture references `ConnectorType`, so none changed.

### Slice B (developer, Sonnet 5.5)

Commits on `feat/phase-1-connector-reliability`, base `cacc1e4`: `71939a8` fix(api): an object-storage connector is tested at the endpoint in its settings; `badf03e` fix(api): Oracle, MongoDB, Kafka and SFTP are "not testable from the console", not "failed"; `aeb9dbf` test(g6): the object-storage connector is tested at its dial endpoint and a wrong key is refused; `6ab12f5` docs(api): MySQL and SQL Server discovery has no live test; SRC-9 owns it.

Commands, all in `/home/hv/lakehouse-src6`:
- `df -h /` before each cargo run: 60-63 GB free.
- `cd rust && cargo fmt --check`: clean (after one `cargo fmt`, which changed only my hunks in `connector_probe.rs` and the new test file).
- `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-src6-target CARGO_BUILD_JOBS=2 cargo clippy -p lakehouse-api --all-targets -- -D warnings`: finished, no warnings, on the final tree (first run after B2 failed on one missing backtick in a doc comment, `MySQL`, fixed).
- `... cargo clippy --workspace --all-targets --all-features -- -D warnings`: finished, no warnings.
- `python3 ops/lint/check_intra_package_imports.py`: OK. `python3 ops/lint/check_bare_iceberg_count.py`: OK. `python3 -m py_compile ops/g6/g6_ingest_matrix_test.py`: OK.
- No TypeScript changed; none run.

Existing assertions changed: none. `unsupported_kind_never_fabricates_a_latency_or_success` passes unchanged by reading (message still contains the kind).

Added tests: unit (`connector_probe.rs`) `files_connector_is_tested_at_its_dial_endpoint_not_its_host`, `files_candidate_with_a_new_dial_endpoint_is_dialled_there_not_at_host`, `files_dial_loopback_endpoint_is_blocked_by_default`, `files_dial_without_an_endpoint_is_unsupported_with_no_latency`, `files_dial_with_the_sftp_protocol_is_unsupported_with_no_latency`, `files_connector_with_an_invalid_dial_is_misconfigured`, `orchestrator_only_adapters_are_not_testable_here_with_no_latency`, `unsupported_message_does_not_list_what_is_supported`; integration `tests/connector_not_testable.rs::an_oracle_credential_is_saved_unverified_and_its_test_leaves_health_alone`. The legacy-host S3 tests (adapter `None`, "still reaches the dial") are the existing ones, untouched.

SEC-14 handoff limit "For a `files` connector the candidate probe reads `connector.host`" (`2026-10-08-sec-14-connector-repoint.md`, line 323) no longer holds: B1 makes the `files` probe read `dial`. That file was not edited.

Files outside the plan's list: `rust/crates/lakehouse-store/src/connectors.rs` (doc comment of `ConnectorTestResult.supported` said only PostgreSQL and S3 are supported; wrong, comment only); the B4 commit also corrects the adjacent `testcontainers` sentence in `connector_discover.rs`.

B3 and the managed-credential question: in the g6 override `/run/secrets` is the read-only `g6_secrets` volume in `lakehouse-api` and in `dagster-code-location`. `PUT .../credential` writes `connector_managed_*` into the API's `CONNECTOR_SECRETS_DIR` (`/run/secrets`), so with the right pair it would fail (503), and a file it wrote would not reach Dagster. The gate therefore does only the steps that write nothing: test ok, wrong secret key 422 (the probe runs on an in-memory candidate before any write), test still ok. The "right pair gives `verified: true`" step is NOT in the gate; it needs a compose change (a writable credential volume shared by the API and Dagster in the override). Stopped at that boundary as instructed.

Not verified (no `cargo test`, no docker): every Rust test above; the B3 gate steps (CI's g6 job only; the wrong-key 422 assumes RustFS rejects `rustfsadmin-wrong` with an auth error, as `classify_object_store_error` would report); that `sslMode: "disable"` Oracle dial in the integration test passes `Dial::parse` (read from `validate_sql_dial_post_parse`, not run); the `health = 'degraded'` insert against the table's check constraint, if any.

## Review

### Slice A, 2026-10-08, at `564506f`

Read the full diff of `origin/main..564506f` (14 files) against tasks A1–A4.
No `BLOCKER`.

Re-run by the planner on `564506f`:

- `cd rust && cargo fmt --check`: clean.
- `cargo clippy -p lakehouse-store --all-targets -- -D warnings`
  (`CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-src6-target`,
  `CARGO_BUILD_JOBS=2`): no warnings.
- `bun run typecheck`: clean. `bun run lint`: 0 errors, 6 warnings, all
  `no-unused-vars` in files this slice does not touch.
- `bun run test`: 872 pass, 1 skip, 0 fail, 873 tests in 99 files. The
  developer's one failure on a first run did not recur here; unidentified.

Findings:

- `SHOULD-FIX` 1. `files-dial-form.tsx`: the "Protocol" `Label` no longer
  names a control, and the text beside it is a bare `p`. Tie them
  (`aria-labelledby`) or render the pair as plain text without `Label`.
- Accepted: `unsupported_rows_have_no_adapter` now skips Google Sheets by
  name. The rule it tested stopped being true by decision D5, and the two new
  tests pin the Sheets row instead. Not a weakened test (rule 2).

Not verified, by anyone: the three store tests and migration `0061` against
PostgreSQL. `cargo test` is not run on this machine; the first run is CI's.
The console was not opened in a browser; that is the product owner's QA.

### Slice A fix round, 2026-10-08, at `b90efe5`

`SHOULD-FIX` 1 is fixed in `b90efe5` (label id plus `aria-labelledby` on a
group; one new test finds the text by its label). Re-run by the planner on
`b90efe5`: `bun run typecheck` clean; `bun run lint` 0 errors, 6 warnings in
untouched files; `bun run test` 873 pass, 1 skip, 0 fail, 874 tests in 99
files. No Rust changed since `564506f`. No open `BLOCKER` or `SHOULD-FIX` on
slice A. Slice B is not started: PR #82 and PR #85 are still drafts.

### Slice B, 2026-10-08, at `6ab12f5`

Read the diff of `cacc1e4..6ab12f5` (6 files) against tasks B1–B4, the
SEC-14 and SEC-15 decisions, and the developer's own "not verified" list.
No `BLOCKER`.

Re-run by the planner on `6ab12f5`:

- `cd rust && cargo fmt --check`: clean.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  (`CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-src6-target`,
  `CARGO_BUILD_JOBS=2`): no warnings.
- `python3 ops/lint/check_intra_package_imports.py`,
  `python3 ops/lint/check_bare_iceberg_count.py`,
  `python3 -m py_compile ops/g6/g6_ingest_matrix_test.py`: pass.
- `bun run typecheck` clean; `bun run lint` 0 errors, 6 warnings in files
  this work does not touch; `bun run test` 891 pass, 1 skip, 0 fail, 892
  tests in 100 files (after the merge of `main`).

Checked by reading, the two points the developer could not settle:

- `health = 'degraded'` is allowed by `connector_health_check`
  (`0013_connectors.sql:66`).
- The test's Oracle dial (`sslMode: "disable"`, no DN) passes
  `validate_sql_dial_post_parse` (`ingest_spec.rs:921`).

Findings:

- Accepted gap, not a finding against the developer: task B3's third step
  (`PUT …/credential` with the right pair answers `verified: true`) is not
  in the gate. In `ops/g6/docker-compose.g6.override.yml` the credential
  directory is a read-only volume in the API, so the request cannot be
  stored there. The developer stopped and reported instead of editing
  compose, as told. The plan asked for something the gate cannot do; that
  is the planner's error. Acceptance row 3 is therefore proven only by the
  product owner's QA. A writable shared credential volume in the gate
  belongs to `SRC-9`.
- The `SEC-14` handoff's limit "for a `files` connector the candidate probe
  reads `connector.host`" no longer holds after B1: a re-point of an
  object-storage connector is now tested at the new endpoint.

Not verified, by anyone: every Rust test added in slices A and B (eight
unit tests in `connector_probe.rs`, `tests/connector_not_testable.rs`, three
store tests) and migration `0061` on PostgreSQL; the g6 steps. `cargo test`
and the gates are not run on this machine. Their first run is CI's, on the
pull request. Nothing was opened in a browser.

B5 (planner): `CHANGELOG.md` entry extended; `SRC-14` added to
`docs/core/BACKLOG.md` for a connection test that runs in the orchestrator.
