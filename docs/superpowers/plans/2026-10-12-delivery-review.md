# Delivery review, 2026-10-12

The planner's review of backlog work that was merged without a planner
review, or is waiting in an open pull request. For each ticket it says
whether the work is done, what is left, and which new backlog item holds the
rest, so the product owner can assign it back.

Base: `main` at `fa01cc1`. Method: the diff of each pull request read against
its spec in `docs/core/specs/`, its feature page, its plan and `AGENTS.md`.
Each blocker below was checked again at the code by the planner. No cargo
build, test run or stack was run for this review (disk). The CI results quoted
are GitHub's.

The two gitleaks checks are red on `main` and on every pull request:
`.env.staging` (`SEC-13`) for the working tree, and a historical finding
(`SEC-1`) for the full history. They are not counted against any PR.

## 1. Merged pull requests

| Ticket | PR | Verdict | Left over, now in |
| --- | --- | --- | --- |
| `SRC-6` | #101 | Done, waiting for acceptance | Acceptance row 3 is in `SRC-9` |
| `SRC-7` | #101 | Mostly done | `SRC-17`, `SEC-10` |
| `SRC-8` | #101 | Mostly done; one blocker | `SRC-16` (P1), `SRC-18` |
| `SEC-15` | #82 | Done, waiting for acceptance | A message fix, in `SEC-25` |
| `SEC-14` | #85 | Theft path closed; TLS (part B) not started | `SEC-14` stays Building; `SEC-25` |
| `DATA-11` | #98 | Mostly done | `DATA-24`; `DATA-20` (exists) |
| `DATA-9` | #71, #73 | Built, not accepted | Feature page status corrected here |
| `DATA-17` | #76 | Partly: Excel and Parquet only | Stays Building; `SEC-22` widened |
| `SEC-9` | #78 | Mostly done; one gap | `SEC-24` (P1) |
| `SEC-11` (assistant part) | #78 | Done for the removed endpoints | `SEC-26` for the chat tools |
| `AI-16` (first line) | #79, #109–#112 | Partly | `AI-19` |
| `AI-17` | #81 | Built, not accepted | Owner's checklist |

### `SRC-6`, `SRC-7`, `SRC-8` (#101)

- All five new routes are in `POLICY_TABLE` and checked both ways by the
  loops in `tests/route_auth.rs`.
- Migrations 0062 to 0065 carry why-headers and are numbered uniquely.
- The specs are intact after the merge commits.
- The planner's own review findings in the three plans are all closed.

**BLOCKER, now `SRC-16`.** A connector that is loaded after the fact (files,
REST, MongoDB, Kafka, SFTP) and set to "pause" stops for good the first time
it sees a change:

- `lakehouse-store/src/schema_diff.rs:285` pauses it but records every change
  as applied, so nothing waits for approval.
- Approve then answers 404 (`routes/schema_changes.rs:662`).
- Changing the policy does not lift the pause.
- The console shows "Paused … until a change is approved" next to "Nothing is
  waiting for a decision".

A second stuck case: a pending type change that Approve refuses keeps the
connector paused after every other change is approved.

**SHOULD-FIX**

- `routes/load_alerts.rs:285-308` writes the dedupe row before delivering, so
  one rule-store failure loses the alert silently.
- `lakehouse-alerts/src/lib.rs:1685` discards whether delivery worked, and an
  alert instance is saved either way. Feature-page row 5 ("a failed delivery
  is shown") cannot pass.
- The Alerts page and the alerts crate still say nothing raises
  `connector_schema_change`. SRC-8 raises it.
- The pause is enforced only on the due list and on `POST …/ingest/run`. A run
  already queued, or one launched from Dagster, still loads.
- No gate proves an e-mail or webhook arrives, and `SRC-7-RESULT.md` was never
  written (principle 5).
- Seven CI-fix commits after the last planner review have no review entry.
  The planner read 8cb774e (the administrator rule) and it is correct.

### `SEC-15` (#82)

All target rows are met at the code:

- The compose default is empty (blocked).
- No redirect is followed.
- Every API-side dial uses the address `resolve_checked` approved.
- The CDC delete checks before it dials.
- No resolved address appears in a message.

One SHOULD-FIX: the refusal text at `connector_probe.rs:1032` has runs of
spaces inside the sentence.

### `SEC-14` (#85)

The theft path is closed: the ingest-spec PUT, the credential PUT with a dial
override, the PATCH of `host` and the assistant's tool. The PR merged without
a planner review ("The reviewer has not yet read the diff line by line"); this
section is that review.

Not done (part B):

- PostgreSQL still uses `PgSslMode::Prefer` (`connector_probe.rs:998`), and
  MySQL `Preferred` (`:1015`).
- SQL Server encrypts but does not check the certificate (`trust_cert()`,
  `connector_probe.rs:534`, `connector_discover.rs:358`).

**SHOULD-FIX, now `SEC-25`.**

- A connector that signs in with no credential (Kafka with auth `none`) can
  never be re-pointed: 409 without a credential, 400 with one.
- The PATCH-`host` refusal for a `files` connector protects nothing since
  SRC-6, and its doc (`routes/connectors.rs:401`) is stale.

### `DATA-11` (#98)

| Target | State |
| --- | --- |
| What is searched | Met |
| Matching | Met |
| Search from every page | Met |
| Filters | Partly: Owner and Tags hide below 1500 and 1760 px windows (`data-explorer-page.tsx:189-190`), so a laptop has neither |
| Speed (500 ms for 10,000 tables) | Not measured: the bench was never run and `DATA-11-RESULT.md` does not exist |
| Permissions | Partly: tenant check before the cached copy; per-user visibility is `DATA-20` |

Acceptance rows 10, 11, 13 and 14 are not run. SHOULD-FIX: the search column
query (`routes/catalog.rs:738`) reads `dataset_column` without `FINAL`, so it
can match old column rows. All of this is now `DATA-24`.

### Uploads: `DATA-9`, `DATA-17`, `SEC-22` (#73, #76)

`SEC-22` is still open at the code:

- The workbook is parsed while it is opened (`upload_workbook.rs:205`, `:210`),
  already in `POST /api/uploads`.
- The parser pre-allocates from counts the file declares.
- The `catch_unwind` guard cannot catch an allocation failure.
- The module doc at `upload_workbook.rs:70-74` says the opposite (rule 1).
- By reading the crate source (not run), the Parquet reader trusts sizes from
  the file's footer and pages in the same way.
- Conversions run on blocking threads with no limit on how many at once.
- Every preview converts the whole file again.

`SEC-22` is widened to cover all of this.

`DATA-17` has Excel and Parquet. It does not have:

- JSON, JSON Lines and Avro.
- Ten files at once.
- 2 GB.
- Resumable uploads.
- Compressed files.
- Several files into one table.

It stays Building. The Excel and Parquet decisions on `features/upload-file.md`
are not signed. `DATA-9`'s feature page said "In build" while the backlog said
In Acceptance; corrected in this change.

### Assistant: `SEC-9`, `SEC-11`, `AI-16`, `AI-17`

`SEC-9`:

- `/api/agent/*` is gone.
- The chat's `run_sql` runs Query Studio's own handler with the caller's
  principal, so masking, row filters and `query:read` apply.

**Gap, now `SEC-24` (P1).** The system prompt's DATA MAP lists tables, row
counts and sample values to any signed-in user who passes the tenant check
(`routes/ai/mod.rs:318-345`), with no `catalog:read` or `query:read` check.
The seeded Dashboard Viewer role holds only `dashboard:read`
(`0002_seed_identity.sql:46`), yet gets sample values Query Studio would refuse
it. Masked columns and filtered tables are left out correctly.

**SHOULD-FIX, now `SEC-26`.** About thirty chat-tool sites put `err.to_string()`
into the tool result, which reaches the response's `toolTrace` (principle 4).
None of them were added by the reviewed PRs.

`AI-16`:

- Built: background drafting of meanings, `GET`/`PUT /api/semantic` with the
  right permissions and route tests, and the Meaning view.
- Not built: the other lines (plain-language table search, classification,
  quality help, connector building).
- `AI-16-AC1` needs a plan in `docs/superpowers/plans/`, and the feature pages
  say plans are not kept in the repository. Now `AI-19`.

`AI-17`: built; all 12 checklist results are blank.

## 2. Open pull requests

| PR | Ticket | Verdict | Action taken |
| --- | --- | --- | --- |
| #113 | SRC-6/7/8 status | Correct | **Merged** |
| #77 | flaky tests | Already on `main` as c1c9906 | Closed as superseded, with a comment |
| #97 | `SEC-23` | Ready; one docs conflict | Main merged in by the planner (docs only); merge when CI is green |
| #100 | `DATA-16` | Ready; two docs conflicts | Main merged in by the planner (docs only); merge when CI and `g8-time-travel` are green |
| #106 | `DATA-12` part 1 | BLOCKER: migration `0062` collides with main's | Commented |
| #105 | `SEC-10`–`SEC-13` | Two BLOCKERs: the merge with main does not compile and main's load alerts skip the webhook check; the code was written by the planner model | Commented |
| #104 | `BI-18` phase 1 (draft) | Waits for #105; the assistant filter tool (`AI-2`) is missing | Commented |
| #114 | `SRC-9` (draft) | Close; g9 uses the default RustFS key; MySQL and SQL Server CDC need a decision | Commented |
| #115 | `SRC-10` (draft) | Far: history (SCD 2) needs re-planning; refresh and clear not started | Commented |
| #87 | `SEC-16` part A (draft) | Meets part A; 154 commits behind main | Commented |
| #86 | `SEC-17` (draft) | Meets its rows; large conflict with the new upload code; duplicates #104's CSV rule | Commented |
| #83 | `SEC-18` (draft) | Still needed; the merge must keep `SEC-15`'s compose default | Commented |

#97 and #100 had only docs conflicts (`CHANGELOG.md`, `BACKLOG.md`). The
planner resolved them by keeping both sides, in a merge commit on each branch.
No product code was changed. CI decides the rest.

## 3. Decisions for the product owner

1. **MySQL CDC and SQL Server CDC** (#114): build the Debezium templates
   (`SRC-4`), or mark both connector types unsupported now. Today they are
   offered and cannot load.
2. **`SRC-10` history mode** (#115): the plan's stop condition was hit (dlt
   upserts per 1000 rows, so a failed load can leave closed rows with no
   successor). Re-plan it, or defer history.
3. **Phase 1 scope** (#104): add the filter tool (`AI-2`) to phase 1, as the
   assistant-parity rule requires, or rescope it in writing.
4. **Sign** the Excel and Parquet decisions on `features/upload-file.md`.
