# UX-5: remove the "`*` Required" line from every form

Owner, 2026-11-19: "paling diatas gausah dikasih keterangan * required, user
juga udah tahu" — the line at the top of a form that explains `*` is not
needed; users already know what a star means. Follows `UX-4` (#595).

Base: `origin/main` at `9c702ba00`. Branch: `ux-5-no-required-legend`.
Developer: GLM 5.3 (opencode-go). Planner and reviewer: Claude Opus.

## Decisions (made; do not change)

1. `RequiredLegend` is shown nowhere. Every call site is removed.
2. The component itself is deleted (`src/components/patterns/required-legend.tsx`)
   with its test case, since it has no consumer left ("no mocks — remove
   consumers", CODE-STANDARD). No replacement text, tooltip or sr-only note.
3. The `showLegend` prop exists only to avoid a double legend (UX-1-R7). With
   no legend it is dead: remove the prop from every dial form,
   `connector-form-parts.tsx` and their callers, and the UX-1-R7 comments that
   explain it.
4. Stars stay exactly as UX-4 left them (`RequiredMarker`, `required` on
   `Label`/`FieldLabel`). Native `required` / `aria-required`, validation,
   disabled buttons, defaults and error messages do not change.
5. A wrapper element that existed only to hold the legend (for example
   `{cronChoice === "custom" ? <RequiredLegend /> : null}` in
   `connector-ingest-panel.tsx`) is removed; layout is otherwise unchanged.

## Anchors (verified at `9c702ba00`)

Call sites (`git grep -n "RequiredLegend\|showLegend" -- src`):
`admin/invite-user-page.tsx:99`, `admin/role-form-page.tsx:186,341`,
`alerts/alert-rule-page.tsx:580`, `catalog/asset-access.tsx:181`,
`catalog/asset-quality.tsx:121`, `connectors/connector-create-page.tsx:451,485,493`,
`connectors/connector-edit-page.tsx:434,455,462`,
`connectors/connector-form-parts.tsx:273-383,463-511`,
`connectors/connector-ingest-panel.tsx:769,862`,
`connectors/dial-forms/{cdc,files,kafka,oracle,rest,sftp,sql,mongo,sheets}-dial-form.tsx`,
`dashboards/chart-builder.tsx`, `governance/quality-rule-create-page.tsx`,
`pipelines/pipeline-editor.tsx`. Tests: `components/ui/required.test.tsx:49`,
`connectors/dial-forms/dial-forms-required.test.tsx:196`.

## Tasks (one commit each, in order)

1. **Remove every `RequiredLegend` call site and the `showLegend` prop.**
   Check: `git grep -n "RequiredLegend\|showLegend\|required-legend" -- src`
   prints nothing except what task 2 deletes; `bun run typecheck` passes.
2. **Delete the component and fix tests.** Delete `required-legend.tsx`; drop
   the legend case from `required.test.tsx`; update
   `dial-forms-required.test.tsx` so it no longer passes `showLegend`. Add one
   assertion per form family that had a legend (connector create, alert rule,
   role form) that no element with text `Required` explaining `*` is
   rendered, while a star is still rendered on a required input of a mixed
   form. Do not remove or weaken any assertion about `required`,
   `aria-required`, validation, disabled buttons or error messages.
   Check: `bun run test` for the touched test files, then full `bun run test`.
3. **Docs.** `docs/core/features/required-field-consistency.md`: replace the
   UX-4 legend sentence with "no form shows a `* Required` line (UX-5, owner
   2026-11-19)"; update the acceptance checklist line accordingly; the
   shared-pieces sentence no longer lists `RequiredLegend`.

Full block before handoff: `bun run typecheck && bun run lint && bun run test`.
No Rust, Python or compose is touched; run none of them.

## Out of scope

Any change to which inputs show a star; the chat composer; backend.

## Handoff

UX-5, developer handoff (GLM 5.3, 2026-11-19). Branch
`ux-5-no-required-legend`, three commits, one per task, in plan order:

1. `70f5f1323` — every `RequiredLegend` call site removed, the `showLegend`
   prop removed from every dial form, `DialFormFor`, `CredentialFields`
   and their callers (connector create/edit), together with the UX-1-R7
   comments, the UX-1-R10 legend comments on the role form, the UX-1
   comment in the chart builder, the UX-1-R8 comment in the pipeline
   editor, and the cron wrapper that held only the legend. The mongo and
   sheets dial forms kept the prop only for `DialFormFor`'s shared call
   signature and never rendered a legend; the prop is gone there too. One
   sequencing note: `dial-forms-required.test.tsx:196` passed
   `showLegend={false}` to `SqlDialForm`, and tsconfig typechecks test
   files, so leaving that caller in place would have failed this commit's
   `bun run typecheck` check — decision 3 covers "their callers", so the
   `showLegend={false}` went with this commit instead of task 2's.
   Checks: `git grep -n "RequiredLegend\|showLegend\|required-legend" -- src`
   printed only the component and `required.test.tsx` (both deleted in
   task 2); `bun run typecheck` passed. The touched test file
   (`dial-forms-required.test.tsx`) was *not run* at this commit: the plan
   defers its legend assertions to task 2, so the file was red here by
   design.
2. `a97fdf916` — `required-legend.tsx` and its legend test case deleted;
   the legend-presence assertions flipped to absence (standalone
   `SqlDialForm`, chart builder, pipeline editor, and the UX-1-R7 opt-out
   test, which tested a prop that no longer exists); one new assertion
   per family that had a legend (connector create, alert rule, role
   form): no element with the text `Required` is rendered while a star
   stays on a required input of a mixed form. Two of the flipped
   assertions (`chart-builder.test.tsx:829`,
   `pipeline-create-page.test.tsx:1029`) were not in the plan's anchor
   list; they asserted the legend by text and would have failed the full
   suite otherwise. No assertion about `required`, `aria-required`,
   validation, disabled buttons or error messages was removed or
   weakened. Checks: `bun run typecheck` passed; the touched test files
   `bun test src/components/ui/required.test.tsx
   src/features/connectors/dial-forms/dial-forms-required.test.tsx
   src/features/connectors/connector-create-page.test.tsx
   src/features/alerts/alert-rule-page.test.tsx
   src/features/admin/role-form-page.test.tsx
   src/features/dashboards/chart-builder.test.tsx
   src/features/pipelines/pipeline-create-page.test.tsx` → 173 pass,
   0 fail, 912 expect() calls, 7 files; full `bun run test` → 4565 pass,
   1 skip, 0 fail, 14111 expect() calls, 424 files.
3. `c0524997f` — `docs/core/features/required-field-consistency.md`: the
   UX-4 legend sentence replaced with "no form shows a `* Required` line
   (UX-5, owner 2026-11-19)", the checklist line updated to match, and
   `RequiredLegend` dropped from the shared-pieces sentence. Docs only;
   no checks required.

Full block before handoff, run in the foreground on the final commit:
`bun run typecheck && bun run lint && bun run test` — typecheck passed;
lint 0 errors and 11 warnings, every warning in a file this branch does
not touch (pre-existing: data-table, sidebar, sso-page, alerts-page,
login-page, use-copilot, use-data-table, dashboard-specs, uploads.test,
http.ts); test 4565 pass, 1 skip, 0 fail, 14114 expect() calls, 424
files. `git status` clean before every commit; only this branch's files
staged. Not run, per the plan: cargo, python, compose (nothing outside
`src/` TypeScript and the one feature doc was touched). Nothing else
skipped; nothing *not verified* beyond the commit-1 test-run note above.

## Review

(planner appends here)
