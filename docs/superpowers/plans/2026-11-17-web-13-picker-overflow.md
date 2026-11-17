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

Developer: HY4 flash Free (`opencode-go/hy4-flash-free`), acting under the
owner's one-task WEB-13 exception to the AGENTS.md planner model. Codex is
planner/reviewer; no Lark, PR or merge action was taken by the developer.

- Feature branch: `fix/web-13-picker-overflow`, created from the doc-plan base
  `acdf607bafcbbbc1c818b55de614f35d6979d3f2` (which is based on
  `origin/main` `34d1276a2818d1f4a88e9a937eacb8da0b66f7b9`). The prepared
  checkout was clean at the old parent `eb9eaa18e` before branching; the base
  had advanced by a large console build, so the branch was created from the
  doc-plan commit rather than the old parent, as instructed. No parent,
  worktree or main branch was reset or disturbed. No merge was required.

### Commits (one task per commit, in plan order)

1. `79bc70208b2cd3dfb0c746f53f024cf683ba4626` — Task 1, Roles picker
   containment. `src/features/admin/role-table-access.tsx` and
   `src/features/admin/role-form-page.test.tsx`.
2. `441f8b5d1ab461e523cd5afdd8a26e5dbdaa1acb` — Task 2, sibling pickers.
   `src/features/admin/permission-picker.tsx` + test,
   `src/features/connectors/connector-ingest-panel.tsx` + test,
   `src/features/governance/table-picker.tsx` + test,
   `src/features/pipelines/pipeline-pickers.tsx` + test.

No product code or tests were added in the final docs/CHANGELOG commit.

### Source changes, by plan anchor

- `role-table-access.tsx:393` and `:419` — the schema Label and the table row's
  grid/Label now bound `min-w-0`/`flex-1` ancestors and wrap their name in an
  explicit `min-w-0 break-all` span. The `Checkbox` primitive is already
  `shrink-0`. Whole-schema, unlisted-grant, disabled and save semantics are
  untouched; full names stay as DOM text and accessible names.
- `permission-picker.tsx:199` — the unknown-token Badge overrides the base
  `shrink-0`/`whitespace-nowrap`/`h-5` with `max-w-full`, `shrink`,
  `whitespace-normal` and `h-auto` (consumer override via `cn`, no primitive
  change); the token wraps in a `break-all` span and the remove button is
  `shrink-0`. The known-permission grid at `:329` gets `min-w-0 flex-1` and a
  `break-all` span for `perm.name`; descriptions, wildcard coverage and
  disabling are unchanged.
- `connector-ingest-panel.tsx:1154` and `:1216` — the loaded-column and key
  checkbox labels get `min-w-0`, `shrink-0` native checkboxes and `break-all`
  spans. The locked-column reason span is kept; exact excluded/primaryKey
  values are unchanged.
- `table-picker.tsx:156` — the trigger gets `min-w-0` and a `title` with the
  full selected key; `FacetedBadgeList` gets `className="min-w-0 flex-1"` and
  `badgeClassName="max-w-full font-mono [&>span]:min-w-0"` so the selected
  badge truncates within its width. The option span at `:173` wraps with
  `break-all`. Held tables not in the catalog are still offered whole.
- `pipeline-pickers.tsx:240` — same consumer-only badge/trigger bounds for the
  `ColumnsField` multi-select, with the exact comma-list value on the title;
  option span at `:251` wraps with `break-all`.

### Audit disposition

- Ordinary `Select` controls in `pipeline-pickers.tsx` (`ZoneField`,
  `ColumnField`) were audited against `src/components/ui/select.tsx` and left
  unchanged: `SelectValue` already carries `min-w-0 flex-1 overflow-hidden`
  with `line-clamp-1` on the trigger (phase 2 QA R14), `SelectItemText` carries
  `min-w-0 flex-1`, and `SelectContent` is viewport-capped with
  `overflow-x-hidden`. No mismatch found, so no speculative fix was made.
- `src/components/ui/label.tsx`, `badge.tsx` and `faceted.tsx` were read but not
  modified. All containment is consumer-side, as decided.
- `src/features/connectors/table-finder.tsx` was used as the reference
  (`min-w-0`/`flex-1`/`truncate` plus complete-name `title`) and not changed.
- Fixtures use only synthetic long identifiers (no screenshot table/client
  names). No new product dependency, secret, hostname, IP or port was added.
  No Rust, Python, compose or migration files were touched.

### Verification (foreground, this branch's final code)

Baseline on the doc-plan base before any edit, for an honest comparison:

```
bun run typecheck                    -> exit 0
bun run lint                         -> exit 0, 11 warnings, 0 errors
bun run test                         -> 4431 pass, 1 skip, 0 fail,
                                        13567 expect() calls, 4432 tests / 409 files
```

Final, after the two code commits and before handoff:

```
bun run typecheck                    -> exit 0
bun run lint                         -> exit 0, 11 warnings, 0 errors
                                        (same 11 pre-existing warnings; none added)
bun run test                         -> 4444 pass, 1 skip, 0 fail,
                                        13659 expect() calls, 4445 tests / 409 files
                                        (+13 tests, +92 expect() calls vs baseline)
git diff --check                     -> clean
git status --short                   -> clean (nothing generated staged)
```

Scoped checks run per task: the touched suites (`role-form-page.test.tsx`
30 pass; `permission-picker.test.tsx` 12 pass; `table-picker.test.tsx` 9 pass;
`pipeline-pickers.test.tsx` 11 pass; `connector-ingest-panel.test.tsx` 94 pass)
and file-scoped `eslint` on every touched file, plus a full `typecheck`.

No cargo, Python, compose or migration checks were run: this is a TypeScript
and docs-only change. No check was weakened, skipped or rerun to get green.
The `act(...)` console warnings in the test output are pre-existing React test
noise present at baseline; they are not failures.

### Not verified / owed

- **Geometry NOT VERIFIED.** At 320px and 768px the changed text and pickers
  were not measured. No layout engine exists in this checkout: `playwright`,
  `playwright-core`, `puppeteer`, `puppeteer-core`, `jsdom` and
  `@playwright/test` are all absent (only `@happy-dom/global-registrator` is a
  dev dependency, and a probe shows `offsetWidth` and
  `getBoundingClientRect().width` are both `0` in happy-dom). DOM/class
  assertions prove the wrapping structure and full value/behavior, not visual
  geometry. No product dependency was installed to fake a measurement, and no
  `preview_*` tool was used for self-verification. The manual QA checklist
  below remains outstanding; no screenshot success is claimed.
- Lark bug `reczz28adM9F7B5k` (QA owner Rizky) was not touched by the developer,
  as instructed.
- No PR was opened or merged; no branch was pushed to main or force-pushed.

### Blockers

None. No plan/code mismatch was found; no scope expansion was needed.

## Review

Pending independent Codex review under the owner's WEB-13 exception.

### Codex review — 2026-11-17

Reviewed the five consumers and added regression tests against plan commit
`acdf607ba` and developer head `4775dd481`. No changes to grants, selection values,
API clients or shared UI primitives. Exact identifier/save/removal behavior is
covered. No prior tests removed or weakened.

- SHOULD-FIX S1: pipeline selected-chip list allows two chips to wrap, but its
  `size=sm` trigger inherited fixed h-7. At a narrow width a second chip row can
  escape vertically. Corrected the consumer to h-auto/min-h-7/py-1; selection and
  title remain unchanged. Provenance cited at the site. This small review
  correction is authored by Codex; the implementation otherwise is HY4's.
- Geometry remains NOT VERIFIED. DOM tests do not prove CSS dimensions. Manual
  QA at the plan's narrow/wide fixtures is still owed before acceptance.

Final independent verification on the review-corrected code, in foreground:

- `bun run typecheck`: exit 0.
- `bun run lint`: exit 0; 0 errors, 11 warnings, all in unchanged files.
- `bun run test`: 4444 pass, 1 skip, 0 fail; 13654 expects; 4445 tests across
  409 files, 192.95 seconds. This is the reviewer run, not copied from handoff.
- `bun test src/features/pipelines/pipeline-pickers.test.tsx`: 11 pass, 0 fail,
  35 expects (review correction scoped check).
- `git diff --check`: clean. `git status --short` before commit: only the
  pipeline consumer and this review entry; no generated artifacts staged.
- No Rust/Python/compose commands (untouched).

No open code-review BLOCKER. S1 corrected. Browser geometry/manual acceptance
remains not verified; the QA owner has regression case `LH-WEB-13-01` (Not run).
The fix is ready for PR/CI, not claimed accepted on a deployed build.

### Merge record — 2026-11-17

PR #565 merged as `23d111fb00714ae3a582eda7e0454a775bb1856e` after all
applicable CI jobs passed on reviewed head `725e91901cfd81f9e982b9327172fe3d18b75383`,
including Frontend Lint/Typecheck/Test/Build, repository lints, dependency review,
cargo audit/deny and both secret scans. Untouched-language jobs were skipped by
change scope. No review comments or merge conflicts were open.

WEB-13 Task is Done for code delivery. Visual acceptance is still **Not run**
(`LH-WEB-13-01`); the execution ticket remains QA Review for its assigned tester.
This merge is not a claim of a deployed-build visual pass.
