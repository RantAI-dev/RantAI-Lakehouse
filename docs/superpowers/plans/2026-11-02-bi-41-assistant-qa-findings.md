# BI-41: the assistant's findings from the phase 5–6 QA

Backlog `BI-41` (Task, AI Copilot, P3). Base: `main` at `9c066426`. Planner: Claude Opus.
Developer: another agent (`AGENTS.md`). Reviewers: the planner, and the AI team for every prompt,
schema and AI-15 change (`AGENTS.md`, "Assistant parity").

The row records three findings of the owner's QA on 2026-10-26 in one line each; no transcript is
in the repo (searched: `docs/`, the `BI-4`, `BI-5` and `BI-16c` plans and feature pages). Where the
line does not say enough to know the cause, section 0 says so, and a decision is marked.

## 0. What is wrong, with evidence

**F1. "It stops after a confirmation instead of carrying on with the request." Mostly fixed on
`main` since the QA; one shape is still left out.**

- After the QA, #288 (`bd4b4bc4`, 2026-10-31) added the continuation: the model ends a turn with a
  `NEXT:` line, `take_next_step` returns it as `nextStep` when the turn holds a
  `needs_confirmation` result (`routes/ai/mod.rs:1433-1465`), and the console sends
  "Confirmed and saved: …; Continue: <step>" once the person confirms
  (`src/features/copilot/use-copilot.ts:472-530`, at most three times). #306 and #317 made the
  message name what was saved. The AI-15 harness does the same.
- What is still left out is in the rule itself (`routes/ai/prompt.rs:448`): `NEXT:` is written
  "only when the request needs another object after the draft (a chart on the board being drafted,
  the charts moved to the tab being drafted) … never when the draft is the whole request". A step
  that **changes the drafted object** is not another object. The phase 5 request shape, "add a
  chart of X and make it always fresh", drafts the chart (`create_chart`, confirmation) and then
  needs `set_cache_rule` on the new chart's id, which does not exist until Confirm. A model that
  follows the rule writes no `NEXT:`, so the console has nothing to continue with and the request
  stops after the confirmation. The rule's test pins the narrow wording
  (`prompt.rs:1166-1200`, clause "only when the request needs another object after the draft").
- *Not verified:* that this is the exact request the owner made. T1's AI-15 case reproduces the
  shape; the owner re-runs the QA step at acceptance.

**F2. "Asked to remember results for 'this chart' it sets the dashboard's rule."**

- On a dashboard the page context lists every tile (`src/features/dashboards/dashboard-page.tsx:912-939`,
  `summarizeTiles` `src/lib/page-context-summary.ts:78-103`) and the board id, and names no chart
  the person is looking at: the console has no "focused tile".
- `set_cache_rule` takes `board` **or** `chart` (`routes/ai/registry.rs:555-565`), and the build
  prompt says "call set_cache_rule with the chart or the board" (`prompt.rs:434`, the build suffix's dashboard line, the sentence after
  `limits_and_freshness!()`). With "this chart" and several tiles, the only id the context offers
  for "this" is the board's, and nothing says not to use it. The AI-15 case
  `dash_make_chart_always_fresh` (`ops/ai_eval/dashboard_cases.json:636-648`) names the chart id in
  the request ("Make chart {chart} always fresh."), so it never exercises "this chart".
- The server cannot tell the person's intent; both calls are valid. The fix is in what the model is
  told and asked to do.

**F3. "Its wording after Stop." The line does not say which Stop.** Two Stops exist; both have a
wording gap in today's code:

- (a) The chat's Stop button. An answer stopped mid-way is saved as an assistant message with empty
  content and `stopped: true` (`use-copilot.ts:416-422`), shown as "Response stopped."
  (`chat-messages.tsx:341-343`). The next request drops every empty message from the history
  (`use-copilot.ts:339`, `.filter((m) => m.content)`), so the model sees the stopped question as an
  unanswered user turn directly before the new one. It then answers both, carries on with the
  stopped request, or apologises for an interruption it cannot see.
- (b) `stop_query` (`routes/ai/tools/limits.rs:58-70`) answers `{ "stopped": n }`, and the kill is
  sent even when `n` is 0 (BI-5 re-review R2). The schema says "Answers how many were stopped (0:
  nothing was running)" (`registry.rs:825-828`); the prompt's sentence (`prompt.rs:371`, shared with
  Ask mode and pinned by hash in `tests/ai_sql_help.rs:363-384`) does not tell the model how to say it.

## 1. Decisions

- **D1 (F1). `NEXT:` also covers a change to the drafted object.** The rule's clause becomes:
  "only when the request needs another object after the draft, or another change to the drafted
  object (its cache rule, its format, its tab)". The two `never` clauses stay. The rule's test is
  changed with the reason in the commit (rule 2: the test pinned the narrow wording on purpose; the
  product decision changes, not the test's rigour).
- **D2 (F2). "This chart" is a chart, and an unclear one is asked.** In Build mode:
  - the `set_cache_rule` sentence of the build suffix and the tool's description say: for "this
    chart", "the chart", "it" about one chart, give `chart`; give `board` only when the person
    means the whole dashboard;
  - when the page context lists more than one chart and the person did not name one, call
    `ask_user` with the charts' titles as options (the existing question tool,
    `registry.rs:364`); with exactly one chart, use it;
  - never set the dashboard's rule for a request about one chart.
  *(planner default)* No console change (no "focused tile" in the page context). *Owner
  alternative, not built:* a tile menu item "Ask the assistant about this chart" that puts the
  chart's id in the page context.
- **D3 (F3a). A stopped answer stays in the history as a fixed line** *(planner default: the
  owner's line names "wording after Stop" without saying which Stop; this decision fixes (a), the
  one that produces wrong wording on its own)*. When the console builds the history it sends a
  stopped assistant message as the content `(The person stopped this answer before it finished.)`
  instead of dropping it. The saved message is unchanged (`content: ""`, `stopped: true`), so
  sessions saved before this change behave the same. No prompt change, so the Ask-mode hash pin is
  untouched.
- **D4 (F3b). `stop_query`'s wording is not changed here** unless the owner says the finding was
  about it *(owner question Q1)*. If it was: one clause in `limits_and_freshness!` ("say how many of
  the person's queries were stopped, from the result; 0 means nothing of theirs was running"), and
  the Ask-mode pin in `tests/ai_sql_help.rs:384` updated by the AI team, with the before and after
  hash in the commit body, as BI-5 S9 did.
- **D5. Assistant parity.** This item is the assistant's work: tool descriptions
  (`set_cache_rule`) and the fixture regenerated; the page sends no new context (D2); three AI-15
  cases (T1, T2). Before merge the AI-15 set is run once and its score recorded next to the last
  run's (`AI-15` row: 25 of 41 after #200 and #202); a case that newly fails is a finding, and no
  case is loosened to pass (rule 2).

**Owner question Q1:** "Wording after Stop" — was it the chat's Stop button (the next answer), or
the assistant's reply to "stop the running query"? The planner default fixes the first (D3).

## 2. Anchors (verified at `9c066426`)

- `rust/crates/lakehouse-api/src/routes/ai/prompt.rs`: `limits_and_freshness!` `:364-373`, the
  build suffix's dashboard line with the `set_cache_rule` sentence `:434`, the confirmation rule `:448`, the rule's test
  `:1166-1200` (test fn at `:1173`, `the_next_step_rule_names_the_drafted_object_and_the_next_line`).
- `rust/crates/lakehouse-api/src/routes/ai/mod.rs:1433-1465` (`take_next_step`).
- `rust/crates/lakehouse-api/src/routes/ai/registry.rs`: `ask_user` `:364`, `set_cache_rule_schema`
  `:555-565`, `stop_query` `:825-828`, `write_tool_schema_fixture` `:2949`.
- `rust/crates/lakehouse-api/src/routes/ai/tools/look.rs:427-481` (`set_cache_rule`).
- `rust/crates/lakehouse-api/src/routes/ai/tools/limits.rs:58-70` (`stop_query`).
- `rust/crates/lakehouse-api/tests/ai_sql_help.rs:363-384` (the Ask-mode pin).
- `rust/crates/lakehouse-api/tests/fixtures/tool_schemas.json` (the `set_cache_rule` entry).
- `src/features/copilot/use-copilot.ts`: `Msg.stopped` `:23-24`, history `:339`, stopped message
  `:416-422`, `confirmTool` and the continuation `:472-530`.
- `src/features/copilot/chat-messages.tsx:341-343` ("Response stopped.").
- `src/features/dashboards/dashboard-page.tsx:912-939` (page context); `src/lib/page-context-summary.ts:78-103`.
- `ops/ai_eval/dashboard_cases.json:636-648` (`dash_make_chart_always_fresh`); `ops/ai_eval/test_ai_eval.py`.

## 3. Tasks (one commit each, in order)

- **T1. The continuation covers a change to the draft (D1).**
  - The clause in `prompt.rs:448`; the test's pinned clause changed to the new wording, and a new
    assertion that the two `never` clauses are still there.
  - AI-15 case `dash_new_chart_then_fresh` (build, `confirm: true`): "Add a bar chart of events by
    year to dashboard {board} and make it always fresh." Expect `create_chart`, then after the
    continuation `set_cache_rule` with `chart` = the new chart's id and `rule: fresh`; `expect_state`
    the new chart's `def.cache.mode` is `fresh`; setup makes the board, `cleanup` removes it.
  - *Accept:* `cargo fmt --check`; `cargo clippy -p lakehouse-api --all-targets -- -D warnings`;
    `cargo test -p lakehouse-api --lib ai::prompt`; `(cd ops/ai_eval && python3 -m unittest test_ai_eval)`.
- **T2. "This chart" (D2).**
  - The build suffix's `set_cache_rule` sentence and the tool's description as D2 says. Regenerate
    the fixture (`cargo test -p lakehouse-api --lib write_tool_schema_fixture -- --ignored`) and
    commit only the `set_cache_rule` hunk.
  - Prompt test: the build suffix names `ask_user` for an unclear "this chart" and says not to set
    the dashboard's rule for one chart.
  - AI-15 cases (build), each with a page context like the dashboard page's (two tiles, then one):
    `dash_this_chart_fresh_asks` — "Make this chart always fresh." over two charts expects `ask_user`
    and **no** `set_cache_rule` with `board`; `dash_this_chart_fresh_one_tile` — the same over one
    chart expects `set_cache_rule` with that `chart`. If the harness cannot pass a page context,
    add that to the runner first in this commit, with its unit test.
  - *Accept:* `cargo test -p lakehouse-api --lib ai::prompt`; `cargo test -p lakehouse-api --lib registry`;
    `(cd ops/ai_eval && python3 -m unittest test_ai_eval)`.
- **T3. A stopped answer stays in the history (D3).**
  - In `use-copilot.ts`, one exported pure function builds the history (the map at `:339`): a
    `stopped` assistant message becomes the fixed line; other empty messages are still dropped.
  - Tests: the history after a stop carries the fixed line between the stopped question and the new
    one; a session saved before (no line stored) is sent the same way; the message on screen still
    reads "Response stopped."; a continuation's history is unchanged.
  - *Accept:* `bun run typecheck`; `bun run lint`; `bun test src/features/copilot`.
- **T4. Only if Q1 says (b) (D4).** The clause and the pin update. *Accept:*
  `cargo test -p lakehouse-api --test ai_sql_help`; `cargo test -p lakehouse-api --lib ai::prompt`.
- **T5. The AI-15 run (D5).** `ops/ai_eval/run_dashboard_eval.sh` against the dev stack with the
  deployment's model; record the score, the new cases' results and any case that changed, in the
  handoff. Without a model or stack: *not verified*, with the reason. *Accept:* the quoted counts.

Full verification block of `AGENTS.md` once before handoff (Rust, TypeScript).

## 4. Out of scope

- A "focused tile" in the dashboard page context (D2 alternative).
- The continuation cap (three) and its message text (#306, #317).
- Approvals for `set_cache_rule` (it stays `WriteLow`, BI-4 handoff note 5).
- Any assistant work outside dashboards (the AI team's own planning).

## 5. Handoff (developer appends below)

### Handoff (developer, 2026-11-02)

Branch `fix/bi-41-assistant-qa-findings`, off `origin/main` merged into the
branch before T1 (merge commit `f3869ca3`). One commit per task, each pushed
to `origin` after it was made; no PR opened.

Commits:

- `ae463666` **T1** — `BI-41 D1`: `NEXT:` also covers a change to the
  drafted object; test clause and the two `never` clauses; AI-15 case
  `dash_new_chart_then_fresh`; the runner fills `{created:<kind>}` in
  `expect_tool.args_contains` (new `fill_created_ids`, shared with
  `fill_created`) and two unit tests for it.
- `6a6a48cf` **T2** — `BI-41 D2`: the build suffix's `set_cache_rule`
  sentence and the tool's description; `tests/fixtures/tool_schemas.json`
  regenerated, only the `set_cache_rule` entry moved; prompt and registry
  tests; AI-15 cases `dash_this_chart_fresh_asks` and
  `dash_this_chart_fresh_one_tile`. The runner already sent a case's
  `context` with every turn (`TurnOptionsTests`), so it needed no change.
- `9ef558ac` **T3** — `BI-41 D3`: exported pure `chatHistory` replaces the
  empty-message filter; `STOPPED_ANSWER_TEXT`; tests in
  `use-copilot.test.tsx` and `chat-messages.test.tsx`.
- T4 **not done**: Q1 was not answered; the plan's default is D3 (the
  chat's Stop), and D4 says `stop_query`'s wording is not changed here.
  The Ask-mode pin `tests/ai_sql_help.rs:384` is untouched.
- T5 **not verified**: `LLM_KEY` is not set in this environment, so
  `ops/ai_eval/run_dashboard_eval.sh` exits 2 before it starts a stack
  ("LLM_KEY must be set"). No score is recorded and no case was run; the
  new cases are unmeasured, as is any case that changed.

Design notes:

- The harness scores one `expect_tool` per case. T1's `create_chart` is
  asserted by the created chart the later `set_cache_rule` names
  (`{created:chart}`) and by `expect_state` finding exactly one chart with
  `def.cache.mode = fresh`; a run without the chart cannot pass.
- `dash_this_chart_fresh_asks` uses `expect_no_tool: ["set_cache_rule"]`
  rather than only "no `set_cache_rule` with `board`": with two tiles the
  suffix says to ask in that turn, and `ask_user`'s own schema says to run
  nothing else then. That is the board-rule finding and a stricter check,
  not a looser one.
- The two new page contexts copy the dashboard page's shape (board id,
  `Tiles and the data they currently show`, `- "title" (kind, id <id>;
  mart): rows: …`). The setup board and the created chart are removed by
  the harness's own cleanup (`collect_created` from setup plus confirms),
  which runs in its `finally`.

Commands run (scoped per the machine rule: no full-workspace cargo run, no
`cargo clean`; `CARGO_TARGET_DIR` is the shared one via `cargo-locked`):

| command | result |
| --- | --- |
| `cargo fmt --check` | pass |
| `cargo clippy -j 2 -p lakehouse-api --all-targets -- -D warnings` | pass, 0 warnings |
| `cargo test -j 2 -p lakehouse-api` | 7021 passed, 0 failed, 43 ignored, 97 suites (lib `2941 passed, 18 ignored`; `main.rs` `2979 passed, 18 ignored`; the rest are integration suites, all ok) |
| `cargo test -j 2 -p lakehouse-api --lib ai::prompt` | 34 passed, 0 failed |
| `cargo test -j 2 -p lakehouse-api --lib ai::registry` | 29 passed, 0 failed, 1 ignored (the fixture writer) |
| `cargo test -j 2 -p lakehouse-api --lib write_tool_schema_fixture -- --ignored` | 1 passed; the fixture diff is the one `set_cache_rule` line |
| `(cd ops/ai_eval && python3 -m unittest test_ai_eval)` | 186 tests, OK |
| `bun run typecheck` | pass |
| `bun run lint` | 0 errors, 10 warnings (all pre-existing, none in this diff) |
| `bun test src` | 3153 pass, 1 skip, 0 fail, 321 files |
| `python3 ops/lint/check_intra_package_imports.py` | OK |
| `python3 ops/lint/check_bare_iceberg_count.py` | OK |

Not run: `cargo clippy/test --workspace` (the machine rule scopes cargo to
the touched crate), dagster pytest and `docker compose config` (no Python
under `dagster/`, no compose change), and a `docker compose up` (none
needed).

**`bun test src/features/copilot` is not green on this machine, and was
not before this branch's changes.** It reports 63 pass, 3 fail, 3 errors:
`chart-draft-card.test.tsx`, `chart-draft-preview.test.tsx` and
`chat-messages.test.tsx` fail with `Export named 'useRouter' not found in
module .../next/navigation.js`. The cause is `mock.module("next/navigation",
…)` in `use-copilot.test.tsx` replacing the whole module with only
`usePathname`, and Bun applying it across the folder's files. It reproduces
with the T3 changes stashed (58 pass, 3 fail, 3 errors), and with the same
folder in another worktree on unrelated code; `bun test src` (the full
suite) is green with all 3153 tests, including the new ones. This file
alone (`bun test src/features/copilot/chat-messages.test.tsx`) is 10 pass,
and `use-copilot.test.tsx` alone is 15 pass. Not fixed here because the
mock is outside BI-41; it needs its own backlog item.

Environment note: this worktree had no `node_modules`. I copied one from
another worktree whose `bun.lock` is byte-identical to this branch's (Bun
1.4.2 locally; CI pins 1.3.12, and `bunx bun@1.3.12` gave the same
folder-only result).

## 6. Review (planner appends below)

### Review round 1 (planner, 2026-11-02, at `7ba66e5b`)

- **T1 (D1).** `NEXT:` now also covers a change to the object just drafted. The AI-15 case is `dash_new_chart_then_fresh`, and the harness fills `{created:<kind>}`.
- **T2 (D2).** "This chart" means a chart, and the assistant asks when it is unclear. The fixture changes in one hunk, the `set_cache_rule` description. There are two AI-15 cases. **AI team:** review that hunk.
- **T3 (D3).** A stopped answer is sent back to the model as one fixed sentence, so the next answer knows it was cut short.
- **T4 is correctly not built.** It applies only if the owner answers Q1 with (b), and D3 is the default.
- **T5 is not verified.** There was no `LLM_KEY` for the model run, and the reason is recorded. The eval harness's own unit tests pass (186).
- **Counts (the developer's):**
  - `cargo test -p lakehouse-api`: 7021 passed across 97 suites;
  - console `bun test src`: 3153;
  - clippy, fmt, typecheck and the ops lints: clean.
- **Already on `main`, not caused here:** `bun test src/features/copilot` alone has 3 fails, from the `mock.module("next/navigation")` leak in `use-copilot.test.tsx`. It reproduces without this PR, so it is filed as a backlog item (a test isolation fix).

**Open: no BLOCKER, no SHOULD-FIX.** The merge waits for the PR's CI.
