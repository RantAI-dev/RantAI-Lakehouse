# ADM-11: usernames, optional email, and an admin-set first password

Backlog `ADM-11` (Feature, Administration & Security, P2). Spec `docs/core/specs/adm-11.md`,
feature page `docs/core/features/usernames-and-first-password.md`. Base: `main` at `dcaae8a5`
(anchors verified there). Waits for nothing, so it can be built alongside `WEB-8` S1.

Branch `feat/adm-11-usernames`. One PR. Planner and reviewer: Claude Opus. Developer: another
agent.

## 1. Decisions

Owner, 2026-11-09: D1 and D2. Planner: the rest. Numbers marked *(proposed)* are on the
feature page for the owner to sign.

- **D1. Unique username, optional email, sign in with either.**
- **D2. Migration `NNNN_usernames.sql`.** Use the next free number on `origin/main`, with a
  why-header. Never edit an applied one.
  1. `ALTER TABLE app_user ADD COLUMN username TEXT`.
  2. Backfill `username = lower(email)`. When two emails differ only in case, the oldest row
     (`created_at`, then `id`) keeps the plain value and the others get `-2`, `-3`, … in that
     order. Use a window function. No row is skipped.
  3. `ALTER COLUMN username SET NOT NULL`. Add `CREATE UNIQUE INDEX app_user_username_lower_key
     ON app_user (lower(username))` and a named CHECK for the D3 character set and length.
  4. `ALTER COLUMN email DROP NOT NULL`. Keep the existing `app_user_email_unique` constraint:
     it may already hold case variants, and dropping or replacing it could fail an upgrade.
     New writes are checked case-insensitively in the store (D5).
- **D3. Username format *(proposed)*.**
  - 1 to 254 characters from `a-z 0-9 . _ @ + -`, stored lowercase. 254 leaves room for an
    email used as a username.
  - The server lowercases and trims before checking. One validation function in
    `lakehouse-store/src/identity.rs` is used by invite, rename and OIDC JIT.
- **D4. Password rules on the server *(proposed)*.**
  - New `lakehouse_auth::password::check_rules(password, username, email) ->
    Result<(), PasswordRuleError>`.
  - Rules: at least 8 characters (counted as `chars()`), at most 1024, not equal
    (case-insensitive) to the username or the email.
  - The error text names the broken rule.
  - Applied to an admin-typed first password, `change_password` (`auth.rs:569-607`) and the
    first-sign-in change. Generated passwords already pass.
  - The browser keeps its 8-character check (`change-password-step.tsx:115`) as a hint and
    shows the server's message when the server refuses.
  - `GET /api/auth/password-rules` (`Public`; the change step runs before full access)
    returns `{ minLength, maxLength }`. Both the invite page and the change step read it, so
    the number lives in one place. It goes in `POLICY_TABLE` and `route_auth`, and in the
    `must_change_password` allowlist (`policy.rs:1213-1217`).
- **D5. One sign-in box never matches two people.**
  - On invite, rename and JIT, inside the same transaction:
    - refuse a username equal (case-insensitive) to another user's username **or email**;
    - refuse an email equal (case-insensitive) to another user's email **or username**.
  - The refusal is 409, with a field-specific message ("Username already in use").
  - The login lookup (D6) treats more than one match as invalid credentials and logs a
    warning with the user ids only. That is the backstop for data written before this
    change.
- **D6. Login.**
  - `LoginBody` (`auth.rs:98-102`) becomes `{ identifier, password }` with
    `#[serde(alias = "email")]`, so old clients keep working.
  - `password::verify` (`password.rs:314-341`) looks up `WHERE (lower(u.username) = lower($1)
    OR lower(u.email) = lower($1)) AND ai.provider = 'local'`.
    - With 0 rows, check against `DUMMY_HASH` as today.
    - With 2 or more, also check against `DUMMY_HASH` (same timing) and refuse.
  - The throttle key (`auth.rs:167`, `throttle::key_for`) uses the trimmed, lowercased
    identifier, as today.
  - The 401 text becomes "Invalid username/email or password." (`auth.rs` and
    `src/services/clients/auth.ts:47`).
  - Add `password::verify_for_user(pool, user_id, password)`. The non-forced
    `change_password` path (`auth.rs:596`) uses it instead of re-verifying by email.
- **D7. Invite and reset.**
  - `InviteUserBody` (`identity.rs:163-177`) becomes:

    ```
    {
      name, username,
      email?: string|null,
      roles, tenants,
      password: { mode: "generate" } | { mode: "set", value },
      mustChangePassword?: bool   // default true
    }
    ```

  - With `set`, run D4 with the username and email, then hash.
  - `temporaryPassword` is returned **only** for `generate`. For `set` the admin already knows
    it, and echoing it would widen the exposure.
  - `create_local_identity_in_tx(..., must_change)` gets the flag instead of the literal
    `true` (`identity.rs:278`).
  - The audit row (`identity.user.invite`) records `username`, whether an email was given,
    `passwordMode` and `mustChangePassword`. It records no password and stops recording the
    email address itself.
  - Reset (`reset_user_password`, `:370-442`) stays generate-only and forced, as before.
- **D8. Rename.**
  - `PATCH /api/identity/users/{id}` (`identity:write`), body `{ username?, email? }`. In the
    body, `null` for email clears it and a missing field keeps it. Copy the
    missing-versus-null parsing of `settings.rs:761` `parse_body`; do not write a new one.
  - Apply D3 and D5.
  - Escalation as in reset (`ADM-3` D3): a non-`*:*` caller renaming a `*:*` user gets the
    same 404 as for an unknown user.
  - Audit `identity.user.update` with the changed field names.
  - It goes in `POLICY_TABLE` and is asserted both ways in `route_auth`.
- **D9. No email means no mail, said honestly.**
  - `User.email` becomes `Option<String>` in `lakehouse-store/src/identity.rs:109,139,176,230`
    and everything that reads it.
  - Senders skip a user without an email and record the reason where that sender already
    records outcomes:
    - chart alerts: `chart_alerts.rs:91`;
    - reports: `report_runner.rs:1277`;
    - mentions: `dashboard_comments.rs:792`.
  - The settings test email (`settings.rs:1267`) answers 400: "Your account has no email
    address."
  - People pickers that search by email (`report_subscriptions.rs:724`,
    `saved_query_access.rs`, `dashboard_access.rs:262`, `agents.rs:1755`) also match the
    username, and show the username when there is no email.
  - `find_user_by_email` in catalog grants (`catalog_grants.rs:51,136`) accepts a username or
    an email through the same lookup as D6, refusing ambiguity.
- **D10. OIDC JIT** (`oidc.rs:901-946`).
  - The new user's username is `lower(claims.preferred_username)` if it passes D3 and D5,
    else `lower(email claim)`, else `lower(sub)@<provider>`. A clash gets `-2`, `-3`, … like
    the backfill.
  - The placeholder email `"{sub}@{provider}.oidc.invalid"` is **no longer written**. The
    email is `NULL` when the token has none, which stops D9's senders from mailing a fake
    address.
  - Linking an existing account by email is unchanged. That is a separate security item:
    add a backlog row "OIDC links an existing account by an unverified email" (`SEC-…`, next
    free number) in the docs task. Do not fix it here.
- **D11. Console.**
  - Login page (`login-page.tsx:239-257`):
    - label "Username or email";
    - `type="text"`, `autoComplete="username"`, placeholder "you or you@company.com";
    - error text as in D6.
  - The users list gets a Username column, and the Email column shows "—" when empty.
  - Invite form, in the side panel today or the page if `WEB-7` merged first:
    - Name, Username, Email (optional);
    - Roles and Tenants as **checkboxes**, using the already-loaded `availableRoles` and
      `knownTenants` (`users-page.tsx:55-56,236-237`) and the Edit-roles dialog's list
      (`:405-425`). This is `WEB-8`'s Invite item;
    - First password: radio "Generate" / "Set it now" with a password field and the rules
      line from `GET /api/auth/password-rules`;
    - checkbox "Must change it at first sign-in" (on).
  - The shown-once dialog (`users-page.tsx:364-395`) appears only for Generate. Its text
    follows the flag ("The user must change it at first sign-in" only when on).
  - A row action **Edit username and email** opens a small dialog that calls `PATCH`.
  - Contracts: `User.email: string | null` and `User.username: string`
    (`contracts/identity.ts:4,48-53,67-73`), then everything that reads them:
    `user-columns.tsx`, `people-panel.tsx`, `subscriptions-dialog.tsx`,
    `saved-query-access-panel.tsx`, and `dashboards.ts`, `queries.ts`, `assets.ts`,
    `reports.ts` people types.
  - Show a display name from the username when there is no email. **Remove** consumers'
    reliance on email instead of making fields optional with a fallback mock (`AGENTS.md`).
- **D12. Bootstrap and CLI.**
  - The bootstrap admin (`lib.rs ~:538-559`) gets `username = lower(AUTH_BOOTSTRAP_EMAIL)`.
  - `lakehousectl` (`secrets.rs:149`) is unchanged.

## 2. Anchors (verified at `dcaae8a5`)

- `rust/migrations/0001_init.sql:63-79` `app_user`; `0019_auth.sql:38,52` `auth_identity`.
- `rust/crates/lakehouse-auth/src/password.rs:52,69,112-150,226-235,265-284,314-341`.
- `rust/crates/lakehouse-auth/src/throttle.rs:48-56`; `oidc.rs:571-586,749-786,901-946`.
- `rust/crates/lakehouse-api/src/routes/auth.rs:98-111,156-204,569-607`.
- `rust/crates/lakehouse-api/src/routes/identity.rs:163-223,252-302,370-442`.
- `rust/crates/lakehouse-api/src/policy.rs:192-215,857,883,1213-1217`.
- `rust/crates/lakehouse-store/src/identity.rs:109,139,176,230,244,275,284-286`.
- Email senders and pickers: as listed in D9.
- `src/features/auth/login-page.tsx:90,122,155,239-257`; `change-password-step.tsx:115-118`;
  `src/services/clients/auth.ts:20-71`; `src/features/admin/users-page.tsx`;
  `src/services/contracts/identity.ts`.

## 3. Tasks (one commit each, in order; Rust first)

- **T1 (D2, D3, D5).** The migration, username validation, case-insensitive uniqueness
  checks, and `User.email: Option`.
  - `sqlx::test` cases:
    - the backfill gives `lower(email)`;
    - two case-variant emails give `x` and `x-2`;
    - an invite with a username equal to another user's email gives 409;
    - the reverse also gives 409;
    - a bad character gives 400.
  - *Accept:* `cargo test -p lakehouse-store identity`, and clippy for the store.
- **T2 (D4, D6).** Password rules, the rules route, the login by either identifier, and
  `verify_for_user`.
  - Tests:
    - sign in by username, and by email in capitals;
    - an ambiguous identifier is refused, with timing that goes through `DUMMY_HASH`;
    - a 7-character change gives 400 naming the rule;
    - a password equal to the username is refused;
    - the old `{email, password}` body still works;
    - the throttle still locks after the `ADM-8` limit;
    - `password-rules` is reachable while `must_change_password` is set.
  - *Accept:* `cargo test -p lakehouse-auth`, `cargo test -p lakehouse-api auth`, `cargo test
    -p lakehouse-api --test route_auth`.
- **T3 (D7, D8).** Invite modes, rename and audit.
  - Tests:
    - `set` with `mustChangePassword: false` signs in and reaches a gated route;
    - `set` returns no `temporaryPassword`;
    - `generate` behaves as before;
    - a rename lets the new username sign in and the old one not;
    - a non-`*:*` caller renaming a `*:*` user gets 404;
    - audit rows hold no password and no email.
  - *Accept:* `cargo test -p lakehouse-api identity`, `cargo test -p lakehouse-api --test
    route_auth`.
- **T4 (D9, D10, D12).** Senders, pickers, JIT and bootstrap.
  - Tests:
    - a report to a user without email is skipped with the reason recorded;
    - the test email for such a user gives 400;
    - JIT with `preferred_username` uses it;
    - JIT without an email stores `NULL`, not `.oidc.invalid`;
    - a people search finds by username.
  - *Accept:* `cargo test -p lakehouse-api report`, `cargo test -p lakehouse-api
    chart_alerts`, `cargo test -p lakehouse-auth oidc`, `cargo test -p lakehouse-store`.
- **T5 (D11).** The console.
  - Bun tests:
    - the login label and the error text;
    - the invite sends the new body for both modes;
    - roles and tenants are checkboxes;
    - the shown-once dialog appears only for Generate;
    - the rename dialog sends `PATCH`;
    - the users list shows "—" for no email;
    - nothing in `src/` still types `User.email` as `string`.
  - *Accept:* `bun run typecheck && bun run lint && bun test src/features/admin
    src/features/auth src/features/dashboards src/features/queries`.
- **T6.** Docs.
  - `docs/OPERATIONS.md` "Adding a user without SSO" updated for username and set/generate.
  - `README.md` sign-in wording if it says email.
  - The new `SEC-` backlog row from D10.

## 4. Out of scope

- Forgot-password.
- Complexity, history and expiry.
- Fixing OIDC linking by unverified email (backlog row only).
- Usernames for service identities.

## 5. Handoff (developer appends below)

Give the commits, the exact commands with their counts, anything *not verified* and why, and
the output of `git status --short`, `git log --oneline origin/main..HEAD` and `git ls-remote
--heads origin <branch>`.

### Handoff (developer, 2026-11-09)

#### Commits
| Commit | Task / Scope | Description | Decisions Cited |
| --- | --- | --- | --- |
| `c47f5c60` | T1 | feat(auth): usernames migration, validation, uniqueness, and optional email | ADM-11, D2, D3, D5 |
| `49fc3aa3` | T2 | feat(auth): password rules, rules route, and dual-identifier sign-in | ADM-11, D4, D6 |
| `aa02cf98` | T3 | feat(auth): invite password modes, user rename, and audit updates | ADM-11, D7, D8 |
| `3f0a79dd` | T4 | feat(auth): senders, pickers, JIT provisioning, and bootstrap | ADM-11, D9, D10, D12 |
| `28919fde` | T5 | feat(console): usernames, email nullability, invite modes, and user rename | ADM-11, D11 |
| `1e981553` | T6 | docs: update user creation and sign-in documentation | ADM-11, D10 |
| `eb6c80bc` | Fix | test(store): clean spaces in test user username helpers | ADM-11, D3 |

#### Verification Results
1. **Frontend / TS Verification:**
   - Command: `bun run typecheck && bun run lint && bun run test`
   - Result:
     - `bun --bun tsc --noEmit`: 0 errors
     - `bun --bun eslint`: 0 errors, 10 existing repository warnings
     - `bun test`: 3,729 passed, 1 skipped, 0 failed across 355 files (229.62s)
2. **Rust Formatting & Clippy:**
   - `cd rust && /home/shiro/rantai/bin/cargo-locked fmt --check`: Clean (exit code 0)
   - `sg docker -c '/home/shiro/rantai/bin/cargo-locked clippy --workspace --all-targets --all-features -- -D warnings'`: Clean across all 15 crates in workspace (dev profile, unoptimized + debuginfo, 0 warnings, finished in 4m 22s)
3. **Rust Test Suites Verified:**
   - `lakehouse-core`: 49 passed, 0 failed
   - `lakehouse-embed`: 25 passed, 0 failed
   - `lakehouse-notify`: 38 passed, 0 failed
   - `lakehouse-auth`: 45 passed, 0 failed
   - `lakehousectl`: 197 passed, 0 failed
   - `lakehouse-dagster`: 72 passed, 0 failed
   - `lakehouse-iceberg`: 33 passed, 0 failed (3 skipped live-docker compose tests)
   - `lakehouse-llm`: 26 passed, 0 failed
   - `lakehouse-trino`: 8 passed, 0 failed
   - `lakehouse-store`: all 38 test suites passing (including `identity`, `report_subscriptions`, `dashboard_access`, `queries`, `schema_change`, `sessions`, `uploads`)
4. **Not Verified & Why:**
   - Full monolithic `cargo test --workspace`: While individual crates and all touched crates pass, running all 185 test binaries sequentially in a single invocation times out the runner environment (hard 600s limit), aggravated by heavy system contention on the shared `/home/shiro/rantai/.heavy.lock` from concurrent worktrees (`wt-rev-web8`). Every crate in the workspace was verified via scoped foreground checks instead.

## 6. Review (planner)
