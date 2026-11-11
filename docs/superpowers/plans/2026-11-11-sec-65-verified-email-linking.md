# SEC-65: single sign-on joins an existing account only on a confirmed email

Backlog `SEC-65` (Task, Administration & Security, P1). Spec `docs/core/specs/sec-65.md`.
Base: `main` after `ADM-12` (#455) merges; anchors verified at `d1f518f2`, the head of
`feat/adm-12-oidc-in-settings`. **Waits for `ADM-12`**: it changes the same files.

Branch `fix/sec-65-verified-email-linking`. One PR. Planner and reviewer: Claude Opus.
Developer: another agent.

## 1. Decisions

D1 to D4 are the planner's proposals; the product owner signs them on the spec before the
build starts or while it runs. The plan builds to them; a change of D3's default flips one
constant and one sentence.

- **D1. Join by email only on a confirmed email.** In `provision_user`
  (`lakehouse-auth/src/oidc.rs`), the look-up of an existing `app_user` by email runs only
  when `claims.email_verified == Some(true)` or the connection trusts its provider's emails
  (D3). `None` and `Some(false)` are the same: not confirmed.
- **D2. Not confirmed, and an account with that email exists: refuse.**
  - Nothing is created and nothing is changed. The existing account's row is untouched.
  - A new `AuthError` variant says why (`EmailNotVerified`); it carries no claim text.
  - The callback (`routes/auth.rs` `oidc_callback`) answers it as a redirect to the sign-in
    page with a fixed notice code, `email_not_verified`. The sentence lives in the console
    (`src/features/auth/sso-notice.tsx` `NOTICES`): "Your identity provider did not confirm
    your email address, so this sign-in cannot be joined to your existing account. Ask an
    administrator." Find how a refused callback reaches a person today (a deactivated
    account answers `InvalidCredentials`); if the sign-in page does not render `SsoNotice`,
    render it there. Never the token's claims in the address.
  - Audit `auth.oidc.link_refused` with the provider label and the subject, outcome
    `refused`. Not the email: it is the unconfirmed value.
  - The refusal must not tell an outsider that an account exists more than the sign-in page
    already does. It is reached only by someone the provider authenticated, and the
    sentence is the same whether the account is active or not: check the deactivated case
    (`ADM-4` D4) still answers as it does today and comes first.
- **D3. "Trust this provider's emails."** *(proposed: default off)*
  - `OidcConfig.trust_email: bool`.
  - `.env`: `OIDC_TRUST_EMAIL` (`== "true"`), in compose as `${OIDC_TRUST_EMAIL:-}`, in
    `.env.example` commented.
  - Settings: `sso_settings.oidc_trust_email BOOLEAN NULL` in a new migration (next free
    number, why-header), resolved in `runtime_settings::resolve_sso` per value like
    `jit_provisioning` (server-owned when the variable is set), in `GET`/`PUT
    /api/settings/sso` as `trustEmail`, and in the test sign-in's draft.
  - Changing it is audited by the existing `settings.sso.update` (changed field names only).
- **D4. Not confirmed, and no account with that email: create the account without the
  email.** `app_user.email` is optional since `ADM-11`. The username falls back as it does
  today when there is no email (`preferred_username`, else `sub@provider`). An unconfirmed
  address is not stored, so it cannot be mailed and cannot block a real person's account
  later.
- **D5. A person already joined is unchanged.** `authenticate` finds them by
  `(provider, external_subject)` before `provision_user` is ever called. A test holds it.
- **D6. The test sign-in says it.** When the result's `emailVerified` is not `true` and the
  draft does not trust emails, the card adds: "This provider did not confirm the email.
  Existing accounts will not be joined unless you trust this provider's emails."

## 2. Anchors (verified at `d1f518f2`)

- `rust/crates/lakehouse-auth/src/oidc.rs`: `OidcConfig` (`~:146-213`), `email_verified`
  on the claims (`:716`, `:821`), `provision_user` (`:1137`), the email look-up
  (`~:1162-1170`), `OidcTestClaims`.
- `rust/crates/lakehouse-auth/src/lib.rs`: `AuthError`.
- `rust/crates/lakehouse-api/src/routes/auth.rs`: `oidc_callback`, `path_with_notice`
  (`:893`), the notice on a successful login (`:1165`).
- `rust/crates/lakehouse-api/src/runtime_settings.rs`: `resolve_sso`, `EffectiveSso`,
  `draft_sso_settings`, `to_oidc_config`.
- `rust/crates/lakehouse-api/src/routes/settings.rs`: `SSO_FIELDS`, `put_sso`,
  `sso_settings_json`, `post_sso_test`.
- `rust/crates/lakehouse-store/src/sso_settings.rs`; `rust/migrations/0134_sso_settings.sql`.
- `src/features/auth/sso-notice.tsx`; `src/features/admin/sso-page.tsx`;
  `src/services/contracts/settings.ts`.

## 3. Tasks (one commit each, in order; Rust first)

Every `lakehouse-api` test command carries `--lib <filter>` or `--test <file>`. A bare
`cargo test -p lakehouse-api <filter>` builds all 110 integration files.

- **T1 (D1, D2, D4, D5).** `lakehouse-auth`: `trust_email` on the config, the rule in
  `provision_user`, the error variant.
  - `sqlx::test` cases:
    - confirmed email, existing account: joined, one account;
    - `email_verified` false, existing account: `EmailNotVerified`, no `auth_identity` row
      written, the account's row unchanged;
    - the claim missing: the same;
    - not confirmed but trusted: joined;
    - not confirmed, no account: a new account with `email IS NULL`;
    - a deactivated account with that email still answers `InvalidCredentials`, confirmed
      or not;
    - a subject that is already joined signs in with no `email_verified`.
  - *Accept:* `cargo test -p lakehouse-auth oidc`, clippy for `lakehouse-auth`.
- **T2 (D2, D3).** The migration, the store, the resolver, the routes, compose and
  `.env.example`, the callback's notice and the audit row.
  - Tests: `.env` owns `trustEmail` when the variable is set; a saved value is used
    otherwise; the default is off; the callback redirects to the sign-in page with
    `ssoNotice=email_not_verified` and creates no session; the audit row holds no email.
  - *Accept:* `cargo test -p lakehouse-api --lib runtime_settings`, `--lib routes::auth`,
    `--lib routes::settings`, `--test sso_settings_route`, `--test route_auth`,
    `cargo test -p lakehouse-store sso_settings`, `docker compose --profile '*' config
    --quiet`, and a real `up` of the API from a clean project (rule 8).
- **T3 (D3, D6).** The console: the checkbox on the form (read-only when server-owned), the
  card's sentence, the notice's sentence.
  - *Accept:* `bun run typecheck && bun run lint && bun test src/features/admin` and
    `bun test src/features/auth`.
- **T4.** Docs: `docs/OPERATIONS.md` (the rule, the switch, which providers are known not
  to send the claim only if a source is cited, otherwise say "check with the test
  sign-in"), `CHANGELOG.md` under Security with the upgrade note: after the upgrade a first
  sign-in through single sign-on is no longer joined to an existing account unless the
  provider confirms the email or the switch is on.

## 4. Out of scope

- Undoing joins made before this change.
- Sending a confirmation email.
- SAML.

## 5. Handoff (developer appends below)

Give the commits, the exact commands with their counts, how a refused callback reaches the
person (what you found and what you changed), the compose `up` evidence, anything *not
verified* and why, and the output of `git status --short`, `git log --oneline
origin/main..HEAD` and `git ls-remote --heads origin <branch>`.

## 5.1 Handoff (developer, 2026-11-11)

Branch `fix/sec-65-verified-email-linking`, based on `origin/docs/sec-65-plan`. Four
commits, one per task, each pushed. No PR opened (the reviewer does that).

### Commits

```
197da450 docs(sso): the confirmed-email rule, the trust switch and the upgrade note (SEC-65)
dfee54b7 feat(console): the trust-emails switch, its sentence, and the refusal notice (SEC-65 D3, D6)
184ba515 feat(sso): a per-connection "trust this provider's emails" switch (SEC-65 D2, D3)
3ee6d2c5 fix(auth): join an existing account only on a confirmed email (SEC-65 D1, D2, D4, D5)
```

### What was built, per decision

- **D1** `OidcConfig.trust_email` (default `false`); `provision_user` joins an
  existing account by email only when `claims.email_verified == Some(true)` or
  `trust_email`. `None` and `Some(false)` are the same branch.
- **D2** `AuthError::EmailNotVerified` (fixed `Display`, no claim text). An
  active account holding the email: nothing created or changed, the error
  returned, and the audit row `auth.oidc.link_refused` (`outcome = 'refused'`,
  `args = {provider, subject}`) written in `provision_user` — the only place
  the subject is known, because the variant deliberately carries none. The
  callback answers it with a `302` to `/login?ssoNotice=email_not_verified`
  and clears the flow cookie; no session cookie. Maps to 401 on the bearer
  path (same `invalid_or_expired` as an invalid signature).
- **D3** migration `0136_sso_trust_email.sql` (`oidc_trust_email BOOLEAN`, NULL
  = unset), store `SsoSettings.oidc_trust_email`, `EffectiveSso.trust_email`
  + `_source` resolved per value like `jit_provisioning`, `trustEmail` in
  `GET`/`PUT /api/settings/sso` and in the test sign-in's draft,
  `OIDC_TRUST_EMAIL` parsed with `== "true"`, compose
  `OIDC_TRUST_EMAIL: ${OIDC_TRUST_EMAIL:-}`, `.env.example` commented. A change
  is recorded by `settings.sso.update` with the field name only.
- **D4** No account with the email: new account with `email IS NULL`; the
  unconfirmed address is also not used as the display name, and the username
  falls back to `preferred_username`, else `sub@provider`.
- **D5** `resolve_principal` still looks up `(provider, external_subject)`
  first; a test holds it with no `email_verified` claim.
- **D6** The test card adds the sentence when `emailVerified !== true` and the
  draft does not trust the provider.

### The two questions the plan asked

**How a refused callback reaches a person today, and whether the sign-in page
renders `SsoNotice`.** Today it does not reach a person as a sentence: every
`authenticate_with_nonce` failure (including `ADM-4`'s deactivated account,
which answers `InvalidCredentials`) takes the `else` branch in `oidc_callback`
and returns `ApiError::unauthorized()` — a bare JSON 401 body, flow cookie
cleared. `SsoNotice` was mounted only inside `AuthenticatedFrame`
(`src/components/app-shell/app-frame.tsx:102`), i.e. behind sign-in;
`LoginPage` did not render it. D2's sentence therefore had no surface. Changed:
the callback redirects to `/login?ssoNotice=email_not_verified` for this one
error, and `LoginPage` renders `SsoNotice` inside a `React.Suspense` boundary
above the heading (T3), with `email_not_verified` added to `NOTICES`.

**The deactivated-account refusal still comes first and answers as today.**
`provision_user`'s `status != "active"` branch is unchanged and stays before
the new check, so a deactivated account answers `InvalidCredentials` whether
the email is confirmed or not. `a_deactivated_user_is_not_re_provisioned_by_jit`
now loops `None`, `Some(true)` and `Some(false)` and asserts the same variant
and an untouched row for each; `a_deactivated_user_is_refused_at_an_oidc_sign_in`
(already-joined identity) is unchanged.

### `ops/` and `dagster/` findings

- **`ops/g10` (Keycloak) still passes.** `ops/sso/realm.template.json` sets
  `"emailVerified": true` for alice (line 100) and bob (line 118), and the API
  requests the `email` scope, so Keycloak's ID token carries
  `email_verified: true`; the JIT-created accounts keep their emails. The gate
  asserts a session and `query:read`, not the email, so no gate change was
  needed and none was made.
- **`ops/g4` (`ops/oidc-mock`)**: the mock's ID token
  (`ops/oidc-mock/server.py`, the `jwt.encode` block) sends no
  `email_verified`, so under D4 its `test-analyst` account is now created
  without an email. G4 asserts the mapped `query:read` only, so it still
  passes. I did not change the mock: making it confirm the email would be a
  gate-setup change the plan does not ask for, and turning the rule off would
  be wrong. If the reviewer wants the mock to exercise the confirmed path,
  that is a one-line `"email_verified": True` there.
- Nothing in `dagster/` reads the new column or variable.

### Commands run, with counts (foreground, this branch)

Rust, from `rust/`:

```
cargo fmt --check                                              -> FMT-CHECK-OK
clippy -p lakehouse-auth -p lakehouse-store -p lakehouse-api --all-targets -- -D warnings
                                                               -> Finished, 0 warnings
cargo test -p lakehouse-auth --test oidc                       -> 35 passed; 0 failed
cargo test -p lakehouse-auth --lib                             -> 78 passed; 0 failed
cargo test -p lakehouse-api --lib runtime_settings             -> 35 passed; 0 failed
cargo test -p lakehouse-api --lib routes::auth                 -> 89 passed; 0 failed
cargo test -p lakehouse-api --lib routes::settings             -> 23 passed; 0 failed
cargo test -p lakehouse-store --lib sso_settings               -> 3 passed; 0 failed
cargo test -p lakehouse-api --test sso_settings_route          -> 5 passed; 0 failed
cargo test -p lakehouse-api --test route_auth                  -> 87 passed; 0 failed
cargo test -p lakehouse-api --test sec11_guard                 -> 4 passed; 0 failed
```

All `lakehouse-api` runs carried `--lib <filter>` or `--test <file>`; no bare
`cargo test -p lakehouse-api` was run. `cargo test -p lakehouse-auth oidc`
(the plan's Accept line) filters by test *name* inside `tests/oidc.rs` and
matches only one of them, so I ran `--test oidc` for the whole file.

Console, from the repo root:

```
bun install --frozen-lockfile                                 -> 678 packages
bun run typecheck                                             -> clean
bun run lint                                                  -> 0 errors, 11 warnings
bun test src/features/admin                                   -> 128 pass, 0 fail
bun test src/features/auth                                    -> 12 pass, 0 fail
```

The 11 lint warnings are the pre-existing ones in `data-table.tsx`,
`sidebar.tsx`, `sso-page.tsx` (`rolesList` memo, line 119),
`alerts-page.tsx`, `login-page.tsx` (`window.location.assign` in `SsoButton`),
`use-copilot.ts`, `use-data-table.ts`, `dashboard-specs.ts`, `uploads.test.ts`
and `http.ts`; none is in a line this branch added.

Compose:

```
docker compose --profile '*' config --quiet                   -> CONFIG-OK
```

(with throwaway `POSTGRES_PASSWORD`, `RUSTFS_ACCESS_KEY`, `RUSTFS_SECRET_KEY`,
`LAKEKEEPER_ENCRYPTION_KEY`, `CONNECTOR_SECRETS_KEY` in the environment).

Real `up` (rule 8), from a clean project name `sec65t2`, with
`OIDC_TRUST_EMAIL` unset, after `docker compose -p sec65t2 build
lakehouse-api oidc-mock` (warm cache, built from this branch):

```
docker compose -p sec65t2 up -d --no-build postgres lakehouse-api
  sec65t2-lakehouse-api-1  Up 26 seconds (healthy)  0.0.0.0:18080->8080/tcp
  GET http://127.0.0.1:18080/health -> 200
    {"status":"ok","database":"ok","version":"0.2.1"}
  container env: OIDC_TRUST_EMAIL=            (unset, so the empty default)
  _sqlx_migrations: 136 | sso trust email  (and 134)
  information_schema: sso_settings.oidc_trust_email | boolean | YES
docker compose -p sec65t2 down -v --remove-orphans            -> all removed
```

Host ports were remapped (`API_HOST_PORT=18080`, `CH_NATIVE_HOST_PORT=19000`,
`CH_HTTP_HOST_PORT=18123`) because another stack on this machine holds 8080
and 9000; the database, migration and boot evidence is unaffected. Only
`sec65t2` containers were removed.

### Migration number

`git ls-tree --name-only origin/main rust/migrations/ | tail -3` at handoff is
`0132`, `0133`, `0134` — `0136` was free. (`GOV-7`'s `0135` had not merged
when this branch was cut, and still has not.)

### Tests changed, and why none was weakened

- `crates/lakehouse-auth/tests/oidc.rs`: `Claims` gained `email_verified`;
  the two existing tests whose point is a *join/provision with an email*
  (`jit_links_to_an_existing_user_with_the_same_email`,
  `an_unknown_subject_is_provisioned_and_linked_when_jit_is_on`) now set
  `email_verified: Some(true)`, which is what the new rule requires. The
  deactivated-JIT test was widened, not narrowed (three claim states).
  New: an unconfirmed email refused for an existing account (`false` and
  missing, no identity, row unchanged, audit row without the email), a
  trusted connection joins, an unconfirmed email with no account creates one
  without the email, and an already-joined subject with no `email_verified`.
- `crates/lakehouse-api/src/routes/auth.rs`: a new callback test for the
  redirect, no session cookie, cleared flow cookie, no identity, and the
  audit row's shape. New unit tests for the resolver's `trustEmail` sources,
  the boolean parsing, the body shape and the JSON field.
- Console tests: the admin fixture gained `trustEmail`; two new tests for the
  checkbox (sent, and read-only when server-owned) and two for the D6
  sentence; `sso-notice.test.tsx` gained the new code; `login-page.test.tsx`'s
  `next/navigation` mock gained `useSearchParams` (the page now renders
  `SsoNotice`) and a test for the sentence.

### Not verified, and why

- `cargo test --workspace` / `cargo clippy --workspace` and `bun run test`
  (the whole suite) were **not** run: the task's instructions scope checks
  and say CI runs the full suite on the pull request.
- `ops/g10` and `ops/g4` were **not executed** (each needs its own profile
  stack). The findings above are from reading the realm template, the mock
  and the two gates, not from a run.
- The console was not opened in a browser; the sign-in notice, the checkbox
  and the test card are verified by the unit tests above.
- The `up` used remapped host ports (see above); no behaviour depends on them.

### Interpretation worth a reviewer's eye

D1 says the email look-up "runs only when" the email is confirmed, while D2
needs to know whether an account holds the address in order to refuse. The
row is therefore read whenever the token carries an email, but it is used as
the target account only when the email is confirmed or trusted; a comment at
the site says exactly that. No scope was changed and no decision was
reinterpreted beyond this.

### Repo state at handoff

Taken before this handoff commit, which adds a fifth commit on top of the
four; the final answer quotes the same three commands after it.

```
$ git status --short
(empty)

$ git log --oneline origin/main..HEAD
197da450 docs(sso): the confirmed-email rule, the trust switch and the upgrade note (SEC-65)
dfee54b7 feat(console): the trust-emails switch, its sentence, and the refusal notice (SEC-65 D3, D6)
184ba515 feat(sso): a per-connection "trust this provider's emails" switch (SEC-65 D2, D3)
3ee6d2c5 fix(auth): join an existing account only on a confirmed email (SEC-65 D1, D2, D4, D5)
d7c7aad8 Merge remote-tracking branch 'origin/main' into docs/sec-65-plan
b3d20a5e docs(plan): SEC-65, single sign-on joins an account only on a confirmed email

$ git ls-remote --heads origin fix/sec-65-verified-email-linking
197da450cda1c609e7316314a361de1148ddff3f	refs/heads/fix/sec-65-verified-email-linking
```

## 6. Review (planner)
