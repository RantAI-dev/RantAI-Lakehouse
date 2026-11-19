# UX-4: fewer required-field marks

Backlog: `UX-4`. Follows `UX-1` (#568, merge `4e53e41c7`). Base: `main` at `81acc21f2`.
Planner and reviewer: Claude Opus. Developer: GLM 5.3 (opencode-go), owner's choice 2026-11-19.

## Why

The owner, 2026-11-19: UX-1 is "overkill", and a colleague objected to it. UX-1 put a red
`*` on every required input and a "`*` Required" line on 45 surfaces, including boxes
that hold one input (the AI chat box, the Query Studio SQL editor, rename dialogs). A star
only carries information where a form mixes required and optional fields; on a one-input
box, or a form where everything is required, it is noise. Common practice (GOV.UK design
system, Nielsen Norman Group) agrees: no marker on single-input, chat or search boxes; a
legend only where it helps a longer form.

## Decisions (owner approved 2026-11-19, "go ahead")

1. **Stars only where a form mixes required and optional inputs.** A form or dialog in
   which every visible input is required (sign-in, change password, a rename box) shows
   no star.
2. **The "`*` Required" line (`RequiredLegend`) only on a form that shows stars AND has
   four or more inputs.** A shorter mixed form shows its stars without the line.
3. **Required semantics stay on every control.** Native `required` and `aria-required`
   are not removed anywhere; screen readers still hear "required". Validation, disabled
   buttons, defaults and error messages do not change.
4. **The AI chat box goes back to how it was before #568.** No visible "Message" label,
   no tooltip, no hidden "A message is required" text: `src/features/copilot/chat-composer.tsx`
   returns to its pre-#568 markup for those parts (`git show 4e53e41c7^1:src/features/copilot/chat-composer.tsx`),
   keeping any later, unrelated changes from `main`.
5. **The shared pieces stay:** `RequiredMarker`, `RequiredLegend`, and the `required`
   prop on `Label`/`FieldLabel`. Only their call sites change.
6. Out of scope: marking optional fields instead (GOV.UK style), any validation change,
   any new UI.

## The 45 surfaces

Each surface that renders `<RequiredLegend />` on `81acc21f2` (`git grep -l '<RequiredLegend' -- src | grep -v test`).
The developer applies decisions 1 and 2 per form (some files hold two forms, for example
`role-form-page.tsx`, `connector-create-page.tsx`), counting the inputs the user sees.
My expected result, to be confirmed against the code:

- **No stars, no line (one input, or every input required):** `copilot/chat-composer`,
  `copilot/chat-messages`, `copilot/session-parts`, `queries/sql-panel`, `queries/nl-panel`,
  `queries/save-query-page`, `queries/save-sql-source-page`, `dashboards/dashboard-list-page`,
  `dashboards/dashboard-actions`, `overview/alerts-page`, `governance/freshness-target-dialog`,
  `catalog/asset-meaning-view`, `auth/login-page`, `auth/change-password-step`,
  `connectors/upload-file-step`, `connectors/upload-table-step`.
- **Decide from the code (short forms):** `pipelines/pipeline-editor`, `pipelines/agentic-builder-dialog`,
  `catalog/asset-detail-page`, `catalog/asset-quality`, `governance/quality-rule-edit-dialog`,
  `connectors/dial-forms/files-dial-form`, `dashboards/folder-dialogs`, `dashboards/sql-source-panel`,
  `dashboards/chart-builder`, `connectors/connector-scope-fields`, `admin/invite-user-page`.
- **Expected to keep stars and the line (long mixed forms):** the other dial forms (`sql`, `mongo`,
  `oracle`, `sftp`, `kafka`, `cdc`, `rest`, `sheets`), `connector-form-parts`,
  `connector-ingest-panel`, `connector-create-page`, `connector-edit-page`, `admin/role-form-page`,
  `admin/service-identity-create-page`, `admin/tenant-create-page`, `catalog/asset-access`,
  `governance/quality-rule-create-page`, `alerts/alert-rule-page`.

A star also counts as a "mark" when it is drawn for a custom control (`<RequiredMarker />`
next to a picker or a group legend): the same rules apply.

## Tasks (one commit each, in order)

1. **Table.** Append to this plan, under "Handoff", a table of every form on the 45
   surfaces: file, form, inputs shown, required inputs, decision (keep stars + line /
   stars only / none). Commit it before changing code, so the review checks the rule
   against the table. *Check:* 45 files covered, every row follows decisions 1 and 2.
2. **Single-input and all-required surfaces.** Remove `<RequiredLegend />` and the `required`
   prop on the label (or the `<RequiredMarker />`) on the "none" rows; keep `required` /
   `aria-required` on the control. Restore the chat box (decision 4). *Check:* `git grep`
   shows no `RequiredLegend` or label `required` in those files; every control that had
   `required`/`aria-required` still has it.
3. **Short mixed forms.** Remove only the `<RequiredLegend />` on "stars only" rows.
   *Check:* the stars remain, the line is gone.
4. **Tests.** Update the tests #568 added to the new rule, and say why in the commit
   (AGENTS.md rule 2: the behaviour changed on purpose, the tests follow it). For the
   "none" surfaces, a test asserts no star and no "Required" line AND that the control is
   still required (`toBeRequired()` or `aria-required`). For a kept form, the existing
   star assertions stay. Do not delete a test that checks validation or a disabled button.
   *Check:* `bun run test` passes; no assertion about validation was removed.
5. **Docs.** `docs/core/features/required-field-consistency.md`: add a "Decisions" entry
   for UX-4 (decisions 1 to 4, dated) and change the checklist lines that say "Required
   inputs show `*` and their form or step explains `* Required`" and "Copilot inputs
   show action-specific requirements" to the new rule. `CHANGELOG.md`: one line under
   Unreleased. Do not touch `BACKLOG.md` (the planner owns it).

## Verification before handoff

```bash
bun run typecheck && bun run lint && bun run test
git status   # nothing generated staged
```

No Rust, Python or compose changes, so no cargo, pytest or compose runs.

## PR slicing

One PR, branch `fix/ux-4-fewer-required-marks`. The planner opens it.

## Handoff

(developer appends here)

## Review

(planner appends here)
