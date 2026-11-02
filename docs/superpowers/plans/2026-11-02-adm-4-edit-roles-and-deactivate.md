# ADM-4: change a user's roles and tenants, and deactivate a user

New backlog item `ADM-4` (Task, Administration, P1). **Waits for `ADM-3`**, because it changes the
same files (`routes/identity.rs`, `users-page.tsx`, `contracts/identity.ts`). Branch it off
`main` once ADM-3 is merged, or off `feat/adm-3-invite-with-password` and rebase. Planner:
Claude Opus. Developer: another agent (`AGENTS.md`). Product owner, 2026-11-02: "build those
too".

## 0. What is wrong

- An admin can grant roles and tenants only at invite time. `POST /api/identity/users` sets
  them, and `PUT /api/identity/users/{id}/tenants/{tenant_id}` can add a tenant. Nothing
  changes a user's roles afterwards or takes a tenant away.
- `app_user.status` already exists (`'active' | 'inactive'`, `0001_init.sql:63`), but **nothing
  enforces it**. An inactive user still signs in with a password, keeps working sessions, and
  signs in through SSO. `git grep "'inactive'"` finds it only in the report runner's recipient
  filter.
- There is no way to switch a person off when they leave.

## 1. Decisions (planner, 2026-11-02)

- **D1. Change roles.**
  - `PUT /api/identity/users/{id}/roles` with body `{ "roles": ["Analyst", …] }`, needing
    `identity:write`. It replaces the user's local role set, and an unknown role name is a 400.
  - SSO-mapped roles (`OIDC_ROLE_MAP`) are not stored and are not touched. The user keeps the
    union of the two, as today.
- **D2. Remove from a tenant.** `DELETE /api/identity/users/{id}/tenants/{tenant_id}`, the
  counterpart of the existing add, under the same "who may" rule as the add (`*:*`, or a member
  of that tenant).
- **D3. Deactivate and reactivate.**
  - `POST /api/identity/users/{id}/deactivate` and `/reactivate` (`identity:write`) set
    `status`.
  - Deactivating **ends every session of that user at once**, and revokes their API tokens if
    users own any. Check `lakehouse-auth`; service identities are separate and untouched.
  - The user's data, dashboards and audit history stay.
- **D4. Inactive means no access, on every path.** Enforce it in:
  - local login (`POST /api/auth/login`): an inactive user gets the same 401 as a wrong
    password, so nothing is enumerated;
  - session validation (`lakehouse-auth/src/session.rs` `validate_session`), so an old cookie
    stops working;
  - OIDC sign-in, including JIT: a deactivated user is refused and **not re-provisioned**;
  - the bearer and principal load (`load_principal_for_user`).

  Test each path.
- **D5. No lock-out, no escalation.**
  - Only a `*:*` caller may change the roles of a `*:*` user, grant a role that holds `*:*`, or
    deactivate a `*:*` user. Anyone else gets the same 404 as an unknown user, the rule of
    ADM-3's D3.
  - A caller may not deactivate themselves, or remove their own `*:*`.
  - Any change that would leave **no active user holding `*:*`** is refused with a 409, in the
    same transaction as the change.
- **D6. Audit.** Write `identity.user.roles.set` (with old and new role names),
  `identity.user.tenant.remove`, `identity.user.deactivate` and `identity.user.reactivate`, each
  naming the actor and the target.
- **D7. The console.**
  - The users page gets **Edit roles** (a multi-select of the roles), **Remove from tenant**,
    and **Deactivate / Reactivate** with a confirm dialog. The status shows as a badge.
  - Contract, then client, then `services/index.ts`, then the feature.
- Every new route goes in `POLICY_TABLE`, asserted both ways in `tests/route_auth.rs`.

## 2. Tasks (one commit each)

- **T1 (D1, D2, D5, D6).** The routes, the store functions and the guards.
  - `sqlx::test` tests:
    - set the roles;
    - an unknown role is a 400;
    - a non-`*:*` caller granting Platform Admin gets a 404;
    - the last `*:*` user cannot lose `*:*` (409);
    - removing a tenant works;
    - the audit rows are written;
    - the route rows are in `route_auth.rs`.
- **T2 (D3, D4).** Deactivate and reactivate, enforced on every path.
  - Tests:
    - a deactivated user's login gets 401, the same as a wrong password;
    - their existing session cookie is refused;
    - OIDC sign-in and JIT are refused and not re-provisioned (with the test IdP the auth tests
      already use);
    - reactivating restores login;
    - a self-deactivate is refused;
    - deactivating the last `*:*` gets 409.
- **T3 (D7).** The console, with Bun and Testing Library tests.
- **T4.** A feature page, `docs/core/features/manage-users.md`, with an acceptance checklist QA
  can run, and the `docs/OPERATIONS.md` note.
- *Accept (each task):* the scoped cargo commands for `lakehouse-api`, `lakehouse-auth` and
  `lakehouse-store`; `bun run typecheck && bun run lint && bun test src/features/admin` for T3.

## 3. Out of scope

Deleting a user, which would lose authorship and audit, so deactivation is the answer. Self-service
profile editing. Syncing users from SSO.

## 4. Handoff (developer appends below)

## 5. Review (planner appends below)
