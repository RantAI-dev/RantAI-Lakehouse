# GOV-8, GOV-9: the role form holds what a role may read

Backlog `GOV-8` and `GOV-9` (Tasks, Governance, P1), both under the feature `GOV-7`
(`docs/core/features/table-access-per-role.md`). Base: `main` at `3cdd8332` (anchors
verified there). One PR slice, console only: branch `feat/gov-8-9-role-form-data-access`.

Planner and reviewer: Claude Opus. Developer: another agent.

Why: after `GOV-7` an administrator sets a role's tables on Edit role only. A new role is
created reading every table and has to be opened a second time to be narrowed, and the
masks and row filters that name a role are visible only on the Access rules page. The
owner asked (2026-11-12) that creating and editing a role cover its data access.

## 1. Decisions

Owner, 2026-11-12: D1. Planner: the rest.

- **D1. Data access is set on roles, never on one person.** Invite user and the Users page
  keep choosing roles only. Nothing in this plan adds a per-user grant.
- **D2. Create role gets the same section as Edit role** (`GOV-8`):
  `RoleTableAccessSection`, below the three fields, with a draft that starts at
  `{ scope: "all", grants: [] }`. One Create button. The role is created first
  (`POST /api/identity/roles`); its tables are written second
  (`PUT /api/identity/roles/{id}/table-access`, with the id the create returned), and only
  when the draft differs from "All tables". The test for "differs" is the one Edit role
  already uses, `tableAccessChanged`, against `{ scope: "all", grants: [] }`: no second
  rule. A role whose permissions grant `*:*` shows the section disabled with the same
  sentence as Edit role and sends no table-access write (`GOV-7` D4).
- **D3. If the second request fails, the page says so on Edit role.** The role then exists
  and reads every table. The page goes to `/admin/roles/<id>/edit?tables=unsaved`, and Edit
  role shows a notice above its tables section while that query value is present:
  "This role was created, but its tables were not saved. It reads every table until you
  save them here." The draft is not carried over; the administrator picks the tables again.
  A role that was just created is held by nobody, so nobody reads more than intended
  meanwhile. A retry on the Create page is not offered: a second click would create a
  second role or be refused for the name.
- **D4. Edit role lists the masks and row filters that name the role** (`GOV-9`), read-only,
  in a section **Masks and row filters on this role** below the tables section. Rules are
  still made, enforced, stopped and deleted on a table's Access tab (`WEB-3` S3-D4); this
  section only shows them and links there.
  - Source: `governanceService.listPolicies`, mapped with `accessRule`. A rule is listed
    when `ruleClause(rule)` is not `null` and its `roles` contains the role's **stored**
    name (`role.name`, not the text in the Name field) as an exact entry. Exact and
    case-sensitive, because that is the engine's own match (`condition_names_role`).
  - Each row: the table (`AccessRuleTable`, a link to the Access tab when the API sent
    `assetId`, plain text otherwise), what it does (`does`: "Masks email and phone",
    "Filters rows"), and its status (`PolicyStatusPill`), so a draft rule does not read as
    enforced.
  - A rule written as plain text is not listed: the engine skips it, and it names no role
    in a form that can be matched. One line under the list says so and links to
    `/governance/policies`.
  - States. No `policy:read` (`useAuth().hasPermission`): no request is sent, and the
    section says "Seeing the masks and row filters on a role needs the permission
    policy:read." Loading: a skeleton. The read fails: the error with a Retry, and Save
    still saves the role. No rule names the role: "No mask or row filter names this role.
    They are added on a table's Access tab."
  - Not on Create role: a role that does not exist yet is named by no rule the page could
    honestly attribute to it.
- **D5. New code lives in the admin feature and reuses the governance helpers.** A new file
  `src/features/admin/role-access-rules.tsx` imports `accessRule` and `ruleClause` from
  `@/features/governance/access-rules`, and `AccessRuleTable` and `PolicyStatusPill` from
  `@/features/governance/policy-columns`. No second parser of `conditions` (`AGENTS.md`
  rule 4). Features already import one another this way
  (`admin/settings-page.tsx` → `governance/tag-keys-page`).

## 2. Anchors (verified at `3cdd8332`)

- `src/features/admin/role-form-page.tsx:41` `tableAccessChanged`; `:54` `RoleFields`;
  `:93` `RoleCreatePage` (three fields, `createRole`, then `router.push("/admin/roles")`);
  `:176` `RoleEditForm` (role PUT, then the table-access PUT when changed; `:280` the
  section).
- `src/features/admin/role-table-access.tsx:183` `RoleTableAccessSection` takes
  `idPrefix`, `value`, `onChange`, `accessError`, `disabledReason`; it loads its own
  catalog. `grantsUnrestricted(permissions)` is exported from the same file.
- `src/services/clients/identity.ts:165` `createRole` returns the `Role` (with `id`);
  `:197` `setRoleTableAccess(id, input)`.
- `src/features/governance/access-rules.ts`: `ruleClause` (returns `null` unless the API
  called the rule `enforceable` and its `conditions` hold `roles` and `table`),
  `accessRule`, `accessTabHref`.
- `src/features/governance/policy-columns.tsx:42` `PolicyStatusPill`; `:61`
  `AccessRuleTable`.
- `src/services/clients/governance.ts:122` `listPolicies`; the route needs `policy:read`
  (`rust/crates/lakehouse-api/src/policy.rs:343`).
- `rust/crates/lakehouse-api/src/policy_engine.rs:117` `condition_names_role`: the exact
  match D4 copies.
- `src/features/admin/role-form-page.test.tsx`: `stubApi` (answers `/api/auth/me` with
  `permissions: ["identity:write"]`, records writes in order), `renderPage`, and the
  `next/navigation` mock whose `useSearchParams` returns an empty set.
- `docs/OPERATIONS.md`, "Limiting a role to some schemas or tables (`GOV-7`)".

## 3. Tasks (one commit each, in order; push after every commit)

- **T1. Create role offers the tables section (`GOV-8`, D2 and D3).**
  `role-form-page.tsx`: `RoleCreatePage` holds a `TableAccessDraft`, renders
  `RoleTableAccessSection`, and saves as D2 says; on a failed table write it goes to the
  Edit page as D3 says. `RoleEditForm` shows the D3 notice while `tables=unsaved` is in
  the address. Update the `RoleFields` doc comment, which says the section is on Edit only.
  Tests in `role-form-page.test.tsx`, named as full sentences:
  1. creating a role limited to chosen tables sends the role first and then its tables
     (assert the two writes in order, the PUT's path carries the created id, its body is
     `{ scope: "listed", grants: [...] }`, and the page goes back to `/admin/roles`);
  2. a new role left on "All tables" sends no table-access request;
  3. a new role that grants `*:*` shows the section disabled and sends no table-access
     request;
  4. when the tables cannot be saved the page opens the new role's Edit page with
     `?tables=unsaved`;
  5. Edit role shows the notice when the address carries `tables=unsaved`, and not
     otherwise.
  Accept: `bun test src/features/admin/role-form-page.test.tsx`, `bun run typecheck`,
  `bun run lint`.
- **T2. Edit role lists the masks and row filters naming the role (`GOV-9`, D4 and D5).**
  New `src/features/admin/role-access-rules.tsx` (`RoleAccessRulesSection`, props: the
  role's stored name), rendered by `RoleEditForm` below the tables section. Tests (in
  `role-form-page.test.tsx` or a new `role-access-rules.test.tsx`):
  1. only rules naming the role exactly are listed (a rule for `Analyst` shows on the
     `Analyst` role; rules for `analyst` and `Senior Analyst` do not);
  2. a rule with `assetId` links its table to `/data/assets/<id>?tab=access`, and a rule
     without one shows the table as text;
  3. a rule that both masks and filters says both, and a draft rule shows its draft status;
  4. without `policy:read` no request goes to `/api/governance/policies` and the sentence
     in D4 is shown;
  5. when the policies read fails the section shows the error with Retry, and Save still
     sends the role's PUT;
  6. a role named by no rule shows the empty sentence.
  Accept: the test file(s), `bun run typecheck`, `bun run lint`.
- **T3. Operator docs.** `docs/OPERATIONS.md`, in "Limiting a role to some schemas or
  tables": Create role offers the section too; what happens when the tables are not saved
  (D3); and where the masks and row filters on a role are shown and where they are changed.

Before the handoff, once, in the foreground: `bun run typecheck && bun run lint &&
bun run test`, then
`python3 ops/lint/check_bare_iceberg_count.py && python3 ops/lint/check_intra_package_imports.py`.
No Rust changes, so no cargo command at all. CI runs the rest on the draft PR.

## 4. Out of scope

- Any per-user data grant (D1).
- Creating a role and its tables in one request on the server. Today they are two
  requests, on Edit as well.
- Making, changing or deleting a mask or row filter from the role form.
- `SEC-68` (runs with no person behind them) and `VER-1` (row filters end to end).
- Any change under `rust/`, to `POLICY_TABLE` or to a migration.

## 5. Handoff (developer appends below)

### Handoff (developer, 2026-11-12)

**Commits** (branch `feat/gov-8-9-role-form-data-access`, pushed after each)

- `979d8504d` feat(admin): a new role sets its tables too (GOV-8)
- `d9f15a173` feat(admin): Edit role lists the masks and row filters on it (GOV-9)
- `62b755423` docs(ops): creating a role also sets its tables, and where its rules show (GOV-8, GOV-9)

**What changed**

- T1: `RoleCreatePage` holds a `TableAccessDraft` starting at
  `{ scope: "all", grants: [] }`, renders `RoleTableAccessSection`, creates the
  role first and writes its tables second only when `tableAccessChanged` says
  the draft differs from that start. A `*:*` permission disables the section
  with Edit role's sentence and sends no table-access write. A failed table
  write goes to `/admin/roles/<id>/edit?tables=unsaved`; `RoleEditForm` shows
  the D3 notice while that value is present. `RoleFields`' doc comment now
  says the section is on both pages.
- T2: new `src/features/admin/role-access-rules.tsx`
  (`RoleAccessRulesSection`, prop `roleName`) rendered below the tables
  section. It maps `governanceService.listPolicies` with `accessRule`, keeps a
  rule only when `ruleClause` is non-null and its `roles` holds `roleName`
  exactly, and shows `AccessRuleTable`, `does`, `PolicyStatusPill`. No second
  parser of `conditions`.
- T3: `docs/OPERATIONS.md`, the `GOV-7` role-limits section, says Create role
  offers the section, what an unsaved table write means (D3), and where a
  role's masks and row filters are shown and where they are changed.

**Commands run, with counts**

| When | Command | Result |
| --- | --- | --- |
| T1 commit | `bun test src/features/admin/role-form-page.test.tsx` | 21 passed, 0 failed |
| T1 commit | `bun run typecheck` | clean |
| T1 commit | `bun run lint` | 0 errors, 11 warnings (all pre-existing, none in the files touched) |
| T2 commit | `bun test src/features/admin/role-form-page.test.tsx` | 27 passed, 0 failed |
| T2 commit | `bun run typecheck` | clean |
| T2 commit | `bun run lint` | 0 errors, 11 warnings (same pre-existing set) |
| T3 commit | `bun run typecheck` / `bun run lint` | clean / 0 errors, 11 warnings |
| final commit | `bun run test` | 3985 passed, 1 skipped, 0 failed, 11962 expect() calls, 379 files, 139.48s |
| final commit | `python3 ops/lint/check_bare_iceberg_count.py` | "R11 guard: no bare count() ... found." |
| final commit | `python3 ops/lint/check_intra_package_imports.py` | "OK: all intra-package dispar_orchestrate imports resolve" |

T3 touched no test file, so no `bun test` ran on that commit. The known
flaky `ADM-9` (`REL-24`) test did not fail, so the suite was run once.

**Not verified**

- No real browser or `next dev` check of the new UI; the evidence is the jsdom
  tests above.
- No cargo command of any kind (no Rust touched). No compose change (not
  touched), so no `docker compose up`.

**Notes for the reviewer**

- `src/app/(admin)/admin/roles/[id]/edit/page.tsx` gained a `Suspense`
  boundary: T1 makes the form read `?tables=`, and `useSearchParams` needs
  one in the App Router.
- Plan mismatch, worked around in a test only: the permission picker offers
  no way to grant `*:*` from its normal view — the full-access checkbox only
  appears once the value already holds `*:*` (`permission-picker.tsx`). T1
  test 3 therefore names `*:*` in the stubbed `/api/identity/permissions`
  catalog to reach that state, with a comment saying so. The product behavior
  follows D2; no product code was added to enable granting `*:*`.
- `role-form-page.test.tsx`'s `next/navigation` mock now returns a mutable
  `search` set (so the D3 notice can be toggled), and `stubApi` gained
  `permissions`, `mePermissions`, `policies`, `policiesStatus` and a
  module-level `requests` record. No existing test was weakened or deleted.


## 6. Review (planner)

### Round 1 (planner, 2026-11-12) — branch at `f34c9039`, pull request #471

No `BLOCKER`, no `SHOULD-FIX`.

Read: the whole product diff (`role-form-page.tsx`, the new `role-access-rules.tsx`, the
edit route's `page.tsx`), the test diff, and `docs/OPERATIONS.md`.

- **D2.** Create role renders `RoleTableAccessSection` on a draft that starts at "All
  tables", creates the role, and writes the tables with the returned id only when
  `tableAccessChanged` says the draft differs. The `*:*` case sends no table-access write.
- **D3.** A failed table write goes to `/admin/roles/<id>/edit?tables=unsaved`, and Edit
  role shows the plan's sentence while that value is in the address. The table write on
  Create is deliberately not wrapped in `withNotify`, so the failure is said once, on the
  page that can fix it.
- **D4, D5.** The rules list is built from `ruleClause` and `accessRule`, with an exact
  `roles.includes(roleName)` on the stored name; no second parser. Without `policy:read`
  no request is sent. A failed read shows the error and Retry and does not stop Save.
- **Tests.** No existing test was removed or loosened; `stubApi` gained parameters for the
  caller's permissions, the permission catalog and the policies answer. The five T1 tests
  and six T2 tests are there under full-sentence names.
- **The handoff's mismatch** is accepted as written: the permission picker cannot add
  `*:*` to a role that does not hold it, so on Create the disabled state is reachable only
  through a typed value. The guard costs nothing and matches Edit role.
- **`page.tsx`** wraps the edit page in `Suspense` for `useSearchParams`. Correct for the
  App Router; the route is dynamic, so it changes nothing a person sees.

Re-run by the planner at `f34c9039`, in the foreground:

| Command | Result |
| --- | --- |
| `bun test src/features/admin/role-form-page.test.tsx` | 27 pass, 0 fail |
| `bun run typecheck` | clean |
| `bun run lint` | 0 errors, 11 warnings (none in the files touched; the same 11 as `main`) |
| `python3 ops/lint/check_bare_iceberg_count.py` | pass |
| `python3 ops/lint/check_intra_package_imports.py` | pass |

`bun run test` (whole suite) was not re-run by the planner: the developer's run is quoted in
the handoff (3985 passed, 1 skipped, 0 failed) and CI runs it on #471.

Not verified by anyone: the two sections in a browser. The owner's checklist on the
feature page covers that.
