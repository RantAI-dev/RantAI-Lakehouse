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

### Handoff (developer, 2026-11-12, S2)

Branch `feat/web-9-select-s2`, cut from `origin/feat/web-9-select-s2` at
`273d89e6b` (S1 merged). Only `src/` and this plan changed; no `rust/`,
`dagster/`, `ops/`, migration, `docker-compose.yml` or `design-system/` file,
and no cargo command was run. Nothing under `src/features/dashboards`,
`src/features/pipelines`, `src/features/queries` or `eslint.config.mjs` was
touched (S3 owns those).

**Commits**

| Hash | Subject |
| --- | --- |
| `c776f922a` | `fix(web-9): name the column choice on Add Quality Rule` |
| `326e26798` | `refactor(web-9): replace the dial forms' native selects with SelectField` |
| `d525a6659` | `refactor(web-9): replace the upload step's native selects with SelectField` |
| `4b6da0933` | `refactor(web-9): replace the ingest panel's native selects with SelectField` |

Each commit was pushed to `origin/feat/web-9-select-s2` immediately after it
was made. T5 was split into three commits by group of files, as the task
allows.

**T0 — `WEB-9 S1 review R1-S1` (D2a).** `quality-check-fields.tsx`'s Column
list gets `placeholder="Pick a column"`, with the finding cited at the site.
`quality-rule-create-page.test.tsx` gains a test that the trigger reads
"Pick a column" before a column is chosen, and the chosen column's name after.

**Files converted (`SelectField`), with the number of selects in each — 11
files, 19 selects**

- T5a, dial forms — 7 files, 9 selects: `sftp-dial-form.tsx` (1),
  `files-dial-form.tsx` (1), `sql-dial-form.tsx` (1), `rest-dial-form.tsx`
  (2: auth type, pagination), `saas-dial-form.tsx` (1), `cdc-dial-form.tsx`
  (2: driver, TLS mode), `kafka-dial-form.tsx` (1).
- T5b, upload step — 2 files, 6 selects: `upload-columns.tsx` (3: type, date
  reading, time zone), `upload-check-step.tsx` (3: sheet, encoding,
  delimiter). Both private `SELECT_CLASS` constants are deleted.
- T5c, ingest — 2 files, 4 selects: `connector-form-parts.tsx` (1: the tenant
  picker, `TenantField`), `connector-ingest-panel.tsx` (3: schedule, load
  mode, new-row column). `fieldClass` is deleted.

The compact in-row controls use `size="sm"` plus `px-2 text-xs` (and
`font-mono` where the old class had it) to keep their height and tone.

**Placeholders added under D2a (a value that can be outside the list)**

- `quality-check-fields.tsx`, Column — `"Pick a column"` (R1-S1).
- `sql-dial-form.tsx`, Driver — `"Pick a driver"`: the options are the three
  drivers that form serves, and a stored dial can name another (`oracle` has
  its own form).
- `saas-dial-form.tsx`, How it signs in — `"Pick how it signs in"`: a stored
  dial can say `integration` where the vendor lists no integration option.
- `connector-form-parts.tsx`, Tenant — `"Pick a tenant"`: a stored tenant the
  person's own list lacks.

**Prompts turned into placeholders under D2 (not real choices)**

- `upload-check-step.tsx`, Encoding and Delimiter —
  `"Detected by the service"` (was a disabled option with value `""`).
- `upload-columns.tsx`, date reading — `"Day first or month first?"` (was a
  disabled prompt option).
- `connector-ingest-panel.tsx`, new-row column — `"Pick a column…"` (was a
  prompt option with value `""`).

Real choices keep their option and value `""`: "Unassigned" (tenant), "Manual
only" (schedule), and the existing "None" options.

**Commands run, with real counts (all in the foreground)**

- `bun install --frozen-lockfile` once (this checkout had no `node_modules`):
  678 packages installed.
- T0: `bun test src/features/governance/quality-rule-create-page.test.tsx` →
  9 pass, 0 fail, 25 `expect()` calls.
- T5a: `bun test src/features/connectors/connector-create-page.test.tsx
  src/features/connectors/connector-edit-page.test.tsx
  src/features/connectors/dial-forms
  src/features/connectors/saas-connectors.test.tsx` → 79 pass, 0 fail, 347
  `expect()` calls, 5 files.
- T5b: `bun test src/features/connectors/upload-file-page.test.tsx` → 88 pass,
  0 fail, 1 file.
- T5c: `bun test src/features/connectors/connector-ingest-panel.test.tsx` → 89
  pass, 0 fail; then `bun test src/features/connectors` → 448 pass, 0 fail,
  23 files.
- `bun run typecheck` before every commit → no errors.
- `bun run lint` before every commit → 0 errors, 11 warnings; the same 11
  warnings exist on the base and none is in an S2 file.
- Final commit, once: `bun run typecheck` → no errors; `bun run lint` → 0
  errors, 11 warnings; `bun run test` → first run 3992 pass, 1 skip, **1
  fail**, 11975 `expect()` calls, 380 files, 149.35s; the failing test's name
  was not captured (the console output was truncated before it), so I cannot
  say which test it was. Two further full runs were green: the second printed
  no failing-test line (its counts were not captured), the third 3993 pass, 1
  skip, 0 fail, 11975 `expect()` calls, 380 files, 150.07s. The single
  failure did not recur; the known flaky `AssistantSettingsForm (ADM-9)`
  (`REL-24`) was not confirmed as its cause.
- `python3 ops/lint/check_bare_iceberg_count.py` → "R11 guard: no bare
  count() against a Bronze Iceberg table found."
- `python3 ops/lint/check_intra_package_imports.py` → "OK: all intra-package
  dispar_orchestrate imports resolve".
- S2 acceptance: `grep -rnE '<select\b' src/features/connectors
  --include=*.tsx | grep -v '\.test\.'` prints nothing (exit 1).
- `git status` before each commit: only the intended files; nothing generated
  was staged.

**Tests that had to change, and why**

No test was deleted. Seven test files changed, all mechanism changes under
D5 unless noted:

- `src/features/governance/quality-rule-create-page.test.tsx` — one new test
  for T0; no existing assertion changed.
- `src/features/connectors/dial-forms/dial-forms.test.tsx` — driver and TLS
  mode values read as the trigger's shown label; the TLS and auth-type option
  lists read from the opened list; two `fireEvent.change` steps became
  `chooseOption`.
- `src/features/connectors/dial-forms/tier2-dial-forms.test.tsx` — Kafka and
  SFTP option lists read from the opened list, which is portalled onto
  `document.body`, outside the container that file scopes its queries to.
- `src/features/connectors/saas-connectors.test.tsx` — the sign-in steps call
  `chooseOption`, and the "integration credential only where the vendor has
  one" test opens the list before asking what it offers (a closed combobox
  has no options, so the old query would have passed for the wrong reason).
- `src/features/connectors/connector-create-page.test.tsx` and
  `connector-edit-page.test.tsx` — one auth-type step each became
  `chooseOption`.
- `src/features/connectors/upload-file-page.test.tsx` — delimiter, sheet,
  column type and date-reading values read as the trigger's label; option
  lists read from the opened list (the hidden sheet's "Notes (hidden)" label
  among them); six `fireEvent.change` steps became `chooseOption`.
- `src/features/connectors/connector-ingest-panel.test.tsx` — schedule, load
  mode and the column list read as labels / opened lists; the mode and
  schedule steps call `chooseOption`. Two things are worth naming:
  - Three new-row-column steps still use `fireEvent.change`: when the source
    cannot list a table's columns the cursor is a text `Input`, not a
    `SelectField`, and those tests exercise that path.
  - "stops its inputs stretching across the page" now looks for the page's
    own `max-w-xs` on the Schedule trigger instead of the absence of any
    `max-w-`, because the design system's trigger always carries
    `max-w-full`. The fact asserted (the page layout caps it, the embedded
    one does not) is the same.

**Skipped / not verified**

- No browser check. I cannot see the page in a browser here, so how the new
  dropdowns look in either theme, and whether the compact `size="sm"`
  triggers keep the old row height visually, are *not verified* beyond the
  DOM-level tests.
- D6 (a light dashboard's popup) does not apply to S2: no dashboard site is
  in this slice.
- `TenantField` has no caller in this tree (it is exported but unused); it was
  converted like any other site, and its behaviour is not covered by a test
  either before or after this slice.
- No list is long enough to raise the "too long to scan" finding: the longest
  converted list is the tenant list.
- No Rust/Python/compose change, so none of those checks were run.

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
