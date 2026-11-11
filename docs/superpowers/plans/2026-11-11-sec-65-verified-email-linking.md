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

## 6. Review (planner)
