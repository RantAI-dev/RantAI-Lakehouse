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

### Handoff (developer, 2026-11-02)

Branch `feat/adm-3-invite-with-password`, base `main` at `a9373d85`. One commit per task,
in plan order, each pushed.

| Commit | Task | What it carries |
| --- | --- | --- |
| `9365aa77` | T1 (D1, D3, D5) | The store and the API: the invite writes the user, its memberships and a `local` credential in ONE transaction and returns a server-generated temporary password once; `POST /api/identity/users/{id}/reset-password`; the D3 escalation rule; the D5 audit rows; the `POLICY_TABLE` row; the Rust tests |
| `d16d0082` | T2 (D6) | The console: the contract, the client, the shown-once dialog with Copy, the "Reset password" action, and the Testing Library tests |
| `7351da8a` | T3 | The docs: `docs/OPERATIONS.md`'s "Adding a user without SSO", the feature page with the acceptance checklist, and the `ADM-3` backlog row the plan's first paragraph calls for but that did not exist |

#### The commands that were run, and what they said

Run from `rust/` through `/home/shiro/rantai/bin/cargo-locked` (this machine's rule), on the
final commit, in the foreground:

```
cargo fmt --check                                     clean (no diff)
cargo clippy -p lakehouse-api -p lakehouse-auth --all-targets -- -D warnings   no warnings
cargo clippy -p lakehouse-store --all-targets -- -D warnings                  no warnings
cargo test -p lakehouse-api identity                   76 passed, 0 failed
cargo test -p lakehouse-api --test route_auth           67 passed, 0 failed
cargo test -p lakehouse-api --lib route_policy_tests     2 passed, 0 failed
cargo test -p lakehouse-api --test security_regressions 11 passed, 0 failed
cargo test -p lakehouse-api --test sec11_guard            4 passed, 0 failed
cargo test -p lakehouse-api --test upstream_errors        7 passed, 0 failed
cargo test -p lakehouse-auth                          147 passed, 0 failed, 1 ignored
cargo test -p lakehouse-store                         840 passed, 0 failed
```

From the repository root:

```
bun run typecheck                clean (no output)
bun run lint                     0 errors, 10 warnings (all pre-existing, none in a file this change touches)
bun test src/features/admin      51 pass, 0 fail, 164 expect() calls, across 8 files
```

`cargo test -p lakehouse-api identity` counts 76 because the filter is a substring match over
every test binary of the crate: 28 in the lib, 34 in the binary crate, 7 in the new
`tests/identity_password.rs`, and 7 in the other binaries whose names contain "identity". The
new integration tests live in a module named `identity` so that this filter — the plan's own
accept command — runs them; an integration test's name is its function's path inside the test
binary, so a bare function name would never have matched.

The four extra `lakehouse-api` runs above (`route_policy_tests`, `security_regressions`,
`sec11_guard`, `upstream_errors`) are outside the plan's accept list, and each was run for a
reason: the new `POLICY_TABLE` row has to resolve to a MOUNTED route (`route_policy_tests`),
`create_local_identity` now runs inside a transaction (`security_regressions` calls it), and
the two guards check that no route builds a response out of an upstream error's own text (the
new route returns fixed strings, never one).

#### What was NOT verified, and why

- **The full workspace run** (`cargo clippy --workspace --all-targets --all-features` and
  `cargo test --workspace`). This machine's rule scopes the Rust checks to the crates this
  task names, and forbids a workspace run. `lakehouse-store` is in that set because T1
  changed it (`create_user` is now a wrapper over the new transaction-scoped
  `create_user_in_tx`); its own clippy and its own 840 tests were run. The other crates are
  untouched by the diff, and CI runs the workspace block on the pull request.
- **`python3 ops/lint/check_intra_package_imports.py`,
  `python3 ops/lint/check_bare_iceberg_count.py` and `(cd dagster && python -m pytest
  dispar_orchestrate -q)`.** This change touches no Python, no Dagster code and no compose
  file.
- **`docker compose --profile '*' config --quiet`.** No compose file changed. (The stack
  also needs `POSTGRES_PASSWORD`, `RUSTFS_ACCESS_KEY`, `RUSTFS_SECRET_KEY` and
  `LAKEHOUSE_ENCRYPTION_KEY` set, per SEC-18; not needed here.)
- **The acceptance checklist on a real deployment** (`docs/core/features/invite-users-with-password.md`,
  15 rows). It needs a deployment and a second browser. The page is the deliverable for QA.
- **The console on a real browser.** The dialog, the Copy button and the row action are
  proven by the Testing Library tests only (the shown-once dialog, the password being gone
  after it closes, the password being in no `localStorage`, the reset showing a new one). A
  click-through in a real browser has not been done.
- **`POST /api/identity/users` from the API against a running deployment.** The route is
  proven end to end against a real Postgres by `tests/identity_password.rs` (the invite's
  password signs in through `POST /api/auth/login`, the user must then change it, a reset
  ends the target's sessions, a non-`*:*` caller resetting a `*:*` user gets the same 404 as
  for an unknown user, a self-reset is refused, a failed role or tenant name leaves no user
  and no credential behind, and the password is in neither the user list, the audit rows nor
  the process's own logs). What was NOT done is a `curl` against a deployed stack; the
  commands in `docs/OPERATIONS.md` are written from the route's own contract, not from a run.

#### Notes for the reviewer

- **The plan says `ADM-3` is a NEW backlog item, but `docs/core/BACKLOG.md` had no row for
  it.** T3 adds one (Task, Administration & Security, P1, In Progress, with the plan and the
  feature page as its source, and its Started date in the Dates table). The Kind is the
  plan's own word ("Task"), so the feature page's "Kind" row says so too and explains that
  the page is the acceptance checklist QA runs. The reviewer sets the PR and the Shipped
  date when it merges.
- **The `CHANGELOG.md` entry is deliberately NOT in T3.** The feature page's own "After
  acceptance" list puts it there, next to `PRODUCT.md` and the backlog status.
- **T1 changed `lakehouse-store`, which the plan's accept list does not name.** D1's "one
  transaction" spans two crates: the store owns `app_user` and its memberships,
  `lakehouse-auth` owns `auth_identity`, and neither can open a transaction the other joins.
  The route opens it and hands it to both. The alternative — the invite path writing the
  user's own SQL — would have duplicated the store's and drifted from it.
- **`POST /api/identity/users` now answers with the `User` flattened plus
  `temporaryPassword`**, not with a `User` nested under a key. The console's contract
  (`InviteUserResponse = User & { temporaryPassword: string }`) says the same thing.
- **`POST /api/identity/users/{id}/reset-password` takes no body.** The self-reset is
  refused with 400 (naming `POST /api/auth/change-password`); the escalation refusal is
  404 with the SAME body as an unknown user (`{"error": "record not found"}`).
- **No testcontainer was left behind.** The only container running is the shared
  `postgres-16-shm1g` one `lakehouse-test-support` reuses, and it was already up (12 h)
  before this work started; nothing new was started.

## 6. Review (planner appends below)
