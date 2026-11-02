# BI-40: "Test run now" works for people again

Backlog `BI-40` (Task, Monitoring, P2). Base: `main` at `9c066426`. Planner: Claude Opus.
Developer: another agent (`AGENTS.md`). Source: `BI-19` owner QA 2026-10-28, finding QA-8
(`docs/superpowers/plans/2026-10-18-bi-19-sharing.md`, section 8, "Owner QA, 2026-10-28").

## 0. What is wrong, with evidence

The Alerts page's row button "Test run now", its menu item "Execute test run" and the page's
"Run all now" all call the scheduler's door, `POST /api/alerts/run`:

- `src/features/alerts/alerts-page.tsx:725` builds `runAction` on `alertRuleService.runRules`;
  `:875-876` is "Run all now"; `src/features/alerts/rules-columns.tsx:275` and `:298`
  are the row button and the menu item.
- `src/services/clients/alerts.ts:95-101` posts to `/api/alerts/run[?id=]` through `apiFetch`.

Since D4 that door admits only the scheduler (`rust/crates/lakehouse-api/src/routes/alerts.rs:613`
`check_run_token`, called first in `run` at `:1125-1137`):

- with `ALERTS_RUN_TOKEN` set, a request without the token is `ApiError::unauthorized()` (401);
- with it unset, anything but a `PrincipalId::Service` principal is a 503
  ("alerts run is not configured …").

A person's browser session has neither, so the button gets 401 (or 503). `apiFetch` turns every
401 outside `/api/auth/*` into a redirect to `/login` (`src/services/http.ts:98-106`), so the
person is thrown out of the console for pressing a test button. The button has not worked for
people since D4; QA-8 reproduced it and verified the rule through the scheduler path instead.

Two more gaps sit next to it in today's code:

- The assistant's `run_alert_rule` (`routes/ai/tools/alerts.rs:140-196`) runs one rule without
  `check_run_token` (its doc `:16-23` says why) but it is a **second copy** of the run wiring
  (sources, `run_rules`), and it skips what the route does after `run_rules`: chart rules are
  never read (`lakehouse_alerts::run_rules` answers them `CHART_SKIPPED`,
  `lakehouse-alerts/src/lib.rs:1540`, `:3097-3099`; only the route calls
  `chart_alerts::evaluate`, `routes/alerts.rs:1186-1196`), and a fired result is not persisted
  as an `alert_instance` nor a recovery resolved (`persist_fired_results` / `resolve_recovered`,
  `:1198-1201`). That duplication is a rule 4 finding.
- `run_alert_rule` checks only the tool's `alert:write` (`registry.rs:2249-2252`). It does not
  check that the caller may see the rule (`visible_rules`, `routes/alerts.rs:113`) or could
  manage it (`authorise_existing_rule`, `:341`), which `PUT`/`DELETE` both check.

## 1. Decisions

- **D1. A new route for a person, the scheduler's door unchanged.** `POST /api/alerts/test?id=<rule>`
  runs one rule now for a signed-in person. `POST /api/alerts/run` and `check_run_token` stay
  exactly as they are (D4 is not weakened, `AGENTS.md` principle 3). `POLICY_TABLE`:
  `RequiresAuth`, with the handler deciding, like `POST /api/alerts` (`policy.rs:405-419`).
- **D2. Who may test a rule: whoever may manage it.** All three must hold, in this order:
  1. the rule is in what the caller's list shows (`visible_rules` over the one rule), else
     `404 alert rule not found.` (an unknown id answers the same, so the route is no oracle);
  2. `authorise_existing_rule` passes (`SRC-7` D7: connector, upload and quality rules);
  3. the caller holds `alert:write`, **or** the rule is their own chart rule (the
     `chart_alerts::may_delete` test, `chart_alerts.rs:285-295`, renamed `may_manage` and used by
     both delete and test; C3 lets a person without the permission own a chart alert).
  Otherwise `403 alert:write`. A signed-in person never gets a 401 from this route.
- **D3. A test run is a real run of that one rule** *(planner default)*: the same path the
  scheduler's `?id=` run takes. If the condition holds the email or webhook is really sent, the
  rule's state is settled (fires once, recovers once, `BI-19` D11), a fired result is saved as an
  `alert_instance` and a recovery resolves it. This is what the button did before D4 and what
  `run_alert_rule` already does. The scheduler's extra passes (quality, schema versions, semantic
  drafting, lineage, pipeline-late) do not run; a `pipeline_late` rule answers `skipped` with the
  sentence "A late-pipeline alert is checked by the 15-minute pass, not by a test run." (honest,
  principle 2). *Owner alternative, not built:* a dry run that sends nothing and does not touch
  the state.
- **D4. One helper for one rule.** `routes::alerts::run_one(state, id) -> Result<Vec<RunResult>, ApiError>`
  holds the wiring now inline in `run` (`:1139-1201`: senders, sources, `run_rules(Some(id))`,
  `chart_alerts::evaluate(Some(id))`, the merge, persist, resolve). `run` calls it for the rule
  part of the tick (with `only = query.id`), the new route calls it, and the assistant's
  `run_alert_rule` calls it. Chart rules are then tested and run by the assistant too.
- **D5. "Run all now" is removed from the page.** The tick is the scheduler's; a person cannot
  call it (D1). The page's scheduling note (`alerts-page.tsx:923-931`, text from `:924`) stays, as operator text.
- **D6. The console never reads this route's refusal as a lost session.** The route answers 403
  and 404, not 401 (D2). `apiFetch` is unchanged: a real 401 from it is a lost session.
- **D7. Assistant parity.** `run_alert_rule` goes through D2's guard and D4's helper. Its schema
  description says chart alerts are included and what a test run does; regenerate
  `tests/fixtures/tool_schemas.json`. No new tool. The page sends no new context. AI-15: one case
  "Test my alert <rule> now" in `ops/ai_eval/dashboard_cases.json` that expects `run_alert_rule`
  with the id (confirmation as today, `Risk::WriteLow`). The AI team reviews this part.

## 2. Anchors (verified at `9c066426`)

- `rust/crates/lakehouse-api/src/routes/alerts.rs`: `visible_rules` `:113`,
  `authorise_existing_rule` `:341`, `delete` `:535` (uses `may_delete` `:547-553`),
  `check_run_token` `:613`, `persist_fired_results` `:990`, `resolve_recovered` `:1071`,
  `run` `:1125` (the wiring `:1139-1201`), `RunQuery` `:583`.
- `rust/crates/lakehouse-api/src/routes/chart_alerts.rs`: `evaluate` `:194`, `may_delete` `:285`.
- `rust/crates/lakehouse-api/src/routes/ai/tools/alerts.rs`: module doc `:1-23`,
  `run_alert_rule` `:140-196`.
- `rust/crates/lakehouse-api/src/routes/ai/registry.rs`: `run_alert_rule_schema` `:1281-1286`,
  its `ToolSpec` `:2249-2252`, `write_tool_schema_fixture` `:2949`.
- `rust/crates/lakehouse-api/src/routes/mod.rs:1197-1204` (the alert routes).
- `rust/crates/lakehouse-api/src/policy.rs:400-422` (the alert rows and their comment).
- `rust/crates/lakehouse-api/tests/route_auth.rs:251`
  (`the_alert_routes_are_signed_in_at_the_router_and_the_handler_asks_for_the_right`);
  `rust/crates/lakehouse-api/tests/chart_alerts.rs` (runs through `GET /api/alerts/run`).
- Console: `src/services/contracts/alerts.ts:183-186` (`runRules`), `src/services/clients/alerts.ts:95-101`,
  `src/features/alerts/alerts-page.tsx:725`, `:804`, `:875-876`, `:923-931`,
  `src/features/alerts/rules-columns.tsx:272-301`, `src/services/http.ts:98-106`.

## 3. Tasks (one commit each, in order)

- **T1. The helper (D4), no behaviour change.**
  - Move the rule part of `run` into `run_one(state, only: Option<&str>)`. `run` keeps
    `check_run_token` first and the scheduler's passes after it, unchanged.
  - `run_alert_rule` calls it with `Some(id)` (its own copy of the sources goes). Its chart-rule
    and instance gaps close as a consequence; say so in the commit body.
  - Tests: `tests/chart_alerts.rs` stays green unchanged (the tick still fires once and recovers
    once); a `lib` test that `run_alert_rule` on a chart rule no longer answers `CHART_SKIPPED`.
  - *Accept:* `cargo fmt --check`; `cargo clippy -p lakehouse-api --all-targets -- -D warnings`;
    `cargo test -p lakehouse-api --lib ai::tools::alerts`; `cargo test -p lakehouse-api --test chart_alerts`.
- **T2. The route and the guard (D1, D2, D3, D6).**
  - `POST /api/alerts/test?id=`; 400 "id is required" without an id. `may_delete` becomes
    `may_manage` (same body, both callers). The guard order is D2's; the run is `run_one(Some(id))`
    plus the `pipeline_late` sentence of D3. Answer `{ "ran": n, "results": [...] }`, the shape
    `runRules` already reads.
  - `POLICY_TABLE` row `("POST", "/api/alerts/test", Policy::RequiresAuth)` with a comment that
    names D2 and that `/api/alerts/run` is unchanged. Route registered in `routes/mod.rs`.
  - `route_auth.rs`, both ways: no session → 401; the seeded Analyst (no `alert:write`) on a
    threshold rule → 403; the Analyst on their own chart rule → not 401/403; the Data Engineer
    on a threshold rule → not 401/403; and `POST /api/alerts/run` with the Analyst's session is
    still 401/503 (D4 kept).
  - Route tests (new `tests/alert_test_route.rs`, wiremock `ClickHouse` and a recording sender as
    `chart_alerts.rs` does): a rule the caller cannot see and an unknown id both 404 with the same
    body; a threshold rule over its line sends once and saves one instance; a second test while
    still over sends nothing (D11); a `pipeline_late` rule answers the D3 sentence; another
    tenant's connector rule → 404 (D2.1) for a restricted caller.
  - Assistant (D7): `run_alert_rule` runs D2's guard before `run_one` and returns its sentence as
    `{ "error": … }`. Test: an `alert:write` holder of tenant A gets the 404 sentence for tenant
    B's connector rule.
  - *Accept:* `cargo fmt --check`; `cargo clippy -p lakehouse-api --all-targets -- -D warnings`;
    `cargo test -p lakehouse-api --test alert_test_route`; `cargo test -p lakehouse-api --test route_auth`;
    `cargo test -p lakehouse-api --lib policy`; `cargo test -p lakehouse-api --lib ai::tools::alerts`.
- **T3. Assistant schema and the standard request (D7).**
  - `run_alert_rule` description: "Run one alert or digest rule now (by id), as a test: chart
    alerts included. If its condition holds, the email or webhook is really sent and the alert
    is marked as fired." Regenerate the fixture with
    `cargo test -p lakehouse-api --lib write_tool_schema_fixture -- --ignored`, and commit only the
    `run_alert_rule` hunk of `tests/fixtures/tool_schemas.json` (check the diff).
  - One AI-15 case (D7) in `ops/ai_eval/dashboard_cases.json`, with setup and `cleanup`.
  - *Accept:* `cargo test -p lakehouse-api --lib registry`; `(cd ops/ai_eval && python3 -m unittest test_ai_eval)`.
    The AI-15 run itself needs a model: record it as run with its score, or *not verified*.
- **T4. The console (D5, D6).**
  - Contract: `runRules` is replaced by `testRule(id: string, signal?)` on `AlertRuleService`
    (no optional id: a person tests one rule). Client posts `/api/alerts/test?id=`; a 403 reads
    "You can test only the alerts you manage." from the server's sentence, a 404 the server's.
  - The row button and the menu item call `testRule`; "Run all now" and its handler are removed.
  - Tests: the client posts to `/api/alerts/test` and never to `/api/alerts/run`; the page has
    no "Run all now"; a 403 shows the sentence and does not navigate (the `apiFetch` redirect
    is not reached; assert `window.location.assign` is not called).
  - *Accept:* `bun run typecheck`; `bun run lint`; `bun test src/features/alerts src/services/clients`.
- **T5. Docs.** `CHANGELOG.md` line (people can test their alerts again; "Run all now" is gone);
  the module doc of `routes/alerts.rs` gains the new route. *Accept:* `git diff --stat` shows docs only.

Full verification block of `AGENTS.md` once before handoff (Rust, TypeScript, `git status`).

## 4. Out of scope

- A dry run (D3, owner alternative).
- Any change to `POST /api/alerts/run`, `check_run_token` or the scheduler's token (`D4`).
- A rate limit on test runs (a test is one rule and the D11 state already stops repeats).
- Mart and metric rules reading as their owner (`BI-39`).
- `pipeline_late` single-rule evaluation.

## 5. Handoff (developer appends below)

## 6. Review (planner appends below)
