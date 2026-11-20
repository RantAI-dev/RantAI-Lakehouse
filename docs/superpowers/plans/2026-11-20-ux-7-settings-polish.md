# `UX-7`: no browser-drawn lists, and help text that fits one line

Planner and reviewer: Claude Opus. Developer: Claude Sonnet (a separate
session). Base: `main` at `08db10f7c`. Branch: `fix/ux-7-settings-polish`.

## Why

Owner QA, 2026-11-20, on Settings → Reporting:

1. "Report time zone" opens the **browser's own list**: a `<datalist>` under
   an `Input` (`reporting-settings-page.tsx:200-217`), a long unstyled black
   list of 400-odd names, not the console's dropdown. The `WEB-9` lint rule
   refuses a native `<select>` but not a `<datalist>`, and four more slipped
   in: `connectors/table-finder.tsx:403`, `dashboards/format-section.tsx:536`,
   `governance/lineage-page.tsx:106`, `pipelines/pipeline-editor-panes.tsx:286`.
   The subscriptions dialog already replaced the same field with
   `TimeZoneSelect` (owner QA 2026-11-16: "the free-text field with a
   `datalist` read as a typing exercise").
2. "Numbers" (a section heading) sits directly on "Number locale" (the
   field's label), two lines saying the same thing.
3. Help paragraphs run up to seven lines (the downloads one is 422
   characters). The owner's rule (`DESIGN.md` §2): help and status text in
   the app is **one short line**; details go in a tooltip.

## Decisions (planner, 2026-11-20)

1. **No browser-drawn list anywhere in `src/`.** Every `<datalist>` becomes
   the console's `Combobox` (`@/components/ui/combobox`). A field that took
   free text keeps taking free text (a typed value that is not in the list
   is still accepted, as today); a field that only took list values keeps
   that. The `WEB-9` lint block in `eslint.config.mjs` gains a second
   selector in the **same** `no-restricted-syntax` array:
   `JSXOpeningElement[name.name='datalist']`, message naming
   `Combobox` (`@/components/ui/combobox`) and `UX-7`. Update the block's
   comment to say both.
2. **Report time zone uses `TimeZoneSelect`.** Move
   `src/features/dashboards/time-zone-select.tsx` to
   `src/components/patterns/time-zone-select.tsx` (it is now used by two
   features), update the subscriptions dialog's import, and add a
   `disabled?: boolean` prop (the Reporting form is read-only for a
   non-admin). Keep the `zoneList` test seam of `ReportingSettingsForm`
   working (tests pass a short list): give `TimeZoneSelect` an optional
   `zones?: (held: string) => string[]` defaulting to `listTimeZones`.
3. **One label per field.** Remove the "Numbers" `h2`; the field's label
   becomes "Number format". A section heading stays only where it groups
   more than one control (palette, fonts, sites, domains, downloads,
   public links).
4. **Help is one line; details are in a help tip.** Add one shared
   component, `src/components/patterns/help-tip.tsx`: a 20 px ghost icon
   button (`CircleHelp`, `size-3.5`, muted, `aria-label="More about
   <heading>"`) placed right after a section heading, that opens a small
   `Popover` with the detail text, on hover/focus on desktop and on tap on a
   phone (Base UI `Popover.Trigger openOnHover` plus click). Max width
   ~280 px, `text-xs`, the app's popover surface. Use it only where the
   table below gives a "tip"; no other changes of meaning.
5. Every help line below must fit on **one line** at the Settings width
   (`max-w-sm`, 384 px, `text-xs`): at most ~55 characters. Copy is final;
   use it as written.

### Copy (final)

`reporting-settings-page.tsx`

| Where | Line under the heading | Tip |
|---|---|---|
| Report time zone | *(none; the label says it)* | – |
| First day of the week | *(none)* | – |
| Number format (was Numbers / Number locale) | Separators in every chart. | A chart that set its own separators keeps them. |
| Company palette | {MIN}–{MAX} colours. New charts use them. | – |
| Fonts | Fonts a dashboard may use. | – |
| Allowed content sites | Sites image and web-page cards may show. | Written https://host or https://host:port, no wildcards. At most {MAX_CONTENT_SITES}. With none listed, no image or web-page card can be saved. |
| Outside email domains (was "Reports: outside email domains") | Outside domains a scheduled report may reach. | Written example.com, no wildcards. At most {MAX_REPORT_DOMAINS}. With none listed, no report leaves the company. |
| Block downloads for these roles | Checked roles cannot export or download. | Covers PDF, Excel, CSV, image and dashboard-file downloads, Query Studio result downloads and Copy as TSV. It cannot stop copying from the screen. A person who also holds an unchecked role still can, and Platform Admin is never blocked. |
| Public links | Off stops every public link at once. | Nothing is deleted: turning it on again brings back the links that have not expired. |

Other Settings pages (same rule, found by the planner):

| File:line (main `08db10f7c`) | Line | Tip |
|---|---|---|
| `query-limits-settings.tsx:145` | Roles with their own time limit. The longest wins. | – |
| `query-limits-settings.tsx:203` | A dashboard may wait this long to answer. | With 30 minutes, a dashboard may take 30 minutes to answer. Exports keep their own limits. |
| `role-access-rules.tsx:100` | Rules written as plain text are not listed. + the existing "Open access rules" link | – |
| `role-table-access.tsx:264` | People read every table any of their roles allows. | A table a role may not read is hidden everywhere, and a query naming it answers "not found", as for a missing table. |
| `role-table-access.tsx:276` | Saving the role leaves these as they are. | – |
| `maps-page.tsx:122` | First region: {name}. Each region needs its own name. | – |
| `service-identity-create-page.tsx:85` | *(unchanged: it carries the reader address)* | – |

## Tasks

- **T1** Lint: the `datalist` selector (decision 1). Check: `bun x eslint`
  on a scratch file with a `<datalist>` reports the UX-7 message (do not
  commit the scratch file).
- **T2** `TimeZoneSelect` moved + `disabled` + `zones` (decision 2);
  Reporting's time zone uses it. Check: the Reporting tests pass with a
  short zone list; a non-admin sees it disabled.
- **T3** `HelpTip` (decision 4) with a test: it has an accessible name, opens
  on click and on keyboard focus + Enter, and shows the text.
- **T4** Reporting page copy and the Number format label (decisions 3, 5).
- **T5** The other Settings pages' copy.
- **T6** The four other datalists → `Combobox` (decision 1), each keeping its
  free-text or list-only behaviour. Check: each site's existing tests pass;
  add one test per site that the suggestions come from the console's list
  (no `datalist` in the DOM).
- **T7** `CHANGELOG.md` entry; `docs/core/BACKLOG.md` row `UX-7` stays as the
  planner wrote it.

Out of scope: any other wording, layout or width change; Settings sections
not named here.

## Checks (this VM)

Never the full `bun run test` or `bun run lint`. Wrap every bun command in
`flock /tmp/claude-1000/lh-bun.lock systemd-run --user --scope -q -p
MemoryMax=3G -p MemorySwapMax=0 nice -n 15 …`. Per commit:
`bun x tsc --noEmit -p .`, `bun x eslint <changed files>`, and
`bun test <the test files of the changed features>`. No prettier.

## Handoff

Developer: Claude Sonnet. Branch `fix/ux-7-settings-polish`, not pushed.

Commits (one per task):

```
  ee3a4ef3f UX-7 docs(changelog): Settings help and no browser-drawn lists
  6d19f13dc UX-7 feat(ui): the other datalists become the console's own suggestion field
  1e33939ce UX-7 feat(settings): one-line help on the other Settings pages
  fa36a02b2 UX-7 feat(settings): one-line help and one label per field on Reporting
  744f13f66 UX-7 feat(ui): HelpTip, a help popover that opens on hover, focus and tap
  614c59b09 UX-7 feat(settings): the report time zone uses TimeZoneSelect, moved to patterns
  59250aeb7 UX-7 chore(lint): refuse a native datalist, use Combobox
```

T1 lint selector, T2 `TimeZoneSelect` moved + `disabled` + `zones`, T3 `HelpTip` + test, T4 Reporting copy and Number format, T5 other Settings pages, T6 four datalists, T7 CHANGELOG.

Commands (each wrapped in flock + systemd-run, MemoryMax=3G), on the final commit:

- `bun x tsc --noEmit -p .`: clean.
- `bun x eslint <every changed src file> eslint.config.mjs`: 0 errors, 0 warnings.
- `bun test src/features/admin src/features/dashboards/subscriptions-dialog.test.tsx src/features/dashboards/format-section.test.tsx src/features/connectors/table-finder.test.tsx src/features/governance/lineage-page.test.tsx src/features/pipelines src/components/patterns`: 464 pass, 0 fail, 45 files.
- T1 check: eslint on a scratch `<datalist>` file reported the `UX-7` message (scratch file not committed).
- `grep "<datalist" src` (non-test): only a doc comment remains.

Notes and deviations:

- `HelpHint` (`patterns/help-hint.tsx`) already exists with the same popover; `HelpTip` follows the plan (accessible name "More about <heading>", 20 px) and its doc comment says when to use which. Not merged, per the plan.
- T6 adds one shared `src/components/patterns/suggest-input.tsx` (free text + the console's `Combobox`) used by all four sites instead of four copies. A field that already holds one of the names lists them all when opened.
- Line numbers in the T5 table had moved; edits were matched by their text. The `query-limits` line at "205" has no heading, so its `HelpTip` sits at the end of that line (heading name "the time limit").
- Existing tests that read the old long paragraphs or the `datalist` were updated (download-switch paragraph, "Numbers" heading, schema and pipeline-table suggestions). Added: HelpTip test, a Reporting help-tip test, series-name test, lineage-page test.
- Not verified: no browser or phone check of the popover and the combobox (happy-dom only); one-line fit of the copy at 384 px was not measured. `bun run lint` and the full `bun run test` were not run (VM memory); CI runs them.
- `node_modules` symlink used as is; no `bun install`.

## Review (planner, 2026-11-20, at `7b956e9f1`)

No BLOCKER. Lint selector, `TimeZoneSelect` move, `SuggestInput` (free text
kept, the console's `Combobox`), and the copy are as planned.

- **SHOULD-FIX 1 (rule 4, duplicated helper; the planner's miss in decision
  4).** `patterns/help-tip.tsx` repeats `patterns/help-hint.tsx` (same
  popover, hover + press, same surface); only the accessible name and the
  hover fill differ. `HelpHint` is the pattern the owner already passed in QA
  (alert and subscriptions dialogs, 2026-11-16): the ⓘ sits at the **end of
  the one-line helper**, and its accessible name is the detail itself. Use
  it here too, one look across the app: delete `help-tip.tsx` and its test;
  in Reporting and query limits put `<HelpHint>` at the end of each help
  line that has a tip (`<p …>Line. <HelpHint>{detail}</HelpHint></p>`, the
  way `chart-alert-dialog.tsx` does), with the detail as one string (build
  it with a template literal where it holds a constant); headings go back to
  a plain `h2`, and "Number format" back to a plain `Label` with the hint on
  its help line. Decision 4 is replaced by this. Keep one test that a
  Reporting hint opens and shows its detail.

## Handoff 2

Fixes PR UX-7 review SHOULD-FIX 1 in one commit: `HelpTip` and its test are
deleted; Reporting, query limits and "Tables this role can read" put the
existing `HelpHint` at the end of each one-line help (constants in a template
literal), headings are plain `h2`, "Number format" is a plain `Label` with the
hint on its help line. The Reporting tests open the hint by its detail name.

Commands (wrapped): `bun x tsc --noEmit -p .` clean; `bun x eslint src/features/admin/*.tsx`
0 errors (1 existing warning in untouched `sso-page.tsx`); `bun test` on the
same set as before minus the deleted HelpTip test: 461 pass, 0 fail, 44 files.
Not verified: browser/phone check; the one-line fit at 384 px.
