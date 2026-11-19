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

(developer appends here)

## Review

(planner appends here)
