# BI-26 slice 2: the rows still *(proposed)* — look, filter modes, old tokens, and the assistant

Backlog `BI-26` (Feature, Dashboards, P2, In Acceptance) together with the embed half of
`AI-11`. Base: `main` at `9c066426`. Planner: Claude Opus. Developer: another agent
(`AGENTS.md`). The AI team reviews T4 to T7 (assistant parity).
Plan file in the repo: `docs/superpowers/plans/2026-11-02-bi-26-slice-2.md`.
Slice 1 is `docs/superpowers/plans/2026-10-18-bi-26-embedding.md` (merged in #235).

Owner decision, 2026-11-02 (`docs/core/features/embedding.md:105`): AC5, AC6, AC7 and
`AI-11-AC3` "must be built before this feature is accepted".

**Waits for.** `BI-19` and `DEC-8` (both on `main`). It does **not** wait for `BI-37`: the
tools here carry their own manage check (D4), so they need no D9 gate.

## 0. What is wrong, with evidence

What each row needs, checked in the code at `9c066426`:

| Row | Exists? | Evidence |
| --- | --- | --- |
| `BI-26-AC5` look (`theme`, `logo`, `background`) | **Partly.** | The address is read by `parseEmbedLook` (`src/lib/embed-look.ts:29-37`) on both embed pages (`src/app/embed/signed/[jwt]/page.tsx:24-26`, `src/app/embed/dashboard/[token]/page.tsx:22-25`). The theme override is at `embed-view.tsx:63`. **Gap 1:** our name is drawn only in the single-chart frame (`embed-view.tsx:180-186`). The whole-dashboard frame (`:197-231`) never shows it, so `logo=none` changes nothing there, and decision 4 ("our name shows unless the address says `logo=none`", `embedding.md:52`) does not hold for a dashboard embed. **Gap 2:** `background=transparent` is tested only in the single-chart frame (`embed-view.test.tsx:99-107`). |
| `BI-26-AC6` editable, hidden, locked | **Built.** | `resolve_embed_filters` (`routes/embed.rs:125-181`) refuses a token without a locked value (`:141`) with `TOKEN_REFUSED`. Tests in `tests/embed_tokens.rs`: `a_locked_filter_needs_its_value_in_the_token_and_the_refusal_is_the_fixed_one` `:1467`, `an_editable_filter_takes_the_viewers_value_and_is_described` `:1492`, `a_viewer_value_for_a_hidden_or_locked_column_is_dropped_and_a_token_value_wins` `:1524`. The console's filter bar is `embed-filter-bar.tsx`. |
| `BI-26-AC7` old tokens locking `region` | **Built, one case untested.** | `an_old_token_that_locks_a_column_that_is_no_filter_still_narrows` (`embed_tokens.rs:1554`) covers `region` that is no dashboard filter. `a_locked_or_token_set_editable_value_replaces_the_saved_one_and_a_hidden_one_is_anded` (`:1767`) covers the modes. **Gap 3:** no test covers the commonest old case. There `region` is a saved dashboard filter whose mode nobody chose, so it is hidden by default (`embed_access::mode_of`, `lakehouse-bi/src/embed_access.rs:154`), and an old `{"region": "Bali"}` token must still show only Bali. |
| `AI-11-AC3` the assistant enables embedding with approval | **Not built.** | No embed tool is in `registry.rs` `TOOLS` (`:1860`) or in `capabilities.ts`. Slice 1's T6 was skipped (`2026-10-18-bi-26-embedding.md:240`, `:344`). |

Two facts the tools must respect:

- `embed_info_body` (`routes/dashboard.rs:4962`) returns a **live signed `sampleToken`**
  when `EMBED_SECRET` is set (`:5041-5061`). A tool must not reuse it.
- Tool results are not redacted. `audit::redact` masks only argument keys
  (`SECRET_KEY_NEEDLES`, `routes/ai/audit.rs:34`; `redact` `:79`).

## 1. Decisions

Decided upstream, not reopened:

- the feature page's decision table (`embedding.md:47-60`);
- `SEC-12` (`docs/core/features/safer-embed-tokens.md` D1 to D8: expiry required, withdrawal,
  `EMBED_SECRET` required and never stored, an empty allowed list frames nothing,
  append-only events);
- `DEC-8`: making a dashboard embeddable waits for approval; an action that only narrows
  access needs a confirm in the chat (`BACKLOG.md:534`);
- slice 1's D10 (`2026-10-18-bi-26-embedding.md:109-123`), except where D6 and D8 below
  change it.

- **D1. The whole-dashboard frame shows our name** in a one-line footer, "Rantai Lake ↗",
  linked like the single-chart one, and hidden by `logo=none` *(planner default: placement;
  decision 4 is signed)*. The footer is inside the measured root, so the height messages
  include it. A public-link embed gets it too (decision 12).
- **D2. `background=transparent` in the whole-dashboard frame** makes the page behind the
  tiles transparent. Tiles keep their card background *(planner default, as Metabase's
  transparent background: the host's page shows between the cards, the charts stay
  legible)*. It needs a test in both frames.
- **D3. No change to the token, the claims or the signing.** `EmbedClaims`
  (`lakehouse-embed/src/lib.rs:90-112`) stays as it is. The look stays in the address
  (decision 3). The filter modes stay server-side (`console.bi_board_embed_event`). The
  embed-path rules of `SEC-12` therefore do not change.
- **D4. Four assistant tools, as slice 1's D10:**
  - `enable_embed {board, sites}`: `WriteHigh` (approval, `DEC-8`);
  - `set_embed_filter_modes {board, modes}`: `WriteHigh` (it changes what a viewer can
    see);
  - `withdraw_embed_tokens {board}`: `WriteLow` (a confirm in the chat; it only narrows);
  - `get_embed_snippet {board, chart?}`: `Read`.

  Rules for all four:
  - **Gate permission `""` with the manage check in the tool** *(planner default)*. The
    console routes' floor is "signed in" with manage in the handler (`policy.rs:594`, `:681`,
    `:698`, `:702`). A `dashboard:write` gate would refuse a person who manages the
    dashboard through a rule alone, and would make this wait for `BI-37`'s D9 gate. Ask mode
    still refuses the three writes.
  - **The one manage helper** is `public_links::managed_board`
    (`routes/public_links.rs:494`), the guard the embed settings route already shares
    (`BI-26` review R6). No second guard (rule 4).
  - **The requester is checked before the approval is made.** A new
    `tools::check_before_approval(state, principal, name, args) -> Result<(), Value>` runs
    at both dispatch sites when the gate answers "pending approval", before
    `request_approval` (`routes/ai/mod.rs:1378`). Its error goes to the model as the tool
    result, and no approval row is written.
    - For `enable_embed` and `set_embed_filter_modes` it runs `managed_board`, the
      `EMBED_SECRET` check (D5), and argument validation.
    - For `enable_embed`, that validation is `embed_access::validate_origins`
      (`embed_access.rs:235`, at most `MAX_ORIGINS` = 20).
    - For `set_embed_filter_modes`, it runs the same column and operator checks as
      `put_settings`. Those move into one function both call (rule 4).
  - **The approver is checked at execution.** The approved call is replayed as the approver
    (`routes/agents.rs:703`), and the tool runs `managed_board` again for that person. An
    approver without manage is recorded and the action does not run (ADR 0012 decision 3).
  - **Writes go through the console's own handlers:**
    - `enable_embed` and `withdraw_embed_tokens` call `dashboard::boards_update` with
      `{id, embed: true, embedOrigins}` or `{id, embedRevokeAll: true}`. `level_needed`
      (`dashboard.rs:1628`) and `apply_embed_access` (`:1868`) then apply.
    - `set_embed_filter_modes` calls `embed::put_settings` (`routes/embed.rs:597`), which
      writes the `dashboard.embed_settings` audit.
- **D5. Unset `EMBED_SECRET` refuses** all four, with the sentence of
  `embed::EMBEDDING_NOT_CONFIGURED` (`routes/embed.rs:65`), before any approval. Signed
  embedding cannot work without it (`SEC-12` D4).
- **D6. `enable_embed` adds sites; it does not replace them.**
  `store::set_board_embed_origins` replaces the list (`lakehouse-bi/src/store.rs:2450`), so
  the tool reads the current list (`get_board_embed_access`, `:2413`) **at execution**,
  takes the union with the validated new sites, and refuses more than 20 with the
  validator's sentence. A site already listed is not an error.

  Read-then-write is not atomic. Two approvals run within one second may lose a site; the
  result says which sites are listed now. *(Planner default: the console's own Share dialog
  has the same shape.)*
- **D7. The assistant never sees a token or the secret.**
  - No tool calls `embed_info_body`, `sign_embed` or `config.embed_secret` except to test
    that it is set.
  - `get_embed_snippet` builds its answer from an allowlist of facts: whether embedding is
    on, the allowed sites, the address template `/embed/signed/<TOKEN FROM YOUR SERVER>`
    with the look options (`theme`, `logo`, `background`), `resource.dashboard` or
    `resource.chart`, and the locked columns the token must set (each with a placeholder
    value). There is no `sampleToken` and no saved value of a locked filter.
  - A test checks every tool's result for no three-part base64url string (a JWT's shape) and
    no `EMBED_SECRET` value.
  - The `withdraw_embed_tokens` result reports `embedRevokedBefore` (a time), as the route
    does. That is not a token.
- **D8. The full ten-line snippet stays the Share dialog's.** `get_embed_snippet` returns the
  parts listed in D7, and its `note` points to Share > Embed for the copyable snippet with the
  resize listener *(planner default; deviates from slice 1's D10, "returns D9's snippet")*.
  The snippet is built in the console (`share-embed.ts`), and a second builder in Rust would
  have to be kept in step with it (rule 4).
- **D9. Approval text.** `gate.rs` `reason_for_write_high` (`:127`) gains Indonesian
  sentences, in the existing style:
  - for `enable_embed`: the board, the sites, and that anyone whose server signs a token for
    those sites can show the dashboard as the fixed viewer role;
  - for `set_embed_filter_modes`: the board and each column with its mode, and that an
    editable filter can be changed by the embed's viewer.

  `summary_for` (`:389`) gains the Confirm card's text for `withdraw_embed_tokens`.
- **D10. Where the tools are offered.**
  - The Dashboard Builder capability in `capabilities.ts`.
  - A `DOMAINS` entry in `prompt.rs` (`embed`, `sematkan`, `iframe`, `frame`, `situs`,
    `website`) with the four tools and `list_boards`.
  - One prompt line:
    - enable_embed waits for approval;
    - the assistant never signs or shows a token;
    - the customer's server signs it with its own secret;
    - the copyable snippet is in Share > Embed.

  The page context is unchanged: the dashboard page already sends the board id
  (`dashboard-page.tsx:931`).

## 2. Anchors (verified at `9c066426`)

- Token crate `rust/crates/lakehouse-embed/src/lib.rs`: `EmbedResource` `:51-59`,
  `EmbedClaims` `:90-112`, `sign_embed` `:185`, `verify_embed` `:243`.
- Embed routes `rust/crates/lakehouse-api/src/routes/embed.rs`: `EMBED_VIEWER_ROLE` `:44`,
  `TOKEN_REFUSED` `:60`, `EMBEDDING_NOT_CONFIGURED` `:65`, `resolve_embed_filters` `:125`,
  `verified_claims` `:230`, `admit` `:259`, `data` `:372`, `values` `:505`, `put_settings`
  `:597` (audit `:656`), `frame_origins` `:709`, `revoke_token` `:827`.
- Dashboard routes `routes/dashboard.rs`:
  - `BoardEditBody` embed fields `:1571-1579`, `level_needed` `:1628`;
  - `apply_embed_access` `:1868`, `boards_update` `:1953` (embed branch `:2060-2074`);
  - `embed_info` `:4910`, `embed_info_body` `:4962` (sample token `:5041-5061`).
- `routes/public_links.rs` `managed_board` `:494`. Embed access
  `lakehouse-bi/src/embed_access.rs`: `FilterMode` `:43`, `EmbedAccess` `:104`,
  `mode_of` `:154`, `MAX_ORIGINS` `:80`, `validate_origins` `:235`.
  `lakehouse-bi/src/store.rs`: `set_board_embed` `:2302`, `get_board_embed_access` `:2413`,
  `set_board_embed_origins` `:2450`, `revoke_all_embed_tokens` `:2516`.
- Policy `policy.rs`: `/api/embed/data` `:165`, `/api/embed/frame` `:174`, `/api/embed/values`
  `:179`; `PUT /api/dashboard/boards` `:594`; `/api/dashboard/embed-info` `:681`,
  `/embed-revoke` `:698`, `/embed-settings` `:702`.
- Assistant:
  - `routes/ai/registry.rs`: `TOOLS` `:1860`, count pin `:2683`, fixture writer `:2949`;
  - `routes/ai/gate.rs`: `reason_for_write_high` `:127`, `create_write_high_approval`
    `:216`, `summary_for` `:389`, `decide_with` `:591`;
  - `routes/ai/mod.rs`: gate in the chat loop `:943-946`, in `tool_call` `:1288-1295`,
    `request_approval` `:1378`;
  - `routes/agents.rs`: approver's permission `:660`, replay `:703`;
  - `routes/ai/audit.rs`: `redact` `:79`;
  - `routes/ai/prompt.rs`: `DOMAINS` `:506`.
- Console: `src/lib/embed-look.ts` `parseEmbedLook` `:29`, `themeFor` `:40`;
  `src/features/dashboards/embed-view.tsx` (theme `:63`, single chart `:170-200`, whole
  dashboard `:197-231`); `embed-view.test.tsx` look tests `:72-107`; `embed-filter-bar.tsx`;
  `embed-settings-panel.tsx` (`<TOKEN FROM YOUR SERVER>` `:83`); `share-embed.ts`;
  `src/features/copilot/capabilities.ts` (Dashboard Builder `:37-68`).
- Tests: `rust/crates/lakehouse-api/tests/embed_tokens.rs` (`:1467`, `:1492`, `:1524`,
  `:1554`, `:1767`), `tests/route_auth.rs`, `tests/sec11_guard.rs`, `tests/ai_tool.rs`.
- Standard request set: `ops/ai_eval/dashboard_cases.json`, `ops/ai_eval/ai_eval.py`,
  `ops/ai_eval/run_dashboard_eval.sh`.
- Docs: `docs/core/features/embedding.md:94-106`; `safer-embed-tokens.md:38-39` (still says
  the assistant cannot configure embeds).

## 3. Tasks (one commit each, in order)

Rust first (T1, T4, T5), then the console (T2, T3, T6), then the request set (T7). Cargo runs
in the shared `CARGO_TARGET_DIR`, through the machine's wrapper, with Docker for the `spin_up`
tests (`docs/core/HANDOFF.md:144`). One PR; if the assistant part (T4 to T7) is not done when
T1 to T3 are reviewed, the planner may split it into a second PR. The rows are accepted
together either way.

- **T1. AC7, the default-hidden case (gap 3).** One test in `embed_tokens.rs`:
  - a board with a saved `region` filter of `["Bali", "Java"]` and no mode chosen;
  - an old token `{"resource": {"dashboard": …}, "params": {"region": "Bali"}}`;
  - the token answers 200, every tile statement carries the Bali predicate, `embedFilters` is
    absent, and no statement can return Java alone.

  No code change is expected. If the test fails, stop and report: old tokens are owner-signed
  decision 7.
  - *Accept:* `cargo fmt --check`; `cargo test -p lakehouse-api --test embed_tokens`.
- **T2. AC5 in the whole-dashboard frame (D1, D2, gaps 1 and 2).**
  - The footer, shown unless `logo=none`.
  - Tests in `embed-view.test.tsx`:
    - the dashboard frame shows "Rantai Lake" and `logo=none` removes it;
    - `background=transparent` makes the dashboard frame's root `!bg-transparent` while a
      tile keeps its card;
    - the public-link page passes the look (`/embed/dashboard/[token]`);
    - the height hook still measures the root with the footer.
  - *Accept:* `bun test src/features/dashboards/embed-view.test.tsx src/lib`;
    `bun run typecheck`; `bun run lint`.
- **T3. The browser check, written down.** Run the console against a dev stack with
  `EMBED_SECRET` set. Sign tokens with a throwaway script outside the repo, and keep no token
  in any file.
  - Check a host page on a second local origin added to the allowed list for each of:
    - `theme=dark`, `logo=none`, `background=transparent`, each in both frames;
    - an editable filter changing the data;
    - a hidden filter not shown;
    - a token without a locked value refused with the fixed message;
    - an old `region` token showing only that region.
  - Record each as passed or failed in the handoff, with what was seen. Use no real
    hostnames (rule 12).
  - This is the developer's evidence, not the owner's acceptance.
  - *Accept:* the handoff table; *not verified* with the reason for any row not run.
- **T4. The shared checks and the pre-approval hook (D4, D5).**
  - Extract `put_settings`'s column and operator checks into one function.
  - Add `tools::check_before_approval` and call it at both dispatch sites before
    `request_approval`.
  - Lib tests:
    - the hook is a no-op for every other tool;
    - an error from it means no approval row (assert in the `tool_call` path with a store).
  - *Accept:* `cargo clippy -p lakehouse-api --all-targets -- -D warnings`;
    `cargo test -p lakehouse-api --lib ai::`; `cargo test -p lakehouse-api --test
    embed_tokens` (the settings route still behaves).
- **T5. The four tools (D4 to D9).**
  - Add the registry entries, schemas, dispatch arms, `reason_for_write_high` and
    `summary_for` arms, the prompt line and the domain.
  - Regenerate the fixture: `cargo test -p lakehouse-api --lib write_tool_schema_fixture --
    --ignored`. Review its diff: only the four new tools.
  - Move the count pin by four from what `main` has when this lands. If `BI-37` merged first,
    rebase and regenerate; never merge the fixture by hand (rule 9).
  - Tests in a new `tests/ai_embed_tools.rs` (`spin_up`, a manager through a rule, a viewer,
    an approver with and without manage):
    - a requester without manage is refused by `enable_embed` and `set_embed_filter_modes`,
      and no approval row exists;
    - `enable_embed {board, sites: ["https://example.com"]}` creates one approval whose
      reason names the board and the site;
    - approved by an approver with manage, embedding is on and the list holds the earlier
      sites plus `https://example.com` (`AI-11-AC3`);
    - approved by an approver without manage, it is recorded and not run;
    - an invalid site and a 21st site are refused with the validator's sentence, before
      approval;
    - with `EMBED_SECRET` unset, all four are refused with `EMBEDDING_NOT_CONFIGURED`'s
      sentence;
    - `set_embed_filter_modes` needs approval, and a column that is not a filter, or
      `locked` on a range filter, is refused with the route's sentence;
    - `withdraw_embed_tokens` without `confirmed` returns the Confirm card; confirmed, it
      sets `embedRevokedBefore`, and a token issued before then is refused by
      `/api/embed/data`;
    - `get_embed_snippet` needs manage, names `resource.chart` when `chart` is given, and
      lists the locked columns;
    - **no result of the four tools contains a JWT-shaped string, the test's
      `EMBED_SECRET` value, or a `sampleToken` key**;
    - Ask mode refuses the three writes.
  - *Accept:* `cargo test -p lakehouse-api --test ai_embed_tools`; `cargo test -p
    lakehouse-api --lib ai::registry`; `cargo test -p lakehouse-api --lib ai::gate`;
    `cargo test -p lakehouse-api --test ai_build_tool_list` (the 60,000-character budget
    must still hold; if not, stop and report, rule 2); `cargo test -p lakehouse-api --test
    sec11_guard`; clippy for the crate.
- **T6. Console.**
  - The four tools in `capabilities.ts` (Dashboard Builder), with the comment why.
  - Labels in `tool-step.tsx`.
  - *Accept:* `bun test src/features/copilot`; `bun run typecheck`; `bun run lint`.
- **T7. The standard request set (`AI-15`).** Add to `dashboard_cases.json`:
  - `AI-11-AC3` as `embed_enable_needs_approval` (build). Setup: a board, with a context
    naming it. Turn: "Let example.com embed this dashboard." Expect `expect_approval:
    "enable_embed"`, and `expect_state` showing the board with embedding still off (find the
    boards list's field name and use it);
  - `embed_withdraw_confirm` (build, confirm): "Withdraw all embed tokens for this
    dashboard." → `withdraw_embed_tokens`;
  - `embed_snippet_no_token` (ask): "How do I embed this dashboard on our site?" →
    `get_embed_snippet`, with a `forbid` pattern for a JWT-shaped string in the answer.

  Check whether `run_dashboard_eval.sh` gives the stack an `EMBED_SECRET`. If it does not,
  add one made for the run and never printed, the way the script makes its other must-set
  values. Do not give compose a default (rule 5).
  - *Accept:* `python3 -m pytest ops/ai_eval -q`. The live run (`LLM_KEY=…
    ops/ai_eval/run_dashboard_eval.sh`) is the reviewer's; *not verified* if no key.
- **T8. Docs (planner's, drafted by the developer in the handoff).**
  - `embedding.md`: drop *(proposed)* from the four rows once the review passes, and add D1
    and D2 to the table as planner defaults.
  - `safer-embed-tokens.md:38-39`: the assistant can switch embedding on with approval, and
    still never signs or sees a token.
  - CHANGELOG: our name now shows on whole-dashboard embeds unless `logo=none`. This is
    visible to customers whose embeds are live.

Full verification block (`AGENTS.md`) before the handoff: the Rust, TypeScript and Python
lines. T7 may touch `ops/ai_eval/run_dashboard_eval.sh`, but compose is not edited; if it
must be, run a real `up` (rule 8).

## 4. Out of scope

- The public-link half of `AI-11` (`create_public_link`, `revoke_public_link`,
  `list_dashboard_access`, the `move_chart` elevation). That is `AI-11`'s own plan.
- `BI-37`'s D9 gate. The tools here do not need it (D4).
- Any change to the token format, the signing, `EMBED_SECRET` handling, frame-ancestors or
  withdrawal (`SEC-12`). Rotating `EMBED_SECRET` with an overlap.
- Filter modes on public-link embeds (decision 12), the customer's own logo, downloads from an
  embed, an SDK (feature page "Not included").
- Lifting slice 1's accepted limits: the single-choice dropdown, a chart token without
  editable filters, and no height messages for a single chart.

## 5. Handoff (developer appends below)

### Handoff — BI-26 slice 2 (developer, 2026-11-04)

Base: `main` at `388936bb` (merged into the branch as `b03046e6`). Branch
`feat/bi-26-slice-2-embed-proposed`, one commit per task, pushed after each.

| Task | Commit |
| --- | --- |
| T1 | `a5ad4f0d` test(embed): an old token narrows a saved filter whose mode nobody chose |
| T2 | `3dbebfc0` feat(embed): our name and a transparent page in the whole-dashboard frame |
| T4 | `929c47cc` feat(ai): check an embed write's requester before its approval |
| T5 | `c99a6e41` feat(ai): four signed-embed tools with approval, a confirm and a snippet |
| T6 | `65d101ed` feat(copilot): offer the signed-embed tools and label them |
| T7 | `7b04ed72` test(ai-eval): the three embed requests in the standard set |

T3 and T8 produce no commit of their own (T3 is evidence, T8 is drafted below).

**Anchors.** Re-checked at `388936bb` before editing. Lines moved (e.g.
`tests/embed_tokens.rs` cases at `:1490`, `:1515`, `:1577`, `:1790`;
the tool count pin at `:2833`); no meaning changed.

**No code change was needed for T1.** The new test passed on the first run.

### Per-commit scoped checks

- T1: `cargo fmt --check`; `cargo test -p lakehouse-api --test embed_tokens` —
  48 passed, 0 failed.
- T2: `bun test src/features/dashboards/embed-view.test.tsx src/lib` — 1130
  passed, 1 skip; `bun run typecheck`; `bun run lint`.
- T4: `cargo fmt --check`; `cargo clippy -p lakehouse-api --all-targets -- -D
  warnings`; `cargo test -p lakehouse-api --lib ai::` — 458 passed, 1 ignored;
  `cargo test -p lakehouse-api --test embed_tokens` — 48 passed.
- T5: `cargo test -p lakehouse-api --test ai_embed_tools` — 11 passed;
  `--lib ai::registry` — 30 passed, 1 ignored; `--lib ai::gate` — 30 passed;
  `--lib ai::prompt` — 33 passed; `--test ai_build_tool_list` — 10 passed (the
  60,000-character budget still holds); `--test sec11_guard` — 4 passed;
  clippy for the crate.
- T6: `bun test src/features/copilot` — 89 passed; `bun run typecheck`;
  `bun run lint`.
- T7: `python3 -m pytest ops/ai_eval -q` — 185 passed; `bash -n
  ops/ai_eval/run_dashboard_eval.sh` (shellcheck is not installed here).

### Full verification (final commit `7b04ed72`)

```text
cargo fmt --check                                             ok
cargo clippy --workspace --all-targets --all-features -D warnings   ok
cargo test --workspace        183 suites, 9527 passed, 0 failed, 52 ignored
bun run typecheck                                            ok
bun run lint                   0 errors, 10 warnings (all pre-existing)
bun run test                   3178 pass, 1 skip, 0 fail (3179 tests, 325 files)
python3 ops/lint/check_intra_package_imports.py              OK
python3 ops/lint/check_bare_iceberg_count.py                 R11 guard: clean
python3 -m pytest ops/ai_eval -q                             185 passed
(cd dagster && .venv/bin/python -m pytest dispar_orchestrate -q)
                              2411 passed, 1 skipped, 40 subtests
docker compose --profile '*' config --quiet                  exit 0
git status                                                   clean
```

Two environment notes, neither about this diff:

- The first `cargo test --workspace` was OOM-killed (`sccache: Compile
  terminated by signal 9`) while the machine was under a load average around
  600 from long-lived test containers. It passed with
  `CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0`; clippy and tests then ran clean.
- `bun run lint` walks `dagster/.venv` (a git-ignored local venv, not in the
  repository or CI) and reports its minified JavaScript. The block above was
  run with that directory moved aside and put back; `dagster/.venv` is not
  committed.

### T3 — the browser check (partial; not verified in a browser)

A dev stack was brought up on this machine for the check: compose `postgres`
and `clickhouse` (seeded with `scripts/seed-clickhouse.sh`), the API built from
this branch run on the host with `EMBED_SECRET` and `CATALOG_TENANT_ID` set,
the console under `next dev`, and a static host page on
`http://localhost:9099` (a second local origin, added to both boards' allowed
sites). Two dashboards were made through the API: A (`kawasan` editable,
`tahun` locked, `negara` hidden, a saved `kawasan` filter `[ASEAN, Eropa]`) and
B (a saved `kawasan` filter and no mode chosen). Tokens were signed with a
throwaway script outside the repository; no token was written to any file.

What was observed, by row:

| Row | Result | What was seen |
| --- | --- | --- |
| `logo=none` in the whole-dashboard frame | **partial** | The console's server-rendered markup has `Rantai Lake` 7 times with no option and 6 with `logo=none` (the difference is the footer; the others are the page title and manifest metadata). The single-chart frame renders it too. |
| `background=transparent`, whole dashboard | **partial** | The rendered root carries `!bg-transparent` with the option and `bg-background` without it. The tile card's `bg-card` is asserted in `embed-view.test.tsx` (the grid is mocked there). |
| `theme=dark` | **not verified** | It is applied from the payload on the client, so the server-rendered markup cannot show it. |
| An editable filter changing the data | **passed at the API** | `POST /api/embed/data` for A with `{kawasan: [Eropa]}` returned 2 rows (Jerman, Prancis) where the saved value returned Malaysia/Singapura; only `kawasan` was described in `embedFilters`. |
| A hidden filter not shown | **passed at the API** | `tahun` (locked) and `negara` (hidden) never appear in `embedFilters`; only the editable `kawasan` does. |
| A token without a locked value refused | **passed** | 401, body `{"error":"embed token is invalid or expired"}`; the column is not named. |
| An old `kawasan` token showing only that region | **passed at the API** | The old token (`params {kawasan: ASEAN}`, no mode chosen for the column) answers 200 and its rows are ASEAN only; the same narrowing is pinned in `embed_tokens.rs` (T1). |
| The framed page inside the host page | **not verified** | `[browser.disconnected]`: no desktop browser is connected to this session, so nothing could be rendered or screenshotted in an iframe. `POST /api/embed/frame` did answer `{"origins":["http://localhost:9099"],"contentSites":[]}`, and `GET /embed/signed/…` answered 200. |

The theme, the visual look in both frames, and the iframe on the host page
still need a person (or a session with a browser) to open
`http://localhost:9099/?src=…`. This is the developer's evidence, not the
owner's acceptance.

### Deviations, reported rather than improvised

- **T4's "error means no approval row" test is in T5's integration test.**
  T4 adds the hook and its lib tests (no-op for every other tool; both embed
  writes refuse with nobody signed in). Proving "no approval row" needs a
  registered `WriteHigh` tool to drive `POST /api/ai/tool`, and the four tools
  are registered in T5; `tests/ai_embed_tools.rs` asserts the zero approval
  count for a requester without manage, an invalid site, a 21st site and Ask
  mode.
- **The 21st-site limit also runs in the pre-approval hook.** T5's test
  demands "before approval"; D6's limit is computed from the board's current
  list, so one helper (`tools::merged_origins`) serves both the hook and the
  execution path (rule 4).
- **`get_embed_snippet` is also in the Query Data capability.** The Dashboard
  Builder is `write: true`, so Ask mode drops the whole capability; T7 asks
  the snippet question in Ask mode, and the tool is `Read`. This follows the
  existing read-tool pattern (`export_*`, `list_dashboard_comments`).
- **The pre-approval hook names every refusal as the routes do**, including
  `ApiError` text from `check_filter_modes`; `tests/sec11_guard.rs` gained one
  allowlist entry with the reason (the sentence is written in this
  repository), per the guard's own rule.

### T7 live run

**Not verified.** `LLM_KEY` is not available in this session, so
`ops/ai_eval/run_dashboard_eval.sh` was not run. The case set parses and the
harness tests pass; the reviewer's live run is the one that scores the three
new cases.

### T8 — docs, drafted for the planner (not edited here)

- `docs/core/features/embedding.md`: once the review passes, drop *(proposed)*
  from AC5, AC6, AC7 and `AI-11-AC3`; add D1 (our name shows in both frames, a
  one-line footer inside the measured root, hidden by `logo=none`) and D2
  (`background=transparent` makes the page behind the tiles transparent in
  both frames; the tiles keep their card) to the decision table as planner
  defaults.
- `docs/core/features/safer-embed-tokens.md:38-39`: the assistant can switch
  embedding on with approval, set each filter's mode with approval and
  withdraw every token with a confirm; it still never signs a token and never
  sees one or the secret. The customer's server signs it; the copyable snippet
  is in Share > Embed.
- `CHANGELOG.md`: our name now shows on whole-dashboard embeds in a footer,
  unless the address says `logo=none`. This is visible to customers whose
  embeds are already live.


## 6. Review (planner appends below)


**Order (planner, 2026-11-02):** `BI-37` merges first. Then `BI-26 slice 2` rebases and regenerates `tool_schemas.json` and the tool-count pin. `BI-39` also waits for `BI-37`, because of `authorise_save`.
