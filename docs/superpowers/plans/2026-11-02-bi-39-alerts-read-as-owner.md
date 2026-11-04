# BI-39: every alert reads as its owner, a chart alert reads one tile, and the rules list names the chart

Backlog `BI-39` (Task, Monitoring, P3). Base: `main` at `9c066426`. Planner: Claude Opus.
Developer: another agent (`AGENTS.md`). Sources: `BI-19` part C review backlog notes and owner QA
finding QA-9 (`docs/superpowers/plans/2026-10-18-bi-19-sharing.md`, section 8).

**Order.** Start after `BI-40` merges *(planner default)*: both change `routes/alerts.rs`,
`routes/chart_alerts.rs` and `routes/ai/tools/alerts.rs`, and BI-40 adds the one-rule helper
(`run_one`) this plan's tests run through. Building them at once would mean resolving the same
hunks twice (`AGENTS.md` rule 9).

## 0. What is wrong, with evidence

**Part A. Mart and metric rules read as a fixed role, chart rules as their owner.**

- Feature decision C4 (`docs/core/features/sharing-and-delivery.md:98`): "An alert reads the data
  as the person who set it (their masking and row filters), like a scheduled report." `BI-19`
  built it for chart rules only (planner decision 2 of 2026-10-26, cited at
  `routes/chart_alerts.rs:24-25`).
- Every other statement goes through `ApiSqlGate` (`routes/alerts.rs:808-811`, impl `:866-879`),
  which rewrites with `[EMBED_VIEWER_ROLE]` ("Dashboard Viewer") and no placeholder values
  (`PlaceholderValues::none()`), whoever saved the rule. The trait has no way to know whose
  rule it is: `SqlGate::gate(&self, sql)` (`lakehouse-alerts/src/lib.rs:2117-2135`), called from
  `current_value` `:1779-1802` and `metric_value` `:1813` / `:1881`.
- A new mart or metric rule gets no owner at all: `chart_alerts::authorise_save` keeps the stored
  owner for a non-chart rule and sets none on a new one (`chart_alerts.rs:349-355`, the line at `:353`, decision 2).
- The assistant's `create_alert_rule` / `update_alert_rule` (`routes/ai/tools/alerts.rs:69-114`)
  never call `authorise_save`, so a rule saved through the assistant never has an owner either.
- The assistant is told the old behaviour: `create_alert_rule`'s description ends "it is read with
  the fixed viewer role every alert uses, not with the user's access" (`registry.rs:1237`, and the
  fixture `tests/fixtures/tool_schemas.json:4119`).

Effect: a mart rule set by somebody whose role sees a column the Dashboard Viewer role does not
(or under a row filter) watches a different number from the one that person sees; a rule over a
mart the viewer role cannot read is refused on every tick.

**Part B. A chart alert reads the whole dashboard to get one tile.** `chart_alerts::read`
(`chart_alerts.rs:96-191`) calls `dashboard::payload_for` (`:150`) for the chart's board and then
takes `results[chart_id]` (`:159`). `get_body` (`routes/dashboard.rs:339`) builds a future for
every built-in tile and every stored chart of the board (`:461-470`, `:540-560`), so a board of
twelve charts runs twelve statements per chart rule per tick. `fresh_chart` (`:113`) only stops
that one tile being served from memory; the other eleven are still computed or looked up. Declared
as a deviation in the part C handoff ("a chart alert reads the whole dashboard payload as its owner
(one board read per chart rule per tick)").

**Part C. The rules list cannot name the chart.** A chart rule stores the chart id only
(`AlertRule.chart`, `lakehouse-alerts/src/lib.rs:385-389`). The list answers the stored rules
(`routes/alerts.rs:54-69`), so the console writes "Dashboard chart · visitors goes above 1"
(`src/features/alerts/rules-columns.tsx:70-87`, QA-9 fix) because it has no title to show.

## 1. Decisions

- **D1. Every threshold rule (mart, metric, chart) reads as its owner.** *(owner to sign: this
  widens what can leave the console for a mart or metric rule from the Dashboard Viewer role's
  view to the owner's own view; it is what C4 says and what reports already do (`RPT-1`
  decision 1). Sending to anyone but oneself still needs `alert:write`, C3.)*
  - The owner's roles, masking, row filters and the principal and tenant placeholders apply, the
    same rewrite `payload_for` gets (`routes/dashboard.rs:237-245`).
  - Digest rules are not changed: they keep the fixed role (`run_digest` passes no owner). Out of
    scope, section 4.
- **D2. The owner is whoever saved the rule last** (`BI-19` C review C-R2, applied to every
  threshold rule): `authorise_save` sets `input.owner_id = Some(caller)` for a saved `alert` rule,
  chart or not. A digest and the event kinds keep today's `stored owner` line. `watches_differ`
  already counts `owner_id` (`lib.rs:1705`), so a change of owner resets the state: correct, the
  number is read as somebody else now.
- **D3. The assistant saves through the same guard.** `create_alert_rule` and `update_alert_rule`
  call `chart_alerts::authorise_save` after their scope checks (rule 4). With no principal (a
  schedule) nothing changes: `authorise_save` returns early for a non-chart rule. `BI-37`'s plan
  makes the same call (to accept chart alerts from the assistant); if `BI-37` merges first, this
  part of T2 is already done and only its test is added here.
- **D4. A rule saved before owners existed keeps reading as the Dashboard Viewer role** until it
  is saved again. Nobody is made its owner by guess (principle 2). The edit dialog says so in one
  muted line (T4).
- **D5. When the owner cannot read any more, the rule stops and says why**, as a chart rule does
  (`lakehouse_alerts::stop`, `lib.rs:1546`):
  - owner disabled or gone: "The person who set this alert is disabled or no longer exists."
    (the existing `OWNER_GONE`, `chart_alerts.rs:53`, moved to one shared place);
  - owner no longer holds `alert:write` and the target is not their own address *(planner
    default)*: "The person who set this alert no longer has the alert permission." The permission
    is what allowed the rule to send to others (C3), so a rule must not outlive it;
  - the policy engine refuses the statement for the owner: the engine's fixed sentence, recorded
    as a check that could not run (state unchanged, C2), as today.
- **D6. The gate learns whose rule it is, the crate stays free of the API.** `SqlGate::gate`
  becomes `gate(&self, owner: Option<&str>, sql: &str) -> Result<String, GateRefusal>` with
  `GateRefusal::{Refused(String), Stop(&'static str)}` in `lakehouse-alerts`. `run_alert` maps
  `Stop` to `stop(...)` and `Refused` to the existing skipped path. `ApiSqlGate` resolves the owner
  with `routes::render::principal_of` (`render.rs:194`, the reader the PDF, the reports and chart
  alerts already use), never a second copy.
- **D7. A chart alert reads one tile.** `DashboardQuery` gains a crate-internal `only: Option<String>`
  (`#[serde(skip)]`, so no request can set it) and `only_chart(id)`. When set, `get_body` adds no
  built-in tile and keeps of `stored_for_board` only that chart; everything else (filters, tabs,
  parameters, grain, masks, the 404 for a board the person cannot see) is unchanged. `chart_alerts::read`
  uses `.only_chart(id).fresh_chart(id)`. The PDF's chart image (`render::data` with a chart token)
  uses it too; `narrow_to_chart` (`render.rs:149`) stays as the guard on what leaves.
- **D8. The list names the chart live, never stored** *(planner default)*. A stored title would go
  stale on a rename and would keep a chart's name after the reader lost access (the same reason the
  bell is built live, `BI-19` part E deviation 1). `GET /api/alerts` adds, to each chart rule whose
  chart the caller can see **now**, `chartTitle`, `dashboardId` and `dashboardTitle`. A chart rule
  whose chart the caller cannot see carries none of them, and the console keeps "Dashboard chart".
  One read of the caller's visible charts and boards per list, not one per rule.
- **D9. Assistant parity.**
  - `create_alert_rule`'s description: replace the fixed-viewer clause with "it is read as the
    person who saves it (their masking and row filters); if they lose access or the alert
    permission, it stops and says why." Regenerate the fixture.
  - `list_alert_rules` goes through the same list helper, so it gets `chartTitle` and
    `dashboardTitle` (D8) and can name a chart alert.
  - The page sends nothing new. AI-15: one case "Which chart are my alerts on?" over a seeded chart
    rule, expecting `list_alert_rules` and the chart's title in the answer. The AI team reviews
    this part.

## 2. Anchors (verified at `9c066426`)

- `rust/crates/lakehouse-alerts/src/lib.rs`: `AlertRule.metric` doc `:366-372`, `AlertRule.chart`
  `:385-389`, `owner_id` doc `:404-409`, `AlertRuleInput.owner_id` `#[serde(skip)]` (in the struct
  at `:430`), `save_rule` `:1345`, `stop` `:1546`, `watches_differ` `:1688-1706`, `current_value`
  `:1779-1802`, `metric_value` `:1806-1813` (gate call `:1881`), `digest_text` gate call `:1993`,
  `SqlGate` `:2117-2135`, `run_rules` `:2795`, `run_alert` `:3087-3117`, test gates `:3333-3360`,
  `:4949-4970`.
- `rust/crates/lakehouse-api/src/routes/alerts.rs`: `list` `:54-69`, `visible_rules` `:113`,
  `ApiSqlGate` doc and struct `:798-811`, `impl SqlGate` `:866-887`.
- `rust/crates/lakehouse-api/src/routes/chart_alerts.rs`: module doc decision 2 `:24-25`,
  sentences `:52-64`, `read` `:96-191` (query `:141-149`, `payload_for` `:150`), `visible_to`
  `:249-282`, `authorise_save` `:318-388` (non-chart owner line `:353`, chart owner `:385`).
- `rust/crates/lakehouse-api/src/routes/dashboard.rs`: `DashboardQuery` `:51-74`, `for_render`
  `:79`, `fresh_chart` `:113`, `payload_for` `:232`, `get_body` `:339`, `on_default` `:461`,
  `stored_for_board` `:463-470`.
- `rust/crates/lakehouse-api/src/routes/render.rs`: `data` `:69` (query `:115-121`),
  `narrow_to_chart` `:149`, `principal_of` `:194`.
- `rust/crates/lakehouse-api/src/routes/ai/tools/alerts.rs`: `parse_input` `:41-54`,
  `list_alert_rules` `:56`, `create_alert_rule` `:69`, `update_alert_rule` `:92`.
- `rust/crates/lakehouse-api/src/routes/ai/registry.rs:1236-1237` (`create_alert_rule`).
- `rust/crates/lakehouse-bi/src/store.rs`: `Board` `:492` (`name`), `list_boards` `:1327`.
- Tests: `rust/crates/lakehouse-api/tests/chart_alerts.rs` (recording fake), `routes/alerts.rs`
  test module `:2197-2292` (`ApiSqlGate`).
- Console: `src/services/contracts/alerts.ts:97-108` (chart fields of `AlertRule`),
  `src/features/alerts/rules-columns.tsx:65-87`, `src/features/alerts/alerts-page.test.tsx:311-322`.

## 3. Tasks (one commit each, in order; Rust first, batched)

- **T1. The gate knows the owner (D6, D5), in `lakehouse-alerts`.**
  - `GateRefusal`, the new `gate` signature, `current_value` / `metric_value` take `owner`,
    `run_digest` passes `None`, `run_alert` passes `rule.owner_id.as_deref()` and maps `Stop`.
  - Correct the comments that say every rule reads as the fixed role (`:366-372`, `:404-409`,
    `:1806-1811`, rule 1).
  - Unit tests with the test gates: an owned rule's statement reaches the gate with its owner; a
    legacy rule's with `None`; `Stop` gives state `stopped` with the reason and sends nothing;
    `Refused` leaves the state and records the sentence.
  - *Accept:* `cargo fmt --check`; `cargo clippy -p lakehouse-alerts --all-targets -- -D warnings`;
    `cargo test -p lakehouse-alerts`.
- **T2. The API reads as the owner and saves the owner (D1–D5).**
  - `ApiSqlGate::gate`: `None` → today's rewrite; `Some(id)` → `principal_of`; `Ok(None)` →
    `Stop(OWNER_GONE)`; no `alert:write` and not an own-address email → `Stop(NO_PERMISSION)`;
    store failure → `Refused("The person who set this alert could not be read.")`; otherwise
    `rewrite_sql_for_roles` with the owner's role names and placeholder values.
  - The owner sentences move from `chart_alerts.rs` to one place both use (rule 4).
  - `authorise_save` line `:353`: an `alert` rule is owned by the caller (D2); others unchanged.
    Update its doc (`:308-312`) and the module doc decision 2 (`chart_alerts.rs:24-25`).
  - The assistant's create and update call `authorise_save` (D3).
  - Tests:
    - `routes/alerts.rs` test module: the gate rewrites an owned rule with the owner's role (a
      role with a mask gives the masked expression in the statement); a legacy rule keeps
      `Dashboard Viewer`; a disabled owner gives `Stop`.
    - `tests/chart_alerts.rs` (or a new `tests/mart_alerts.rs`): a mart rule saved by the Data
      Engineer stores `ownerId`; re-saved by another `alert:write` holder it is owned by them and
      its state is reset; an owner disabled → `stopped` with the sentence and nothing sent.
    - `ai::tools::alerts`: `create_alert_rule` stores the caller as owner.
  - *Accept:* `cargo clippy -p lakehouse-api --all-targets -- -D warnings`;
    `cargo test -p lakehouse-api --lib alerts`; `cargo test -p lakehouse-api --test chart_alerts`
    (and `--test mart_alerts` if added).
- **T3. One tile (D7) and the chart names (D8).**
  - `DashboardQuery::only_chart`; `get_body` narrows as D7 says; `chart_alerts::read` and
    `render::data` (chart token) use it.
  - The list: one helper `listed_rules(state, principal, rules) -> Vec<Value>` used by `GET
    /api/alerts` and `list_alert_rules`; it runs `visible_rules`, then reads the caller's
    `list_stored_charts` and `list_boards` once (only when a chart rule is in the list) and adds
    the three fields to the chart rules whose chart is visible.
  - Tests:
    - a board of three stored charts: a chart alert's check sends exactly **one** tile statement
      to the engine (count the wiremock requests whose body carries a chart statement), and its
      number is the same as before;
    - the chart image's data still answers that chart only, and a token for another chart is
      still refused (`tests/render_data.rs`);
    - the list gives `chartTitle` and `dashboardTitle` for a visible chart, none of the three for
      a chart the caller cannot see, and a request with `only` in the query string cannot narrow
      `GET /api/dashboard` (`serde(skip)`; the board answers all its tiles).
  - *Accept:* `cargo clippy -p lakehouse-api --all-targets -- -D warnings`;
    `cargo test -p lakehouse-api --test chart_alerts`; `cargo test -p lakehouse-api --test render_data`;
    `cargo test -p lakehouse-api --lib dashboard`.
- **T4. Assistant schema and request (D9).**
  - The `create_alert_rule` description; regenerate with
    `cargo test -p lakehouse-api --lib write_tool_schema_fixture -- --ignored` and commit only that
    tool's hunk.
  - The AI-15 case of D9 in `ops/ai_eval/dashboard_cases.json` with setup and `cleanup`.
  - *Accept:* `cargo test -p lakehouse-api --lib registry`; `(cd ops/ai_eval && python3 -m unittest test_ai_eval)`.
    The AI-15 run needs a model: its score, or *not verified*.
- **T5. The console (D4, D8).**
  - Contract: `AlertRule` gains `chartTitle?`, `dashboardId?`, `dashboardTitle?` (absent when the
    caller cannot see the chart; the comment says so, so the `?` is a real absence, not a mock).
  - `chartAlertCondition` writes `<chartTitle> on <dashboardTitle> · <condition>` when present,
    "Dashboard chart · …" otherwise; the title links to the dashboard.
  - The rule dialog, for an `alert` rule with no `ownerId`: "This alert was set before alerts had
    owners and reads as the Dashboard Viewer role. Saving it makes it read as you."
  - Tests: the title and the dashboard name with them; "Dashboard chart" without; the D4 line for
    a rule with no owner and not for an owned one.
  - *Accept:* `bun run typecheck`; `bun run lint`; `bun test src/features/alerts`.
- **T6. Docs.** Feature page `sharing-and-delivery.md`: C4's row says it covers every threshold
  alert (the planner sets its status; the developer adds the sentence only). `CHANGELOG.md`: "an
  alert on a table column or a metric now reads as the person who saved it; rules saved before
  keep the Dashboard Viewer role until saved again" (breaking on upgrade: no; behaviour change at
  next save). *Accept:* docs only in `git diff --stat`.

Full verification block of `AGENTS.md` once before handoff.

## 4. Out of scope

- Digest rules reading as their owner (D1). A digest sends a whole dashboard; changing whose view
  it carries is a separate decision.
- Caching the owner's principal across the rules of one tick (one identity read per rule; measure
  before optimising).
- Rechecking an own-address rule when the owner's email changes (a `BI-19` C backlog note).
- Storing chart titles (D8).
- The assistant saving chart alerts (`BI-37`).

## 5. Handoff (developer appends below)

**Developer handoff, 2026-11-04.** Branch `fix/bi-39-alerts-read-as-owner`, based on `origin/main`
merged at `2592d722` (the plan's `BI-40`, `SEC-50` slice 2, `BI-37`, `BI-38`, `QS-3` all in).
No PR opened, no force-push, no rebase; every commit was pushed.

### Commits (one per task, in plan order)

| Commit | Task | What it changes |
| --- | --- | --- |
| `e30fb8cc` | T1 | `GateRefusal`, `SqlGate::gate(owner, sql)`, `current_value`/`metric_value` take the owner, `run_alert` maps `Stop` to `stop(...)`, `run_digest`/`digest_text` pass `None`; the fixed-viewer comments corrected. The API's `ApiSqlGate` only adapts to the new signature here (behaviour unchanged), so the tree compiles at every commit; T2 fills it in. |
| `ff8a1b5d` | T2 | `ApiSqlGate::gate` resolves the owner with `render::principal_of` and rewrites with their roles/placeholders (`D1`); `D5` stops with `OWNER_GONE`/`OWNER_NO_PERMISSION`, a store failure is a `Refused` (`C2`); `authorise_save` makes the caller the owner of every saved `alert` rule (`D2`); the assistant's create/update already went through `save_rule_with_checks` (`BI-37`), so `D3` only needed its test. |
| `da65a9fd` | T3 | `DashboardQuery::only` (`#[serde(skip)]`) + `only_chart`; `get_body` narrows; `chart_alerts::read` and `render::data` use it (`D7`); `listed_rules` + `chart_titles` for `GET /api/alerts` and `list_alert_rules` (`D8`). |
| `2bdc80aa` | T4 | `create_alert_rule` description, regenerated `tool_schemas.json` (one tool's hunk), the AI-15 case `alert_names_its_chart` in `ops/ai_eval/dashboard_cases.json`. |
| `bb80b68e` | T5 | Contract `chartTitle`/`dashboardId`/`dashboardTitle`, `chartAlertScope`/`chartAlertHref`, the linked chart name in the rules table, `legacyOwnerNote` in the rule dialog (`D4`). |
| `400db1a4` | T6 | `sharing-and-delivery.md` C4 sentence (status untouched), `CHANGELOG.md` Unreleased > Fixed. |
| `65a11250` | T2 fix | `tests/sec11_guard.rs`: the gate's own sentences need their allowlist entry once T2 puts `to_string()` behind `GateRefusal` (full block found this, see below). |

### Scoped checks, per task (run in the foreground)

- T1: `cargo fmt --check` clean; `cargo clippy -p lakehouse-alerts --all-targets -- -D warnings`
  finished with no warning; `cargo test -p lakehouse-alerts` -> 130 passed, 1 ignored, 0 failed.
- T2: `cargo clippy -p lakehouse-api --all-targets -- -D warnings` clean;
  `cargo test -p lakehouse-api --lib alerts` -> 71 passed, 0 failed;
  `cargo test -p lakehouse-api --test chart_alerts` -> 19 passed, 0 failed (15 at the T2 commit,
  4 more added by T3).
- T3: `cargo test -p lakehouse-api --test render_data` -> 13 passed, 0 failed;
  `cargo test -p lakehouse-api --lib dashboard` -> 248 passed, 3 ignored, 0 failed;
  `cargo test -p lakehouse-api --test chart_alerts` -> 19 passed.
- T4: `cargo test -p lakehouse-api --lib registry` -> 52 passed, 1 ignored, 0 failed;
  `(cd ops/ai_eval && python3 -m unittest test_ai_eval)` -> 185 passed.
- T5: `bun test src/features/alerts` -> 32 passed, 0 failed; `bun run typecheck` clean;
  `bun run lint` -> 0 errors (10 pre-existing warnings, none in the files this branch touches).
- T6: `git diff --stat` -> docs only (`CHANGELOG.md`, `docs/core/features/sharing-and-delivery.md`).

### Full block, on the final commit (`65a11250`)

- `cd rust && cargo fmt --check` -> clean.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` -> finished, no warning.
- `cargo test --workspace` -> exit 0, 182 `test result: ok` lines, 0 `FAILED`; 9,526 passing
  results and 52 ignored in the whole run (the API's lib and bin targets run the same unit tests,
  so the largest two lines read 2,975 and 3,013 passed).
  - One earlier attempt failed 18 tests in `--test query_tenant_scope` with
    `Connection reset by peer` / `SSLRequest` errors from `tests/common`: the shared Postgres
    testcontainer was restarted under two test runs on this machine. The same binary passed
    19/19 on its own and the whole workspace passed on the rerun; no code changed for it.
- `bun run typecheck` clean; `bun run lint` -> 0 errors, 10 warnings (all in files this branch does
  not touch); `bun run test` -> 3,176 passed, 1 skipped, 0 failed (325 files).
- `python3 ops/lint/check_intra_package_imports.py` -> OK;
  `python3 ops/lint/check_bare_iceberg_count.py` -> no bare `count()` found;
  `(cd ops/ai_eval && python3 -m unittest test_ai_eval)` -> 185 passed.
- `git status` -> clean after the final commit.

*Not run, with the reason:* `docker compose --profile '*' config --quiet` and `docker compose up`
(no compose, compose profile, `.env.example` or migration changed); the Dagster suite
(`(cd dagster && python -m pytest dispar_orchestrate -q)`: no Dagster code touched — the only
Python-adjacent change is the `ops/ai_eval` case file, whose own tests ran).

### Not verified

- **The AI-15 run and its score.** `alert_names_its_chart` is a `mode: ask` case; scoring it
  needs a model key and the eval stack, which this session did not have. The case parses and the
  harness's own tests pass (185), but the model has not been asked "Which chart are my alerts on?".
- **A real `docker compose up`** (not requested for this change; see above).
- **The console in a browser.** The T5 tests render the rules table cell and call the pure helpers;
  no browser was opened, and no page was previewed.

### Notes for review

1. **`D5`'s "and the target is not their own address" is satisfied by construction, not read at
   the gate.** `SqlGate::gate` receives the owner id and the statement, never the rule's target
   (`D6`'s signature), so `ApiSqlGate::gate` stops an owner without `alert:write` unconditionally.
   Today that cannot hit a self-only rule: a rule sent only to the owner's own address is the chart
   kind, which `chart_alerts` reads through `payload_for` and never through this gate, and a mart or
   metric rule is only ever saved by somebody holding `alert:write` (`authorise_save`). The sentence
   and that reasoning sit at the check (`alerts.rs`, `ApiSqlGate::gate`). If the planner wants the
   target read literally, `gate` would need it — a change to `D6`'s contract, so it is flagged here
   rather than made.
2. **The list reads the caller's visible charts twice** — once in `chart_alerts::visible_to` (it
   already did) and once in `chart_titles` — not once per rule (`D8`). Collapsing them would mean
   threading a pre-read id/title list through `visible_to`; not done, to keep the change small.
3. **Test fixtures changed on purpose.** `tests/chart_alerts.rs`'s `FakeCh` now serves the owner it
   just saw inserted (so a save's re-read matches ClickHouse's `ReplacingMergeTree(updated_at)
   FINAL`, which is how `D2`'s "a change of owner resets the state" becomes observable) and can
   hold up to three charts with distinct statements (the result cache computes one statement per
   key, so identical charts can never prove `D7`). `tests/render_data.rs`'s charts likewise got one
   distinct statement each.
4. **`tests/sec11_guard.rs` gained an allowlist entry** for `GateRefusal::Refused(err.to_string())`
   with its reason: `StatementGate::check` answers with this repository's own sentences. The line
   existed before `BI-39` as `.map_err(|err| err.to_string())?` under the older entry; the commit
   that moves it behind `GateRefusal` needed its own, and the full block is what found it.
5. **No migration, no dependency change.** `owner_id` already existed (`BI-19`); a rule saved
   before this branch keeps it empty and keeps the viewer role (`D4`).

## 6. Review (planner appends below)


**Order (planner, 2026-11-02):** starts after `BI-40` merges, on the same machine.
