# QA round, leftovers (P2): the stored tenant, a test that fails in one process, e-mail not set up, a publish that fails early

Fixes for four backlog rows left over from the QA round on staging (2026-11-02 to 11-12) and
its reviews: `ADM-15` (the active tenant kept in the browser), `REL-25` (a test that fails when
its binary runs in one process), `RPT-2` (under `RPT-1`, case `LH-RPT-1-06`, decision `DEC-18`)
and `DATA-56` (under `DATA-1`, #484 review R4).

Base: `main` at `2e7b178ea` (every anchor below was read there, in a checkout detached at
`origin/main`). Branch: `fix/qa-round-leftovers-p2`. One pull request for the whole slice,
opened as a draft by the planner as soon as T1 is pushed, so CI (x86_64) runs every test.

Planner and reviewer: Claude Opus. Developer: another agent (not Opus). The developer pushes
after every commit and does not change a decision below. If the code is not what a task says,
stop that task and report the mismatch in the handoff.

Five pull requests of the same round are open and merge before or around this one: #483, #485,
#486, #487 and #489. This plan changes no file they change today (checked against their
branches at this commit). One task works in a directory one of them owns, and says so: T5
(`src/features/dashboards/*`, #486). There, **find each anchor by the text quoted, not by
its line number**. The same holds for every anchor if `main` has moved when the work starts.

What QA and the reviews saw:

- `ADM-15` (#483 review, round 1): after a second person picks their tenant, the first
  person's next sign-in on the same browser starts in a tenant that is not theirs.
  Tenant-scoped pages answer 404 until they pick one, and since `QS-8` their query tabs are
  keyed under that tenant.
- `REL-25` (a developer run, 2026-11-12): `cargo test -p lakehouse-api --test report_runner`
  ends "23 passed, 1 failed" on `main`, five runs of five. The failing test is
  `a_pdf_failure_is_logged_for_the_operator_and_the_record_stays_fixed`. It passes alone and
  under `nextest` (one process per test), which is why CI is green.
- `RPT-2` (QA `LH-RPT-1-06`, fail, Major): with e-mail not set up, six "Send now" and one
  scheduled run each recorded a failed delivery with the reason "Email is not set up on this
  deployment.", `consecutiveFailures` stayed 0, `paused` stayed false, and nothing told the
  owner of the subscription. The product owner then decided (`DEC-18`) to keep the counting
  as it is and to show the state on the list.
- `DATA-56` (#484 review R4): a publish in open format that fails before the export starts
  writes no run, so the card never says "Last publish failed" for it. A scheduler whose
  every run fails that way leaves the card on "Up to date" or "Out of date".

## 1. Decisions

- **D1. One task per commit, in the order of section 2** (Rust first, then the console, then
  the documents). Each commit body cites the backlog ID and, where there is one, the QA case
  or the review (`LH-RPT-1-06`, `#483 review round 1`, `#484 review R4`). Each fix site
  carries a short comment naming it (rule 13), for example `// ADM-15 (#483 review): …`.
- **D2. What the product owner and the planner already decided is not reopened.** It is
  restated in D3, D8, D11 and D16 and made exact against the code in the decisions that
  follow each.

### ADM-15, the active tenant kept in the browser

- **D3. The rule (decided).** The stored tenant (`lh_active_tenant`) is cleared on sign-out.
  Whenever the session answers (`GET /api/auth/me`), a stored tenant that is not among the
  signed-in person's tenants is ignored and replaced by the person's first tenant, and the
  stored value is corrected. A person whose stored tenant is still theirs keeps it.
- **D4. After the session answers, the stored value is always one of the person's own
  tenants, or absent.** Made exact:
  - stored and among `me.tenants` → kept;
  - stored and not among them, or nothing stored, and the person has tenants → the first of
    `me.tenants` is **written**;
  - the person has no tenant → the key is **removed** (the navbar shows no switcher for such
    a person, `src/components/app-shell/tenant-switcher.tsx:44`, so they could never
    correct a stale value themselves).

  Writing the first tenant when nothing is stored goes one step past D3, on purpose. With
  nothing stored the console shows `me.tenants[0]` and sends no header, and the server then
  takes the first of `principal.tenant_ids`. Those two lists are not in the same order:
  `tenant_ids` is read `ORDER BY tenant_id`
  (`rust/crates/lakehouse-auth/src/repository.rs:141-142`), `me.tenants` comes from
  `list_tenants_by_ids`, which has no `ORDER BY`
  (`rust/crates/lakehouse-store/src/identity.rs:1260-1264`). Once sign-out clears the key
  (D3), every sign-in would start in that state. Writing the tenant the console shows makes
  the header say it, so the console and the server agree. It widens nobody's reach: the
  server already resolves a person with tenants to one of their own
  (`rust/crates/lakehouse-api/src/tenant_scope.rs:76-90`), and "no tenant at all" is kept
  for a person who has none.
- **D5. Sign-out clears; a session that merely ended does not.** `logout()` removes the key
  before it calls the API, so it is gone even if that request never returns. The `401`
  redirect (`redirectToLogin`, `src/services/http.ts:78-85`) does not clear it: a person
  whose session expired signs in again and keeps their choice ("a person whose stored tenant
  is still theirs keeps it"); if somebody else signs in instead, D4 corrects it before any
  page renders. A failed `GET /api/auth/me` (no session, or the network) changes nothing in
  storage.
- **D6. Requests to `/api/auth/*` never carry `X-Tenant`.** `apiFetch` already treats that
  prefix specially for the `401` (`src/services/http.ts:105`). Without this, the sign-in
  request and the session check of the next person would still carry the previous person's
  tenant in the D5 case. No handler under `/api/auth/` reads the header (checked:
  `rust/crates/lakehouse-api/src/routes/auth.rs` does not call `tenant_scope`, on `main` and
  on the #487 branch). After D4 to D6 the first request that can carry a tenant is made
  after the session answered and the value was corrected: pages render only once
  `status === "authenticated"` (`src/components/app-shell/app-frame.tsx:49-56`), and the
  correction is written before that status is set (T7).
- **D7. One function decides, and the server already refuses.** `resolveActiveTenant(stored,
  tenants)` is used by `load()` and by the context value, so `activeTenantId` can never be a
  tenant the person is not in. `setActiveTenant(id)` ignores an id that is not among the
  person's tenants. The server needs no change: a header naming a tenant the caller is not
  in is `404` (`tenant_scope.rs:76-90`), which is what QA saw.
- **D8. Every reader and writer of the key** (grep of `lh_active_tenant`,
  `ACTIVE_TENANT_STORAGE_KEY`, `readActiveTenantId`, `writeActiveTenantId`,
  `activeTenantId`, `setActiveTenant`, `X-Tenant` under `src/`, at this commit):
  - the key and its helpers: `src/services/http.ts:41`, `:48-55` (read), `:63-70` (write);
  - the header: `src/services/http.ts:100-103`, the only place `X-Tenant` is set. The pages
    that do not use `apiFetch` (public link, embed, print view, `src/proxy.ts`) send no
    tenant at all;
  - the provider: `src/features/auth/auth-provider.tsx:120` (state), `:139-141` (read on
    mount), `:160-163` (`setActiveTenant`), `:178` (the value);
  - readers of `activeTenantId`: `src/components/app-shell/app-navbar.tsx:45`, `:127`, `:136`
    (the switcher), `src/features/connectors/connector-create-page.tsx:74-95`,
    `src/features/connectors/connector-scope-fields.tsx:80`, `:118`; on the #483 branch also
    `src/features/queries/query-studio-page.tsx` and `save-query-page.tsx` (the draft key).
    All of them read the provider's value, so T7 fixes them without touching them.

### REL-25, the test that fails in one process

- **D9. The cause (read, not run by the planner).** The test installs its capture with
  `tracing::subscriber::set_default` (`rust/crates/lakehouse-api/tests/report_runner.rs:958`),
  which is the default of the test's own thread only. The line it waits for is logged by
  `upstream_error::report` (`rust/crates/lakehouse-api/src/upstream_error.rs:323`), one call
  site shared by every upstream failure. `tracing-core` 0.1.36 decides once per call site,
  the first time any thread reaches it, whether anybody is interested, and keeps the answer
  for the whole process; while one subscriber exists it asks only the default of the thread
  that got there first (`callsite.rs:308-321`, `:544-570`). Another test of the binary
  reaches that call site on its own thread, which has no subscriber, the answer "never" is
  kept, and this test's record is dropped before its subscriber is asked. The test starts
  with `a_`, so it creates its subscriber first and then waits for its database: the other
  order almost never happens. `rust/crates/lakehouse-api/tests/common/mod.rs:211-215`
  already describes the same effect.
- **D10. The fix is in the test only: one subscriber for the whole test binary, and a
  capture per thread.** A subscriber set for the process (once, behind `std::sync::Once`)
  is the default of every thread, so no thread can record "never". Its writer keeps a record
  only on the thread that wrote it, and only while a test on that thread is capturing. A
  `#[tokio::test]` runs its future, and every task it spawns, on the test's own thread, so
  the test reads exactly what its own run logged: another test's "pdf render failed" cannot
  satisfy it. `with_subscriber` / `WithSubscriber` is **not** used: it sets the same
  per-thread default and does not change what D9 describes. Product logging is not touched,
  and the test's three assertions stay word for word.
- **D11. The other captures.**
  - `rust/crates/lakehouse-api/tests/identity_password.rs:184-238`: one subscriber set for
    the process, and an assertion that a text is **absent** from the log. Not this weakness.
    Not changed.
  - `rust/crates/lakehouse-api/src/upstream_error.rs:837-846` (`with_captured_log`, unit
    tests): the same per-thread pattern, on the same call site. The window there is the few
    instructions between creating the subscriber and logging, not a database start, and no
    failure has been seen. Not changed in this slice (section 4); reported so the backlog can
    hold it.
  - No other file under `rust/crates/lakehouse-api/tests/` captures the log.

### RPT-2, a subscription on an installation with no e-mail

- **D12. What the owner decided (`DEC-18`), unchanged here.** A delivery that fails because
  e-mail is not set up is recorded with its reason and is not counted towards the pause
  (`rust/crates/lakehouse-api/src/routes/report_runner.rs:345-353`); the subscriptions list
  says e-mail is not set up. This slice changes no counting, no pausing, no stored reason
  and no line of `report_runner.rs`.
- **D13. The list answers a typed field per subscription, `notSent`.**
  `GET /api/reports/subscriptions?board=` gives every item
  `"notSent": { "cause": "email_not_set_up", "lastAttemptAt": <RFC 3339 text or null> }`
  while e-mail is not set up, and `"notSent": null` while it is. `lastAttemptAt` is the
  start of the subscription's newest delivery **if** that delivery failed with the reason
  `EMAIL_NOT_SET_UP`; otherwise `null` (nothing has been attempted since e-mail went away,
  or nothing ever). The console does not compare sentences to find this out. "E-mail is set
  up" is read once per request, from the same answer that fills `emailSetUp`.
- **D14. What the row shows.** While `notSent.cause` is `email_not_set_up`, the row shows
  **"Not sent: e-mail is not set up on this installation."**, followed by
  **" Last attempt: {time}."** when `lastAttemptAt` is there. That line stands in place of
  the status line ("Sent", "Not sent yet", "Failed. …"); a paused subscription keeps its
  "Paused. …" line under it. An unknown `cause` shows the status line as today. The banner
  above the list and the result of "Send now" are unchanged.
- **D15. The assistant.** `list_report_subscriptions` answers the same `warning` that
  `create_report_subscription` already gives while e-mail is not set up
  (`rust/crates/lakehouse-api/src/routes/ai/tools/reports.rs:193-199`, `:303-305`). No
  argument changes, so `tests/fixtures/tool_schemas.json` is not regenerated. The AI team
  reviews T4.
- **D16. The feature page.** `docs/core/features/scheduled-reports.md` has no acceptance row
  for case 06 at this commit (its checklist ends at `RPT-1-AC5`). T8 adds `RPT-1-AC6` and a
  section quoting `DEC-18`. `DEC-18` is cited for exactly what it says; the words on screen
  are marked *(proposed, not yet signed)*.

### DATA-56, a publish that fails before the export starts

- **D17. The rule (decided).** A failure that happens after the caller is authenticated and
  allowed and the mart is a valid, existing Gold mart of the caller's tenant becomes a
  recorded failed run. A refusal of the request itself records nothing. The stored text is
  the classified sentence, never upstream text. A scheduler run (`?ifChanged=true`) follows
  the same rule.
- **D18. "The mart exists" is one read, after the tenant gate.** The route reads
  `mart_exists` (`rust/crates/lakehouse-api/src/routes/gold.rs:872-874`, the helper `PUT
  …/publication` uses) right after `gated_source_table` (`gold.rs:455`). Before it answers
  `true`, nothing is recorded; after it, every failure is, up to and including the export.
- **D19. Exactly what is recorded and what is not**, in the order of `export`
  (`gold.rs:429-581`):

  | Step (line at this commit) | Answer | Recorded |
  | --- | --- | --- |
  | `check_export_token` (`:437-442`) | 401, 403, 503 "not configured" | no: the caller is not allowed |
  | `Ident::new` (`:444-445`) | 400 | no: not a valid name |
  | `gated_source_table` (`:455`) | 404, 422, 503 of the tenant gate | no: the mart is not shown to be the caller's |
  | `mart_exists` (new) fails | classified 422 or 503 | no: nothing is known about the mart, and the engine that failed holds the history |
  | `mart_exists` answers `false` | the route goes on as today | no early failure is recorded for it |
  | `mart_content_fingerprint` (`:459-465`) fails | classified 422 or 503 | **yes** |
  | `?ifChanged=true`, the history read (`:472-478`) fails | classified 422 or 503 | **yes, attempted** (D21) |
  | `?ifChanged=true` finds nothing to do (`:479-481`) | 200 `skipped` | no, as today |
  | the per-mart lock is held (`:487-497`) | 409 | no: the publish that holds the lock records its own run, and a row here would be newer than it and hide its result |
  | `read_catalog_token` (`:498`) fails | 503 | **yes** |
  | `export_mart` (`:531`) | any | yes, as today (unchanged) |

- **D20. The stored text is #484's conversion and nothing else.** The row's `error` is
  `stored_error(…)`, that is `ApiError::to_string()` of the error the response carries
  (`gold.rs:634-636`). For a fingerprint or history failure that is "The database request
  failed. Reference: …" or "The database is unavailable. Reference: …"
  (`upstream_error::ch_error`). For the token file it is the fixed sentence of
  `lakekeeper_token::token_unavailable_message` (`lakekeeper_token.rs:38-40`): it names the
  purpose and the setting, never the path or the io error, and it has no reference because
  `read_token_file` makes none. No new sentence is written and no sentence embeds upstream
  text. `lastFailure` (`gold.rs:1015-1031`) carries the time and that sentence, no person's
  name.
- **D21. If the history store is what failed, nothing can be recorded.** The history table
  lives in the same ClickHouse the mart does. The write is attempted in every "yes" case of
  D19 and stays best-effort: a write that fails is logged by `record_export_history`
  ("failed to record gold export history", `gold.rs:622-625`) and the response is the
  failure the caller already had. With ClickHouse down the card cannot say "Last publish
  failed"; its own read fails too. This is said in the route's documentation (T2).
- **D22. A scheduler run adds one failed row per run while the cause lasts.** Since
  `DATA-49` a mart whose newest run failed is never skipped, so each scheduled run tries
  again and records again. That is what makes the card honest; it is one small row per mart
  per run.
- **D23. No new refusal is added.** The route has no refusal for "publishing is switched
  off" (it never reads the switch; the scheduler asks only for the enabled marts) and none
  for a name that is not in `system.tables` (such a name goes on and fails where it fails
  today). This slice adds neither: `tests/gold_export_auth.rs` asserts that a publish against
  an engine that lists no table is **not** answered `404`, and the acceptance gate publishes
  without the switch. The run that `export_mart` itself records is not changed, also for
  such a name. Question 2 at the end.
- **D24. How it is built.** `record_export_history` and `stored_error` take
  `Result<&GoldExportResult, &ApiError>` instead of a reference to an owned `Result`, so an
  early failure can be recorded from the error alone and then returned. An early failure's
  row starts at the moment `mart_exists` answered; the export's own `started_at`
  (`gold.rs:530`) is not moved, so a successful run's time means what it meant.

### Text, and the assistant

- **D25. Text on screen (English, as the console).** New: "Not sent: e-mail is not set up on
  this installation." and " Last attempt: {time}." (D14). Nothing else is new: `DATA-56`
  shows sentences that exist, `ADM-15` and `REL-25` show none.
- **D26. Assistant parity.** `RPT-2` is the only dashboard behaviour here; its part is T4
  (D15). `DATA-56` is not a dashboard feature; the assistant's own publish tool is in
  section 4. `ADM-15` and `REL-25` change nothing the assistant uses.

## 2. Tasks

Rust first (T1 to T4), then the console (T5 to T7), then the documents (T8). In every Check,
"must fail" names a temporary wrong change the developer makes, sees the test fail, and
undoes; it is never committed. Say in the handoff that each was done.

### T1. The PDF-failure test reads its own thread's log (REL-25)

Test code only. `rust/crates/lakehouse-api/tests/report_runner.rs`.

- Anchors: `:928-947` (`struct Captured` and its two `impl`s), `:949-974` (the test;
  `set_default` at `:958`, the three assertions at `:966-973`). The product line it waits
  for: `src/routes/report_runner.rs:945-946` (`pdf_failure(err)`), which logs through
  `src/routes/pdf_export.rs:117-126` and `src/upstream_error.rs:314-328`.
- Before changing anything, run `cargo test -p lakehouse-api --test report_runner` once and
  write the counts in the handoff (expected: 23 passed, 1 failed, the test named above). If
  it passes, say so and go on.
- Change, replacing `Captured` (nothing else uses it):
  1. A `thread_local!` holding `RefCell<Option<Vec<u8>>>`, `None` at first: what this
     thread is keeping.
  2. A unit struct (for example `ThreadCapture`) that implements `std::io::Write` by
     appending to that thread-local **when it is `Some`** and doing nothing when it is
     `None`, and `tracing_subscriber::fmt::MakeWriter` by returning itself.
  3. A guard (for example `LogCapture`) with:
     - `start()`: inside a `static` `std::sync::Once`, build
       `tracing_subscriber::fmt().with_writer(ThreadCapture).with_ansi(false).finish()` and
       install it with `tracing::subscriber::set_global_default(…)`, with an `expect` whose
       message says that this test binary installs no other global subscriber (a capture
       that silently records nothing must not pass); then set the thread-local to
       `Some(Vec::new())`;
     - `text()`: the kept bytes as a `String` (lossy);
     - `Drop`: set the thread-local back to `None`.
  4. In the test, replace `:953-958` with `let capture = LogCapture::start();` and `:971`
     with `let log = capture.text();`. Do not change the fixture, the `tick`, the assertion
     on the stored reason, or the two assertions on the log.
  5. Above the helper, a comment that says why (D9 in four lines: the per-thread default,
     the call site shared with other tests, the answer kept for the process), citing
     `REL-25`. Update the doc comment of the test only if it no longer matches.
- Do not use `set_default`, `with_default` or `with_subscriber` in this file. Do not change
  any file under `src/`.
- Check:
  - `a_pdf_failure_is_logged_for_the_operator_and_the_record_stays_fixed` (name and
    assertions unchanged) passes with the whole file: `cargo test -p lakehouse-api --test
    report_runner`, **five runs in a row, all green**; quote the five count lines.
  - New, a plain `#[test]` in the same file:
    `a_log_capture_keeps_what_its_own_thread_logs_and_nothing_from_another_thread`. It
    starts a capture, logs `tracing::error!("logged by another thread")` from a
    `std::thread::spawn(…)` that it joins, logs `tracing::error!("logged by this thread")`
    itself, and asserts the text contains the second and not the first.
  - Must fail: (a) with `let _logged = crate::routes::pdf_export::pdf_failure(err);`
    removed from `src/routes/report_runner.rs:946`, the PDF test fails on
    `log.contains("pdf render failed")` (it reaches the line it is named for); (b) with the
    `Some`/`None` check removed from the writer so every thread appends to one shared
    buffer, the new test fails.

### T2. A publish that fails after the mart is found is a recorded failed run (#484 review R4; DATA-56)

`rust/crates/lakehouse-api/src/routes/gold.rs`,
`rust/crates/lakehouse-api/src/gold_export_history.rs` (comments only),
`rust/crates/lakehouse-api/tests/gold_publication.rs`.

- Anchors in `gold.rs`: `:429-581` (`export`), `:455` (the gate), `:459-465` (the
  fingerprint), `:472-481` (`?ifChanged=true`), `:487-497` (the lock), `:498` (the token),
  `:530` (`started_at`), `:551-557` (`triggered_by`), `:558-567` (the one recording call),
  `:596-629` (`record_export_history`), `:634-636` (`stored_error`), `:872-874`
  (`mart_exists`), `:1622-1646` (the unit test of `stored_error`).
- Change, in this order:
  1. `stored_error` takes `Result<&gold_export::GoldExportResult, &ApiError>` and returns
     `result.err().map(ToString::to_string)`. `record_export_history` takes the same type
     as its last parameter; inside it use `export_result.is_ok()` and `export_result.ok()`
     (the type is `Copy`). The existing call passes `export_result.as_ref()`. The unit test
     at `:1622` passes `result.as_ref()` and `Ok(&GoldExportResult { … })`; its assertions
     do not change (say in the commit that only the parameter type moved).
  2. Move the `triggered_by` binding (`:551-557`, unchanged) up to just after the gate.
  3. After `gated_source_table` (`:455`):
     `let exists = mart_exists(&state.clickhouse, &state.config.gold_source_schema,
     mart_ident.as_str()).await.map_err(|e| upstream_error::ch_error(&upstream_error::DATABASE, &e))?;`
     then `let attempt_started_at = time::OffsetDateTime::now_utc();`.
  4. A private helper next to `record_export_history`:
     `async fn early_failure(state, exists: bool, mart, triggered_by, attempt_started_at,
     content_fingerprint: &str, err: ApiError) -> ApiError`. When `exists`, it calls
     `record_export_history(…, attempt_started_at, time::OffsetDateTime::now_utc(),
     content_fingerprint, Err(&err))`; it always returns `err`. Its doc comment states the
     rule of D19 and D21 and cites `DATA-56` and `#484 review R4`.
  5. The fingerprint (`:459-465`): `let current_fingerprint = if exists { match … }
     else { None };`. On `Err(e)`: build the error exactly as today
     (`upstream_error::ch_error(&upstream_error::DATABASE, &e)`) and
     `return Err(early_failure(…, "", err).await.into());`.
  6. `?ifChanged=true` (`:472-481`): keep the condition; `match` the answer of
     `unchanged_since_last_export_reason`: `Ok(Some(reason))` returns the skip as today,
     `Ok(None)` goes on, `Err(err)` returns
     `Err(early_failure(…, current_fingerprint.as_deref().unwrap_or(""), err).await.into())`.
  7. The lock (`:487-497`): unchanged, and a comment that a `409` is not recorded and why
     (D19).
  8. The token (`:498`): `match`; on `Err(err)` return through `early_failure` with the
     current fingerprint as in step 6. The lock guard is still held while the row is
     written.
  9. Leave `started_at` (`:530`), the call of `export_mart` and the recording after it as
     they are.
  10. Comments that are no longer true (rule 1): the route's doc comment (`:384-428`) gains
      a section "What is recorded" with the table of D19 in words and the sentence of D21;
      `record_export_history`'s doc (`:581-589`, "every `POST` records exactly one row");
      `gold_export_history.rs:19-24` (module doc) and `:250-253` (`NewGoldExportRun`).
- Do not add a refusal for a mart that is not there or for a switch that is off (D23). Do
  not change `read_catalog_token`, `lakekeeper_token.rs`, `gold_export.rs`, or the
  assistant's tool.
- Check, in `tests/gold_publication.rs` (real ClickHouse, the harness of `:90-106`, which
  has no token file; helpers `seed_mart` `:132`, `Sessions` `:157`, `send` `:180`,
  `history_row_count` `:520`):
  1. `a_publish_that_cannot_read_the_catalog_token_is_recorded_and_the_card_says_it_failed`
     (the test the #484 review named). Seed a mart, `POST /api/gold/export/<mart>` as a
     `gold:export` session: `503`, and `body["error"]` equals
     `lakehouse_api::lakekeeper_token::token_unavailable_message("gold-export",
     "LAKEKEEPER_GOLD_EXPORT_TOKEN_FILE")`. Then `GET /api/gold/export/<mart>/publication`:
     `lastFailure.reason` starts with that sentence and equals the response's `error`,
     `lastFailure.failedAt` is a string, `lastExportedAt` is null. Then
     `GET /api/gold/exports?mart=<mart>`: `runs[0].status` is `failed` and `runs[0].error`
     is the same sentence. `history_row_count` is 1.
  2. `a_scheduled_publish_that_fails_before_the_export_is_recorded_each_time`. Same mart
     shape, `POST …?ifChanged=true` twice: both `503`, neither body has `skipped`,
     `history_row_count` is 1 after the first and 2 after the second.
  3. `a_publish_whose_fingerprint_read_fails_is_recorded_with_the_classified_sentence`.
     Create `serving.<base>` (as `seed_mart` does), create a view
     `serving.<mart>` as `SELECT * FROM serving.<base>`, then drop `serving.<base>`: the
     view is in `system.tables` and cannot be read. `POST /api/gold/export/<mart>`: `422`,
     `body["error"]` starts with "The database request failed. Reference: ". Then
     `GET /api/gold/exports?mart=<mart>`: `runs[0].status` is `failed`, `runs[0].error`
     equals the response's `error`, and it does not contain the base table's name. If
     ClickHouse refuses the `DROP`, or the `POST` answers another status, leave this one
     test out, report exactly what was answered, and stop there: the planner corrects the
     plan.
  4. `a_refused_publish_and_a_name_that_is_not_a_mart_record_no_run`. First call
     `gold_export_history::ensure_gold_export_run_table(&ch)` (a refused request never
     creates the table, and `history_row_count` reads it). Then, each followed by
     `history_row_count == 0` for the name used: (a) a seeded mart, a session without
     `gold:export` (`sessions.with("query:read")`): `503` with an `error` that starts with
     "gold export is not configured"; (b) a seeded mart, a person holding `gold:export`
     who belongs only to another seeded tenant (create the principal with
     `create_principal_with_permissions`, insert one `app_user_tenant` row for the tenant
     whose id ends in `…0002`, not for `CATALOG_TENANT`): `404`; (c) a name that was never
     seeded, a `gold:export` session of the catalog's tenant: `503` (the token file), no
     row.
  5. Two assertions written for the old behaviour change, and the commit says why (rule 2:
     the old numbers stated that an early failure writes nothing, which is the defect):
     `:604-608` (`busy_mart`) becomes 2, and `:1112-1116` (`retry_after_failure_mart`)
     becomes 3, each with a message that the `503` at the token file is now a recorded
     failed run.
  - Must fail: test 1 and test 2 with the token site put back to
    `read_catalog_token(…).await?`; test 3 with the fingerprint site put back to
    `.map_err(…)?`; test 4 (c) with the `if exists` removed from `early_failure`.
  - Not tested, and why: the `409` (a second publish in flight cannot be held from a test
    of the router) and the failed history read of `?ifChanged=true` (the history table
    cannot be made to fail without failing every other test of the binary, which shares one
    ClickHouse, `:43-52`). The reviewer reads those two sites.
  - Also run, unchanged, as a guard on the new order of reads:
    `cargo test -p lakehouse-api --test gold_export_auth`.

### T3. The list says, per subscription, that it is not sent while e-mail is not set up (LH-RPT-1-06; RPT-2)

`rust/crates/lakehouse-api/src/routes/reports.rs`,
`rust/crates/lakehouse-api/tests/report_routes.rs`.

- Anchors: `reports.rs:50-52` (the import from `report_runner`), `:429-439`
  (`delivery_json`), `:471-479` (`settings_flags`), `:558-599` (`list`; the last delivery
  at `:591`, `canEdit` at `:593`, `settings_flags` at `:596`), `:911` (unit tests);
  `report_runner.rs:94` (`EMAIL_NOT_SET_UP`);
  `lakehouse-store/src/report_subscriptions.rs:478-497` (`DeliveryStatus::as_str`),
  `:520-537` (`Delivery`).
- Change:
  1. Add `EMAIL_NOT_SET_UP` to the import at `:50-52`.
  2. A private function `not_sent_json(email_set_up: bool, last: Option<&Delivery>) ->
     Value`: `Value::Null` when `email_set_up`; otherwise
     `json!({ "cause": "email_not_set_up", "lastAttemptAt": … })`, where the time is
     `last.filter(|d| d.status == DeliveryStatus::Failed.as_str() && d.reason.as_deref() ==
     Some(EMAIL_NOT_SET_UP)).and_then(|d| d.started_at.format(&Rfc3339).ok())` (import
     `time::format_description::well_known::Rfc3339`, as `routes/capture_lag.rs:45` does).
     Its doc comment cites `RPT-2`, `DEC-18` and QA `LH-RPT-1-06`, and says that nothing
     here counts or pauses.
  3. In `list`: move `let mut body = settings_flags(&state).await;` above the loop, read
     `let email_set_up = body["emailSetUp"] == json!(true);` from it (one read of the
     setting per request, one owner of "is e-mail set up"), and in the loop, next to
     `canEdit`, set `item["notSent"] = not_sent_json(email_set_up, last);`.
  4. `subscription_json`, `get`, `create`, `update`, `send_now` and `deliveries` are not
     changed: `notSent` is the list's field, like `canEdit`. Say so in the list's doc
     comment (`:550-557`) and in the module doc's "Email not set up" section (`:27-33`).
- Check:
  - Unit tests in `reports.rs`: `not_sent_is_null_while_email_is_set_up`;
    `not_sent_names_the_cause_and_the_time_of_a_last_attempt_that_failed_for_it` (the time
    parses back as RFC 3339 to the delivery's `started_at`);
    `not_sent_has_no_time_when_the_last_delivery_was_sent_or_failed_for_another_reason`
    (also with no delivery at all).
  - `tests/report_routes.rs`:
    `the_list_says_a_subscription_is_not_sent_while_email_is_not_set_up_and_nothing_is_counted`.
    With `fixture(&[])` (no mail server): create a subscription as the owner; the list's
    item has `notSent == {"cause":"email_not_set_up","lastAttemptAt":null}`; `POST
    …/send`; the list's item now has a `lastAttemptAt` that is a string and parses as RFC
    3339, `lastDelivery.status == "failed"`, `consecutiveFailures == 0`, `paused == false`.
  - `the_list_says_nothing_of_the_kind_while_email_is_set_up`: the fixture of
    `send_now_to_an_unreachable_mail_server_gives_the_fixed_reason_only` (`:583-589`, a mail
    server named on a closed local port); create a subscription; the item's `notSent` is
    null and `emailSetUp` is `true`.
  - Must fail: the first integration test and the second unit test with `not_sent_json`
    returning `Value::Null` always; the second integration test with it ignoring
    `email_set_up`.

### T4. The assistant's list says so too (RPT-2, assistant parity)

`rust/crates/lakehouse-api/src/routes/ai/tools/reports.rs`,
`rust/crates/lakehouse-api/tests/ai_report_tools.rs`.

- Anchors: `tools/reports.rs:193-199` (`email_note`), `:300-306` (how `create` adds
  `warning`), `:312-326` (`list`).
- Change: in `list`, build the answer as `let mut out = json!({ "items": items });` and add
  `out["warning"] = json!(note)` when `email_note(state).await` is `Some`, exactly as
  `create` does. Reuse `email_note`; write no second sentence. No tool argument changes;
  do not regenerate `tests/fixtures/tool_schemas.json`.
- Check: `tests/ai_report_tools.rs`,
  `listing_reports_warns_while_email_is_not_set_up`: with the file's `fixture()` (no mail
  server), create one subscription through the tool as
  `updating_and_listing_work_on_the_callers_own…` does (`:357-366`), call
  `list_report_subscriptions`, and assert `listed["result"]["warning"]` equals "Email is
  not set up on this deployment, so nothing is sent until it is." and that the items are
  still listed. Must fail: with the two added lines removed.

### T5. The subscriptions dialog shows "Not sent" on each row (LH-RPT-1-06; RPT-2)

`src/services/contracts/reports.ts`, `src/features/dashboards/report-words.ts`,
`src/features/dashboards/subscriptions-dialog.tsx` and their two test files. The two files
under `src/features/dashboards/` are in a directory #486 owns: **find the anchors by their
text**. This dialog is the only place that lists subscriptions (checked: no other file
under `src/features/` calls `reportService.list`).

- Anchors: `contracts/reports.ts:61-81` (`ReportSubscription`; `lastDelivery` and
  `canEdit` are its last fields); `report-words.ts:49-56` (`describeStatus`);
  `subscriptions-dialog.tsx:174` (`<div className="text-xs">{describeStatus(s)}</div>`),
  `:117-121` (the banner, unchanged); `src/lib/format.ts:94-106` (`formatDateTime`, which
  answers "—" for a time it cannot read).
- Change:
  1. Contract: `notSent?: { cause: "email_not_set_up" | string; lastAttemptAt: string |
     null } | null`, with a comment: set by the list only; why the subscription is not
     being sent now (`RPT-2`, `DEC-18`).
  2. `report-words.ts`: export `NOT_SENT_EMAIL = "Not sent: e-mail is not set up on this
     installation."` and `describeNotSent(s): string | null`. It returns `null` unless
     `s.notSent?.cause === "email_not_set_up"`; otherwise `NOT_SENT_EMAIL`, followed by
     `` ` Last attempt: ${formatDateTime(at)}.` `` when `lastAttemptAt` is a string that
     `formatDateTime` can read (not "—"). Reuse `formatDateTime`; write no date code.
  3. Dialog row: compute `const notSent = describeNotSent(s)`. When it is not `null`,
     render it as a `<p>` with the banner's amber text classes (`text-amber-700
     dark:text-amber-400`, `text-xs font-medium`) and **no** `role`; render the existing
     status line only when `notSent` is `null` or `s.paused`. A comment at the site: `// QA
     LH-RPT-1-06 (RPT-2, DEC-18): …`.
  4. Do not change the banner, the buttons, "Send now" or its result line, or
     `EMAIL_NOT_SET_UP`.
- Check:
  - `report-words.test.ts`:
    `describeNotSent says the cause, and the time when there was an attempt` (compare
    against `formatDateTime` of the same text, not a literal, so the machine's time zone
    does not matter); `describeNotSent says nothing for a subscription that can be sent or
    for a cause it does not know`.
  - `subscriptions-dialog.test.tsx`:
    `shows Not sent with the time of the last attempt on a subscription while email is not
    set up, instead of its last status` (list answers `emailSetUp: false` and an item with
    `notSent` and a failed `lastDelivery`: the sentence and the time are on the row, and
    "Failed. Email is not set up on this deployment." is not);
    `keeps the Paused line under Not sent` (the same item, paused with a reason);
    `shows the last status as before when the list says nothing of the kind` (`notSent`
    null).
  - Must fail: the first dialog test with the status line rendered unconditionally; the
    first `report-words` test with the time left out.

### T6. Calls to `/api/auth/*` carry no tenant, and the key can be removed (#483 review; ADM-15)

`src/services/http.ts`, `src/services/http.test.ts`.

- Anchors: `http.ts:23-30` (the module comment on `X-Tenant`), `:63-70`
  (`writeActiveTenantId`), `:87-91` (`resolveUrl`), `:98-109` (`apiFetch`; the header at
  `:100-103`, the `/api/auth/` test at `:105`).
- Change:
  1. `export function clearActiveTenantId(): void`, next to the writer and shaped like it
     (`removeItem`, the same `typeof window` guard and `try`/`catch`).
  2. In `apiFetch`, compute once `const isAuthCall =
     resolveUrl(input).startsWith("/api/auth/")`; attach `X-Tenant` only when it is false;
     use the same constant for the `401` test.
  3. The module comment says that the header is left off `/api/auth/*` and why (D6), citing
     `ADM-15`.
- Check, in `http.test.ts`:
  `leaves X-Tenant off a call to /api/auth even when a tenant is stored` (stored tenant,
  `apiFetch("/api/auth/me")` and `apiFetch("/api/auth/login", { method: "POST" })`: no
  header; `apiFetch("/api/connectors")` in the same test: the header);
  `clearActiveTenantId removes the stored tenant and never throws when storage does` (the
  hostile-storage stub of the test at `:34-47`). The three existing tests stay as they are.
  Must fail: the first with the `isAuthCall` condition removed.

### T7. The stored tenant is the signed-in person's, and sign-out clears it (#483 review; ADM-15)

`src/features/auth/auth-provider.tsx`, a new
`src/features/auth/auth-provider-tenant.test.tsx`, and one test in
`src/components/app-shell/tenant-switcher.test.tsx`.

- Anchors in `auth-provider.tsx`: `:7` (the import from `@/services/http`), `:48` (the
  re-export of the helpers), `:58-65` (the doc of `activeTenantId`), `:115-120` (the
  state and its comment), `:122-131` (`load`), `:137-141` (the read on mount), `:153-158`
  (`logout`), `:160-163` (`setActiveTenant`), `:173-178` (the value). Do not touch
  `src/features/auth/login-page.tsx` (#487): it already calls `refresh()` after a sign-in
  (`:133`), which is `load`.
- Change:
  0. Import `clearActiveTenantId` at `:7` and add it to the re-export at `:48`.
  1. `export function resolveActiveTenant(stored: string | null, tenants: readonly { id:
     string }[]): string | null`: `stored` when a tenant has that id; else
     `tenants[0]?.id ?? null`. Doc comment: D4, citing `ADM-15`.
  2. `load`, after `authClient.me()` answers and **before** `setUser` and `setStatus`:
     read the stored value, resolve it against `me.tenants`; `null` →
     `clearActiveTenantId()`; different from what was stored → `writeActiveTenantId(…)`;
     then `setStoredActiveTenantId(resolved)`. The storage write comes first because
     `apiFetch` reads storage at each call and pages render when the status flips. The
     `catch` branch is unchanged (D5).
  3. Remove the read-on-mount effect (`:137-141`): nothing reads the state before a person
     is loaded, and the value is trusted only after the session answered.
  4. `logout`: `clearActiveTenantId()` first, then the existing `await
     authClient.logout()`, then `setStoredActiveTenantId(null)` with the existing state
     changes.
  5. `setActiveTenant(id)`: return without doing anything unless `user?.tenants` has that
     id (the callback then depends on `user`).
  6. The value: `activeTenantId: user ? resolveActiveTenant(storedActiveTenantId,
     user.tenants) : null`.
  7. Rewrite the comments at `:58-65`, `:115-119` and `:173-177` (they say the stored value
     is not reset and that a stale id falls back; that is no longer what happens).
- Check, in the new test file (stub `next/navigation` and `fetch` as
  `tenant-switcher.test.tsx:10-20` and `:98-123` do; record the URL and the `X-Tenant`
  header of every request; clear `localStorage` before and after each test). The probe
  component makes one `apiFetch("/api/connectors")` when the status is `authenticated`.
  1. `signing out clears the stored tenant`: a person with tenants `a1`, `a2`, stored `a2`;
     call `logout()`; the key is absent, and the sign-out request carried no `X-Tenant`.
  2. `a second person never sends or shows the first person's tenant`: stored `a2` (left by
     a person whose session ended), the session answers a person with tenants `b1`, `b2`:
     the shown `activeTenantId` is `b1`, the key holds `b1`, the session check carried no
     `X-Tenant`, and the request to `/api/connectors` carried `b1`; no recorded request
     carried `a2`.
  3. `a person signing in after another signed out starts in their own first tenant`:
     person A (`a1`, `a2`) picks `a2`, signs out; the stub now answers person B (`b1`,
     `b2`); call `refresh()`; shown `b1`, stored `b1`.
  4. `a person whose stored tenant is still theirs keeps it`: stored `b2`, the person has
     `b1`, `b2`: shown `b2`, stored `b2`, the request carried `b2`. (A control: it passes
     on today's code too; say so in the handoff.)
  5. `a person with no tenant has none stored`: stored `a2`, the person has no tenant:
     shown none, the key is absent, the request carried no `X-Tenant`.
  6. `a failed session check leaves the stored tenant alone`: stored `a2`, the session
     check answers `401`: the key still holds `a2`. (A control.)
  7. `picking a tenant that is not the person's changes nothing`: `setActiveTenant("zz")`:
     shown and stored unchanged.
  8. Unit: `resolveActiveTenant keeps a tenant that is the person's, else takes the first,
     else none`.
- The existing test `setActiveTenant writes lh_active_tenant and a sibling useAuth consumer
  observes the change` (`tenant-switcher.test.tsx:141-166`) gives the person **no** tenants
  and switches to `"a"`, a state the console cannot reach (no switcher is shown) and the new
  rule refuses. Correct it, and say why in the commit (rule 2): the person has `TENANTS`
  (`a`, `b`); wait until `a` is shown; then switch to `b` from a click; expect `b` shown
  and stored. The other tests of that file stay as they are.
- Must fail: test 1 with the clear removed from `logout`; tests 2, 3 and 5 with the
  correction removed from `load`; test 7 with the guard removed from `setActiveTenant`.
- If a test of another file fails because a tenant is now stored after the provider
  renders, do not change its assertion: clear `localStorage` in that file's `beforeEach`,
  and name the file in the handoff.

### T8. The documents say what is now true (RPT-2, DATA-56)

Documents only; no code. The developer does not edit `docs/core/BACKLOG.md` or
`docs/core/PRODUCT.md` (the reviewer does, after the merge).

- `docs/core/features/scheduled-reports.md`:
  - After the section "Owner decisions at the phase 4 QA (2026-10-23)" (`:63-65`), a new
    section, with exactly these two points:

    > ## Owner decision of 2026-11-12 (`DEC-18`)
    >
    > - On an installation with no e-mail set up, a delivery that fails for that reason is
    >   recorded with its reason and is not counted towards the pause; the subscriptions
    >   list says e-mail is not set up (`DEC-18`, QA `LH-RPT-1-06`). This is the exception
    >   to decision 7's pause after 5 failures in a row.
    > - On each subscription the list shows "Not sent: e-mail is not set up on this
    >   installation." and, when the last attempt failed for that reason, its time. Changed
    >   2026-11-12 (`RPT-2`), *(proposed, not yet signed)*.

  - In the acceptance checklist, after `RPT-1-AC5` (`:75`) and before `AI-10-AC1`:

    > - `RPT-1-AC6` With e-mail not set up on the installation, "Send now" and a scheduled
    >   run each record a failed send with the reason "Email is not set up on this
    >   deployment."; the failures are not counted and the subscription is not paused
    >   (`DEC-18`). The subscriptions list shows on that subscription "Not sent: e-mail is
    >   not set up on this installation." with the time of the last attempt. Row added
    >   2026-11-12 (`RPT-2`) for QA `LH-RPT-1-06`; the words on screen are *(proposed, not
    >   yet signed)*.

  - In "Asking the assistant" (`:56-61`), one sentence: `list_report_subscriptions` answers
    a `warning` while e-mail is not set up (`RPT-2`).
  - Do not edit the Decisions table (`:34-47`).
- `docs/core/features/gold-publish-per-mart.md`, at the end of "Limits to tell a customer"
  (after the point that ends "only "Publish now" works.", `:79-80`):

  > - A publish that fails before the copy starts (for example, the catalog's credentials
  >   cannot be read) is recorded as a failed publish with its reason, also for an
  >   automatic publish. A request that is turned away is not a publish and is not
  >   recorded: no permission, a table of another organisation, or another publish of the
  >   same table still running. If the database that keeps the publish history is itself
  >   down, nothing can be recorded. Changed 2026-11-12 (`DATA-56`), *(proposed, not yet
  >   signed)*.

  Do not edit its Decisions table or any "Signed" cell.
- Check: `git diff --stat` shows only these two files in this commit.

## 3. Verification

The build machine is a strong ARM64 host (20 CPUs, 121 GB of memory, Docker running); see
section 3a for the checkout and the cargo settings. CI on the pull request runs on x86_64
and is the final word: a check that is green here and red there is red.

Per commit, only for what the commit touched. Never `cargo clean`. Never a bare
`cargo test -p lakehouse-api <filter>` and never `cargo test --workspace` (CI runs the full
suite): a unit test runs with `--lib` and its name, an integration test file runs with
`--test <file>`.

- Rust, from `rust/`: `cargo fmt --check`, then
  `cargo clippy -p lakehouse-api --all-targets -- -D warnings`, then the tests of the task:

  | Task | Tests |
  | --- | --- |
  | T1 | `cargo test -p lakehouse-api --test report_runner`, once before the change (write down the counts), then five times in a row after it, all green |
  | T2 | `cargo test -p lakehouse-api --lib the_stored_error_of_a_failed_publish_is_the_classified_sentence`; `cargo test -p lakehouse-api --test gold_publication`; `cargo test -p lakehouse-api --test gold_export_auth` |
  | T3 | `cargo test -p lakehouse-api --lib not_sent_`; `cargo test -p lakehouse-api --test report_routes` |
  | T4 | `cargo test -p lakehouse-api --test ai_report_tools` |

- TypeScript: `bun install --frozen-lockfile` once, then per commit `bun run typecheck`,
  `bun run lint`, and `bun test` on the touched test files:
  `src/features/dashboards/report-words.test.ts` and
  `src/features/dashboards/subscriptions-dialog.test.tsx` (T5);
  `src/services/http.test.ts` (T6); `src/features/auth/auth-provider-tenant.test.tsx`,
  `src/components/app-shell/tenant-switcher.test.tsx` and, as a guard,
  `src/features/auth/login-page.test.tsx` (T7). Do not run prettier.
- Always: `python3 ops/lint/check_bare_iceberg_count.py && python3
  ops/lint/check_intra_package_imports.py`.
- No Python, no migration, no route added (`POLICY_TABLE` and `tests/route_auth.rs` are not
  touched), no compose file: none of their checks apply.

Before handoff, once, in the foreground, on the final commit:

```bash
cd rust && cargo fmt --check
cargo clippy -p lakehouse-api --all-targets -- -D warnings
cargo test -p lakehouse-api --lib the_stored_error_of_a_failed_publish_is_the_classified_sentence
cargo test -p lakehouse-api --lib not_sent_
for i in 1 2 3 4 5; do cargo test -p lakehouse-api --test report_runner || break; done
cargo test -p lakehouse-api --test gold_publication
cargo test -p lakehouse-api --test gold_export_auth
cargo test -p lakehouse-api --test report_routes
cargo test -p lakehouse-api --test ai_report_tools
cd .. && bun run typecheck && bun run lint && bun run test
python3 ops/lint/check_bare_iceberg_count.py && python3 ops/lint/check_intra_package_imports.py
git status   # nothing generated staged
```

The full `cargo test --workspace` is CI's on the pull request; list it in the handoff as
*not verified locally, CI*. The handoff quotes each command with its counts, names every
test added or changed so it can be found in the CI log, says which "must fail" changes were
made and seen to fail, and says *not verified* for anything not run (rule 7).

What the planner read and did not run, and the developer's runs must confirm. If one of
these answers differently, report the answer and stop that task; the plan is corrected, not
the assertion.

- That the `report_runner` binary fails as D9 says and passes five times after T1. The
  cause is read from `tracing-core` 0.1.36 and from the note in `tests/common/mod.rs`; no
  test was run.
- That the record the PDF test waits for is written on the test's own thread (the runner
  logs it inside `tick`, which the test awaits on a current-thread runtime).
- That `set_global_default` succeeds in that binary (nothing else in it or in
  `tests/common/mod.rs` installs one).
- That ClickHouse 26.8 lets a view outlive the table it reads, and that reading it then
  fails with a statement error that becomes `422` (T2, test 3).
- That a person holding `gold:export` who belongs only to another tenant is answered `404`
  by the gate in the `gold_publication` harness, and a session without the permission
  `503` "gold export is not configured" (T2, test 4).
- That no other console test depends on nothing being stored after the provider renders
  (T7; the full `bun run test` is the proof).
- That `formatDateTime` reads the RFC 3339 text the API writes (T5).

## 3a. Machine

- Checkout: `/home/rismana/rantai/lakehouse-agents-1` (the planner fills in `<n>`).
- Before any cargo command:
  `export CARGO_TARGET_DIR=/home/rismana/rantai/cargo-target-1 CARGO_BUILD_JOBS=6`.
- The machine is shared. Work only under `/home/rismana/rantai/`. Start no container you do
  not remove.
- The Rust integration tests start containers: one Postgres through
  `lakehouse-test-support`, reused when it already runs, and `gold_publication` starts a
  ClickHouse of its own. Before the first `cargo test --test …`, run `docker ps -a` and
  write down what is there. After the last one, remove what your runs started and nothing
  else (`docker rm -f <id>`); a container that was there before belongs to another
  checkout. Say in the handoff which you removed.

## 4. Out of scope

- **The order of `tenants` in `GET /api/auth/me`** (`list_tenants_by_ids` has no `ORDER
  BY`, D4). After T7 the console sends the tenant it shows, so the two orders no longer
  have to agree. A browser whose storage cannot be written (private browsing) still sends
  no header and shows `me.tenants[0]`; that case is as it is today.
- **Clearing the stored tenant when a session expires** (D5), **two tabs signed in as two
  people**, and **a tenant list that comes back empty because its read failed**
  (`routes/auth.rs:376-381` answers an empty list then; T7 removes the stored choice, which
  costs the person their last pick and nothing else).
- **The unit tests of `upstream_error.rs` that capture the log with `with_default`** (D11):
  the same pattern with a far smaller window; not seen failing. For the backlog.
- **Any change to counting, pausing, the stored reason, the banner or "Send now"** of a
  report (`DEC-18`), and **an e-mail or other notice to the owner** of a subscription that
  is not sent.
- **A refusal of `POST /api/gold/export/<mart>` for a mart whose switch is off or whose
  name is not a table** (D23), and **the run that `export_mart` records for such a name**.
- **The assistant's publish tool** (`routes/ai/tools/gold.rs`): it calls `export_mart`
  itself and writes no run of any outcome. Not a finding of #484 R4; for the backlog.
- **A reference in the token-file sentence** (D20): `read_token_file` is shared with the
  outside-reading door, which repeats the same sentence.
- **`docs/core/BACKLOG.md` and `docs/core/PRODUCT.md`**, and the QA case `LH-RPT-1-06`
  itself, which lives in the QA base: the planner rewords it there.
- Everything else in the QA round that is not on rows `ADM-15`, `REL-25`, `RPT-2` and
  `DATA-56`.

### Open questions for the owner

None of these blocks the plan.

1. The new row says "installation"; the banner above the list and the stored reason say
   "Email is not set up on this deployment." Should all three use one word? The planner's
   proposal: keep them for now (the stored reason is compared by the runner and is in every
   past record), and choose one word in a later pass over the texts.
2. The console hides "Publish now" while a mart's switch is off, but the API still
   publishes such a mart when asked (`POST /api/gold/export/<mart>` never reads the
   switch). Should the API refuse it? The planner's proposal: its own backlog item, because
   the acceptance gate, several tests and the assistant's tool publish without the switch.
3. A session that expires keeps the person's last tenant for their next sign-in; only
   "Sign out" clears it (D5). The planner's proposal is to keep that; say if an expired
   session should clear it too.

## 5. Handoff

## 6. Review
