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

### Handoff (developer, 2026-11-11, S1)

Branch `feat/web-7-s1-admin-forms`, off `origin/main` at `8c551164`. Base, developer-side:
TypeScript only; no Rust, migration, compose or S2 code touched.

**Commits**

- `c4f7e7e2` — T1/D1: `src/components/patterns/form-page.tsx` and its test.
- `9aac6e87` — T2/D2–D4: Invite user, Create service identity, Create role, Edit role, Create
  tenant, New tag key and Edit tag key are pages; `ROUTE_ACCESS` entries; the invite's
  shown-once password crosses the redirect in memory.
- `8f902808` — T3/D2: Add quality rule is a page.
- This handoff is its own docs commit.

T4/D8 was **not** done: `CreateSheet` still has two users (`save-query-sheet.tsx`,
`save-sql-source-sheet.tsx`, both S2), so per the plan it waits for S2 T7. The grep is below.

**Anchors re-found (plan's were verified at `dcaae8a5`; these moved on `main`)**

| Anchor | At `dcaae8a5` | Now (before this branch's edit) |
| --- | --- | --- |
| `users-page.tsx` `<CreateSheet>` | 315 | 349 — ADM-11 grew the file by 285 lines; removed here |
| `service-identities-page.tsx` `<CreateSheet>` | 187 | 188; `openCreateSheet` 133 → 134; removed here |
| `roles-page.tsx` `<CreateSheet>` | 158 | 159; `RoleFields` 185 → 186; `EditRoleSheet` 236 (unchanged); removed/moved here |
| `tenants-page.tsx` `<CreateSheet>` | 140 | 140 (unchanged); removed here |
| `tag-keys-page.tsx` `<CreateSheet>` | 189 | 189 (unchanged); removed here |
| `data-quality-page.tsx` `<CreateSheet>` | 359 | 359 (unchanged); removed here |
| `create-sheet.tsx` | 18-72 | 18-72 (unchanged) |
| `nav-config.ts` `ROUTE_ACCESS` | — | did not exist at `dcaae8a5`; ADM-13 (`4189c0b1`, PR #454) added `ROUTE_ACCESS` and `canOpen` |

**D2 "requires", checked against the server**

| Form | Route | Server route and permission (`policy.rs`) | `ROUTE_ACCESS` used |
| --- | --- | --- | --- |
| Invite user | `/admin/users/invite` | `POST /api/identity/users` → `identity:write` (871) | `identity:write` |
| Create service identity | `/admin/service-identities/new` | `POST /api/identity/service-identities` → `identity:write` (910) | `identity:write` |
| Create role | `/admin/roles/new` | `POST /api/identity/roles` → `identity:write` (901) | `identity:write` |
| Edit role | `/admin/roles/[id]/edit` | `PUT /api/identity/roles/{id}` → `identity:write` (905) | `identity:write` |
| Create tenant | `/admin/tenants/new` | `POST /api/identity/tenants` → `identity:write` (908) | `identity:write` |
| New tag key | `/admin/settings/tag-keys/new` | `POST /api/governance/tag-keys` → `governance:write` (363) | `governance:write` |
| Edit tag key | `/admin/settings/tag-keys/[key]/edit` | `PUT /api/governance/tag-keys/{key}` → `governance:write` (364) | `governance:write` |
| Add quality rule | `/governance/data-quality/rules/new` | `POST /api/governance/{kind}` → `governance:write` (396) | `governance:write` |

No row differs from the plan's table: the tag-keys row ("the permission `tag-keys-page.tsx:44`
checks") is `governance:write`, the same grant the page already used. The handler adds no
narrowing on top of the policy for these eight routes.

**What changed in behaviour**

- D1: `FormPage` takes the required `submittingLabel`, so a new page says "Creating…" and an
  edit page says "Saving…". Nothing renders a sheet.
- D3: `src/features/admin/invite-password.ts` is a module-level in-memory store. The invite page
  `rememberInvitePassword`s the temporary password and pushes `/admin/users`; the Users list
  `takeInvitePassword`s it once in a mount effect and opens the existing one-shot dialog. Tests
  assert the push target holds no password, `localStorage` and `sessionStorage` do not, and a
  remount does not reopen the dialog. The invite form had no success toast before, so none was
  added; the password dialog is its feedback.
- D4: Edit role and Edit tag key load by id/key through the existing `listRoles` / `tagKeysService.list`
  reads (no new endpoint exists or was added), so a reload works; an unknown id/key shows
  `EmptyState`.
- `GOV-7` S2: the role form body is still `RoleFields`, exported from `role-form-page.tsx` and
  used by both pages, so a new section lands in one place.
- Layering: the invite form's password-rules read now goes through `authService` (registered in
  `services/index.ts`, type moved to `contracts/auth.ts`) instead of importing
  `@/services/clients/auth` from a feature; the client no longer declares the type.
- Test infrastructure note: a Base UI dialog's own Close button starts an exit animation
  happy-dom never settles (the runner hangs after the click). The "shown once" property is
  asserted by unmounting and remounting the list, which still proves the store is single-use;
  no assertion was dropped.

**D8 grep (`grep -rn 'CreateSheet' src --include=*.tsx --include=*.ts`)**

Imports remaining: `src/features/queries/save-query-sheet.tsx:5` and
`src/features/queries/save-sql-source-sheet.tsx:6` only. Every admin and governance user is
gone; `create-sheet.tsx` itself and doc-comment mentions remain. T4 therefore moves to S2 T7.

**Commands run (foreground, this machine, final commit)**

- `bun run typecheck` → `$ bun --bun tsc --noEmit`, no errors.
- `bun run lint` → 0 errors, 10 warnings, all pre-existing and none in a touched file
  (`data-table.tsx`, `ui/sidebar.tsx`, `alerts-page.tsx`, `login-page.tsx`, `use-copilot.ts`,
  `use-data-table.ts`, `dashboard-specs.ts`, `uploads.test.ts`, `services/http.ts`).
- `bun test src/components/patterns` → 15 pass, 0 fail, 30 expect() calls, 3 files.
- `bun test src/components/app-shell` → 27 pass, 0 fail, 111 expect() calls, 4 files (includes
  the ADM-13 route-coverage test with the eight new pages).
- `bun test src/features/admin` → 112 pass, 0 fail, 1546 expect() calls, 19 files.
- `bun test src/features/governance` → 65 pass, 0 fail, 166 expect() calls, 11 files.

**Not verified locally: the rest, left to CI.** The whole `bun run test` suite was not run on
this machine (its memory ceiling, as briefed); no cargo command was run (no Rust change); D5's
editor round trip is S2, not measured here.

`git status --short` after the code commits: clean (only this handoff pending).
`git log --oneline origin/main..HEAD`: `c4f7e7e2`, `9aac6e87`, `8f902808`.
`git ls-remote --heads origin feat/web-7-s1-admin-forms`: the branch exists and points at the
pushed head.

### Handoff (developer, 2026-11-11, S1 round 1 fix)

**Commits.** `a88b7237` (S2, the merge of `origin/main`, already on the branch) and `99ad77b1`
(this fix). One commit for S1: the store change, its two callers, the new store test and the
two page tests are one atomic change — the signature and both call sites cannot land apart.

**Found in the checkout after the restart.** The earlier session's S1 work was uncommitted on
`feat/web-7-s1-admin-forms` and touched only the files this task owns: `invite-password.ts`
(modified), `invite-user-page.tsx`, `users-page.tsx` and the two `*.test.tsx` (modified), plus a
new untracked `invite-password.test.ts`. Nothing else was dirty, the branch matched
`origin/feat/web-7-s1-admin-forms` at `a88b7237`, and no pull was needed. I kept that work,
checked it against the finding, and committed it. No stash, reset or deletion was performed.

**What the fix does.** `rememberInvitePassword(value, userId, now = Date.now())` stores the
value with the `/api/auth/me` id of the signed-in administrator and the clock reading;
`takeInvitePassword(userId, now = Date.now())` hands it back only to that id and only when
`now - rememberedAt <= INVITE_PASSWORD_TTL_MS`, and clears the module variable in every branch
(other user, expired, nothing stored). `INVITE_PASSWORD_TTL_MS = 2 * 60_000` is a named
constant documented as a proposed number from review round 1. The invite page takes the id from
`useAuth()` and drops an unattributed password rather than leaving it for the next sign-in; the
Users list asks for `user.id` in its mount effect, so a value left in the tab is cleared and not
shown. Both clocks are defaulted parameters, so `invite-password.test.ts` moves time by
injection and sleeps nowhere. The URL, `localStorage` and `sessionStorage` assertions are
unchanged. The existing tests changed only by passing the reader's id (`"u1"`); no assertion was
dropped or weakened — `invite-user-page.test.tsx` keeps its 32 invite assertions and
`users-page.test.tsx` gains one test for the other-user case.

**Commands run (foreground, this machine, `99ad77b1`)**

- `bun install --frozen-lockfile` → `Checked 779 installs across 753 packages (no changes)`.
- `bun run typecheck` → `$ bun --bun tsc --noEmit`, no output, exit 0.
- `bun run lint` → exit 0, 10 problems (0 errors, 10 warnings), the same pre-existing set the
  S1 handoff listed; a `grep` of the output for `features/admin` found none.
- `bun test src/features/admin` → 117 pass, 0 fail, 1555 expect() calls, 20 files.
- `bun test src/features/admin/invite-password.test.ts` → 4 pass, 0 fail, 6 expect() calls,
  1 file.
- `bun test src/components/app-shell` → 27 pass, 0 fail, 111 expect() calls, 4 files.

**Not verified locally: the rest, left to CI.** `bun run test` (the whole suite) was not run on
this machine; no cargo command was run (no Rust change); no compose check (no compose change).
`git status --short` before the commit: exactly the six S1 files, the new test untracked; after
it, clean apart from this handoff and then clean.

### Handoff (developer, 2026-11-11, S2)

Branch `feat/web-7-s2-editors`, off `origin/main` at `d95480d3`. TypeScript only: no Rust, no
migration, no compose. `tool_schemas.json` is untouched.

**Commits**

- `0066f5f3` — T5/D2/D5: Save query and Save as SQL source are pages, with the sessionStorage
  draft handoff.
- `58c34f58` — T6/D2/D4/D6: the chart builder is a page; the save moves to `dashboardService`;
  the openers (dashboard and assistant draft) navigate; the parameter values ride in the address.
- `b94d8efb` — T7/D2/D4/D7/D8: the alert rule form is a page; the two `apiFetch` loads move to
  `dashboardService`; `CreateSheet` is deleted.
- This handoff is its own docs commit.

**Anchors re-found** (the plan's were verified at `dcaae8a5`; `main` moved to `d95480d3`)

| Anchor | At `dcaae8a5` | At `d95480d3` (before this branch) |
| --- | --- | --- |
| `chart-builder.tsx` `ChartBuilder` | 351 | 351 (unchanged) |
| `chart-builder.tsx` the specs `apiFetch` | "1180-1237" | preview 1160, save 1222; the span the plan names is the preview block, not the save |
| `chart-builder.tsx` `<ParamDialog` | 1656 | 1662 (+6) |
| `chart-builder.tsx` the dialog wrapper | 1773-1779 | 1779-1785 (+6) |
| `chart-builder.tsx` Formula/ValueGroup/Hierarchy dialogs | 1883/1903/1917 | 1889/1909/1923 (+6) |
| `dashboard-page.tsx` edit opener | 1438 | 1438 (unchanged) |
| `dashboard-page.tsx` new opener | 1443 | 1443 (unchanged) |
| `chart-draft-card.tsx` `<ChartBuilder>` | 267 | 267 (unchanged) |
| `alerts-page.tsx` `alertRuleFormFields` | 147 | 147 (unchanged) |
| `alerts-page.tsx` `alertRuleSaveInput` | 186 | 186 (unchanged) |
| `alerts-page.tsx` `RuleEditDialog` | 479 | 479 (unchanged) |
| `alerts-page.tsx` the raw loads | "766-787" | boards 754, fields 762, `loadFields` 775-787 — the plan's span was already ~12 lines low at `dcaae8a5` |
| `alerts-page.tsx` `<RuleEditDialog>` render | 952 | 952 (unchanged) |
| `create-sheet.tsx` | 18-72 | 18-72 (unchanged); deleted here |
| `save-query-sheet.tsx` | — | `SaveQuerySheet` 48, `<CreateSheet>` 150; deleted here |
| `save-sql-source-sheet.tsx` | — | `SaveSqlSourceSheet` 23, `<CreateSheet>` 44; deleted here |
| `query-studio-page.tsx` save openers | — | 107 (`onSave`), 204/217 (the two sheets); deleted here |

**D2 "requires", checked against the server**

| Form | Route | Server route and permission | `ROUTE_ACCESS` used |
| --- | --- | --- | --- |
| Save query | `/query-studio/save` | `POST /api/query/saved` → `query:read` (`policy.rs:475`) | `query:read` |
| Save as SQL source | `/query-studio/save-source` | `POST /api/dashboard/sources` → `dashboard:sql` (`policy.rs:680`) | `dashboard:sql` |
| New chart | `/dashboards/[id]/charts/new` | `POST /api/dashboard/specs` → `RequiresAuth`, level resolved in the handler (`policy.rs:647`) | `[]` |
| Edit chart | `/dashboards/[id]/charts/[chartId]/edit` | `PUT /api/dashboard/specs` → `RequiresAuth`, same (`policy.rs:648`) | `[]` |
| New alert rule | `/alerts/rules/new` | `POST /api/alerts` → `RequiresAuth`, handler decides (`policy.rs:424`) | `alert:write` |
| Edit alert rule | `/alerts/rules/[id]/edit` | `PUT /api/alerts` → `RequiresAuth`, same (`policy.rs:425`) | `alert:write` |

Three of these differ from the plan's table, because the server's rule is wider or narrower:

- **Charts.** The plan wrote "`dashboard:write` or the board's edit level". The route is
  `RequiresAuth` and the handler requires `edit` on the dashboard, which a rule can grant without
  `dashboard:write`; a `dashboard:write` entry would shut a rule-only editor out. So both entries
  are `[]` and the page loads the board and renders `PermissionState` below `edit` (the plan's
  parenthetical).
- **The built-in dashboard.** `GET /api/dashboard/access` refuses `default` with 400 — it is not a
  stored row (`managed_subject`, `BUILT_IN`). The page reads the level the server uses at the
  root instead (`chart-page.ts` `builtInBoardEditable`: `identity:write` or `dashboard:write`,
  matching `baseline_of` / `level_at_root`). Without that the most common board would have shown
  an error.
- **Alert rules.** The plan's `alert:write` is what this form's saves take, but the route floor is
  `RequiresAuth`: an owner may change their own chart alert without `alert:write`
  (`policy.rs:417-425`, `may_manage`). This page's create form cannot produce a chart alert (that
  dialog stays on the dashboard), so the entry states the permission its own writes need. The
  gap it leaves: a chart alert's owner without `alert:write` is refused by the *edit* page's
  entry although the server would allow the save. Flagged, not worked around.

**D5 — does the editor keep its text across the trip?** Yes, verified before building. Query
Studio keeps its tabs in `localStorage` under `QUERY_DRAFT_KEY`
(`rantai-lakehouse:query-studio-draft`, `lib/query-draft.ts`), restores them on mount in
`use-query-studio.ts`, and writes them through `useDebouncedCallback` (400 ms), which **flushes
the pending write on unmount** (`hooks/use-debounced-callback.ts`). Navigating to the save page
unmounts the studio, so the last keystrokes are flushed and the same SQL, tabs, parameters and
`values` come back on return. No persistence was added by this slice.

**D6 — `paramValues`.** They are preview-only, as the plan suspected. `buildPayload` adds
`payload.params = paramValues` only when `forPreview` is set (`chart-builder.tsx`), and the
preview effect keys on them; the save body never carries `params` or the values.
`chart-builder-page.test.tsx` ("draws the preview with the address's parameter values and saves
the chart without them") asserts both halves against the request bodies. The card they travel in
is `?params=<encodeURIComponent(JSON.stringify(values))>`, read by `chartParamValues`.

**D6 — the assistant draft path (for the AI team).** The draft card no longer opens the builder.
"Edit in builder" writes `{ board, def, complete? }` to `sessionStorage` under
`lakehouse.chartDraft` (`features/dashboards/chart-page.ts` `writeChartDraft`), where `def` is the
same chart definition the card already sends to `/specs/preview`, and `complete` names the chat's
message and step indices when the card came from a pending `create_chart`. It then pushes
`/dashboards/<board>/charts/new`. The New chart page takes the draft once on mount with
`takeChartDraft(board)`, which deletes the key whatever it held, ignores a draft for another
board, and returns it only if it still parses; the builder then shows it filled in. On a
successful save the page calls the app frame's `useCopilot().completeToolStep(messageIndex,
stepIndex, { created: true, title, kind, mart, board, via: "builder" })`, so the chat marks the
step saved as it did when the card's own builder closed — the Copilot provider lives in
`AppFrame` and survives the navigation. `tool_schemas.json` does not change.

**D8 grep (`grep -rn 'CreateSheet' src --include=*.tsx --include=*.ts`)** — no import and no
render: `create-sheet.tsx` is gone. Six prose mentions remain, each saying where a form came
from: `form-page.tsx:12`, `tag-key-form-page.tsx:24`, `quality-rule-create-page.tsx:22`,
`tenant-create-page.tsx:14`, `invite-user-page.tsx:16`, `service-identity-create-page.tsx:15`.

**What changed in behaviour, beyond the pages themselves**

- Saving query: the page shares the editor's "change or copy" rule
  (`saved-query-write.ts`), and after a successful save it points the stored query tab at the
  query it produced (`rememberSavedTab`, `lib/query-draft.ts`). That is what the sheet did in
  place; without it, a copy just made would still offer "Save as new".
  `useQueryStudio.save`/`saveAct` now have no UI caller; I kept them (their tests assert the
  create/update decision, and their assertions are also in the new page test) rather than
  deleting tested code the plan did not ask me to remove — flagged for the review.
- Charts: `onSaved` goes back to `/dashboards/<id>`, which reloads. The dialog's in-place
  placement on the open tab (`loadAndPlace`) is gone with it, so with several tabs a new chart is
  drawn where the board draws charts with no stored box, not on the tab that was open. The plan
  asks for the reload; this is the difference it implies.
- Charts: Edit reads the chart through `GET /api/dashboard/specs` (the list) because the server
  has no by-id route; that is one whole-list read per Edit visit, noted as a cost.
- Alerts: the list still owns "New rule"/"Edit" as navigation, and still writes a rule back when
  toggled, with the same `alertRuleSaveInput`.

**Commands run (foreground, this machine, final commit `b94d8efb`)**

- `bun install --frozen-lockfile` → `Checked 779 installs across 753 packages (no changes)`.
- `bun run typecheck` → `$ bun --bun tsc --noEmit`, no errors.
- `bun run lint` → 0 errors, 10 warnings, the same set S1 recorded (`data-table.tsx`,
  `ui/sidebar.tsx`, `alerts-page.tsx` — its `rules` conditional in the list, pre-existing, now at
  128 — `login-page.tsx`, `use-copilot.ts`, `use-data-table.ts`, `dashboard-specs.ts`,
  `uploads.test.ts`, `services/http.ts`). No touched file gained one.
- `bun test src/features/queries` → 377 pass, 0 fail, 28 files.
- `bun test src/features/dashboards` → 622 pass, 0 fail, 2542 expect() calls, 79 files.
- `bun test src/features/copilot` → 140 pass, 0 fail, 500 expect() calls, 12 files.
- `bun test src/features/alerts` → 38 pass, 0 fail, 100 expect() calls, 3 files.
- `bun test src/components/app-shell` → 27 pass, 0 fail, 115 expect() calls, 4 files (the ADM-13
  route-coverage test with the six new pages, and the new `canOpen` assertions).
- `bun test src/components/patterns` → 15 pass, 0 fail, 30 expect() calls, 3 files (after
  `create-sheet.tsx` was deleted).
- `bun test src/services` → 212 pass, 0 fail, 593 expect() calls, 19 files.
- `bun test src/lib/query-draft.test.ts` → 20 pass, 0 fail, 1 file. A single file rather than the
  folder: `rememberSavedTab` (the tab handoff) belongs in `lib/query-draft.ts`, which owns the
  draft's shape, and the brief's folder list does not include `src/lib`.

**Not verified locally: the rest, left to CI.** `bun run test` (the whole suite) was not run on
this machine; no cargo command was run (no Rust change); no compose check (no compose change).

`git status --short` after the three code commits: clean (only this handoff pending).
`git log --oneline origin/main..HEAD`: `0066f5f3`, `58c34f58`, `b94d8efb`.
`git ls-remote --heads origin feat/web-7-s2-editors`: the branch exists and points at the pushed
head.


## 6. Review (planner)

### S1, round 1 (planner, 2026-11-11) — branch at `5771aa14`

Read: the access map entries (`nav-config.ts`), `form-page.tsx`, `invite-password.ts`, the
service registration (`services/index.ts`, `clients/auth.ts`), and every new page's service
calls and navigation. The form bodies are moved code; I checked that their tests moved with
them (the seven invite tests are in `invite-user-page.test.tsx`, 32 assertions against 27
removed from `users-page.test.tsx`; the other four forms likewise) rather than reading each
field. I ran no check locally; the pull request (#461) runs the suite.

- D1 to D4 are built as planned. Eight pages, each with a `ROUTE_ACCESS` entry that matches
  the submit route in `policy.rs`; edit pages load by id; no feature imports a client.
- T4 (D8) is rightly left for S2: `CreateSheet` still has the two Query Studio users.
- The invite page reads the password rules through `authService`, which is the registry
  doing its job, not a bypass.

**No BLOCKER.**

**SHOULD-FIX**

- **S1. The shown-once password can outlive the person who made it.** `invite-password.ts`
  keeps it in a module variable until the Users list takes it. Signing out is a
  `router.push("/login")` (`auth-provider.tsx`), not a reload, so the variable survives it:
  an administrator who invites, does not land on the list (a failed navigation, a quick click
  elsewhere) and signs out leaves the password for whoever signs in next in that tab and
  opens Users.
  - `rememberInvitePassword` records who made it (the signed-in user's id) and when;
    `takeInvitePassword(userId)` returns it only to the same user and only within two minutes
    *(proposed)*, and clears it in every other case.
  - Tests: another user gets nothing and the value is gone; an old value gets nothing.
- **S2. Merge `origin/main`** (the branch is behind).

### S1, round 2 (planner, 2026-11-11) — branch at `b8624ce2`

Read the fix (`a88b7237..99ad77b1`, six files under `src/features/admin/`).

- **S1: closed.** `rememberInvitePassword(value, userId, now)` records who and when;
  `takeInvitePassword(userId, now)` clears the value in every case and returns it only to the
  same user within `INVITE_PASSWORD_TTL_MS` (two minutes, marked *(proposed)*). The invite
  page drops a password it cannot attribute; the Users list waits for the signed-in user
  before it asks. `invite-password.test.ts` covers the same user once, another user, and an
  old value. The moved tests changed only by passing the reader's id.
- **S2: closed** (`a88b7237`).

**No open BLOCKER and no open SHOULD-FIX.** I ran no check locally; merged only when #461 is
green on its last commit.

### S2, round 1 (planner, 2026-11-11) — branch at `cdc600a1`

Read: the six access-map entries, `chart-builder-page.tsx`, `chart-page.ts`, `save-draft.ts`,
the change to `chart-draft-card.tsx`, and the service additions. The save-query, save-source
and alert-rule forms are moved code; I checked that their tests moved with them and did not
shrink (save query 70 assertions to 82, save source 3 to 10, alert rule 16 in its own file,
none removed from `alerts-page.test.tsx`) rather than reading each field. I ran no check
locally; #463 runs the suite.

- D5 to D8 are built as planned. `CreateSheet` is deleted; the three direct `apiFetch` calls
  are in `dashboardService`; SQL and chart drafts travel in `sessionStorage`, the dashboard's
  parameter values in the address and only into the preview.
- The three places where the developer used the server's rule instead of the plan's table
  are accepted:
  - the chart pages are `[]` and the page asks for `edit` on the dashboard, because
    `POST`/`PUT /api/dashboard/specs` judge the level in the handler;
  - the built-in dashboard reads the root level;
  - the alert rule pages take `alert:write`. A person's own chart alert is edited from the
    chart's dialog on the dashboard, which stays a dialog (D9), so nothing is lost here.

**No BLOCKER.**

**SHOULD-FIX**

- **S1. The builder can mark the wrong chat step as saved** (T6; for the AI team's review
  too). The draft carries `complete: { messageIndex, stepIndex }` and nothing that names the
  conversation. The assistant's dock is on the builder page, so a person can start a new
  chat or open another conversation before saving; `completeToolStep` then writes "created"
  onto whatever sits at those two indexes in the conversation that is open now, a result
  that never happened (principle 2). The draft records the conversation it came from
  (`useCopilot().sessionId`), and the page marks the step only when the open conversation is
  still that one. Tests for both.
- **S2. The edit page takes any chart id.** `dashboardService.getChart(chartId)` is not
  checked against the dashboard in the address, so `/dashboards/A/charts/<a chart of B>/edit`
  opens B's chart under A's back link and A's access check. The server still judges the save
  by the chart's own dashboard; the page should show "Chart not found" when the chart's
  dashboard is not the one in the address. A test.
- **S3. `useQueryStudio.save` and `saveAct` have no caller left** (the developer's own
  note). Remove them and the tests that only exercise them; the rule they held is in
  `saved-query-write.ts`, which is tested.

