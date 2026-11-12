# WEB-9: one dropdown in the console, readable in the dark theme

Backlog `WEB-9` (Task, Platform, P1). Base: `main` at `3cdd8332` (anchors verified there).
Console only. Three PR slices.

Planner and reviewer: Claude Opus. Developer: another agent.

Why (owner, 2026-11-12, with a screenshot of Add Quality Rule): in the dark theme the list
of a dropdown opens white with near-white text, so only the highlighted option can be
read, and the control looks like the browser's default. The cause is 61 native `<select>`
elements in 34 files (`grep -rnE '<select\b' src --include=*.tsx`, tests excluded), each
with its own class string, while 45 other files already use the design system's `Select`
(`src/components/ui/select.tsx`, Base UI). The page never sets `color-scheme`, so the
browser draws a native list in its light colours and the options inherit the dark theme's
light text.

## 1. Decisions

Planner; the owner asked for the fix, not for the shape of it.

- **D1. The browser is told the theme.** `src/app/globals.css` sets `color-scheme: dark`
  under `.dark` and `color-scheme: light` under `.theme-light` (a dashboard fixed to light
  inside a dark console, `BI-17` D6), with a comment saying why. This alone makes every
  remaining native control readable (a native list, a date input, a scrollbar), and it
  lands in the first slice so the unreadable list is gone before the replacement is
  complete. `design-system/` is the shared system and is not edited.
- **D2. One component: `SelectField`**, in `src/components/patterns/select-field.tsx`,
  wrapping the design system's `Select`, `SelectTrigger`, `SelectValue`, `SelectContent`,
  `SelectGroup`, `SelectLabel` and `SelectItem`. Sites pass data, not markup:

  ```ts
  type SelectOption = { value: string; label: string; disabled?: boolean }
  type SelectOptionGroup = { label: string; options: SelectOption[] }
  type SelectFieldProps = {
    id?: string
    value: string
    onValueChange: (value: string) => void
    options: readonly (SelectOption | SelectOptionGroup)[]
    placeholder?: string        // shown while `value` matches no option
    disabled?: boolean
    className?: string          // on the trigger; a site keeps its width (`w-full`)
    size?: "sm" | "default"
    "aria-label"?: string
    "aria-invalid"?: boolean
  }
  ```

  - The trigger shows the option's **label**, never its raw value.
  - `id` goes on the trigger, so an existing `<Label htmlFor>` keeps naming it.
  - A value that matches no option shows the placeholder. A site that today renders an
    extra option for a stored value the list lacks (`WEB-8` D8, for example
    `quality-check-fields.tsx` "(not in this table)") keeps doing so by adding that option
    to `options`.
  - An empty-string option that is only a prompt (`Pick a column`, `Choose a chart…`,
    `Pick a table`) becomes `placeholder`. One that is a real choice (`None`, `All`,
    `No connector (read a ClickHouse table directly)`, `Unassigned`) stays an option whose
    value is `""`. If Base UI cannot hold `""` as an item value, `SelectField` maps it to a
    private sentinel inside the component; no site sees the sentinel.
- **D2a. A value that can be outside the list gets a placeholder (planner, 2026-11-12,
  from S1 review R1-S1).** A native select with a value none of its options had drew the
  first option as if it were chosen; `SelectField` draws an empty trigger instead. That is
  the honest reading, but an empty box says nothing. Every site whose value can be `""`
  or otherwise outside `options` passes a `placeholder` that says what to pick.
- **D3. Every native `<select>` becomes `SelectField`.** No site keeps its own class
  string (`selectClassName` constants are deleted). Behaviour does not change: the same
  options, the same order, the same disabled states, the same `onChange` effect.
  `<optgroup>` (the two saved-query forms) becomes a `SelectOptionGroup`.
- **D4. A lint rule keeps it that way.** `eslint.config.mjs` gets a `no-restricted-syntax`
  entry refusing a JSX `<select>` under `src/`, with the message "Use SelectField
  (`@/components/patterns/select-field`); a native select is unreadable in the dark theme
  (`WEB-9`)". It lands in the last slice, when no native select is left. An exception
  needs a comment at the site saying why (`AGENTS.md`, choke points).
- **D5. Tests drive the new control the way a person does.** One helper,
  `src/components/patterns/select-field.testing.ts`, exports
  `chooseOption(trigger: HTMLElement, optionName: string | RegExp)`: it opens the trigger
  and picks the option with the pointer events the four existing `pickOption` copies use
  (`pipeline-create-page.test.tsx:189`). A test that set a native select with
  `fireEvent.change` is rewritten to call it; what the test asserts does not change. A
  test is never deleted or loosened to get green (`AGENTS.md` rule 2).
- **D6. Popups inside a light dashboard.** The list is rendered in a portal on `<body>`,
  outside a `.theme-light` subtree. For the two sites that can sit in one
  (`dashboards/embed-filter-bar.tsx`, `dashboards/filters/filter-editors.tsx`) the
  developer checks what the `Select`s already on that surface do and does the same; if the
  list comes out dark inside a light dashboard, that is written in the handoff as a
  finding, not fixed by inventing a second theme mechanism.

## 2. Anchors (verified at `3cdd8332`)

- `src/components/ui/select.tsx`: the design system's `Select` parts (Base UI
  `@base-ui/react/select`).
- `src/app/globals.css:16`: `@custom-variant dark (&:is(.dark *):not(:is(.theme-light *)))`;
  `design-system/tokens/colors.css:16` (`:root, .theme-light`) and `:81` (`.dark`). No
  `color-scheme` is set anywhere but `src/features/landing/landing.css:25`.
- The screenshot's page: `src/features/governance/quality-rule-create-page.tsx:19`
  (`selectClassName`) and `quality-check-fields.tsx:72` (the Check list).
- Native selects per area (files, selects): governance 5, 10; catalog 5, 5; connectors
  11, 19; dashboards 7, 16; pipelines 2, 6; queries 4, 5: 34 files, 61 selects. No
  `<select multiple>` among them. (Corrected 2026-11-12 from the S1 handoff: the plan
  first said governance 6, 11 and connectors 11, 17, the planner's miscount.)
- Empty-string options: `connector-form-parts.tsx:585`, `upload-columns.tsx:110`,
  `connector-ingest-panel.tsx:1055`, `pipeline-detail-page.tsx:435`,
  `embed-filter-bar.tsx:81`, `embed-settings-panel.tsx:122`,
  `pipeline-create-page.tsx:525` and `:573`, `asset-about.tsx:175`,
  `column-lineage.tsx:163`.
- `<optgroup>`: `queries/save-query-page.tsx:233`, `queries/saved-query-move-dialog.tsx:146`.
- Existing test helper copies: `pickOption` in `pipeline-create-page.test.tsx:189`,
  `pipeline-pickers.test.tsx:16`, `uploads-panel.test.tsx:23`, `upload-file-page.test.tsx:51`.

## 3. Slices and tasks (one commit each, in order; push after every commit)

### S1. The fix and the first two areas — branch `feat/web-9-select-s1`

- **T1. `color-scheme` (D1).** `src/app/globals.css` only.
- **T2. `SelectField` and its test helper (D2, D5).** New
  `select-field.tsx`, `select-field.testing.ts`, `select-field.test.tsx`. Tests, named as
  full sentences: the trigger shows the chosen option's label, not its value; choosing an
  option calls `onValueChange` with its value; a value no option has shows the
  placeholder; a disabled option cannot be chosen; an option whose value is the empty
  string can be chosen and shows its label; a group shows its label and its options; a
  `<Label htmlFor>` names the trigger.
- **T3. Governance** (5 files): `quality-check-fields.tsx`, `quality-rule-create-page.tsx`,
  `quality-rule-edit-dialog.tsx`, `quality-schedule.tsx`, `freshness-target-dialog.tsx`,
  and any other file under `src/features/governance` the grep below still names.
- **T4. Catalog** (5 files): `asset-about.tsx`, `asset-grants.tsx`, `asset-quality.tsx`,
  `column-lineage.tsx`, `column-sensitivity.tsx`.

Accept, S1: `grep -rnE '<select\b' src/features/governance src/features/catalog
--include=*.tsx | grep -v '\.test\.'` prints nothing; the tests of every file touched
pass; `bun run typecheck`; `bun run lint`.

### S2. Connectors — branch `feat/web-9-select-s2`, cut from `main` after S1 merges

- **T5.** The 11 files under `src/features/connectors` (19 selects), dial forms included.

Accept: the same grep on `src/features/connectors` prints nothing; tests, typecheck, lint.

### S3. Dashboards, pipelines, queries, and the lint — branch `feat/web-9-select-s3`, cut from `main` after S1 merges

- **T6. Dashboards** (7 files, 16 selects), with D6's check.
- **T7. Pipelines and queries** (6 files, 11 selects), `<optgroup>` as groups.
- **T8. The lint rule (D4).** It merges after S2, so `main` has no native select left when
  it lands; the PR is brought up to date with `main` first and `bun run lint` is run again.

Accept: `grep -rnE '<select\b' src --include=*.tsx | grep -v '\.test\.'` prints nothing
(after S2 is in the branch); tests, typecheck, lint.

### Every slice, once before the handoff, in the foreground

`bun run typecheck && bun run lint && bun run test`, then
`python3 ops/lint/check_bare_iceberg_count.py && python3 ops/lint/check_intra_package_imports.py`.
No Rust changes, so no cargo command. CI runs the rest on the draft PR.

## 4. Out of scope

- Any change to what a dropdown offers or does. This plan changes the control, not the
  choices.
- The design system's own files (`design-system/`), and the look of `Select` itself.
- Other native controls (date and time inputs, checkboxes, file inputs). D1 makes them
  follow the theme; restyling them is not part of this.
- Merging the four `pickOption` copies in existing tests into the new helper.
- Combobox or search inside a long list. A site whose list is too long to scan is written
  in the handoff as a finding.

## 5. Handoff (developer appends below, one entry per slice)

### Handoff (developer, 2026-11-12, S1)

Branch `feat/web-9-select-s1`, off `feat/web-9-select-s1` at `be14f8cc6` (the plan commit).
Only `src/` and this plan changed; no `rust/`, `dagster/`, `ops/`, migration,
`docker-compose.yml` or `design-system/` file, and no cargo command was run.

**Commits**

| Hash | Subject |
| --- | --- |
| `1d1a2e447` | `fix(web-9): set color-scheme so native controls follow the theme` |
| `c55a0ebcc` | `feat(web-9): add SelectField, the console's one dropdown` |
| `599ea5d29` | `refactor(web-9): replace governance's native selects with SelectField` |
| `c0a4995f5` | `refactor(web-9): replace catalog's native selects with SelectField` |

Each commit was pushed to `origin/feat/web-9-select-s1` immediately after it was made.

**Files converted (`SelectField`), with the number of selects in each**

- T3, `src/features/governance` — 5 files, 10 selects:
  `quality-check-fields.tsx` (5), `quality-schedule.tsx` (2),
  `quality-rule-create-page.tsx` (1), `quality-rule-edit-dialog.tsx` (1),
  `freshness-target-dialog.tsx` (1). `selectClassName` deleted in every file
  that had one.
- T4, `src/features/catalog` — 5 files, 5 selects: `asset-about.tsx`,
  `asset-grants.tsx`, `asset-quality.tsx`, `column-lineage.tsx`,
  `column-sensitivity.tsx`.

Plan/code mismatch, reported not improvised: section 2 says governance has 6
files and 11 selects, but at this branch's base the acceptance grep named
5 files and 10 selects. T3 converted exactly what the grep names, as the task
text allows. No file under either area has a native `<select>` left.

Per D2: the empty option that was only a prompt became `placeholder` at
`column-lineage.tsx` ("Pick a column"); `asset-about.tsx`'s "None" is a real
choice and stays an option with `value: ""`, held behind `SelectField`'s
private sentinel (Base UI treats `""` as nothing selected). `quality-schedule`
keeps its minute values as strings at the boundary and converts back in
`onValueChange`; no stored value changed.

**Commands run, with real counts (all in the foreground)**

- `bun install --frozen-lockfile` once (this checkout had no `node_modules`):
  678 packages installed.
- T2: `bun test src/components/patterns/select-field.test.tsx` → 7 pass, 0 fail.
  The seven tests are the ones T2 lists; names are full sentences.
- T3: `bun test src/features/governance` → 78 pass, 0 fail, across 13 files.
- T4: `bun test src/features/catalog` → 369 pass, 0 fail, across 14 files.
- `bun run typecheck` before every commit → no errors.
- `bun run lint` before every commit → 0 errors, 11 warnings; the same 11
  warnings (unused vars, hook deps, `window.location.assign`) exist on the
  base and none is in an S1 file.
- Final commit, once: `bun run typecheck && bun run lint && bun run test` →
  typecheck no errors; lint 0 errors, 11 warnings; test 3981 pass, 1 skip,
  0 fail, 11945 `expect()` calls, 380 files, 140.47s. The known flaky
  `AssistantSettingsForm (ADM-9)` test (`REL-24`) did not fail, so the suite
  was run once.
- `python3 ops/lint/check_bare_iceberg_count.py` → "R11 guard: no bare
  count() against a Bronze Iceberg table found."
- `python3 ops/lint/check_intra_package_imports.py` → "OK: all intra-package
  dispar_orchestrate imports resolve".
- S1 acceptance:
  `grep -rnE '<select\b' src/features/governance src/features/catalog --include=*.tsx | grep -v '\.test\.'`
  prints nothing (exit 1).
- `git status` before each commit: only the intended files; nothing generated
  was staged.

**Tests that had to change, and why**

No test was deleted or loosened. Four files were rewritten where they set a
native select with `fireEvent.change`, now calling `chooseOption`:

- `src/features/governance/quality-rule-create-page.test.tsx` — the two
  WEB-8 column tests asserted `tagName === "SELECT"`, `.value` and
  `select.options`. The control is a button now, so they assert the same
  facts through the combobox: it is a combobox (not the free-text `INPUT`),
  its trigger shows the kept value's label (including "(not in this table)"),
  and the open list holds the same options in the same order.
- `src/features/catalog/asset-detail-tabs.test.tsx` — the lineage test also
  asserted "Pick a column" as a `<select>` option; D2 makes it the
  placeholder, so it asserts the placeholder in the trigger and the two
  column options in the open list. The other changed steps only swap
  `fireEvent.change` for `chooseOption`; where the old test read
  `select.value` it now reads the trigger's label ("days", "High"), which is
  what a person sees.
- `src/features/catalog/column-lineage.test.tsx` — same mechanism change and
  the same "Pick a column" placeholder change as above.
- `src/features/governance/data-quality-page.test.tsx` — mechanism only; the
  Unit assertion reads the trigger label.

**Skipped / not verified**

- No browser check. I cannot see the page in a browser here, so D1's
  `color-scheme` on a native control, and the look of the new popup in the
  dark theme, are *not verified* beyond the DOM-level tests above.
- D6 (a light dashboard's popup) belongs to S3's dashboard sites and was not
  exercised in this slice.
- No Rust/Python/compose change, so none of those checks were run.
- No visual finding about list length to report: the converted lists are the
  same lengths they were (longest is the quality check list, unchanged).

### Handoff (developer, 2026-11-12, S3)

Branch `feat/web-9-select-s3`, cut from `origin/feat/web-9-select-s3` at `273d89e6b`
(S1 merged). Only `src/` and this plan changed; no `rust/`, `dagster/`, `ops/`,
migration, `docker-compose.yml` or `design-system/` file, no cargo command, and
nothing under `src/features/connectors` or `src/features/governance` (S2's files).

**Commits**

| Hash | Subject |
| --- | --- |
| `a53a09756` | `refactor(web-9): replace the subscription form's native selects with SelectField` |
| `c767c7d7c` | `refactor(web-9): replace the rest of the dashboards' native selects with SelectField` |
| `38df542c6` | `refactor(web-9): replace the pipelines' and queries' native selects with SelectField` |
| `c6b56ddfa` | `chore(web-9): refuse a new native select in the console's lint` |

Each was pushed to `origin/feat/web-9-select-s3` immediately after it was made.

**Files converted (`SelectField`), with the number of selects in each**

- T6, `src/features/dashboards` — 7 files, 16 selects, split over two commits:
  - `subscriptions-dialog.tsx` (7): repeat, weekday, day of the month, paper,
    orientation, one file per chart, values.
  - `chart-alert-dialog.tsx` (2), `embed-settings-panel.tsx` (3),
    `embed-filter-bar.tsx` (1), `filters/filter-editors.tsx` (1 native `<select>`
    in the shared `PlainSelect`, used at 6 call sites; `PlainSelect` deleted),
    `people-panel.tsx` (1), `public-link-panel.tsx` (1).
- T7, `src/features/pipelines` and `src/features/queries` — 6 files, 11 selects:
  `pipeline-create-page.tsx` (5), `pipeline-detail-page.tsx` (1),
  `query-column-statistics.tsx` (1), `saved-query-access-panel.tsx` (2),
  `save-query-page.tsx` (1, `<optgroup>` → `SelectOptionGroup`),
  `saved-query-move-dialog.tsx` (1, `<optgroup>` → `SelectOptionGroup`).

Every private class string is deleted; a site keeps only its width
(`w-full`, `w-fit`, `min-w-52`, `w-20`) and, where the old control was the `h-7`
size, passes `size="sm"`. The two folder pickers' groups keep the same order and
the same labels (`Personal space (only you)` first, `Shared` second).

**D2 and D2a: every placeholder at these sites**

D2, an empty option that was only a prompt became the placeholder:

- `embed-settings-panel.tsx`: `Choose a chart…` (was the empty option).
- `pipeline-create-page.tsx`: `Choose one of the connector's tables` (was the
  empty option; `aria-label` still says which connector).

D2, an empty option that is a real choice stays an option with `value: ""`:

- `pipeline-create-page.tsx`: `No connector (read a ClickHouse table directly)`.
- `pipeline-detail-page.tsx`: the reads-as empty choice, whose label is still
  "Nobody — this pipeline cannot run" or "Choose a user…" by state.
- `saved-query-move-dialog.tsx`: `Shared` (the server reads it as `null`).
- `embed-filter-bar.tsx`: `All` (it clears the filter).

D2a, a value that can be outside the options gets a placeholder:

- `chart-alert-dialog.tsx`, Series: `Pick a series`. The state starts at
  `seriesChoices[0] ?? ""`, so a chart with no measure started as an empty box.
- `people-panel.tsx`, Level: `Pick a level`. A row's level can be above the
  viewer's own, so `levelAtLeast` drops it from the offered list and the value
  matched no option.
- `embed-settings-panel.tsx`, Mode: `Pick a mode`. A hidden-only column offers
  only `Hidden`, so a saved mode of another kind matched no option.
- `pipeline-create-page.tsx`, Source table: the D2 placeholder also covers a
  stored table the connector no longer lists.
- Sites whose value is always one of the options pass none: the subscription
  form, the alert condition, the expiry choice, the access levels (their lists
  append the row's current level), the column statistics column (it falls back
  to the first column), both folder pickers, and the embed filter bar (a current
  value outside the list is added as an option, as before).

One difference of markup, reported not hidden: the embed filter mode's hint was
a `title` on the `<select>`; `SelectField` takes data, not markup, so the
`title` sits on a wrapper `<span>` around it (an ancestor's title still shows on
hovering the trigger).

**D6: a popup inside a light dashboard**

Read `src/lib/dashboard-look.ts` (a dashboard fixed to light gets a
`.theme-light` wrapper), `src/components/ui/select.tsx` (the popup is portalled
to `<body>`, outside that wrapper, and reads `bg-popover` /
`text-popover-foreground` from whatever tokens its ancestors set), and the
`Select`s already on that surface (`grain-switch.tsx`, `param-controls.tsx`,
`chart-builder.tsx` and the rest of `src/features/dashboards`, 24 files use it).
They do exactly what `SelectField` now does: no theme class of their own, so a
list opened from a `.theme-light` dashboard comes out in the console's tokens,
not the light ones. Finding, per D6: this is unchanged behaviour of the shared
`Select`, not something these two sites introduced. It is **not** verified in a
browser; no second theme mechanism was invented.

**T8: the lint rule (D4)**

`eslint.config.mjs` had no `no-restricted-syntax` entry, so one block was added
(not replaced) with `selector: "JSXOpeningElement[name.name='select']"`, the
message D4 names, and `files: ["src/**/*.{ts,tsx}"]`. On this commit
`bun run lint` reports **19 errors, all `no-restricted-syntax`, in 11 files,
every one under `src/features/connectors`** (the S2 branch's native selects:
`connector-form-parts.tsx` 1, `connector-ingest-panel.tsx` 3,
`dial-forms/cdc-dial-form.tsx` 2, `files-dial-form.tsx` 1,
`kafka-dial-form.tsx` 1, `rest-dial-form.tsx` 2, `saas-dial-form.tsx` 1,
`sftp-dial-form.tsx` 1, `sql-dial-form.tsx` 1, `upload-check-step.tsx` 3,
`upload-columns.tsx` 3). No exception was added for those files and the rule was
not weakened. Once S2 merges and the branch is brought up to date, lint should be
back to 0 errors and the base's 11 warnings; the planner runs it.

**Commands run, with real counts (all in the foreground)**

- `bun install --frozen-lockfile` once (this checkout had no `node_modules`):
  678 packages installed.
- T6 commit 1: `bun test src/features/dashboards/subscriptions-dialog.test.tsx`
  → 11 pass, 0 fail. `bun run typecheck` → no errors. `bun run lint` → 0 errors,
  11 warnings (the same 11 as the base).
- T6 commit 2: `bun test src/features/dashboards` → 629 pass, 0 fail, 79 files.
  `bun run typecheck` → no errors; `bun run lint` → 0 errors, 11 warnings.
- T7: `bun test src/features/pipelines src/features/queries` → 499 pass, 0 fail,
  40 files. `bun run typecheck` → no errors; `bun run lint` → 0 errors,
  11 warnings.
- T8: `bun run lint` → 19 errors (all the rule, all in `src/features/connectors`),
  11 warnings.
- Final commit, once:
  - `bun run typecheck` → no errors.
  - `bun run test` → 3992 pass, 1 skip, 0 fail, 11970 `expect()` calls, 380 files,
    151.15s. The known flaky `AssistantSettingsForm (ADM-9)` test (`REL-24`) did
    not fail, so the suite was run once.
  - `python3 ops/lint/check_bare_iceberg_count.py` → "R11 guard: no bare count()
    against a Bronze Iceberg table found."
  - `python3 ops/lint/check_intra_package_imports.py` → "OK: all intra-package
    dispar_orchestrate imports resolve".
  - `bun run lint` → 19 errors, 11 warnings (the T8 result above).
- S3 acceptance grep, after T8:
  `grep -rnE '<select\b' src --include=*.tsx | grep -v '\.test\.' | grep -v 'src/features/connectors'`
  prints nothing (exit 1). The 19 left in the repo are exactly S2's.
- `git status` before each commit: only the intended files; nothing generated
  was staged.

**Tests that had to change, and why**

No test was deleted or loosened. Where a test set a native select with
`fireEvent.change` or read `HTMLSelectElement.value` / `.options`, it now calls
`chooseOption` and reads the trigger's shown label or the open list, as S1 did:

- `dashboards/subscriptions-dialog.test.tsx`, `chart-alert-dialog.test.tsx`,
  `embed-settings-panel.test.tsx`, `people-panel.test.tsx`,
  `public-link-panel.test.tsx`, `filters/filter-editors.test.tsx`,
  `embed-view.test.tsx` — mechanism only; an option list is read after opening
  the trigger, the trigger's label replaces `.value`.
- `dashboards/embed-settings-panel.test.tsx` additionally reads two controls'
  option lists in two renders: a bare trigger click opens a list and does not
  close it, so the second control's list would otherwise sit beside the first.
- `pipelines/pipeline-create-page.test.tsx` — the connector and source-table
  lists are read after opening (their options are no longer always in the DOM),
  and the source-table prompt moved to the placeholder, so the list is asserted
  to hold the two tables only.
- `queries/save-query-page.test.tsx` and `queries/saved-queries-page.test.tsx` —
  the folder picker waits for `/folders` to be read (`act(async () => …)`) before
  opening the list; the labels are the ones the tree already showed.
- `queries/saved-query-move-dialog.test.tsx`, `query-results-section.test.tsx`,
  `saved-query-access-panel.test.tsx` — mechanism only, the same facts.

**Skipped / not verified**

- No browser check: I cannot see the page in a browser here, so neither the new
  popup in the dark theme nor D6's light-dashboard popup is verified beyond the
  DOM-level tests. The owner's own look at a dashboard is still the real proof.
- T8's clean lint is not verifiable on this branch by design (S2's files are not
  here); the planner runs it after bringing the branch up to date with `main`.
- No Rust/Python/compose change, so none of those checks were run.
- No finding about list length: every converted list has the same length it had
  (the longest are the connector's tables and the month/day lists, unchanged).

## 6. Review (planner)

### S1, round 1 (planner, 2026-11-12) — branch at `a192fe75`, pull request #472

No `BLOCKER`. One `SHOULD-FIX`, carried into S2.

Read: `select-field.tsx`, `select-field.testing.ts`, `globals.css`, every converted site
(5 governance files, 5 catalog files), and the four changed test files.

- **D1.** `.dark { color-scheme: dark }` and `.theme-light { color-scheme: light }` in
  `src/app/globals.css`, with the reason. `design-system/` is untouched.
- **D2.** `SelectField` passes `items` so the trigger shows the label, holds `""` as a
  private sentinel that no site sees, shows the placeholder for a value no option has, and
  renders groups. Its seven tests are the seven the plan lists.
- **D3.** The ten governance selects and five catalog selects keep their options, order
  and effect; every `selectClassName` in the two areas is gone. `None` on a governed tag
  stays a real choice; `Pick a column` became the placeholder.
- **D5.** The changed tests assert what they asserted before, through the trigger's shown
  label and the opened list instead of `HTMLSelectElement.value` and `.options`. None was
  removed.
- **The handoff's mismatch** is the planner's miscount, corrected in section 2 by name.

**R1-S1 `SHOULD-FIX`.** `quality-check-fields.tsx`, the Column list: on Add Quality Rule
the column starts as `""`, so the trigger is an empty box. Before, the native select drew
the first column as if chosen while the form still held `""`, which was worse, but an
empty box does not say what to do. Give it `placeholder="Pick a column"`. Decision D2a
above makes this a rule for S2 and S3; the fix itself is the first task of S2.

Re-run by the planner at `a192fe75`, in the foreground:

| Command | Result |
| --- | --- |
| `bun test src/components/patterns/select-field.test.tsx src/features/governance src/features/catalog` | 454 pass, 0 fail, 28 files |
| `bun run typecheck` | clean |
| `bun run lint` | 0 errors, 11 warnings (the same 11 as `main`) |
| S1's acceptance grep | prints nothing |

`bun run test` (whole suite) was not re-run by the planner: the developer's run is in the
handoff (3981 pass, 1 skip, 0 fail) and CI runs it on #472.

Not verified by anyone: how the control looks in a browser, in either theme. The owner
looks at Add Quality Rule after the merge.
