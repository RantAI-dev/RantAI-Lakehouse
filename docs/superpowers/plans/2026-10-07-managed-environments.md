# Environments as a managed list — Implementation Plan

**Status:** proposed on 2026-10-07 at the product owner's request ("write
the plan first"). **Not started, and not to be started until the decisions
in section 3 are signed and `main` is merged into this branch** (section 8).
Written by the planner (Claude Opus) for a developer agent, under the role
split in `AGENTS.md` on `main`. The planner writes no product code.

**Base:** `feat/connectors`, after the merge of `main`.

**Where it comes from:** another team recommended that a connector's
environment "can be CRUD". Nothing in `docs/core/` on `main` says what they
meant, so this plan takes the reading that is useful on its own and cheap
to extend: the environment becomes a list an administrator manages, and
one mark on it says which entries are production. It does not make an
environment change what the system does (section 7). Section 9 is the
question to put to that team before work starts.

This file carries what a feature page would (what the user can do, limits,
checklist). It has no backlog ID yet; one is proposed in section 8.

---

## 1. What is there today

- A connector's environment is free text: `connector.environment TEXT NOT
  NULL` (`rust/migrations/0013_connectors.sql`), required and non-blank on
  create and edit (`routes/connectors.rs`).
- The console offers three hard-coded suggestions, `production`, `staging`
  and `development`, over a text box (`ENVIRONMENT_PRESETS` in
  `src/features/connectors/connector-scope-fields.tsx`).
- It is shown as a column in the Sources list (searchable there) and as a
  pill on the connector's page. Nothing else reads it: not the
  orchestrator, not a policy, not a schedule.
- A service identity has the same free-text field
  (`service_identity.environment`, `rust/migrations/0001_init.sql`;
  `src/features/admin/service-identities-page.tsx`). These two are the
  only columns of that name in the migrations.
- On the dev stack the values in use are `production` (five connectors)
  and `staging` (one).

What is wrong with it: the three suggestions cannot be changed without a
release; free text lets `prod`, `Production` and `production` live side by
side and split the list's filter; renaming a stage means editing every
connector; nothing says which stage is the real one.

## 2. What the user can do when this is done

1. **An administrator manages the list** on a new page, Administration →
   Environments: see every environment with how many connectors and
   service identities use it; add one; rename one; change its description
   and whether it is production; delete one nothing uses.
2. **A connector's environment is picked, not typed.** The connector form
   shows the list; so does the service identity form.
3. **A rename reaches everything that uses the name**, in one step.
4. **Production is marked**: in the picker, in the Sources list and on the
   connector's page, and the connector form says in a sentence when the
   chosen environment is production.
5. **Nothing can be left pointing at an environment that is gone**: one
   that is in use cannot be deleted, and the page says how many things use
   it.

## 3. Decisions (proposed; the product owner signs before a developer starts)

1. **One list for the deployment, not one per tenant.** An environment
   names a stage of the source systems; tenants are kept apart by the
   tenant, which does not change. Per-tenant lists would need a second
   answer to "who manages them" and are not asked for.
2. **The name is the key, and the columns that hold it stay as they are.**
   A new table `environment (name PRIMARY KEY, description, is_production,
   created_at, updated_at)`. `connector.environment` and
   `service_identity.environment` each gain a foreign key to it, `ON
   UPDATE CASCADE ON DELETE RESTRICT`. So a rename reaches every user of
   the name, a delete of one in use is refused by the database itself,
   and **the connector and service identity contracts do not change**:
   they still carry `environment: string`.
3. **What is there is kept.** The migration first inserts every distinct
   value the two columns hold, exactly as stored, then the three defaults
   if absent (`production`, `staging`, `development`), then adds the
   constraints. `production` is the only one marked production. No value
   is rewritten.
4. **A new or renamed name** is lower-case letters, digits and hyphens,
   starts with a letter or digit, at most 40 characters, and may not
   differ from an existing name only by case. Names already stored that
   break the rule stay valid; the page shows them and they can be renamed.
5. **Order** is production first, then by name. No manual ordering.
6. **Who:** reading the list needs only a signed-in user (the connector
   and service identity forms both need it, and a stage's name is not a
   secret inside a deployment). Adding, changing and deleting need a new
   permission, `environment:manage`, which the platform administrator
   holds through `*:*`; no other role gets it by this change.
7. **Every write is recorded** in the audit trail, as the identity routes
   record theirs.
8. **The production mark is information only.** It changes a pill, a
   sentence in a form and the order of a list. It gates nothing
   (section 7).

## 4. Anchors (verified at `e198590`; re-find by symbol after the merge)

- Tables: `rust/migrations/0013_connectors.sql` (`connector.environment`),
  `rust/migrations/0001_init.sql` (`service_identity.environment`).
- Store: `rust/crates/lakehouse-store/src/connectors.rs` (`environment` in
  the row, the insert ~765, the update ~1649), `identity.rs` (~771).
- API: `routes/connectors.rs` (create ~688 `required("environment", …)`,
  update ~332 `non_blank("environment", …)`), `routes/identity.rs` (~616),
  `policy.rs` (`POLICY_TABLE`; the identity and connector entries ~450-477),
  `tests/route_auth.rs`. Audit: `lakehouse_store::audit` as used in
  `routes/identity.rs`.
- AI tools: `routes/ai/registry.rs` (~340, `environment` as a free string
  in the connector tool's schema), `routes/ai/tools/connectors.rs`.
- Console: `src/features/connectors/connector-scope-fields.tsx`
  (`EnvironmentInput`, `ENVIRONMENT_PRESETS`), `connector-create-page.tsx`,
  `connector-edit-page.tsx`, `connectors-columns.tsx` (the Environment
  column), `connectors-page.tsx` (search), `connector-detail-page.tsx`
  (the pill); `src/features/admin/service-identities-page.tsx` (its text
  box); the admin page to model on: `src/features/admin/tenants-page.tsx`
  (`DataTable`, `CreateSheet`, `useServiceAction`, `withNotify`);
  navigation: `src/components/app-shell/nav-config.ts`; routes under
  `src/app/(admin)/admin/`.

## 5. Tasks

One task per commit, each leaving the tree building and its tests green.

### E1 — The table and the store

- One migration, the next free number after the merge of `main`, with a
  why-header: decision 2 and 3, in that order of statements. Never edited
  once applied.
- `lakehouse-store`: a module for the list (`list` with the two usage
  counts, `get`, `insert`, `update` of name / description / production
  mark, `delete`), every value bound.
- **Accept:** `sqlx::test`s: the migration keeps an odd legacy value
  (`Production EU`) and its connector still reads it; a rename shows on
  the connectors and service identities that used the name; a delete of
  one in use fails and of one unused succeeds; the usage counts.

### E2 — The routes

- `GET /api/environments` (`Policy::RequiresAuth`), `POST
  /api/environments`, `PATCH /api/environments/{name}`, `DELETE
  /api/environments/{name}` (the three writes:
  `Policy::RequiresPermission("environment:manage")`). Each in `POLICY_TABLE` and asserted both
  ways in `tests/route_auth.rs`. Handlers return `ApiResult<ApiJson<T>>`.
- Refusals are fixed sentences, never the database's text: a name that
  breaks decision 4 (400); a name that exists, also by case (409); an
  unknown name (404); a delete of one in use (409, saying how many
  connectors and service identities use it).
- Creating or editing a connector or a service identity with an
  environment the list does not hold is refused (422, naming the field)
  by a lookup before the write; the foreign key is the backstop, and its
  error is classified, not surfaced.
- The connector tool's schema in `routes/ai/registry.rs` says the
  environment must be one from the list.
- Each write records an audit event.
- **Accept:** route tests for each refusal and each success; the
  connector and service identity refusals; `route_auth` for the four
  routes; an audit row per write.

### E3 — The page

- Contract `src/services/contracts/environments.ts`, client, service in
  `services/index.ts`; features import `@/services` only.
- `src/features/admin/environments-page.tsx` and its route under
  `src/app/(admin)/admin/environments/`, modelled on the tenants page:
  the list (name, production mark, description, used by), a sheet to add
  and to edit, a confirmed delete that is disabled with its reason while
  the environment is in use. Without `environment:manage` the page is
  read-only. A navigation entry under Administration.
- **Accept:** component tests: the list and its counts; add; rename;
  mark as production; delete and the disabled delete; each refusal shown
  in words; the read-only page.

### E4 — The pickers and the marks

- One picker component for an environment, used by the connector form
  (create and edit) and the service identity form: the list as choices,
  production marked, the chosen one's description under it; a link to the
  management page for those who may manage; an empty list says that an
  administrator has to add one. `ENVIRONMENT_PRESETS` and the text box
  go.
- A connector whose stored environment is not in the list cannot occur
  after E1; the picker still shows such a value as "not in the list"
  rather than dropping it, so a restore of older data is visible.
- The connector form says in one sentence when the chosen environment is
  production. The Sources list's Environment column and the connector
  page's pill carry the production mark; the list filters by the list's
  values.
- **Accept:** component tests for the picker in both forms, the empty
  list, the sentence, the marks, the filter; the existing create and edit
  tests updated, each changed assertion listed in the handoff.

### E5 — Documents

- `CHANGELOG.md`; `docs/OPERATIONS.md` (the table, the constraints, what
  a restore of older data needs); `docs/FEATURE_COVERAGE.md`; after the
  merge of `main`, a feature page under `docs/core/features/` and the
  backlog entry (section 8).
- **Accept:** each sentence names only what exists.

## 6. Acceptance checklist (product owner)

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | Administration → Environments | `production` (marked), `staging`, `development`, each with its count of connectors and service identities | |
| 2 | Add `uat` with a description | It is in the list, used by 0 | |
| 3 | Sources → New connector, step 3 | The four environments to pick from, production marked; no text box | |
| 4 | Pick `production` | A sentence says the connector reads a production system | |
| 5 | Create a connector in `uat`; rename `uat` to `pre-prod` | The connector shows `pre-prod` without being edited | |
| 6 | Try to delete `pre-prod` | Refused, saying one connector uses it | |
| 7 | Move that connector to `staging`; delete `pre-prod` | It is gone | |
| 8 | Try to add `Production` | Refused: it differs from `production` only by case | |
| 9 | Sign in as a user without the permission | The page is read-only; the connector form still lists the environments | |
| 10 | Administration → Service Identities, new | The same list to pick from | |

## 7. Not in this plan

- **Rules by environment.** An environment gating anything: approval to
  edit a production connector, a pipeline to Gold refusing a
  non-production source, schedules or limits by environment, promotion
  from one stage to the next. Each is a policy decision of its own; the
  production mark is what such a rule would later read.
- Environments per tenant.
- Merging two environments in one step (move their users by hand, then
  delete one).
- Colours, icons, manual ordering.

## 8. Before work starts

1. The product owner signs section 3, or changes it.
2. The question in section 9 is answered.
3. `main` is merged into `feat/connectors`. This plan adds a migration,
   and the branch's own two already carry numbers `main` has since used;
   they are renumbered in that merge, and this one takes the next free
   number after it.
4. A backlog entry on `main`'s `docs/core/BACKLOG.md`, proposed as
   "Environments as a managed list" under the area that holds Sources,
   with the ID that file gives it.

## 9. To ask the team that recommended it

"By 'environment can be CRUD', do you mean that the list of environments
is managed (add, rename, delete, one place to see them), or that an
environment should change what the system allows or does? If the second:
which rule first?" The first is this plan. The second is section 7 and
needs its own decision record.

## 10. Working on this machine

To be filled when the work is scheduled: the checkout, what else is
uncommitted in it, the cargo target directory, and the rule that the dev
stack is rebuilt by the reviewer. The rules in section 6 of
`2026-10-05-sample-tab-data-preview.md` apply unchanged.

## 11. Handoff (developer)

## 12. Review (planner)
