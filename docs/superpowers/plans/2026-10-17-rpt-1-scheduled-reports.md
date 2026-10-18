# RPT-1: scheduled reports by email (with AI-10, schedule half)

Planner and reviewer: Claude Opus. Developer: a non-Opus coding agent.
Branch: `feat/bi-phase-4` (worktree `/home/hv/lakehouse-phase4-build`), after
`BI-1` (`docs/superpowers/plans/2026-10-17-bi-1-export.md`) is committed:
this feature attaches its PDF and data files. Feature page:
`docs/core/features/scheduled-reports.md` (decisions 1 to 10, planner
defaults confirmed at the phase 4 QA). Spec: `docs/core/specs/rpt-1.md`,
`docs/core/specs/ai-10.md`.

Anchors read on 2026-10-17 on `origin/main` (`48f3295b`) and
`feat/bi-phase-3` (`100c466a`); find each by the symbol named.

## 1. Decisions already made (do not re-ask, do not change)

The feature page's table, plus:

- **D1 Storage in Postgres.** Migration `rust/migrations/0088_report_subscriptions.sql`
  (why-header): `report_subscription` (id, board, owner principal id,
  schedule kind and fields, time zone, formats and chosen charts as JSON,
  filters and parameters as JSON, skip_when_empty, paused + paused_reason,
  consecutive_failures, next_run_at, created/updated), `report_recipient`
  (subscription, either a user id or an email address, never both),
  `report_delivery` (subscription, started/finished, status
  `sent|failed|skipped`, recipient count, reason — a fixed sentence, never
  SMTP or Gotenberg text), and `reporting_settings.report_allowed_domains
  TEXT[] NOT NULL DEFAULT '{}'`. Store module `lakehouse-store/src/report_subscriptions.rs`.
- **D2 Schedule maths** in one pure module (`lakehouse-bi` or
  `lakehouse-store`, the developer picks the crate that already holds time
  zone code; BI-9's `TimeContext` / `chrono-tz` use): `next_run_after(schedule,
  zone, now) -> DateTime<Utc>`; daylight-saving gaps and repeats handled
  (a time that does not exist moves to the next valid minute; a repeated one
  runs once). Unit-tested across a DST change in a zone that has one
  (Europe/Amsterdam) even though the default zone has none.
- **D3 Runner in the API.** `spawn_report_runner` beside the other
  background tasks in `main.rs` (60-second interval, missed ticks skipped).
  Each tick claims due rows with one statement:
  `UPDATE report_subscription SET next_run_at = <next> ... WHERE id IN
  (SELECT id FROM report_subscription WHERE NOT paused AND next_run_at <= now()
  ORDER BY next_run_at LIMIT 5 FOR UPDATE SKIP LOCKED) RETURNING ...` (the
  next time computed in Rust per row inside the same transaction), so two
  replicas never take the same row and a missed run happens once
  (feature decision 8). Sends run with the `BI-1` PDF semaphore shared, at
  most 2 at a time.
- **D4 Building a report** as the owner (feature decision 1): load the owner
  principal by id with current roles (the `BI-1` render data route's
  loader, one helper), refuse if disabled or no longer allowed to download
  (recorded as a failure with the reason), make the PDF through the `BI-1`
  Gotenberg path with the subscription's filters, and each chosen chart's
  file through the `BI-1` export code with the same filters (same masking).
  Skip-when-empty uses the row counts of the data charts.
- **D5 Email.** `lakehouse-notify` gains attachments: `EmailSender::send_with_attachments(to,
  subject, html, attachments)` building a `multipart/mixed` message with
  `lettre` (already a dependency); the existing `send` stays. One email per
  send with every recipient in Bcc and the deployment's `SMTP_FROM` as
  To, so recipients do not see each other. Subject: "<dashboard> report,
  <date in the subscription's zone>". Body: the dashboard name, the filters,
  who set it up, the decision-1 sentence, a link to the dashboard in the
  console, and which files were left out (feature decision 5).
- **D6 Failure notice.** On a failed send: `consecutive_failures + 1`; an
  in-console notification for the owner through the existing notification
  path if one exists (find it: the bell in the header), else the owner's
  email only; at 5 the row is paused with `paused_reason`. A success resets
  the count. Deliveries older than 90 days are removed by the runner once a
  day.
- **D7 Routes** (all in `POLICY_TABLE`, asserted both ways in
  `tests/route_auth.rs`): `GET/POST /api/reports/subscriptions?board=`,
  `GET/PUT/DELETE /api/reports/subscriptions/{id}`,
  `POST /api/reports/subscriptions/{id}/send` (send now, recorded),
  `GET /api/reports/subscriptions/{id}/deliveries`. Authorisation in the
  handler: viewer of the board + downloads allowed to create; owner or an
  administrator (`settings:write`, as the other admin-only acts in this
  phase) to change. A narrow recipient picker route
  `GET /api/reports/recipients?q=` that returns active users' names and
  emails to anyone allowed to subscribe (not `identity:read`'s full list),
  at most 20 per query.
- **D8 Settings.** Admin > Settings gains "Reports: outside email domains"
  (validated domain names, lower-cased, no wildcards, at most 50).
- **D9 Console.** Dashboard ⋯ "Subscriptions…": list (schedule in words,
  recipients, last delivery status), create/edit dialog (schedule, time
  zone, PDF on/off with paper and orientation, charts and their format,
  recipients with the picker and free addresses, filters prefilled from the
  current view, skip when empty, the decision-1 sentence), "Send now",
  pause/resume, delete, delivery history.
- **D10 Assistant (AI-10).** Tools `create_report_subscription`,
  `list_report_subscriptions`, `update_report_subscription`,
  `delete_report_subscription`. Risk classes: create/update WriteLow when
  the only recipient is the person, and needs approval when any other
  recipient is named (decide per call in the gate, as the existing gate
  decides per tool; feature decision 10); delete needs approval. Named in
  `capabilities.ts`, prompt line, fixture regenerated, count pin.

## 2. Anchors (find by symbol)

- Email: `lakehouse-notify/src/lib.rs` `SmtpConfig`, `build_message`,
  `EmailSender::send`, `deliver`; `routes/alerts.rs` `smtp_config`;
  `config.rs` SMTP fields; compose `SMTP_*`.
- Digests and delivery records to mirror: `lakehouse-alerts/src/lib.rs`
  `run_digest`, `digest_text`; `routes/alerts.rs` `persist_fired_results`,
  `load_alerts::delivery_record`; migrations 0011, 0038, 0068
  (`alert_instance.delivery_status`).
- Background tasks: `main.rs` `spawn_auth_cleanup`, `spawn_run_log_cleanup`,
  `spawn_trash_sweep`.
- Time zone: `routes/settings.rs` `time_context`; `lakehouse-bi` grain /
  `TimeContext`.
- Users: `lakehouse_store::identity::list_users`, `USER_SELECT`, `app_user.email`.
- BI-1 pieces (after it lands): the render-token loader, the Gotenberg
  client and semaphore, the chart export code, `may_download`.
- Assistant: `routes/ai/registry.rs`, `routes/ai/gate.rs` (per-call risk),
  `routes/ai/tools/`, `prompt.rs`, `capabilities.ts`.

## 3. Tasks (one commit each, in order)

**T1 Migration, store, schedule maths (D1, D2).** *Accept:* store tests
(create, recipients exclusive, claim with SKIP LOCKED from two connections
gives disjoint rows, deliveries pruning); schedule tests (every kind, month
end, DST gap and repeat, missed run happens once).

**T2 Email attachments (D5).** *Accept:* message tests (multipart parts,
Bcc, file names, size accounting), no network in unit tests.

**T3 Runner and report building (D3, D4, D6).** *Accept:* tests with a fake
SMTP sender and a wiremock Gotenberg: a due subscription sends once and
records `sent`; skip when empty records `skipped`; owner disabled →
`failed` with the reason; 5 failures → paused; the 20 MB rule leaves a
file out and says so; filters reach the statements; SEC-11 guard.

**T4 Routes and settings (D7, D8).** *Accept:* route tests per rule
(creating needs view + download; outside domain not listed refused;
inactive user refused; 51 recipients refused; only owner or admin edits;
send now records a delivery); `route_auth` both ways; `route_policy`.

**T5 Console (D9).** *Accept:* component tests for the dialog (schedule in
words, outside-domain error, the decision-1 sentence present), webpack
build.

**T6 Assistant (D10).** *Accept:* tool tests (self only → no approval;
another recipient → approval; delete → approval; `AI-10-AC1` arguments),
fixture, count pin, `capabilities.test.ts`.

**T7 Docs.** CHANGELOG; `.env.example` unchanged unless a new variable
appears (none planned).

## 4. Out of scope

Slack/Teams/Google, digest schedules (`BI-19`), unsigned data links in
email, per-recipient masking.

## 5. Must verify, not assume

- Whether an in-console notification path exists for a user (D6).
- That `lettre`'s enabled features cover attachments; if a feature must be
  added, it is a dependency change (`cargo deny`/`cargo audit` green).
- That nothing else in the API already runs a per-minute loop that this
  should join instead of adding one.

## 6. Checks

As the BI-1 plan's section 6, plus `cargo test -p lakehouse-notify` and the
new store tests (testcontainers Postgres).

## 7. Handoff (developer appends)

Developer: Claude Sonnet. Branch `feat/bi-phase-4`, after BI-1 and the phase 3 merge (`7b37ac67`). Nothing pushed, no PR.

### Commits (one per task, plan order)

| Task | Commit | Subject |
| --- | --- | --- |
| T1 | `ca7255e4` | report subscriptions, recipients, deliveries and the schedule maths (migration 0088) |
| T2 | `97183b52` | an email can carry attachments, every recipient in Bcc |
| T3 | `ccec4e8a` | the report runner sends due subscriptions as their owner |
| T4 | `841886c4` | subscription routes, the recipient picker and the outside-domain setting |
| T5 | `654367c7` | Subscriptions on the dashboard menu and the outside-domain setting |
| T6 | `538883a7` | the assistant schedules reports; others need approval |
| T7 | `b6b81f8c` | CHANGELOG |

Only one migration, `0088`; `0089` was not needed.

### Deviations from the plan (all accepted by the planner)

- **Time zone database: `jiff`, not `chrono-tz`.** The workspace has no `chrono-tz` (the zone list lives in the engine). `jiff 0.2.37` was already in `Cargo.lock` through `serde_with` with its bundled `jiff-tzdb`; it is now a direct dependency of `lakehouse-store` with `tzdb-bundle-always`, so the schedule does not depend on the image having `tzdata`. The lockfile gains two dependency edges and no package. `cargo deny check`: advisories, bans, licenses, sources ok. `cargo audit` (the CI ignores): exit 0, the same 2 allowed warnings as before (`paste` unmaintained, `chacha20` yanked).
- **`CONSOLE_PUBLIC_URL`, a new optional variable** (compose `${CONSOLE_PUBLIC_URL:-}`, `.env.example`, `Config::console_public_url`, trimmed and `http(s)` only). The email links to `<it>/dashboards/<id>`. It is never taken from a request, because a header the caller controls would put a link of their choosing in an email to outsiders. Unset: no link.
- **`send_with_attachments` takes the Bcc list first** (the To is the sender by D5).
- **The claim is two statements in one transaction** (`SELECT ... FOR UPDATE SKIP LOCKED`, then one `UPDATE` per row with the next time computed in Rust), not one `UPDATE ... IN (SELECT ...)`.
- **D6 falls back to the owner's email.** No per-user notification list exists (the bell lists open alerts and pending approvals for everyone), so a failed scheduled send emails the owner; the subscription's list shows the status and reason. The owner is emailed after every failed scheduled send (not after "Send now").
- **Saving without email is allowed** (planner note); the list returns `emailSetUp: false`. A PDF is refused at save while the deployment has no PDF service.
- **`report_delivery.origin`** (`schedule` or `manual`) was added to the plan's column list so "Send now" is recorded as such.
- A person's list of a dashboard's subscriptions holds their own; an administrator (`settings:write`) sees all. The assistant changes only the asking person's own subscriptions.
- Refactors of BI-1 code so the runner reuses it (rule 4): `pdf_export::make_pdf` and `PdfFailure` (the route calls it), `dashboard::build_chart_file` (the route calls it), `ChartExportQuery::for_report`, `render::principal_of`, `alerts::smtp_config` made `pub(crate)`; `ai::request_approval` is the one copy of approval creation for the chat loop and `POST /api/ai/tool`.

### "Must verify" findings (section 5)

1. **In-console notification path for a user:** none (above).
2. **`lettre` features:** the existing `builder` feature already covers `MultiPart` and `Attachment`; no dependency change.
3. **Another per-minute loop to join:** none exists (the other background tasks run hourly or daily); `routes::report_runner::spawn` is new.

### How the assistant rules were built (T6, for the AI team)

- `gate::effective_risk` makes `create_report_subscription` and `update_report_subscription` `WriteHigh` for a call whose `recipients` is not empty; otherwise `WriteLow` (Confirm). `delete_report_subscription` is always `WriteHigh`. Ask mode refuses all three.
- An approved action runs as the approver. Before an approval exists, `tools::approval_args` checks the requester's rights with the same `validate_input` the tool uses, and writes their id into the stored arguments as `ownerId`. When the approved call runs, the tool makes the subscription the requester's and validates it as them today; the approver must also be able to download and see the dashboard (`decide_approval` already requires the registry permission `dashboard:read`). `gate::strip_confirmed` drops a model-supplied `ownerId`. Tests: `ai_report_tools.rs` (8).
- **Known limit:** `update_report_subscription` that names no recipients is a confirmation even when the stored subscription already has other recipients, so a person can change what an existing outside list receives (attachments, filters) with Confirm only. Adding or replacing recipients needs approval. If the AI team wants it stricter, the gate would need the stored recipients (an async lookup); not built.
- Standard request set (`AI-15`): `docs/core/specs/ai-15.md` is a spec and no request-set file exists; "Send this dashboard to me every Monday as a PDF." (`AI-10-AC1`) is covered by the test `the_quoted_request_creates_a_monday_pdf_subscription_for_the_person_after_confirm`, not by a request-set entry.

### Checks (`CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-uiux-target CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0`, one cargo command at a time, `--test-threads=2`)

Final tree (`b6b81f8c`, plus this handoff):

- `cargo fmt --check` (workspace): clean. `cargo clippy -p lakehouse-notify -p lakehouse-store -p lakehouse-api --all-targets -- -D warnings`: clean (crate sources touched first, because a second worktree shares the target and left a stale `lakehouse-bi`; several runs failed to compile for that reason and were rerun after a `touch`).
- `cargo test -p lakehouse-notify`: 25. `lakehouse-store`: lib `report_schedule` 15; `--test report_subscriptions` 12; `--test reporting_settings` 8.
- `lakehouse-api --lib`, filters `routes::ai routes::report routes::settings routes::tests policy config pdf export`: 672 passed, 1 ignored (run on the tree before the last test-only edits; the `request_approval` refactor is covered by the integration files below).
- Integration, one file at a time: `report_runner` 14, `report_routes` 10, `reporting_settings_route` 11, `ai_report_tools` 8, `ai_tool` 21, `agents_approval` 13, `ai_export_tools` 7, `route_auth` 49, `sec11_guard` 4. `write_tool_schema_fixture` run once: `tool_schemas.json` +308 lines, 102 to 106 tools.
- `cargo deny check`: advisories ok, bans ok, licenses ok, sources ok (cargo-deny 0.20.2). `cargo audit` with the CI ignores (0.22.2): exit 0, the same 2 allowed warnings (`paste`, `chacha20` yanked).
- TypeScript: `bun run typecheck` clean; `bun run lint` 0 errors, 8 warnings (none in files of this branch); `bun run test` 2234 pass, 1 skip, 0 fail; `bun --bun next build --webpack` succeeds.
- `docker compose --profile '*' config --quiet` with throwaway values: exit 0, `CONSOLE_PUBLIC_URL` reaches `lakehouse-api`.


### Not verified, and why

- **A real mail server or a real Gotenberg run of a report.** The runner is proved with a fake `ReportMailer` and a `wiremock` Gotenberg; `lettre`'s own message is proved by building it and reading it back (multipart, Bcc kept out of the headers and in the envelope). No scratch SMTP container was started. The first real email is the owner's QA (`RPT-1-AC1`, `AC5`).
- **The dialog in a real browser** (component tests only); the console's `next build --webpack` succeeds.
- **The PDF of a real dashboard** as made by the print view: BI-1's open item.
- **Daylight saving against a real clock:** proved by unit tests across Europe/Amsterdam's 2026 changes, not waited for.
- **A crash between claim and send** loses that one send (never sends twice); not tested by killing a process.
- The planner note about a second migration: not needed.

### Fix round 1 (developer, after Review 1)

Commits, one per finding where the hunks allow; each site cites "RPT-1 review fix (BLOCKER|SHOULD-FIX) Rn".

| Finding | Commit | What changed |
| --- | --- | --- |
| R1 | `e7bffd08` | `headless_refusal` refuses `create/update/delete_report_subscription`; `headless_tools` does not offer them; `ownerId` is stripped from every headless call. Tests: refusal and tool list in both modes; `ownerId` dropped. |
| R2 | `b3ec337d` | `tools::report_audience` loads, before the gate at both dispatch sites (chat loop, `POST /api/ai/tool`; headless refuses the tool, R1), who the update leaves receiving the report and whether it does more than narrow; `gate::decide_with` / `effective_risk_with` take it as a server-side argument (never in the arguments), failing closed. A new schedule, chart, filter or recipient for a stored outside list is an approval; pausing, resuming and removing a recipient are Confirms; the Confirm sentence says no recipient is added. Also fixed: the check before an approval validated an update with no recipients (now `keep_stored_recipients`). Tests: the audience rule and the gate (unit), `ai_report_tools` (approval for a new schedule and a new recipient, Confirm for removal and pause). |
| R3 | `044a3a68` | `cap_reason` cuts the stored reason to 500 characters on a character boundary; the email keeps the full list. Tests: unit; ten missing charts send and record. |
| R4, R5 | `4a367f60` | `PUT {"paused": bool}` and a paused-only assistant update go to `apply_pause` without re-validation; a non-owner administrator may only pause, resume, delete and remove recipients (`only_removes_recipients`), and Send now is owner-only. The two share the `update` route's hunks, so one commit. The dialog sends only the flag. Tests: route (pause after the domain and right are gone; the administrator's allowed and refused acts), assistant, dialog. |
| R6, R7 | `2b38335c` | Each file's audit entry has `recipientCount`, `outsideAddresses`, `origin`; a PDF failure goes through `pdf_failure` (logged under a reference), chart failures and read-back failures are logged with the subscription and chart. Tests: audit entries of a scheduled and a manual send; the log holds the cause while the record stays fixed. |
| R8 | `093d2f6b` | At send time a console recipient needs `dashboard:read`, a role that may download, and a shared tenant when the owner has tenants; others are skipped and counted. Tests: audience (unit) and the runner (skipped, then nobody left, then received after joining the tenant). The test fixture in `report_routes.rs` needed a recipient who can view dashboards. |
| R9 | `a08be883` | No owner email after a failed Send now; "Email is not set up" is not counted toward the five; the SMTP envelope holds only the Bcc list; a busy PDF service brings the next run forward 5 minutes once (`store::retry_at`, `is_second_busy`). Tests: envelope, manual failure, six runs without email, `retry_at`, the second-busy rule. |

The NOTE items (a requester holding `agent:approve` approving their own outside send; no rate limit on Send now; a report past the 310 s route budget cut with nothing recorded; the dialog's direct service calls; console Delete without a confirm) are left for the backlog, as the review says.

**Checks** (`CARGO_INCREMENTAL=0`, detached, `--test-threads=1` for the `sqlx::test` files, crate sources touched before each run because the shared target kept a stale `lakehouse-bi`): `cargo fmt --check` clean; `cargo clippy -p lakehouse-notify -p lakehouse-store -p lakehouse-api --all-targets -- -D warnings` clean on the final tree; `lakehouse-api --lib` filters `routes::ai routes::report agents` 422 passed, 1 ignored; integration `report_runner` 19, `report_routes` 12, `ai_report_tools` 11, `agents_approval` 13, `ai_tool` 21, `route_auth` 49, `sec11_guard` 4; `lakehouse-store --test report_subscriptions` 13, `--test reporting_settings` 8; `lakehouse-notify` 25. TypeScript: typecheck clean, lint 0 errors / 8 warnings, `bun run test` 2234 pass, 1 skip, 0 fail.

**Not verified:** the busy-PDF retry end to end (it needs both PDF slots held for the 60 s slot wait; the rule and the store move are tested separately); R8's tenant rule against a real multi-tenant deployment (tested with one tenant); `request_approval` and the audience lookup in the headless loop are not reachable by design (R1).

## 8. Review (planner appends)

### Review 1 (planner, 2026-10-18, at `d9f96362`)

Read against the plan, the feature page, BI-1 section 8 and DEC-8. R1 was checked by the planner in the code: `agents.rs` passes the model's arguments to the tool without stripping and takes `spec.risk` as declared, not `gate::effective_risk`. The reviewer's own test re-run was skipped at the owner's standing request.

**R1 BLOCKER: headless employee runs skip the DEC-8 gate and the owner stamp.**
- **Where:** `agents.rs` (around 1218, 1348, 1371) and `tools/reports.rs` (`ownerId` is trusted).
- **How it fails:** a build-mode employee can create a subscription to outside recipients with no confirm and no approval. With a model-supplied `ownerId`, it can make that subscription as an administrator, whose unmasked data then goes out on a schedule.
- **Decision:** the report write tools are refused in headless runs (`headless_refusal`). This fails closed: a scheduled run has no person to own a report. `ownerId` is stripped in the headless loop as well.
- **Accept:** tests that a headless call with recipients is refused, and that a model-supplied `ownerId` never reaches the tool.

**R2 BLOCKER: changing what an existing outside list receives needs only a Confirm, and the Confirm says "only you".**
- **Decision:** before `gate::decide`, at every dispatch site, an async step loads the recipients the call will leave in place for `update_report_subscription`: the stored ones when none are named, else the named ones. The call is `WriteHigh` when that list holds anyone but the requester, unless it only narrows (pausing, or recipients that are a subset of the stored ones), which is a chat Confirm under DEC-8's refinement.
- The result is passed to `effective_risk` as a server-side argument, never as a model-visible one. The Confirm sentence names the real audience.
- **Accept:** a stored outside list with a new chart needs approval; removing a recipient or pausing is a Confirm.

**R3 SHOULD-FIX: a long "left out" note breaks the delivery record after the email has gone.**
- **Cause:** the reason exceeds the 500-character `CHECK`.
- **Decision:** cap the stored reason on a character boundary; the full list stays in the email.
- **Accept:** a test with 10 left-out charts.

**R4 SHOULD-FIX: Pause and Resume re-validate the whole subscription,** so a report the person no longer wants cannot be stopped once a recipient, domain or right has changed.
- **Decision:** pause and resume only (route and tool) go straight to `set_paused`.
- **Accept:** a test per case.

**R5 SHOULD-FIX: a non-owner administrator can add recipients to someone else's report and "Send now".**
- **Decision:** a non-owner administrator may view, pause, resume, delete and remove recipients, nothing else. Anything else is refused with a sentence.
- **Accept:** a test.

**R6 SHOULD-FIX: the audit does not say who received the files.**
- **Decision:** add the recipient count, the outside addresses and `origin` to each attachment entry.

**R7 SHOULD-FIX: PDF and chart-file failures leave no operator trace.**
- **Decision:** log the cause through `upstream_error`. The stored sentence stays fixed.

**R8 SHOULD-FIX (planner default, owner confirms at QA): recipients' own rights.**
- At send time, a console recipient who no longer has `dashboard:read`, or whose roles may not download, is skipped. The delivery says how many were skipped.
- Tenants: a console recipient must share a tenant with the owner when the owner has tenants.
- **Accept:** tests.

**R9 SHOULD-FIX (from the review's notes):**
- A failed "Send now" does not email the owner; the dialog shows the reason, as the handoff says.
- "Email is not set up" does not count toward the five failures.
- The SMTP envelope holds only the Bcc list, so the `SMTP_FROM` mailbox gets no copy, or the page says it does.
- `PDF_BUSY` retries in 5 minutes once, instead of losing the period's send.

**NOTE (backlog, not here):**
- a requester holding `agent:approve` can approve their own outside send;
- "Send now" has no rate limit;
- a report over the 310 s route budget is cut with nothing recorded;
- the dialog's direct service calls should use `useServiceAction`;
- console Delete has no confirm.

**Accepted deviations:** `jiff`; `CONSOLE_PUBLIC_URL`; Bcc first; the two-statement claim; the owner email in place of an in-console notice; saving without SMTP; the `origin` column; the list scopes; the BI-1 refactors. **Not accepted:** the "known limit", which is R2.
