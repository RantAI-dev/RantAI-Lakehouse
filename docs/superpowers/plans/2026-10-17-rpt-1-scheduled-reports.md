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
| T6 | `538883a7` | the assistant schedules reports; others need approval (committed by the coordinator from my tree) |
| T7 | `b6b81f8c` | CHANGELOG (committed by the coordinator from my tree) |

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

CHECKS_PLACEHOLDER

### Not verified, and why

- **A real mail server or a real Gotenberg run of a report.** The runner is proved with a fake `ReportMailer` and a `wiremock` Gotenberg; `lettre`'s own message is proved by building it and reading it back (multipart, Bcc kept out of the headers and in the envelope). No scratch SMTP container was started. The first real email is the owner's QA (`RPT-1-AC1`, `AC5`).
- **The dialog in a real browser** (component tests only); the console's `next build --webpack` succeeds.
- **The PDF of a real dashboard** as made by the print view: BI-1's open item.
- **Daylight saving against a real clock:** proved by unit tests across Europe/Amsterdam's 2026 changes, not waited for.
- **A crash between claim and send** loses that one send (never sends twice); not tested by killing a process.
- The planner note about a second migration: not needed.

## 8. Review (planner appends)
