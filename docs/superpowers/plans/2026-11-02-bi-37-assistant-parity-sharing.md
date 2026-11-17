
### Handoff (developer, 2026-11-02)

Branch `feat/bi-37-assistant-alerts-files-comments`, based on `main` at
`93cc217` (merged into the branch first: `00877aeb`). No migration was needed,
so `0126` is unused.

Commits, in plan order:

| Task | Commit | What |
| --- | --- | --- |
| T1 | `7a60f249` | `routes::alerts::save_rule_with_checks`, the one save path both routes and both alert tools call (D1) |
| T2 | `c2413c88` | chart alerts in the assistant: chart fields kept, own-address fill, permission `""`, schema, Confirm card, prompt line, tests (D1, D2, D3, D5) |
| T3 | `e7abfa10` | `export_chart` `png`, `export_dashboard` `file`, prompt sentence and export words, tests (D6, D7) |
| T4 | `7c54281e` | `list_dashboard_comments`, the route body extracted to `page_for`, service refusal on the list, prompt line and DOMAINS entry, tests (D8) |
| T5 | (no commit) | verified rather than regenerated here, see "Deviations" |
| T6 | `dea4ac37` | console: capabilities, `EXPORT_LINK` file address, tool label, tests |
| T7 | `16223a31` | six `AI-15` cases and the harness changes (DELETE_ROUTES `alert`, `collect_created` rule id), tests |
| T8+T9 | `6674514d` | the `DASHBOARD_SCOPED` list, `dashboard_reach`, `gate::permission_granted`, the offer and the approver, `tests/ai_dashboard_reach.rs` (D10) |
| T10 | drafted here | the feature page line, for the planner |

#### The `DASHBOARD_SCOPED` list (T8)

Built by reading each handler's viewer/scope resolution (`viewer_for`,
`viewer_for_write`, `scope_for_write`, `require_edit_on*`):

- Reads (`View`): `list_boards`, `list_charts`, `list_sql_sources`,
  `suggest_join` (`Anywhere`); `export_dashboard` (`Board("board")`);
  `export_chart` (`Chart("chart")`).
- Writes (`Edit`): `create_chart` (`Board("board")`), `update_chart` and
  `delete_chart` (`Chart("id")`), `format_chart` (`Chart("chart")` — its
  argument is named `chart`), `set_dashboard_theme` (`Board("board")`),
  `set_cache_rule` (`BoardOrChart`), the five tab tools (`Board("board")`),
  `rename_board` and `delete_board` (`Board("id")`), `arrange_board` and
  `save_board_filters` (`Board("board")`), `move_chart` (`Chart("id")`).
- Off the list, with the reason: `suggest_dashboard` (fixed viewer);
  `describe_mart`, the metric, calculated-field, parameter, hierarchy,
  value-group and map tools (marts and global definitions, not one dashboard);
  the four report tools (`update`/`delete` name a subscription id, so the
  call's dashboard cannot be worked out from the arguments);
  `set_board_view_filters` (returns a filter list and reads nothing).

`Reach::BoardOrChart` is one variant beyond the plan's three: `set_cache_rule`
names a board or a chart, and neither shape alone can say which. Pinned by
`the_dashboard_scoped_list_names_real_tools_and_their_argument`.

#### Deviations, and why

1. **The fixture is regenerated in T2, T3 and T4, not only in T5.** The
   byte-identical snapshot test lives in `ai::registry`, which is part of the
   T2/T3/T4 accept lines; leaving it red between tasks would have meant
   committing a red tree. T5 is still done and verified: the reviewed diff
   against `main` changes exactly `create_alert_rule`, `update_alert_rule`,
   `export_dashboard`, `export_chart` and adds `list_dashboard_comments`
   (110 → 111), the count pin is 111, and the 60,000-character budget holds.
   T5 therefore has no commit of its own: there was nothing left to change.
2. **T8 and T9 are one commit.** The list is dead code until the gate reads
   it, and the crate denies dead code; splitting them would have required an
   `#[allow(dead_code)]` to keep T8 green, which rule 2 forbids.
3. **`set_cache_rule` reach variant** (see above).
4. Two prompt-byte pins moved with T4's line and name both values
   (`ai::tests::the_base_prompt_is_byte_for_byte_what_it_was`,
   `ai_sql_help.rs`'s `SYSTEM_SHA256_WITHOUT_ALLOWLIST`).

#### Verification (final commit `6674514d`)

Run in the shared `CARGO_TARGET_DIR` through `bin/cargo-locked -j 2`, and
scoped to the crate as the brief says:

- `cargo fmt --check` — clean.
- `cargo clippy -p lakehouse-api --all-targets -- -D warnings` — clean.
- `cargo test -p lakehouse-api --lib ai::` — 452 passed, 0 failed, 1 ignored.
- `cargo test -p lakehouse-api --test ai_alert_tools` — 8 passed.
- `cargo test -p lakehouse-api --test ai_comment_tool` — 7 passed.
- `cargo test -p lakehouse-api --test ai_dashboard_reach` — 6 passed.
- `cargo test -p lakehouse-api --test ai_export_tools` — 12 passed.
- `cargo test -p lakehouse-api --test ai_build_tool_list` — 10 passed.
- `cargo test -p lakehouse-api --test ai_sql_help` — 17 passed.
- `cargo test -p lakehouse-api --test ai_tool` — 25 passed.
- `cargo test -p lakehouse-api --test ai_chat_stream` — 3 passed.
- `cargo test -p lakehouse-api --test ai_ask_back_turn` — 5 passed.
- `cargo test -p lakehouse-api --test ai_terms` — 7 passed.
- `cargo test -p lakehouse-api --test ai_cache_rule` — 3 passed.
- `cargo test -p lakehouse-api --test ai_report_tools` — 12 passed.
- `cargo test -p lakehouse-api --test tier1_writehigh_approval` — 5 passed.
- `cargo test -p lakehouse-api --test agents_approval` — 20 passed.
- `cargo test -p lakehouse-api --test agents_run` — 15 passed.
- `cargo test -p lakehouse-api --test chart_alerts` — 11 passed.
- `cargo test -p lakehouse-api --test dashboard_comments_routes` — 12 passed.
- `cargo test -p lakehouse-api --test route_auth` — 73 passed.
- `cargo test -p lakehouse-api --lib alerts` — 59 passed.
- `bun run typecheck` — clean.
- `bun test src/features/copilot` — 87 passed, 0 failed.
- `bunx eslint src/features/copilot` — 0 errors (1 pre-existing warning in
  `use-copilot.ts`).
- `python3 -m pytest ops/ai_eval -q` — 185 passed.
- `python3 ops/lint/check_intra_package_imports.py` — OK.
- `python3 ops/lint/check_bare_iceberg_count.py` — OK.

*Not verified:*

- The full-workspace `cargo clippy --workspace …` / `cargo test --workspace`:
  the brief for this task forbids a full-workspace run, so only the crate's
  `--all-targets` build and the test binaries above ran.
- The live `AI-15` run (`LLM_KEY=… ops/ai_eval/run_dashboard_eval.sh`): no key
  is set on this machine. The reviewer runs it.
- Repo-wide `bun run lint`: it walks the local, ignored `dagster/.venv` and
  reports thousands of errors in vendored files that predate this branch. The
  scoped run above is what this change was checked with.
- `dagster` pytest and `docker compose config`: this branch touches neither.

#### T10 draft (planner commits it)

`docs/core/features/sharing-and-delivery.md`, the limit at :124, replacing
"The assistant's own gate still asks for the global permission, so a person
who holds only a rule on a dashboard cannot use the assistant on it yet (the
D9 gate is not built; with the AI team).":

> - The assistant's dashboard tools check the same level as the console, and a
>   person who holds only a rule on a dashboard can use them there: view to
>   read, list and export it (`list_charts`, `export_chart`,
>   `export_dashboard`), edit to change it (charts, tabs, layout, filters).
>   Every other tool keeps its own permission: a person with only a rule is not
>   offered `run_sql` or anything else global.
