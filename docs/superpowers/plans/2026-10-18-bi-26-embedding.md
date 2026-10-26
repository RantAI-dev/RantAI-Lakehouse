# BI-26: embedding a chart or a dashboard (with AI-11, embed half)

Planner and reviewer: Claude Opus. Developer: a non-Opus coding agent.
Branch: `feat/bi-phase-4` (worktree `/home/hv/lakehouse-phase4-build`),
after `BI-19` part B2 is committed (manage level) and after PR #201 is
merged into `main` and `main` into this branch (see "Blocked by"). Feature
page: `docs/core/features/embedding.md`. Specs: `docs/core/specs/bi-26.md`,
`docs/core/specs/ai-11.md`. `SEC-12` (signed 2026-10-10,
`docs/core/features/safer-embed-tokens.md`) is the base and is not
reopened.

**Blocked by.** `BI-18` (locking a dashboard filter in an embed): the
filters it needs are part A, merged in #104; the planner confirms that this
part counts as merged before T1. **PR #201** (`SEC-12` review fixes, branch
`fix/pr105-review-fixes`, open on 2026-10-17): it moves embed access out of
the board row into `console.bi_board_embed_event`, written only by
`INSERT` and read by aggregation (`get_board_embed_access`,
`get_board_embed_access_at`). At `4fdc6fb5` this branch still keeps embed
access on the board row (`lakehouse-bi/src/embed_access.rs`, "Why on the
board"). D2 below stores the filter modes in that event table; do not
start T2 before #201 is in this branch. T1 does not need it.

Anchors were read on 2026-10-17 on `feat/bi-phase-4` at `4fdc6fb5`, and the
event table on `fix/pr105-review-fixes` at `dce286b9`; find each by the
symbol named.

**Migrations: none.** Everything this plan stores goes into the ClickHouse
event table (D2). If the developer finds a Postgres table is needed, stop
and report: this phase's numbers are used up (`BI-19` plan, D0).

## 1. Decisions already made (do not re-ask, do not change)

The feature page's table, plus these build decisions.

- **D1 Chart tokens.** `lakehouse-embed` `EmbedResource` gains `chart:
  Option<String>`. Exactly one of `dashboard` and `chart` must be set;
  both or neither → refused with `TOKEN_REFUSED` (fail closed). For a chart
  token, `routes/embed.rs` `data` finds the chart (`list_stored_charts`
  with `Viewer::anonymous()`, so a trashed chart or one on a personal board
  is not found) and its board, then applies every board check exactly as
  for a dashboard token: embedding on, withdrawal (`is_withdrawn` on that
  board), allowed sites. `render_board_payload` gains `only_chart:
  Option<&str>`: the payload holds that chart's spec and result only, no
  other chart ids, no text or heading tiles, no tabs, a one-tile layout. A
  dashboard token with `?chart=` keeps today's behaviour (feature decision
  2). Unknown claims stay ignored by serde; a test signs `{"role":
  "Platform Admin", "sub": "<admin id>"}` and shows the read still uses
  `EMBED_VIEWER_ROLE` (feature decision 10). `frame_origins` and
  `revoke_token` accept chart tokens and resolve the chart's board.
- **D2 Filter modes.** Stored as a `filter_modes` kind in
  `console.bi_board_embed_event` (JSON object: filter column → `locked |
  editable | hidden`; the newest row wins, like `origins`), read in the
  same aggregate as the other embed access state. A column missing from
  the object is `hidden` (feature decision 5). Kept out of the board row
  for the reason in `SEC-12` decision 8.
- **D3 Resolving filters on each request.** In `data`, for each of the
  board's saved filters (`board.filters`), by mode: `locked` → the token's
  `params` must name its column, else `TOKEN_REFUSED` (the log says which
  column; the response does not) (feature decision 6); `editable` → the
  viewer's value from the request when present, else the saved value;
  `hidden` → the saved value. Then every `params` entry is applied as today
  through `params_to_filters`, **last**, so a token value always wins and
  can only narrow (feature decision 7). A viewer value for a column that
  is not `editable`, or that the token set, is dropped and logged. Viewer
  values are validated with `FilterDef::validate` / `validate_filters`,
  the same as the dashboard page; an invalid one answers `400` with the
  validator's fixed sentence.
- **D4 Request body.** `POST /api/embed/data {jwt, filters?}`, where
  `filters` is the dashboard page's filter shape. The payload gains
  `embedFilters`: the editable filters only (column, label, kind, current
  value). Locked and hidden filters are not described.
- **D5 Values for editable filters.** `POST /api/embed/values {jwt,
  column}`, `Policy::Public`, asserted both ways in `route_auth.rs` (the
  public-count test moves by one, with its reason). It answers only for a
  column whose mode is `editable`; it runs the same values query as
  `routes/dashboard.rs` `values`, as `EMBED_VIEWER_ROLE`, **with the
  token's locked values and the board's hidden saved values applied**, so a
  dropdown never lists a region the token locked away. At most 1,000
  values. Every token check of `data` runs first (one helper shared by
  `data`, `values` and `frame_origins`; rule 4).
- **D6 Look (console only).** `src/app/embed/signed/[jwt]/page.tsx` and
  `src/app/embed/dashboard/[token]/page.tsx` read `theme` (`light | dark |
  dashboard`, default `dashboard`), `logo` (`none` hides it) and
  `background` (`transparent`) from the address and pass them to
  `EmbedView`. `embed-view.tsx` drops the hard-coded "Rantai Lake" text
  when `logo=none`, overrides the board's look for `light`/`dark`, and
  sets the page and frame backgrounds to `transparent` when asked. Unknown
  values are ignored. No API change.
- **D7 Height messages.** `EmbedView` watches its root with a
  `ResizeObserver` and, when the rounded height changes (debounced 100 ms),
  posts `{type: "rantai-embed:size", height}` to `window.parent` once per
  allowed site as `targetOrigin`. The sites come from `POST
  /api/embed/frame` (already public, already called by `src/proxy.ts`
  through `src/lib/embed-frame.ts`; reuse that helper). No site → nothing
  is posted. The Share dialog's snippet shows the host-side listener (about
  ten lines) that checks `event.origin` against the console's own origin
  before setting the frame's height.
- **D8 Settings route.** `PUT /api/dashboard/embed-settings {board,
  filterModes}`: manage on the board (`BI-19` B2's guard), the columns must
  be the board's filter columns, audit `dashboard.embed_settings`.
  `POLICY_TABLE` row and `route_auth` both ways. `embed_info_body`'s
  sample token takes an optional `chart` and keeps `iat`, `exp` (one hour)
  and the locked filters' saved values as `params`, so the sample works.
- **D9 Share dialog.** The embed section gains: "Dashboard" or "One chart"
  (a chart picker), the filter modes table, the three look options with a
  live preview frame, and the snippet (iframe address with the look
  options, the resize listener, and a server-side signing example naming
  `resource.chart` or `resource.dashboard` and every locked filter).
- **D10 Assistant, AI-11 embed half.** `enable_embed {board, sites}`
  (`WriteHigh`: making a dashboard embeddable, `DEC-8`) switches signed
  embedding on and adds the sites (validated by
  `embed_access::validate_origins`); `set_embed_filter_modes {board,
  modes}` (`WriteHigh`: it changes what a viewer can see);
  `withdraw_embed_tokens {board}` (`WriteLow`, chat confirm: it only
  narrows); `get_embed_snippet {board, chart?}` (`Read`, needs manage)
  returns D9's snippet with `<TOKEN FROM YOUR SERVER>` in place of a token.
  No tool signs a token or reads `EMBED_SECRET`. The requester's manage
  check runs before the approval is created and again for the approver at
  execution, with the helper `BI-19` T10 adds (one helper). Unset
  `EMBED_SECRET` → refused with `EMBEDDING_NOT_CONFIGURED`'s sentence.
  `reason_for_write_high` gains Indonesian sentences naming the board and
  the sites. Named in `capabilities.ts`; prompt line; fixture regenerated;
  count pin.

## 2. Anchors (find by symbol)

- Tokens: `lakehouse-embed/src/lib.rs` `EmbedClaims` (`resource`, `params`,
  `exp`, `iat`, `jti`), `EmbedResource` (`dashboard`), `sign_embed`,
  `verify_embed`, `TokenError`, `DEFAULT_MAX_LIFETIME_SECONDS`,
  `CLOCK_TOLERANCE_SECONDS`.
- Embed routes: `lakehouse-api/src/routes/embed.rs` `data`,
  `frame_origins`, `resolve_frame_origins`, `revoke_token`,
  `public_dashboard`, `render_board_payload`, `board_body`,
  `params_to_filters`, `tile_filters`, `EMBED_VIEWER_ROLE`,
  `TOKEN_REFUSED`, `EMBEDDING_NOT_CONFIGURED`, `token_refused`.
- Dashboard routes: `routes/dashboard.rs` `embed_info`, `embed_info_body`
  (`sampleToken`), `values`, `parse_filters_param`, `BoardEditBody`
  (`embed`, `embed_revoke_all`, `embed_origins`).
- Embed access: `lakehouse-bi/src/embed_access.rs` `EmbedAccess`,
  `validate_origins`, `MAX_ORIGINS`; `lakehouse-bi/src/store.rs`
  `set_board_embed`, `set_board_embed_origins`, `revoke_all_embed_tokens`,
  `withdraw_embed_token`; on `fix/pr105-review-fixes` (`dce286b9`):
  `console.bi_board_embed_event`, `get_board_embed_access`,
  `get_board_embed_access_at`.
- Filters: `lakehouse-bi/src/filters.rs` `FilterDef` (keyed by `column`;
  no separate id), `validate`, `validate_filters`, `applies_on`.
- Access: `lakehouse-bi/src/access.rs` `Viewer::anonymous`.
- Policy: `policy.rs` `POLICY_TABLE` (`/api/embed/data`, `/api/embed/frame`
  public; `/api/dashboard/embed-info`, `/api/dashboard/embed-revoke`), the
  public-count test.
- Console: `src/app/embed/signed/[jwt]/page.tsx` (`?chart=`),
  `src/app/embed/dashboard/[token]/page.tsx`,
  `src/features/dashboards/embed-view.tsx` (`chartId`, the "Rantai Lake"
  text, `useTheme`), `src/features/dashboards/share-dialog.tsx`,
  `src/lib/embed-frame.ts`, `src/proxy.ts`.
- Assistant: `routes/ai/gate.rs` `decide`, `effective_risk`,
  `reason_for_write_high`; `routes/ai/registry.rs` `TOOLS`;
  `routes/agents.rs` `decide_approval`; `capabilities.ts`.

## 3. Tasks (one commit each, in order; Rust first)

**T1 Chart tokens (D1).** *Accept:* `lakehouse-embed` tests (chart and
dashboard both → refused, neither → refused, chart alone accepted);
route tests: a chart token's payload has exactly one chart and no other
chart id anywhere in the body; a chart on another board's withdrawn or
embed-off board refused; a trashed chart refused; the `role`/`sub` claim
test; `frame_origins` and `revoke_token` with a chart token;
`embed_tokens` and `sec11_guard` green.

**T2 Filter modes and resolution (D2, D3, D4).** After #201 is in the
branch. *Accept:* store test (newest `filter_modes` wins); route tests: a
missing locked value refused with the fixed body; an editable value
applied; a viewer value for a locked or hidden column dropped; a token
value wins over a viewer value; a legacy token locking a column that is
no filter still narrows (`BI-26-AC7`); an invalid viewer value → 400.

**T3 Values route and settings route (D5, D8).** *Accept:* the values
route lists only values left after the locked filter (Bali token → only
Bali's rows' values), refuses a non-editable column, caps at 1,000; the
settings route needs manage, refuses a column that is not a filter, is
audited; `route_auth` both ways for both; the public-count test; the
sample token carries `chart` when asked.

**T4 Embed view (D6, D7).** *Accept:* component tests: `logo=none` removes
the name; `theme=dark` overrides the board; `background=transparent`;
editable filter controls render and send `filters`; the size message is
posted with each allowed origin and not when there is none; webpack build.

**T5 Share dialog (D9).** *Accept:* component tests (chart picker changes
the snippet to `resource.chart`; every locked filter appears in the
signing example; modes saved); webpack build.

**T6 Assistant, AI-11 embed half (D10).** *Accept:* tool tests: requester
without manage refused before any approval; `enable_embed` creates an
approval naming the board and the sites, executes as an approver with
manage (`AI-11-AC3`), not as one without; `set_embed_filter_modes` needs
approval; `withdraw_embed_tokens` asks for a confirm; `get_embed_snippet`
contains no token; no `EMBED_SECRET` → refused; fixture; count pin;
`capabilities.test.ts`.

**T7 Docs.** CHANGELOG (the `?chart=` limit and the snippet);
`docs/core/features/safer-embed-tokens.md` "Asking the assistant" is the
planner's to update; feature page status untouched (planner).

## 4. Out of scope

An SDK or React components; editing in an embed; the customer's own logo;
downloads from embeds; secret rotation; filter modes on public links
(feature decision 12); a new signing scheme; anything in `BI-19`.

## 5. Must verify, not assume

- That PR #201 has merged and the event table's aggregate read is the one
  named above when T2 starts; if #201 changed shape in review, report
  before writing D2's kind.
- How the dashboard page encodes its filters in requests (so D4 takes the
  same shape and no second encoding appears).
- That `src/proxy.ts`'s frame-ancestors answer for a chart token is the
  chart's board's list (T1).
- That a `ResizeObserver` loop cannot start (a height change that changes
  the height): test with a frame whose content depends on its width.

## 6. Checks

As the `BI-1` plan's section 6 (same cargo environment:
`CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-uiux-target CARGO_BUILD_JOBS=2`,
one cargo command at a time, `--test-threads=4`, `route_auth` at 2; check
`free -g` / `df -h /`; never `cargo build -p lakehouse-api`; no
`rust/target`; no prettier; no full workspace run). Per task the crates
touched (`lakehouse-embed`, `lakehouse-bi`, `lakehouse-api`), the
`lakehouse-api` lib filters for `embed`, `dashboard` and `ai`, and the
integration files `route_auth`, `sec11_guard`,
`embed_tokens`, `ai_tool`; TS typecheck, lint, tests of the files touched;
the webpack build. No dependency change planned.

## 7. Handoff (developer appends)

### BI-26 T1 to T4 (T5 and T7 not done, T6 skipped), branch `feat/bi-phase-4b`

**Skipped on purpose.** T6 (the assistant, AI-11 embed half) touches
`routes/ai/registry.rs`, `gate.rs`, `prompt.rs` and `mod.rs`, which the AI team
is working in; it is coordinated with them later. Nothing in `routes/ai/` or
`capabilities.ts` was touched.

**Migrations: none.** Filter modes are a `filter_modes` event in
`console.bi_board_embed_event`, with the JSON object kept in the existing
`origins_json` text column (no new column, no Postgres table).

**Commits.**
- T1 `028c23ac` chart tokens. Verified: `lakehouse-embed` lib 23, `embed_tokens` 25, `sec11_guard` 4, `lakehouse-api` lib filter `embed` 14, clippy on `lakehouse-embed` and `lakehouse-api` clean.
- T2 `8fb7aff4` filter modes and their resolution. Verified: `lakehouse-bi` lib filter `embed_access store::tests` 110, `embed_tokens` 30, clippy on `lakehouse-bi` and `lakehouse-api` clean.
- T3 (commit "BI-26 T3 ... NOT VERIFIED") values route, settings route, sample token. **Not verified: nothing has run on this commit.** The tests are written (`embed_tokens` values, settings and sample token tests; `route_auth` `the_embed_values_and_settings_routes_are_gated_in_both_directions`; the policy count test is now 11, renamed `exactly_eleven_public_entries_exist`). The run was stopped when disk fell to 8.8 GB (floor 10 GB).
- T4 `0383ed18` the embed look, editable filters and height messages (console). Verified: `bun test src/features/dashboards src/lib src/features/admin src/services` 1472 pass; typecheck clean; lint 0 errors. Webpack build not run.

**Anchors.** Re-verified by symbol; all found: `EmbedResource`, `verify_embed`, `routes/embed.rs` `data`, `frame_origins`, `revoke_token`, `render_board_payload`, `board_body`, `params_to_filters`, `console.bi_board_embed_event`, `embed_access_from_aggregate_rows`, `values_for_roles`, `filter_fields`, `embed_info_body`. Section 5: the dashboard page sends filters as a JSON array of `FilterDef` (camelCase); D4 takes the same shape. #201's event table is in this branch.

**Deviations and decisions to confirm.**
- Filter modes are stored in `origins_json` (see above).
- A chart token has no editable filters and gets no values route (403): its payload is that chart alone, and the dropdown reads the whole dashboard's relations. Plan silent on this.
- An editable filter must be one of the board's saved filters (`board.filters`). The dashboard drops a filter that filters nothing when saved, so a filter with no saved value cannot be made editable.
- The dropdown is single-choice (`in` with one value, or All) for any column kind; date and number ranges are not offered.
- Single-chart embeds fill the frame and send no height messages; only the whole dashboard does.
- `lakehouse_bi` `ValuesQuery` got a `for_board` constructor and `values_with_limit` (the dashboard's 200 stays; the embed's is 1,000).

**Not done.** T5 (Share dialog), T7 (CHANGELOG), the final checks the coordinator listed (workspace clippy, `public_links`, full TS lint of T5), and a browser check of AC1 to AC7.

### BI-26 T3 verified, T5 and T7 done (final handoff)

- **T3** is now verified (commit "T3 follow-up" fixes one ordering bug the run found: a chart token on a board with a locked filter got 401 instead of 403 from the values route). `cargo test -p lakehouse-api --test embed_tokens -- --test-threads=1` 34 pass; `--test route_auth -- --test-threads=2` 61 pass (includes `the_embed_values_and_settings_routes_are_gated_in_both_directions` and the public count of 11); `--test sec11_guard` 4; `--test public_links -- --test-threads=1` 10; `cargo fmt --check` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean.
- **T5** `bccc8013` the Share dialog (`embed-settings-panel.tsx`): dashboard or one chart, look options, filter modes (saved, marked unsaved until saved), a preview frame loaded on request, the signing example naming every saved locked filter, and the host snippet with the ten-line listener. `bun test src/features/dashboards src/lib src/features/admin src/services` 1485 pass, 0 fail; typecheck clean; lint 0 errors. Webpack build not run (disk; CI runs it).
- **T7** `3bd4a258` CHANGELOG entries for BI-26 (with the `?chart=` limit and the snippet) and for BI-19 part A.
- **Decisions accepted by the coordinator:** filter modes in `origins_json` (no migration); a chart token has no editable filters and no values route; an editable filter must be a saved dashboard filter; the dropdown is single choice; a single chart fills its frame and sends no height.
- **Not done / not verified:** T6 (assistant), a browser check of AC1 to AC7 (a real `ResizeObserver` in a real host page; the preview frame; the dropdown against a real ClickHouse), the webpack build.
- **Shared-target note:** run `find .../debug/deps -type f -perm -u+x -mmin +10 -size +5M -delete` before cargo steps to keep disk up.

### Fix round 1 (BI-26), after "Review 1"

One commit per finding, each citing "BI-26 review fix (...) Rn" at the fix site.
- **R1 BLOCKER** (`resolve_embed_filters`): for a column that is locked, or editable and set by the token, the token's value replaces the saved filter; hidden stays ANDed (old tokens read as before). The values route uses the same function. Tests: saved Bali + token Java for locked, editable and hidden (no Bali predicate for the first two, both for hidden); the dropdown of another filter sees 2025, not the saved 2024.
- **R2** `store::jti_withdrawn_anywhere`: a chart token's `jti` is checked on every dashboard's events. Test: withdraw on A, move the chart to B, the token is still refused and another token of the chart works. The feature page's limits say a "withdraw all" is the dashboard's.
- **R3** `put_settings` refuses locked/editable for a filter whose operator is not `in` (fixed sentence); embed-info lists `hiddenOnlyColumns`; the dialog offers only Hidden for them, with a note. Tests: the refusal for both modes with nothing stored, the list, the panel's options.
- **R4** `frameAncestorsPolicy` always starts with `'self'`; an unreadable list and a path with no token stay `'none'`. Tests: the policy strings.
- **R5** the height hook is enabled only when the payload has loaded and is not a single chart. Tests: nothing is posted while the data request is pending, one message follows; a chart token posts nothing.
- **R6** `put_settings` uses `public_links::managed_board` (`pub(crate)`); `resolve_frame_origins` uses `verified_claims` and `admit` and maps any refusal to an empty list.
- **NOTES taken:** a "why" comment on `fetchFrameOrigins`; `strip_hidden_skips` removes a hidden filter's column from `filtersSkipped` in signed embeds (unit test); the dialog's write flag goes to the panel (`canWrite`, default true); a comment on the audit helper. **R3, R6 and the NOTES are one Rust commit** (they edit the same functions in `routes/embed.rs`), with the console half of R3 in its own commit.

**Checks run:** `embed_tokens` 38 pass; `lakehouse-api` lib filter `embed` 15; `sec11_guard` 4. **Run afterwards on the final tree (disk 9.7 GB or more):** `cargo fmt --check` clean; `cargo test -p lakehouse-api --test route_auth -- --test-threads=2` 61 pass; `cargo clippy --workspace --all-targets -- -D warnings` clean (this covers R1 to R6 in `lakehouse-api` and `lakehouse-bi`). TS: `bun test src/features/dashboards src/lib src/features/admin src/services src/proxy.test.ts` 1491 pass, 0 fail; typecheck clean.

## 8. Review (planner appends)

## 8. Review 1 (planner, 2026-10-24, `028c23ac..76b87392`)

Read-only review against the plan, the feature page and the handoff. Checked fine: chart-token isolation; locked values cannot be overridden; the values route is bounded, masked and bound; hidden filters stay out of the payload; withdrawals and expiry; SEC-11; `POLICY_TABLE` and `route_auth`; independence from public links.

- **R1 BLOCKER: a locked filter can never show a value outside the saved one.**
  - `resolve_embed_filters` keeps the saved filter and ANDs the token's value onto it. A board saved with `Bali` and a token for `Java` gives an empty dashboard.
  - Decision (reading feature decisions 5 and 7): for a column that is locked, or editable and set by the token, the token's value **replaces** the saved filter. Hidden columns stay ANDed, as today, so old tokens (owner answer 16) do not change.
  - Same in the values route.
  - Test: saved Bali plus a Java token shows Java rows, and no Bali predicate is sent.
- **R2 SHOULD-FIX: moving a chart drops its tokens' withdrawals.**
  - Decision: a chart token's `jti` withdrawal is checked across every board's events. A board-wide "withdraw all" applies to the board the chart is on now; the feature page states this limit.
  - Test: withdraw on A, move the chart to B, and the token is refused.
- **R3 SHOULD-FIX: locked and editable work only for `in` filters.**
  - Decision: `put_settings` refuses `locked` or `editable` for any other operator with a fixed sentence, and the dialog offers only "hidden" for those filters with a note.
  - Test: the refusal.
- **R4 SHOULD-FIX: the Share dialog's preview frame is blocked by `frame-ancestors`.**
  - Decision: embed pages also allow `'self'` (the console's own origin) in `frameAncestorsPolicy`; the comment says why.
  - Test: the policy string.
- **R5 SHOULD-FIX: a chart token can post one height message from the skeleton.**
  - Decision: enable the height hook only when the payload is loaded and it is not a single chart.
  - Test: no message before the payload arrives.
- **R6 SHOULD-FIX: two guards are written a second time (rule 4).**
  - `put_settings` must reuse `public_links::managed_board` (make it `pub(crate)`).
  - `resolve_frame_origins` must reuse `verified_claims` and `admit`, mapping any refusal to `[]`.
- **NOTE:**
  - A "why" comment on `fetchFrameOrigins` (no session, so no `apiFetch`).
  - `filtersSkipped` may name a hidden filter's column in combined charts. This predates BI-26; strip it from embed payloads (small).
  - Pass the Share dialog's write flag to `EmbedSettingsPanel`.
  - Rename the audit helper call, or comment on it.

**Decisions accepted:** filter modes in `origins_json` (no migration); a chart token has no editable filters and gets 403 from the values route; editable filters must be saved filters (with R1); the single-choice dropdown for `in` filters (with R3); a single chart sends no height messages (with R5).

### Owner's browser QA, 2026-10-24

Passed:
- Turning signed embedding on.
- One chart with its picker, and its preview.
- The dark look with our name hidden.
- An editable `provinsi` filter, which shows a value picker in the preview.
- The settings are kept after reopening.

BI-26 QA passed.

### Re-review after fix round 1 (Evan's planner, 2026-10-26, at `ac0782ab`)

Every finding of Review 1 is closed in the code:
- **R1 (BLOCKER), closed.** In `resolve_embed_filters` (`routes/embed.rs`), the token's value replaces the saved filter for a locked column and for an editable column the token sets. Hidden columns stay ANDed.
- **R4, closed.** Embed pages allow `'self'` in `frame-ancestors` (`src/lib/csp.ts`). A dry merge with `main`'s SEC-47 CSP keeps this, and every other page keeps `'none'`.
- **R6, closed.** `put_settings` reuses `public_links::managed_board`, and the frame-origins route reuses `verified_claims` and `admit`.
- R2, R3 and R5 are closed per the fix-round handoff, with the tests named there.

Owner QA passed (above). No BLOCKER is open.

`T6`, the assistant's embed tools, is `AI-11`: a separate item Blocked by `BI-19`, `BI-26` and `DEC-8`, so it is not part of PR #235.
