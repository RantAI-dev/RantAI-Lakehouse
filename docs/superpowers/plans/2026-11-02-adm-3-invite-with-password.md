# ADM-3: a new user can sign in without SSO

New backlog item `ADM-3` (Task, Administration, P1). Base: `main` at `a9373d85`. Planner: Claude
Opus. Developer: another agent (`AGENTS.md`).

## 0. What is wrong

`POST /api/identity/users` (Admin → Users → **Invite User**) creates an `app_user` with roles and
tenants, but **no credential**. The only local password in the product is the bootstrap admin's
(`main.rs:563`, `lakehouse_auth::password::create_local_identity(pool, user_id, &password, true)`).
An invited user can therefore sign in only through SSO.
- A site without an identity provider, such as staging or the first customers, cannot give
  anyone but the bootstrap admin an account.
- QA-1 needs accounts for every role (Platform Admin, Data Engineer, Analyst, Dashboard
  Viewer, Approver) and a second tenant, and it cannot have them today.
- There is no way to reset a lost password either.

## 1. Decisions (planner, 2026-11-02, product owner informed)

- **D1. An invite gives a temporary password, shown once.**
  - `POST /api/identity/users` creates the user AND a local credential, with a server-generated
    random password: at least 20 characters, from `rand`'s OS generator, over an alphabet with no
    look-alike characters.
  - `must_change_password = true`.
  - The 201 body is the `User` plus `temporaryPassword`, returned **once**. It is never stored in
    clear, never logged, never in an audit row, never in `GET /api/identity/users`.
  - User, credential, roles and tenants are written in **one transaction**, so a user can never
    be left without the credential the response promised.
- **D2. An admin can reset a password.**
  - `POST /api/identity/users/{id}/reset-password` (`identity:write`) sets a new temporary
    password, returns it once, sets `must_change_password = true` and **ends that user's
    sessions**.
  - Add it to `POLICY_TABLE`, and assert it both ways in `tests/route_auth.rs`.
- **D3. No escalation through a reset.**
  - Only a caller with the unrestricted `*:*` grant may reset the password of a user who holds
    `*:*`. Mirror `add_user_to_tenant`'s rule (`crate::routes::catalog::is_unrestricted`).
  - Anyone else gets the same 404 as for an unknown user, so a probe learns nothing.
  - A caller may not reset their own password here; that is what `change-password` is for.
- **D4. The first sign-in forces the change.** This already holds for the bootstrap admin
  (`must_change_password`, `policy.rs:1111`). An invited user goes through the same screen. Test
  it end to end.
- **D5. Audit.**
  - `identity.user.invite` and `identity.user.password_reset` audit events name the actor and the
    target user, with no password.
  - Follow how `add_user_to_tenant` writes `identity.user.tenant.add`.
- **D6. The console.**
  - After an invite, `src/features/admin/users-page.tsx` shows the temporary password once, in a
    dialog with a Copy button and the line "Shown once. The user must change it at first sign-in."
  - The users list gets a **Reset password** action, which shows the new password the same way.
  - The contract (`contracts/identity.ts`) gets `temporaryPassword` on the invite response and a
    `resetPassword` call. Keep the order: contract, client, `services/index.ts`, feature.
- **D7. SSO stays as it is.** An SSO user can also be given a password this way. That is an
  admin's choice, not a change to SSO.

## 2. Anchors (verified at `a9373d85`)

- `rust/crates/lakehouse-api/src/routes/identity.rs`: `InviteUserBody` (~:149), `create_user`
  (~:174), `add_user_to_tenant` and its tenant rule (~:200+).
- `rust/crates/lakehouse-auth/src/password.rs`: `create_local_identity` (:103),
  `change_password` (:131), `must_change_password` (:159).
- `rust/crates/lakehouse-api/src/policy.rs`: identity routes (:718-720), the first-sign-in
  redirect (~:1111).
- `rust/crates/lakehouse-api/tests/route_auth.rs`.
- `src/features/admin/users-page.tsx`, `src/services/clients/identity.ts` (:93),
  `src/services/contracts/identity.ts`.

## 3. Tasks (one commit each)

- **T1 (D1, D3, D5).** The store and the API.
  - One transaction for an invite.
  - The reset route, with the escalation rule.
  - Audit rows.
  - `sqlx::test` tests:
    - an invite returns a password that signs in through `POST /api/auth/login`;
    - the user must then change it;
    - the password is absent from `GET /api/identity/users`, from the audit rows and from the logs;
    - a failed role or tenant name leaves no user behind;
    - a reset ends the target's sessions;
    - a non-`*:*` caller resetting a `*:*` user gets 404;
    - a self-reset is refused;
    - both route rows in `route_auth.rs`.
  - *Accept:* `cargo test -p lakehouse-api identity`, `cargo test -p lakehouse-api --test
    route_auth`, `cargo test -p lakehouse-auth`, and clippy for both crates.
- **T2 (D6).** The console.
  - The shown-once dialog with Copy, and the Reset password action.
  - Bun and Testing Library tests: the password is shown after an invite and is gone after the
    dialog closes; reset shows a new one.
  - *Accept:* `bun run typecheck && bun run lint && bun test src/features/admin`.
- **T3.** Docs.
  - `docs/OPERATIONS.md`: how to add a user without SSO.
  - A feature page, `docs/core/features/invite-users-with-password.md`, with an acceptance
    checklist QA can run.

## 4. Out of scope

Email delivery of the password, self-service "forgot password", and editing or deactivating an
existing user's roles (a separate item).

## 5. Handoff (developer appends below)

## 6. Review (planner appends below)
