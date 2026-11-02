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

### Handoff (developer, 2026-11-02)

Branch `feat/adm-4-edit-roles-and-deactivate`, off `bed964fa` (the plan's docs merge on top of
ADM-3's merge `c5663c02`). T1 was already committed by the previous session; this session
recovered its ~1,300 uncommitted lines (backed up at
`/home/shiro/rantai/review/adm4-wip-2026-11-02.patch`), finished them, and committed T2–T4.

Commits, in plan order:

| Task | Commit | Subject |
| --- | --- | --- |
| T1 | `a090f1b4` | `feat(adm-4): change a user's roles and take a user out of a tenant` (previous session) |
| T2 | `e578347a` | `feat(adm-4): deactivate and reactivate a user, enforced on every path` |
| T3 | `af98a958` | `feat(adm-4): edit roles, remove from tenant, and deactivate in the console` |
| T4 | `3f188418` | `docs(adm-4): how to change a user's access, and a checklist QA can run` |

What the recovered work was: T2's routes, `set_user_status_in_tx`, the `load_principal_for_user`
and OIDC-JIT refusals, `identity_deactivate.rs`, the OIDC and store tests, the console changes of
T3, and the two T4 documents. It did not compile as found — `users-page.test.tsx`'s `ROW` fixture
had `status: "active" as const`, so the new inactive-user test could not set
`status: "inactive"`; it is now typed as the contract's `User`. That is the only code change made
to the recovered work beyond rustfmt.

One deliberate difference from the plan's wording, for the reviewer: `ADM-4-AC14` on the feature
page now says the three actions are **shown disabled** to a caller without `identity:write`,
because that is what the console does and what the page's existing reset action already does (the
server refuses the write either way). The plan did not decide hidden-versus-disabled.

`docs/core/BACKLOG.md` has no `ADM-4` row yet: adding it with its Status/PR is the planner's step
6, not this slice's. `PRODUCT.md` is likewise untouched (step 7).

Commands run, on the final commit of each slice, from `rust/` through
`/home/shiro/rantai/bin/cargo-locked` (shared `CARGO_TARGET_DIR`, `-j 2`):

- `cargo fmt --check` — clean (also re-run at HEAD).
- `cargo clippy -j 2 -p lakehouse-api -p lakehouse-auth -p lakehouse-store --all-targets -- -D warnings` — clean.
- `cargo test -j 2 -p lakehouse-store --test identity` — 25 passed, 0 failed.
- `cargo test -j 2 -p lakehouse-auth --test oidc` — 28 passed, 0 failed.
- `cargo test -j 2 -p lakehouse-api --test identity_deactivate` — 9 passed, 0 failed (8 ADM-4
  tests plus the TLS-proxy unit test that binary carries).
- `cargo test -j 2 -p lakehouse-api --test identity_roles` — 10 passed, 0 failed.
- `cargo test -j 2 -p lakehouse-api --test route_auth` — 74 passed, 0 failed.
- `cargo test -j 2 -p lakehouse-api --test identity_password` — 8 passed; `--test
  identity_user_tenants` — 6 passed; `--test login_throttle` — 8 passed; `--test
  security_regressions` — 11 passed. All 0 failed. (Chosen because they exercise sign-in, the
  auth gate and identity writes, which D4/D5 touch.)
- `cargo test -j 2 -p lakehouse-api --lib` — 2928 passed, 0 failed, 18 ignored.
- `cargo test -j 2 -p lakehouse-auth` (whole crate) — 0 failed: lib 63 passed, then the seven
  integration binaries 28, 15, 5, 8, 8, 9 and 14 passed; doc-tests 1 ignored (existing).
- `cargo test -j 2 -p lakehouse-store` (whole crate) — 41 `test result: ok` lines (the lib, the 39
  integration binaries, and doc-tests), 0 failed; the `tests/identity.rs` binary is the 25 above.

Console, from the repository root:

- `bun run typecheck` — clean.
- `bun run lint` — 0 errors, 10 warnings, all pre-existing in files this slice does not touch.
- `bun test src/features/admin` — 56 passed, 0 failed, 178 `expect()` calls across 8 files.

**Not verified, with the reason:**

- The full `lakehouse-api` integration suite (96 test binaries, each starting its own Postgres)
  was **not** run end to end; the binaries listed above are the ones that exercise the routes and
  paths this slice changes, plus the crate's library tests. This machine runs other jobs and the
  full workspace block is the reviewer's run (AGENTS.md "Keep build time down"). The reviewer
  should expect CI to be the first full run.
- `bun run test` over the whole console was not run, only `src/features/admin` (the plan's accept
  line for T3).
- No compose file changed, so no `docker compose up` was needed. No test container was left
  behind: `docker ps` before and after this session lists the same three pre-existing containers.

## 5. Review (planner appends below)

### Review round 1 (planner, 2026-11-02, at `b08ce232`)

- **D1/D2** (`a090f1b4`): set a user's roles, and take a user out of a tenant.
- **D3/D4** (`e578347a`): deactivating sets `status` and calls `session::revoke_all_sessions_for_user_in_tx` in the same transaction. An inactive user is refused at local login with the same 401 as a wrong password, at session validation, and at OIDC sign-in, where they are not provisioned again. The principal load reads `status` too.
  - `identity_deactivate.rs` covers login, an existing session, reactivation, self-deactivation, the last administrator, and the audit rows.
  - `lakehouse-auth` `oidc` covers the OIDC refusal (28 tests).
- **D5** (the guards) uses `active_unrestricted_user_ids_in_tx` inside the change's own transaction. A change that would leave no active `*:*` user gets 409.
- **D6** (audit) and **D7** (console: role editor, tenant remover, a status switch behind a confirmation) are both done.
- **AC14 note accepted.** Shown-but-disabled is how the existing reset action already behaves.
- **Counts (the developer's):**
  - `lakehouse-api` `--lib`: 2928;
  - `route_auth`: 74, `identity_deactivate`: 9, `identity_roles`: 10, `identity_password`: 8, `identity_user_tenants`: 6, `login_throttle`: 8, `security_regressions`: 11;
  - `lakehouse-store` `identity`: 25, `lakehouse-auth` `oidc`: 28;
  - console `src/features/admin`: 56.
  - The full `lakehouse-api` suite is left to CI.

**Open: no BLOCKER, no SHOULD-FIX.** The `BACKLOG.md` row is the planner's step after the merge. The merge waits for the PR's CI.
