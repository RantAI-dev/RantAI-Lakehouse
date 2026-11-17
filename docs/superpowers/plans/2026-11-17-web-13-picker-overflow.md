# WEB-13 — long identifiers stay inside pickers

Task bug; one PR. Base: `origin/main` at
`34d1276a2818d1f4a88e9a937eacb8da0b66f7b9`, verified 2026-11-17.
Planner/reviewer: Codex. Developer: HY4 flash Free (`opencode-go/hy4-flash-free`).
Owner explicitly authorised Codex to plan and review **this fix only** on
2026-11-17 after Claude Opus could not run because of its API rate limit.
The planner and product-code author remain separate. No general AGENTS.md change.

## Problem and evidence

The owner supplied a Roles screenshot: an expanded table-access schema contains
an unbroken table identifier that paints beyond the bordered picker. Do not copy
real table names from the screenshot into fixtures or repository prose.

Verified anchors at the base above:

| Site | Observation / disposition |
| --- | --- |
| `src/features/admin/role-table-access.tsx:393` | Schema wrapper can shrink, but its flex Label has no text wrapping policy. |
| `src/features/admin/role-table-access.tsx:408` | Table checkbox row has an unconstrained grid child at 419 and flex Label at 422; intrinsic minimum width admits the screenshot's overflow. Listed and preserved-unlisted tables use this path. |
| `src/components/ui/label.tsx:12` | Label defaults to flex; setting a word-break class on its parent alone is insufficient evidence its anonymous text flex item can shrink. Use an explicit shrinking/wrapping text span where needed. |
| `src/features/admin/permission-picker.tsx:199` | Unknown-token Badge has no width bound, preserves an arbitrary old token, and has removal controls. Known permission row grid at 329 likewise lacks a shrinking bound. |
| `src/features/connectors/connector-ingest-panel.tsx:1154` | Loaded-column checkbox label renders raw dynamic column text as a flex item without containment. Key checkbox label at 1216 repeats it. |
| `src/features/governance/table-picker.tsx:156` | Full-width trigger and selected FacetedBadgeList lack consumer width constraints; option text at 173 has no wrapping/shrinking rule. |
| `src/features/pipelines/pipeline-pickers.tsx:240` | Multi-column Faceted trigger has the same selected badge and option issue at 251. |
| `src/components/ui/faceted.tsx:175` | Badge list is wrapping flex but has no shrinking bound; Badge defaults to shrink-0/w-fit/nowrap (`ui/badge.tsx:9`). Consumer overrides can bound the badge/list without globally changing every control. |
| `src/features/connectors/table-finder.tsx:119` | Existing min-w-0/flex-1/truncate plus complete-name title pattern; use as reference, no broad rewrite. Column name at 260 is already truncated. |

These are code-audited layout risks, not claimed browser reproductions of every
site. The owner screenshot establishes the Roles defect. Audit is bounded to
identifier pickers; not a claim the entire console is overflow-free.

## Decisions

1. Fix at the five consumers above. No global Label, Badge, Faceted, table/grid,
   typography or API behavior changes. No new helper/component is needed for this
   small set of local layout fixes. Keep existing connector table-finder patterns.
2. In Roles and checkbox lists, allow the text region to shrink (`min-w-0`, a
   bounded flexible width) and wrap unbroken identifiers using an explicit span
   (`break-all` or existing anywhere-wrap utility). Keep checkboxes, icons and
   remove buttons from shrinking. Use readable multi-line line-height.
3. Full identifier remains DOM text and accessible name; do not shorten values,
   grants, IDs or submitted payloads. For bounded selected chips, use truncation
   with complete name available through title/accessibility and the wrapping
   option row; cap badge/list/trigger to its available width. Do not hide the
   whole card overflow to conceal the symptom or clip focused controls.
4. Preserve whole-schema grant behavior, unlisted-table explanations, covered
   permissions, disabled/locked columns, checkbox association, keyboard actions
   and exact selection/save payloads. Preserve unknown permissions until removed.
5. Do not redesign unrelated screens or modify security policy. No new default
   credentials, real hostnames/client names or dependency changes. No Rust,
   Python, compose or migrations are required.

## Tasks and acceptance

### 1. Roles picker containment

Change `role-table-access.tsx`; extend its existing component/form regression
coverage using synthetic long schema and table identifiers (both listed and
preserved unlisted grants). Cover create and edit paths through the shared picker.

Acceptance: full label text is present; clicking its label selects the exact
schema/table; saving sends the exact original identifier; unlisted/whole-schema
and disabled behaviors remain unchanged. Layout constraints cover each flex/grid
ancestor and text itself. Cite WEB-13 at the non-obvious containment fix.

Run the touched role tests and file-scoped lint. Commit task 1 independently.

### 2. Same-cause sibling pickers

Change only these consumers, plus their scoped tests as needed:

- `admin/permission-picker.tsx`: known permission rows and unknown-token chips;
  preserve readable descriptions, complete tokens and removable controls.
- `connectors/connector-ingest-panel.tsx`: loaded-column and key checkbox labels;
  preserve locked-column reason and exact selection values.
- `governance/table-picker.tsx`: option labels and selected trigger/badge bounds.
- `pipelines/pipeline-pickers.tsx`: multi-column options and selected badges;
  audit ordinary Select controls but do not change already-contained controls
  without reporting a mismatch first.

Acceptance: long synthetic identifiers stay available in full and can be
selected/deselected/removed without mutating their stored value. Multi-selected
chips and unknown-token removal controls remain operable at a narrow width.
No global primitive changes and no speculative unrelated fixes.

Run only touched suites/file-scoped lint while iterating. Commit task 2 independently.
If a source anchor or required behavior contradicts this plan, stop and report
rather than expanding scope or improvising a new feature.

### 3. Final validation and handoff

Run, in foreground, from the finished branch:

```bash
bun run typecheck
bun run lint
bun run test
git diff --check
git status --short
```

No cargo, Python or compose checks for a TypeScript/docs-only change. Do not
weaken tests/lints or rerun unrelated expensive checks just for reassurance.
Report exact counts and any baseline failures separately.

Geometry check: at narrow and wide containers (320px and 768px are chosen test
fixtures, not product performance targets), verify the changed long-name text
and picker fit, checkboxes remain visible, and focus/removal controls are not
clipped. Use an already-available headless layout/browser test facility if it
exists; do not install product dependencies or use preview_* tools for self-edit
verification. DOM/happy-dom class assertions do not measure layout. If an actual
layout engine is unavailable, mark geometry **not verified**, describe why and
leave the manual QA checklist below outstanding; do not call it visually tested.

Append Handoff: commits, exact checks/counts, audit disposition and all not-verified
items. Update CHANGELOG with a short user-visible fix note in the existing style.
Developer may commit and push feature branch, but does not open/merge a PR. Root
Codex reviews independently, records Review here, reruns verification, opens PR
and merges only through the repository check rules.

## Manual QA before acceptance

- Create and edit role → Only these; expand long schema and select a long table.
- Include a preserved unlisted grant and whole-schema selection.
- Confirm no horizontal escape at narrow/wide layouts, including keyboard focus.
- Confirm full identifiers remain available and payload unchanged.
- Check unknown permission removal; loaded/locked source columns and key choice;
  governance table and pipeline multi-column options/selected chips.
- Results, build and tester must be recorded; no screenshot claim without a run.

## Out of scope

Backend permissions/table access, new picker features, shortening identifiers,
changing accepted identifier limits, global UI primitive redesign and a full
console overflow audit. Existing data and grants are never changed to make a
label fit. No deployment performed by the developer.

## Handoff

Pending HY4 implementation.

## Review

Pending independent Codex review under the owner's WEB-13 exception.
