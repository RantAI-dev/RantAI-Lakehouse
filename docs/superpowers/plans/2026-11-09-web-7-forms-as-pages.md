# WEB-7: side-panel forms become full pages

Backlog `WEB-7` (Task, Platform, P2). Spec `docs/core/specs/web-7.md`. Base: `main` at
`dcaae8a5` (anchors verified there). **Waits for `ADM-13`**, because every new page needs a
`ROUTE_ACCESS` entry, and should follow `ADM-11` and `GOV-7` S2, so the invite and role forms
move once with their new fields. TypeScript only.

Two PR slices:

- **S1, admin and governance forms.** Branch `feat/web-7-s1-admin-forms`.
- **S2, Query Studio saves, chart builder and alert rule.** Branch `feat/web-7-s2-editors`.

Planner and reviewer: Claude Opus. Developer: another agent.

## 1. Decisions

- **D1. One page pattern.**
  - New `src/components/patterns/form-page.tsx` takes `title`, `description`, `backHref`,
    `submitLabel`, `submittingLabel`, `canSubmit`, `submitting`, `error`, `onSubmit`,
    `onCancel` and `children`.
  - Its layout is a max-width column. The header holds the back link and the title, and the
    footer holds Cancel and Save.
  - The submitting label is explicit: "Creating…" on new pages and "Saving…" on edit pages.
  - Rendered under the normal app frame, so the `ADM-13` guard applies.
- **D2. Routes.** Each gets an `app/` page that renders a feature component, plus a
  `ROUTE_ACCESS` entry with the permission its submit route needs.

  | Form | Route | Requires |
  | --- | --- | --- |
  | Invite user | `/admin/users/invite` | `identity:write` |
  | Create service identity | `/admin/service-identities/new` | `identity:write` |
  | Create role | `/admin/roles/new` | `identity:write` |
  | Edit role | `/admin/roles/[id]/edit` | `identity:write` |
  | Create tenant | `/admin/tenants/new` | `identity:write` |
  | New tag key | `/admin/settings/tag-keys/new` | the permission `tag-keys-page.tsx:44` checks |
  | Edit tag key | `/admin/settings/tag-keys/[key]/edit` | same |
  | Add quality rule | `/governance/data-quality/rules/new` | `governance:write` |
  | Save query | `/query-studio/save` | `query:read` (as the save route) |
  | Save as SQL source | `/query-studio/save-source` | the permission `POST` sources needs |
  | New chart | `/dashboards/[id]/charts/new` | `dashboard:write` or the board's edit level (the page loads the board and shows `PermissionState` when it cannot edit) |
  | Edit chart | `/dashboards/[id]/charts/[chartId]/edit` | same |
  | New alert rule | `/alerts/rules/new` | `alert:write` |
  | Edit alert rule | `/alerts/rules/[id]/edit` | `alert:write` |

  Check each "requires" against `policy.rs` and the handler. If one differs, use the
  server's rule and say so in the handoff.
- **D3. After Save**, go back to the list or the origin page (`backHref`) and show the
  existing success toast.
  - The invite's shown-once password dialog (`ADM-3`, `ADM-11`) opens on the users list after
    the redirect. Pass it through in-memory state, such as a small module-level store or a
    React context. **Never** pass it through the URL, `localStorage` or `sessionStorage`.
- **D4. Edit pages load by id.** Edit role, Edit tag key, Edit chart and Edit alert rule fetch
  the object through the existing service on mount, so a reload works. An unknown id shows
  the existing not-found state.
- **D5. Query Studio round trip.**
  - Save query and Save as SQL source need the editor's SQL, parameters and source.
  - Before navigating, write them to `sessionStorage` under one key per form
    (`lakehouse.saveQueryDraft`, `lakehouse.saveSourceDraft`). The page reads the draft and
    deletes it after a successful save.
  - If the page is opened with no draft, it shows "Open this from Query Studio" with a link.
  - **Verify that the editor keeps its text** across the navigation (tabs or buffer
    persistence in `query-studio-page.tsx`). If it does not, stop and report. Do not add
    persistence without the planner.
  - SQL text is not a secret, but do not put it in the URL either: it can be long.
- **D6. Chart builder.**
  - The `ChartBuilder` body (`chart-builder.tsx:351`) is split so the form renders inside
    `FormPage` with its chart-type sidebar. Its nested dialogs stay dialogs (`ParamDialog`
    `:1656`, `FormulaEditorDialog` `:1883`, `ValueGroupDialog` `:1903`, `HierarchyDialog`
    `:1917`), because they are small and contextual.
  - The direct `apiFetch("/api/dashboard/specs")` (`~:1180-1237`) moves to `dashboardService`
    (contract, client, service). This removes a choke-point bypass.
  - Openers to change:
    - `dashboard-page.tsx:1438` (edit) and `:1443` (new) navigate to the routes;
    - `chart-draft-card.tsx:267` (assistant) writes the draft to `sessionStorage`
      `lakehouse.chartDraft` and navigates to New chart, which reads it once.
  - `onSaved` behaviour (refreshing the board) becomes "navigate back to `/dashboards/[id]`",
    which reloads.
  - `paramValues` are carried in the URL query, since they are short dashboard parameter
    values. Check that the builder uses them only for preview.
- **D7. Alert rule.** `RuleEditDialog` (`alerts-page.tsx:479`) becomes `alert-rule-page.tsx`.
  - `alertRuleFormFields` (`:147`) and `alertRuleSaveInput` (`:186`) stay pure functions and
    are reused.
  - The raw `apiFetch` loads of `/api/dashboard/boards` and `/api/dashboard/fields`
    (`:766-787`) move to `dashboardService` methods.
- **D8. `CreateSheet` is deleted** (`components/patterns/create-sheet.tsx`) once it has no
  users. A grep in the handoff shows zero references.
- **D9. Small dashboard pop-ups stay.** These are the `sm:max-w-sm`/`md`/`lg` dialogs in
  section 3 of the research, plus subscriptions, tile and drill dialogs. Nothing else
  changes.

## 2. Anchors (verified at `dcaae8a5`)

- `src/components/patterns/create-sheet.tsx:18-72`; users `users-page.tsx:315`,
  `service-identities-page.tsx:187`, `roles-page.tsx:158,237,267`, `tenants-page.tsx:140`,
  `tag-keys-page.tsx:189`, `data-quality-page.tsx:359`, `save-query-sheet.tsx:150`,
  `save-sql-source-sheet.tsx:44`.
- `src/features/dashboards/chart-builder.tsx:351,1180-1237,1656,1773-1779,1883,1903,1917`;
  `dashboard-page.tsx:1438,1443`; `src/features/copilot/chart-draft-card.tsx:267`.
- `src/features/alerts/alerts-page.tsx:147,186,479,520,766-787,789-799,952`.
- `src/components/app-shell/nav-config.ts` (`ROUTE_ACCESS` from `ADM-13`).

## 3. Tasks (one commit each, in order)

S1:

- **T1 (D1).** `FormPage`, with a test for the labels and Cancel.
- **T2 (D2-D4).** Invite user, service identity, role new and edit, tenant and tag keys move to
  pages. Their tests move with them, and the list pages link to the new routes.
- **T3 (D2).** Add quality rule moves to a page.
- **T4 (D8).** Do this only if every `CreateSheet` user is gone (the Query Studio saves move
  in S2). Otherwise do it in S2 T7.
- *Accept S1:* `bun run typecheck && bun run lint && bun test src/components src/features/admin
  src/features/governance`, plus the `ADM-13` route-coverage test passing with the new pages.

S2:

- **T5 (D5).** Save query and Save as SQL source move to pages, with the draft handoff.
- **T6 (D6).** The chart builder page, the service move, and the openers including the
  assistant draft.
- **T7 (D7, D8).** The alert rule page, the service moves, and deleting `CreateSheet`.
- *Accept S2:* `bun run typecheck && bun run lint && bun test src/features/queries
  src/features/dashboards src/features/copilot src/features/alerts`, plus the route-coverage
  test.

Assistant parity: the chart draft path changes (D6). `tool_schemas.json` must not change. The
AI team reviews T6.

## 4. Out of scope

- Pickers (`WEB-8` S2).
- An unsaved-changes warning.
- Restyling the forms' fields.

## 5. Handoff (developer appends below, one entry per slice)

Give the commits, the exact commands with their counts, the D2 "requires" check, the D5 editor
check result, the D8 grep, anything *not verified* and why, and the output of `git status
--short`, `git log --oneline origin/main..HEAD` and `git ls-remote --heads origin <branch>`.

## 6. Review (planner)
