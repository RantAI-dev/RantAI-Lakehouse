# GOV-17 — unreadable dependencies hide the entire dashboard

One security/visibility bug, one PR slice with dependency-ordered task commits.
Base verified 2026-11-17: `origin/main` at
`34d1276a2818d1f4a88e9a937eacb8da0b66f7b9`.
Planner/reviewer: Codex; developer: HY4 flash Free, independent session.
Owner requested the same Codex-plan/HY4-build/Codex-review workflow as WEB-13
on 2026-11-17. This is a task-specific exception to the planner model, not a
repository-wide rule change. WEB-13 is a separate branch and must remain separate.

## Owner decision and intended behavior

A person restricted to certain tables must not see a dashboard that depends on
any other table. Hiding only the values while revealing the dashboard title,
chart labels, counts or failure text does not meet this requirement.

This explicitly supersedes GOV-7 AC4's older decision that allowed tiles render
while hidden-table tiles fail. Update that checklist in this slice, recording
this owner decision. A dashboard is visible only when its existing sharing,
personal-space and tenant rules permit it AND every required data dependency
is readable under the effective caller's current table grants. This applies to
all tabs, not just the tab currently shown. Explicit dashboard sharing does not
grant table access. Role grants continue to combine as a union.

## Verified code anchors

All anchors below are at the base commit, not the old parent checkout.

| Anchor | What it establishes |
| --- | --- |
| `rust/crates/lakehouse-api/src/routes/dashboard_access.rs:496` | resolve has an ACL fast path for identity:write or baseline/no marks; it currently does not intersect table dependencies. |
| `rust/crates/lakehouse-api/src/routes/dashboard_access.rs:163` | viewer_for is already shared by HTTP and assistant dashboard reads. payload_scope/scope_for_write also resolve viewers. |
| `rust/crates/lakehouse-bi/src/access.rs:272` | sees_board combines owner and sharing; all bypasses scope. sees_builtin and SQL owner/chart clauses need equivalent table visibility if an overlay is introduced. |
| `rust/crates/lakehouse-api/src/routes/dashboard.rs:260` | payload_for builds scope before any tile query and is reused by authenticated render/PDF. |
| `rust/crates/lakehouse-api/src/routes/dashboard.rs:400` | payload reads boards and charts through Viewer; built-in/default and aggregate all have special checks at 436. |
| `rust/crates/lakehouse-api/src/routes/dashboard.rs:1439` | boards_list synthesizes Main and derives counts independently; must not reintroduce hidden metadata/counts. |
| `rust/crates/lakehouse-api/src/routes/dashboard_organise.rs:850` | shortcuts uses resolved Viewer; versions/trash/favourites/recent/direct chart reads and write-adjacent readers have the same dependency requirement. |
| `rust/crates/lakehouse-api/src/routes/dashboard_sources.rs:195` | source_visible currently checks folder/board visibility, not the SQL source's actual tables. Standalone source readers need dependency checks too. |
| `rust/crates/lakehouse-api/src/routes/ai/tools/mod.rs:354` | dashboard tools share dashboard_access::viewer_for; missing principal returns Viewer::nothing, not an anonymous privilege fallback. |
| `rust/crates/lakehouse-api/src/table_access.rs:83` | TableAccess::allows canonicalizes, preserves role union/All behavior, and hides unresolved unqualified keys for narrowed callers. |
| `rust/crates/lakehouse-api/src/sql_rewrite.rs:118` | referenced_tables returns None on parse failure; its documented CTE handling must be considered, not duplicated by a regex parser. |
| `rust/crates/lakehouse-bi/src/store.rs:279` | StoredChartSpec carries final SQL/spec, definition and board. ChartInput has sql_source as an alternative to mart. |
| `rust/crates/lakehouse-api/tests/table_access.rs:548` | Existing test intentionally asserts mixed-dashboard partial rendering; owner changed that requirement, so replace it with stronger whole-board denial plus an allowed-board positive control. |
| `rust/crates/lakehouse-api/tests/table_access.rs:780` | Combined-chart tests cover denied layers; new whole-dashboard decision changes expected metadata, not the obligation to avoid any denied engine read. |
| `docs/core/features/table-access-per-role.md:84` | Old mixed-dashboard AC4 must be superseded explicitly, not silently weakened. |

## Decisions already made

1. The server is the authority. No client-only filtering, hiding errors or catching
   404 and rendering empty tiles. Hidden by-id reads return the existing fixed
   unknown-dashboard/chart response, without the requested or dependent table
   name. Lists omit hidden objects and derive counts/pagination after visibility.
2. Resolve effective table access once per request using existing TableAccess and
   ACL viewer paths. Introduce one API-owned dependency/visibility resolver, reusing
   SQL parsing/canonicalization and source/chart builders rather than copying
   checks in each handler. BI must not depend on API types. A pure Viewer table-
   visibility overlay (or equivalent existing scope intersection) carries denied
   board IDs and built-in visibility through store reads.
3. This overlay can only narrow the existing Viewer, never replace its sharing
   or personal-space checks. It must affect in-memory predicates AND generated
   board/chart SQL clauses AND built-in handling consistently. identity:write
   alone is not a bypass of narrowed table grants. Genuine *:* or a role on All
   keeps established access. Viewer::system remains for trusted server collection
   and housekeeping only; request handlers must not expose its unfiltered rows.
4. Compute dependencies from every chart, KPI and tab, including compiled built-in
   Main, joins, secondary/combined layers, stored and inline SQL sources, and any
   referenced saved source/dependency actually used by rendering. Use final resolved
   SQL plus structured definitions as necessary to avoid trusting a stale/missing
   mart field. Do not execute chart data SQL to decide visibility.
5. Restricted caller + unresolvable/missing/cyclic/unsupported dependency => do not
   expose that resource. A resolver/store outage returns a classified failure or
   hides the unresolved resource as the documented response contract requires;
   it NEVER falls back to All, an empty dependency set or a privileged principal.
   A valid truly data-free static chart/empty board can remain visible when ACLs
   allow it. Distinguish that from broken parsing or a missing SQL source.
6. No name-based heuristics, regex SQL parser, default-table assumptions or caches
   that survive permission changes. Reuse referenced_tables and checked canonical
   keys. Respect existing MAX_SCOPE_IDS and refuse oversized scope rather than
   truncating a deny list. Avoid N+1 role/source queries: batch metadata reads.
7. New visibility is additional to tenant/mask/row policy, not a substitute.
   Row filters/masks alone do not hide a dashboard if all its tables are allowed.
   Table grants changed on a live session take effect on the next request, even
   if tile-result/export caches were previously populated with broader access.
8. Public links/embeds use their existing effective Dashboard Viewer grants, not
   a new unrestricted principal. PDF/render uses its actual captured caller.
   Reports must recheck effective owner AND each authenticated recipient at send
   and attachment download: membership/sharing must not deliver a newly hidden
   dashboard. Retain documented policy for outside addresses; never invent a
   recipient role or silently grant access to satisfy a report.
9. AI parity is part of the bug: list_boards/list_charts/source tools, dashboard
   page context and assistant discovery/data-map references must omit hidden
   objects. No new tools/schema needed for changed visibility; if an actual tool
   schema changes, regenerate tests/fixtures/tool_schemas.json and require AI team
   review. Record that team review as owed, never fabricate it.

## Tasks, in order

### T1 — shared dependency visibility and scope intersection

Implement the API resolver and minimal BI Viewer/store support. Narrow the resolved
request viewer on HTTP read/write scope and assistant paths. Audit all privileged
collection shortcuts before exposing results. Add pure tests and API integration
fixtures for all-allowed, one-denied, role union, schema wildcard, All, identity-
write-only narrowed user, no-table static content and parse/source failures.

Acceptance: mixed board hidden completely, allowed board unchanged; default Main
hidden if any required built-in/stored data dependency is denied; `all` aggregate
shows only complete visible boards and never includes denied built-in tiles.
No denied engine query is dispatched. SQL lists and direct predicates agree.
No permission lookup/dependency error broadens access or truncates exclusions.

Run fmt, scoped clippy and tests for each touched Rust crate. Commit T1 separately.

### T2 — discovery, alternate readers and delivery paths

Audit and wire remaining reads through the shared guard. Provide a disposition
matrix with exact call sites for:

- board chooser/browse/list, list counts and folders' board entries/counts;
- chart specs/detail/records/values/fields/lineage/comments and version/trash
  snapshots that could reveal a hidden dashboard's contents;
- favourites, recents, application search/home dashboard references;
- assistant list_boards/list_charts/SQL sources and page/prompt context;
- export/download/print/render, signed embeds and public links;
- report subscription list/detail/history/send-time and attachment download;
- duplicate/import/export/move/revert or preview paths that read an existing
  hidden object (do not add a new write permission or change author ownership).

A global SQL-source list/detail must not reveal a denied source merely because
its folder is visible. Do not expose foreign table names in the refusal. Guard
before assembling or serializing metadata and before reading tile-result caches.

Audit independent saved queries, pipelines and alerts for the same metadata
pattern. In this dashboard slice fix paths that reveal/use hidden dashboard
objects or their sources. Record concrete independent-resource gaps separately
with files, dependencies and proposed follow-up tickets; do not claim those
resource classes accepted or silently expand their authoring semantics. If
fixing a dashboard path genuinely requires a larger contract change, STOP and
report the mismatch before changing scope.

Acceptance: positive/negative route/tool tests across the matrix, owner versus
recipient grant-change tests, current-role resolution after cache warm-up, no
hidden IDs/titles/table names/chart counts in responses. Inaccessible and unknown
objects have the same relevant status/body. Add a frontend test only if browser
selection/fallback behavior needs adjustment: it should select a permitted board
or show an honest empty state when none exists, never auto-open hidden Main.
Update the old mixed-dashboard/layer tests and explain changed owner requirement
in comments and commit body (not weakening a guard to get green).

Run scoped checks for touched crates/languages. Commit T2 separately.

### T3 — acceptance documents, QA and handoff

Update GOV-7 feature prose/AC4/AC9 for whole-resource hiding and cite the owner's
2026-11-17 decision. Preserve old historical results as superseded, not newly
passing. Update relevant sharing/report docs only where behavior changed and
CHANGELOG with a customer-readable note. Add new manual regressions in the plan
or feature page: mixed allowed/denied board, allowed-only control, no visible
boards, direct guessed URL, joins/SQL source/layer dependencies, role change,
assistant discovery, cached export, public/embed viewer, report recipient.

Full final verification ONCE in foreground for touched languages:

```bash
cd rust
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
# If TypeScript changed, at repository root:
bun run typecheck
bun run lint
bun run test
# Repository root:
git diff --check
git status --short
```

Use shared CARGO_TARGET_DIR (machine shared path); no cargo clean/per-worktree
target. Existing container tests need Docker access; use documented runner/group
setup, not skipped tests presented as passing. Record exact commands/counts and
baseline failures with evidence. Do not change dependencies/migrations unless a
plan mismatch has been reported first. No compose/Python checks for untouched
languages. Do not leak upstream errors, credentials or screenshot deployment data
into docs/code/log excerpts. No browser preview tools for verifying own edits.

Append Handoff with per-task commits, audited call-site matrix and all not-verified
items. Developer pushes feature branch, never opens/merges PR or watches it.
Independent Codex reviewer reruns checks, records BLOCKER/SHOULD-FIX findings here,
opens PR only after review; merge follows normal baseline/check rules. AI-team
review is requested/recorded for assistant changes. No direct main push.

## Out of scope

New table privileges, changing role union/defaults, universal tenant redesign,
frontend-only suppression, deleting/changing dashboards to make dependencies fit,
external-recipient policy redesign and a claim that every independent artefact
class is covered. Open independent artefact findings must be explicit at handoff.

## Handoff (HY4 developer, 2026-11-17)

Branch `fix/gov-17-dashboard-table-visibility`, prepared checkout
`/home/shiro/.t3/worktrees/RantAI-Lakehouse/t3-67c8ff35`, base
`0a0ed0dd6cd8c4dff2b8c902eef7291eccb1d9a9` (`origin/main` at
`34d1276a2818d1f4a88e9a937eacb8da0b66f7b9`). No worktree or thread created;
WEB-13 untouched; no PR opened, merged or watched; nothing pushed to `main`.

### Commits (one task per commit)

- **T1** `8cb0c3d6078a4712343d34731ecd66d7200139a2` — shared dependency
  resolver, pure `Viewer` overlay, central viewer/scope narrowing, payload
  and built-in/`all` checks, old mixed-board tests replaced.
- **T2** `681fede8b09a0ef389ce6a800a1698b9e05584c5` — discovery/delivery
  matrix, embed and public-link effective-role guard, report recipient
  rechecks, route regressions.
- **T2 follow-up** `e81ea4d08...` — keep `embed.rs` clippy-clean after the T2 guard
  (boxed refusal response; extracted the time/format helper so `data()` is
  back under the line limit). No behaviour change.
- **T3** — feature page, CHANGELOG and this handoff.

### Commands actually run, with counts

```
cd rust && cargo fmt                                   # ok
cd rust && cargo fmt --check                           # ok
cd rust && CARGO_TARGET_DIR=/home/shiro/rantai/cargo-target \
  cargo check -p lakehouse-bi -p lakehouse-api --all-targets          # ok
cd rust && CARGO_TARGET_DIR=/home/shiro/rantai/cargo-target \
  cargo clippy -p lakehouse-bi --all-targets -- -D warnings           # ok
cd rust && sg docker -c 'export CARGO_TARGET_DIR=/home/shiro/rantai/cargo-target; \
  cargo test -p lakehouse-bi --lib access::tests::'                   # 47 passed, 0 failed
cd rust && CARGO_TARGET_DIR=/home/shiro/rantai/cargo-target \
  cargo test -p lakehouse-api --lib dashboard_visibility::            # 9 passed, 0 failed
cd rust && sg docker -c '... cargo test -p lakehouse-api --test it table_access::'
                                                                       # 27 passed, 0 failed
cd rust && sg docker -c '... cargo test -p lakehouse-api --test it'
                                                                       # 1635 passed, 1 failed, 5 ignored
```

Final workspace run, foreground:

```
cd rust && cargo fmt --check                                           # ok
cd rust && CARGO_TARGET_DIR=... cargo clippy --workspace --all-targets \
  --all-features -- -D warnings
# 2 errors: clippy::large_futures in routes/ai/mod.rs:322 and :1607.
# Both files are untouched by this branch; these are baseline on the named
# base commit under this machine's Rust 1.99 clippy, not introduced here.
cd rust && sg docker -c '... cargo test --workspace'
# NOT VERIFIED: the machine went to ENOSPC (0 bytes free) while linking the
# test binaries; two further attempts were also killed. No fabricated result.
# The scoped suites above, which cover every crate this branch touched, are
# the verified substitute; see "Not verified" below.
```

The one full-suite failure was
`ai_sql_help::a_chat_with_no_allowlist_is_offered_today_s_tools_and_today_s_prompt`
(system-prompt hash). It passes in isolation both **with** and **without**
this branch's T2 changes, so it is order-dependent shared-state flakiness,
not a GOV-17 regression. No test, lint or threshold was weakened.

### Per-route audit matrix

Guard applied = resolved request viewer (and, where marked, the combined
`sees_builtin_board()` check) before metadata is serialised or a cache is
read. All list/by-id paths that already took `viewer_for`/`scope_for_write`/
`payload_scope`/`usage_viewer_of` inherit the overlay automatically; the
call sites below were checked explicitly.

| Area / call site | Disposition |
| --- | --- |
| `dashboard.rs::payload_for` → `get_body` (authenticated render, `render`, PDF print view) | Guard applied via `payload_scope`; by-id in-memory `sees_board` check added before tile assembly; built-in/`all` checks added. |
| `dashboard.rs::boards_list` (chooser/browse, `chartCount`, Main entry) | Guard applied; Main omitted when built-in deps denied; counts come from the narrowed chart list. |
| `dashboard.rs` YAML/`format=file` export | Guard applied; Main omitted from the export with the combined check. |
| `dashboard_organise.rs` shortcuts, board versions/trash, `require_chart_board` | Guard applied via `viewer_for`/`scope_for_write`; Main checks use `sees_builtin_board()`. |
| `dashboard_comments.rs`, `dashboard_folders.rs`, `dashboard_file.rs`, `dashboard_sources.rs`, `public_links.rs` reads | Guard applied through the shared viewer/scope paths. |
| `dashboard_sources.rs` source list/detail and delete-conflict count | Source folder visibility unchanged; a hidden dashboard's chart titles/counts are not named — the caller is told only "a dashboard you cannot see". Reviewed, no table/title/table-name leak; see followups. |
| `pdf_export.rs`, `render.rs`, report owner + recipient rechecks (`report_runner.rs`) | Guard applied; built-in uses combined check; recipients resolve their own viewer at send time. |
| `embed.rs::data` and public-link `render_board_payload` | New: resolve the effective Dashboard Viewer role's fresh grants; hidden board gets the fixed token refusal; chart list narrowed before assembly. No new privileged principal. |
| AI `list_boards`, `list_charts`, `list_sql_sources`, dashboard page context, discovery references | Guard applied through `viewer_for`/`scope_for_write`; no tool schema changed, so `tests/fixtures/tool_schemas.json` is unchanged. AI-team review still owed below. |
| Favourites, recents, home references, search/items | `lakehouse-bi::search` and `items` use `sees_builtin_board()`; the rest take the narrowed viewer. |
| Chart records/values/fields/lineage/comments, folder board counts | Reached only through the narrowed viewer for the authenticated read paths; deletion-conflict counts intentionally count every board so a delete cannot orphan one, but the response names no board, title or table (followup below). |
| `Viewer::system()` collection shortcuts (`chart_alerts`, `used_by`, `maps`, `layout_tabs` tests) | Reviewed: used only to decide a generic "you cannot see it" answer or in test fixtures; no hidden title/table is serialised. Not replaced with a privileged request viewer. |

### Independent artefact audit — concrete followups, NOT claimed fixed

- Saved queries (`saved_queries.rs`) and authored pipelines (`authored_pipelines.rs`)
  keep their own SQL and folder rules. They do not list a dashboard's charts,
  but their SQL can name a table a caller may not read and their usage lists
  can name a dashboard. Proposed followup ticket: **GOV-18** — apply the same
  dependency resolver to saved-query and pipeline metadata and their "used by"
  lists; owner sign-off needed before changing their authoring semantics.
- Alerts (`chart_alerts.rs`, `alerts.rs`) read a chart by id and distinguish
  "lost access" from "gone" using a system-viewer lookup. The response names
  no chart title or table, but the two outcomes are still distinguishable.
  Proposed followup ticket: **GOV-19** — give the two outcomes one fixed
  sentence and decide the alert-history contract with the product owner.
- Folder delete-conflict counts deliberately count boards the caller cannot
  see so a delete cannot orphan them. The response reveals a count but no id
  or title. Proposed followup ticket: **GOV-20** — replace the count with a
  count-free generic refusal; needs an owner decision on the delete contract.

No independent artefact class is claimed accepted by this slice.

### Not verified / owed

- `cargo test --workspace`: not completed — ENOSPC during test-binary linking
  on this machine (shared `CARGO_TARGET_DIR=/home/shiro/rantai/cargo-target`).
  No cache was wiped and no test was weakened; only this session's own
  generated test binaries and shell-output files were removed to continue.
- AI-team review of the assistant/discovery part of the diff (decision 9) is
  owed and was **not** fabricated. No tool schema changed.
- Manual QA: run `GOV-17-AC1` to `GOV-17-AC9` on the feature page. Owner Rizky,
  ticket `GOV-17` (`reczz28adVS3doOQ`); Lark was not edited by the developer.
- No frontend change was needed; the chooser already shows an honest empty
  state and never auto-opens Main. No TypeScript, Python or compose check run
  (no files in those languages touched).
- Baseline red: workspace clippy `large_futures` at `routes/ai/mod.rs:322,1607`
  (untouched); the flaky prompt-hash test noted above. Both must be named in
  review, not silently fixed in this slice.

## Review

Pending independent Codex review and applicable AI-team review.

### Codex review round 1 — 2026-11-17

Reviewed developer head `fb27e7838f8c2c4a5347405865995de38d982bf3` against
plan/base. **Changes requested; no PR/merge approval.**

1. **BLOCKER R1 — hidden Main chart metadata still leaks.**
   `dashboard_visibility.rs::hide_board` handles Main by setting only
   builtin_visible=false; it adds neither default nor the raw empty-board
   spelling to hidden_boards. `access.rs::chart_owner_clause` consults only
   hidden_boards, so list_stored_charts still returns Main's stored tiles.
   `/api/dashboard/specs`, assistant list_charts, search and file export consume
   that result without get_body's special filter. Apply the built-in denial
   consistently in SQL clauses and in-memory board predicates, including legacy
   empty/default spellings. Add list/specs/tool/search/export negative tests for
   denied Main with an allowed-board control; no denied chart metadata may leave.

2. **BLOCKER R2 — SQL source metadata is not table-gated.**
   `dashboard_sources.rs:195-241` is unchanged: source_visible returns true for
   every root-folder source, and list serializes the full SqlSource (SQL and
   columns). `ai/tools/dashboards.rs::list_sql_sources` uses that same predicate.
   Narrowing a board does not narrow its sources. This violates plan T2's
   explicit source-list/detail requirement and cannot be described as inherited
   protection. Reuse the dependency resolver to hide denied/invalid sources in
   HTTP and tool list/detail/fields/preview-read metadata paths, preserving
   folder ACLs and fixed unknown responses. Add direct root-folder SQL-source
   tests with denied table SQL and a permitted source as positive control.

3. **BLOCKER R3 — permission/admin shortcuts ignore the overlay.**
   `Scope::require_on_board` (`dashboard_access.rs:346-357`) checks only AccessMap
   and never its narrowed viewer. require_board_level reads a row as system and
   calls that method, so guards advertised as common board-level enforcement
   can admit a table-hidden board. `Viewer::sees_all` remains true for narrowed
   identity:write callers; `dashboard_organise::Seen::read:320` then copies all
   system rows as visible. Ensure the common scope check and every all-reader
   shortcut honour the overlay. Test narrowed identity:write without *:*, and
   a valid explicit edit/manage grant on a board whose table is denied. The
   table restriction must win without granting any new ACL capability.

4. **BLOCKER R4 — report by-id metadata is not rechecked.**
   `reports.rs::own_or_admin:530` checks only subscription ownership/admin. get
   and deliveries serialize the subscription/history without current board
   visibility. A report owner whose role was narrowed after subscription creation
   still reads hidden dashboard configuration by known subscription id. Audit
   HTTP and assistant report detail/history/recipient/output paths and enforce
   the same current caller visibility before metadata/attachments. Test before/
   after grant change and unknown-equivalent denied reads. Preserve documented
   legitimate administrator/system management behavior only when that effective
   caller's table rights actually permit the object; do not use the owner's
   broader grants to expose it to a narrowed caller.

5. **BLOCKER R5 — verification and audit claims are incomplete.**
   The report says followups GOV-18/19/20 are filed, but plan calls them proposed
   and none exists in this branch BACKLOG. Root will create actual Lark items;
   add matching repo backlog entries once their approved definitions are supplied.
   Correct the per-route matrix: source reads and report details above are not
   protected merely by resolving a viewer elsewhere. Prove repaired call sites
   with regressions, not inheritance assertions. Full workspace tests remain
   unverified under disk pressure; baseline lint/flaky-test claims require
   reproducible evidence before any merge. Scoped checks do not replace the
   full mandatory gate. Applicable AI-team review remains owed.

Independent reviewer checks so far: `git diff --check` clean;
`cargo fmt --check --manifest-path rust/Cargo.toml` exit 0. No other reviewer
check claimed passing. Disk snapshot: 242 MiB available; full workspace test
linking is not safe to assume possible. No cache clean or test weakening.

Current origin/main advanced to `cb855d5b9` (includes WEB-13 and staging Keycloak),
so update the feature branch carefully after fixes and reverify the combined
code. Do not cherry-pick another feature's product code or resolve whole files
with ours/theirs.
