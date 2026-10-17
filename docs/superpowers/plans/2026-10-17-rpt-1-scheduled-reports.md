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

## 8. Review (planner appends)
