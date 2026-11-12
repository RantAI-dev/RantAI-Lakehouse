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
- Native selects per area (files, selects): governance 6, 11; catalog 5, 5; connectors
  11, 17; dashboards 7, 16; pipelines 2, 6; queries 4, 5. No `<select multiple>` among
  them.
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
- **T3. Governance** (6 files): `quality-check-fields.tsx`, `quality-rule-create-page.tsx`,
  `quality-rule-edit-dialog.tsx`, `quality-schedule.tsx`, `freshness-target-dialog.tsx`,
  and any other file under `src/features/governance` the grep below still names.
- **T4. Catalog** (5 files): `asset-about.tsx`, `asset-grants.tsx`, `asset-quality.tsx`,
  `column-lineage.tsx`, `column-sensitivity.tsx`.

Accept, S1: `grep -rnE '<select\b' src/features/governance src/features/catalog
--include=*.tsx | grep -v '\.test\.'` prints nothing; the tests of every file touched
pass; `bun run typecheck`; `bun run lint`.

### S2. Connectors — branch `feat/web-9-select-s2`, cut from `main` after S1 merges

- **T5.** The 11 files under `src/features/connectors` (17 selects), dial forms included.

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

## 6. Review (planner)
