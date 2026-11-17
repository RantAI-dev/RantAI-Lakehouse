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

## Handoff

Pending HY4 implementation.

## Review

Pending independent Codex review and applicable AI-team review.
