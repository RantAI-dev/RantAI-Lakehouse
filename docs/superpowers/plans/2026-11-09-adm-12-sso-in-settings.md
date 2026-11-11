# ADM-12: single sign-on set in Settings by a Platform Admin (OIDC)

Backlog `ADM-12` (Feature, Administration & Security, P2). Spec `docs/core/specs/adm-12.md`,
feature page `docs/core/features/sso-in-settings.md`. Base: `main` at `dcaae8a5` (anchors
verified there). **Waits for `ADM-11`**, because both change `oidc.rs` JIT provisioning and the
sign-in settings page.

This plan covers **OIDC only**. SAML (owner: built as well) is planned after
[ADR 0017](../../adr/0017-saml-sign-in.md) is decided.

Branch `feat/adm-12-oidc-in-settings`. One PR. Planner and reviewer: Claude Opus. Developer:
another agent.

## 1. Decisions

- **D1. Follow `ADM-7`/`ADM-9` exactly.** That means the one-row settings table, the resolver
  in `runtime_settings.rs`, the `Source` (`server`/`console`/`default`), the sealed secret, the
  30-second cache cleared on save, the 409 `SET_BY_THE_SERVER`, and the audit with changed
  field names only. Reuse their helpers (`secret_field`, `field_json`, `parse_body`,
  `audit_settings_update`, `SettingsCache`). Do not write new ones (rule 4).
- **D2. Migration `NNNN_sso_settings.sql`.** Use the next free number, with a why-header.
  - It creates the singleton `sso_settings`, as in `0128_email_settings.sql`. Its columns:
    - `oidc_issuer TEXT`, `oidc_client_id TEXT`, `oidc_client_secret BYTEA` (sealed);
    - `oidc_provider_name TEXT`, `oidc_groups_claim TEXT`;
    - `oidc_role_map JSONB` (an array of `{group, role}`);
    - `oidc_jit_provisioning BOOLEAN`;
    - `updated_at`, `updated_by`.
  - It also creates `sso_test_run` with:
    - `id UUID PK`, `created_by UUID NOT NULL`;
    - `draft JSONB NOT NULL`, with no secret in it;
    - `draft_secret BYTEA NULL` (sealed);
    - `result JSONB NULL`, `expires_at TIMESTAMPTZ NOT NULL`.
- **D3. `.env` wins as one unit.**
  - If `OIDC_ISSUER` or `OIDC_CLIENT_ID` is set in the environment, the connection is
    server-owned: issuer, client id, secret, and the authorize, token, JWKS and redirect
    URLs. The same applies to `ADM-7` R1 (`runtime_settings.rs:398-529`).
  - Provider name, groups claim, role map and JIT are each server-owned only when their own
    variable is set.
  - **Compose.** Change `OIDC_PROVIDER_NAME:-default`, `OIDC_JIT_PROVISIONING:-false`,
    `OIDC_GROUPS_CLAIM:-groups` and `OIDC_CLOCK_SKEW_SECONDS:-60` (`docker-compose.yml:442-464`)
    to `${X:-}`. Move the defaults into `config.rs` (`:1789-1849`) with `*_from_env` flags, as
    `config.rs:416,424,492,523` do. Otherwise every field would look server-owned.
  - Fix the stale doc comment at `config.rs:561-567`, which says the secret is unused: it is
    sent at `auth.rs:1146` (rule 1).
- **D4. Runtime swap.**
  - `state.oidc` is built once (`state.rs:666-687,776`). Replace it with
    `runtime_settings::oidc(&state) -> Option<Arc<OidcAuthenticator>>`, cached like the
    assistant client (`runtime_settings.rs:947`) and cleared on save.
  - Every reader of `state.oidc` switches: `oidc_start`, `oidc_callback`, `providers`, the
    bearer-JWT path in `auth.rs:63`, and `settings.rs:1118` feature status. The handoff lists
    them.
  - A failure to build, such as discovery failing, logs once and answers as "SSO not
    configured" with a plain message. It never sends the upstream error text to a response
    (principle 4).
- **D5. Who may change it.**
  - Routes `GET/PUT /api/settings/sso` and `POST /api/settings/sso/test`, plus
    `GET /api/settings/sso/test/{id}`, have the floor `RequiresPermission("settings:write")`.
  - `PUT` and both test routes **also** require `is_unrestricted(principal)`
    (`catalog.rs:182`), and return 403 otherwise. The reason: a role map can grant Platform
    Admin.
  - Put a comment at the check that says why it goes beyond the table (`AGENTS.md`, choke
    points).
  - `GET` is readable with `settings:write` and returns `"canEdit": bool`.
  - All rows go into `POLICY_TABLE` and are asserted both ways. Add one `sqlx::test` for the
    403 from a `settings:write`-only caller.
- **D6. Issuer address *(proposed)*.**
  - A console-set issuer must be `https://`. Its host must pass the same internal-address
    check as `check_assistant_address` (`runtime_settings.rs:1047`): refactor to share it,
    not copy it.
  - A `.env` issuer is not checked, which is today's behaviour.
  - Discovery and JWKS fetches for a console-set issuer use the allowlisted resolver /
    `resolve_checked` (`connector_probe.rs:1412`).
- **D7. Role map.**
  - Stored as JSON rows `{group, role}`. A role name must exist when saved (400 naming
    unknown roles).
  - The resolver turns it into the `HashMap` the authenticator takes (`oidc.rs:164-171`).
  - The `.env` form (`config.rs:1609` `parse_role_map`) is unchanged.
- **D8. Test sign-in.**
  - `POST /api/settings/sso/test` takes a draft: the fields in the body, with the secret
    optional. A missing secret means "use the saved one".
    - It validates as for `PUT`, seals the secret into `sso_test_run`, and sets a 10-minute
      expiry *(proposed)*.
    - It returns `{ id, authorizeUrl }`. The authorize URL comes from the existing
      `oidc_start` logic with the draft's settings.
    - The flow cookie `lh_oidc_flow` gets a `test_run_id`.
  - `oidc_callback` (`auth.rs:928-1104`), when the flow carries `test_run_id`:
    - it verifies the token with the draft's settings;
    - it writes `result = { subject, email, emailVerified, groups, mappedRoles, overage }`;
    - it creates **no user, no `auth_identity` and no session**;
    - it writes no audit for a login, only `settings.sso.test`;
    - it redirects to `/admin/settings?section=sign-in&ssoTest=<id>`.
    - An error stores `{ error: "<plain classified message>" }`. A wrong secret gives "The
      provider refused the client ID or secret". Never the raw upstream text.
  - `GET /api/settings/sso/test/{id}` returns the result. Only `created_by` may read it, and
    only before expiry. Anyone else gets 404.
  - Expired rows are deleted when the next test is created. Add no new scheduler.
- **D9. Console.**
  - `sso-page.tsx` becomes the form inside Settings → Sign-in (`settings-page.tsx:133-138`):
    - issuer, client ID, secret (Replace / Clear, as in `email-settings.tsx:187`);
    - provider name, groups claim, JIT checkbox;
    - role map rows with a role `Select` from `identityService.listRoles`.
  - Fields with `source === "server"` are read-only with "Set by the server"
    (`setting-field.tsx:18-33`).
  - "Test sign-in" opens the authorize URL in a new tab. On return it reads `ssoTest` and
    shows the result card.
  - Not `canEdit`: everything is read-only, with a line that a Platform Admin can change it.
  - Contracts: `SsoSettings`, `SsoSettingsInput`, `SsoTestResult` in
    `contracts/settings.ts`, then the client, then the service.

- **D10. Where a connection set in Settings gets its addresses** (planner, 2026-11-11, added
  in review round 1; a gap in this plan, not a developer error). D2 stores an issuer, a client
  ID and a secret, and D3 reads the authorize, token and redirect addresses from `.env` only,
  so a connection saved in Settings had nowhere to send a person.
  - **Authorize, token and key-set addresses** come from the issuer's discovery document
    (`authorization_endpoint`, `token_endpoint`, `jwks_uri`). Extend the discovery read that
    `lakehouse-auth/src/oidc.rs` already has and caches (`SEC-38`); do not write a second
    fetcher (rule 4).
  - The document is remote input. Each address it names must be `https://` and its host must
    pass `resolve_checked` before anything is sent there (D6). One that does not is "SSO is
    not ready", with a plain reason; nothing is fetched from it.
  - **Redirect address:** `<CONSOLE_PUBLIC_URL>/api/auth/oidc/callback` (`config.rs`
    `console_public_url`). Never taken from a request header.
  - **`.env` still wins per value:** a set `OIDC_AUTHORIZE_URL`, `OIDC_TOKEN_URL`,
    `OIDC_JWKS_URL` or `OIDC_REDIRECT_URI` is used as it is today and shows as set by the
    server.
  - **Not ready is said, not hidden.** When a saved connection cannot be used (no
    `CONSOLE_PUBLIC_URL` and no `OIDC_REDIRECT_URI`, discovery unreachable, an address
    refused), `GET /api/settings/sso` returns `"ready": false` with `"notReadyReason"`, a
    plain sentence naming what to set; `GET /api/auth/providers` reports no OIDC, so the
    sign-in page shows no button that would fail; the Settings form shows the reason.
    `PUT` still saves.
  - **A test sign-in uses the draft's own discovery** for all three addresses, never the live
    connection's.

## 2. Anchors (verified at `dcaae8a5`)

- `rust/crates/lakehouse-api/src/config.rs:552-617,1609,1789-1849`.
- `rust/crates/lakehouse-api/src/state.rs:666-687,776`.
- `rust/crates/lakehouse-api/src/routes/auth.rs:63,720-800,830-836,928-1104,1124-1147`.
- `rust/crates/lakehouse-auth/src/oidc.rs:164-171,222,615,749-786,988-1011`.
- `rust/crates/lakehouse-api/src/runtime_settings.rs:79-110,222-270,398-529,947,1047`.
- `rust/crates/lakehouse-api/src/routes/settings.rs:704,761,797,806-822,933-962,1170-1222,1849`.
- `rust/migrations/0128_email_settings.sql`.
- `docker-compose.yml:442-464`.
- `src/features/admin/sso-page.tsx`, `settings-page.tsx:90,133-138`, `setting-field.tsx`,
  `email-settings.tsx`.

## 3. Tasks (one commit each, in order)

- **T1 (D2, D3).** The migration, the store module, the compose and config changes, and
  `resolve_sso` as a pure function.
  - Unit tests:
    - an `.env` issuer makes the connection server-owned;
    - a saved row is used when the environment is empty;
    - the defaults apply when both are empty;
    - the secret opens and fails closed when the key is missing.
  - *Accept:*
    - `cargo test -p lakehouse-api runtime_settings`;
    - `cargo test -p lakehouse-store sso_settings`;
    - `docker compose --profile '*' config --quiet`;
    - a real `docker compose up` of the API profile from a clean project, showing the API
      starts with no `OIDC_*` set (rule 8).
- **T2 (D4, D6, D7).** The runtime swap, the address check refactor, and the role map.
  - Tests:
    - saving a config makes the next `providers` call report OIDC with no restart;
    - an `http://` or internal issuer gives 400;
    - an unknown role in the map gives 400;
    - the existing `.env` OIDC tests still pass.
  - *Accept:* `cargo test -p lakehouse-api auth`, `cargo test -p lakehouse-auth oidc`.
- **T3 (D5, D8).** The routes, the unrestricted rule, the test flow, `route_auth`, and the
  audit.
  - Use a fake OIDC provider in the test, as the existing OIDC tests do.
  - Tests:
    - a test run stores the result and creates no user or session;
    - a wrong secret stores the classified message;
    - another admin reading the result gets 404;
    - an expired run gets 404;
    - a `settings:write`-only caller gets 403 on `PUT`;
    - the audit row holds no secret.
  - *Accept:* `cargo test -p lakehouse-api settings`, `cargo test -p lakehouse-api --test
    route_auth`, `cargo test -p lakehouse-api --lib route_policy_tests`.
- **T4 (D9).** The console.
  - Bun tests:
    - server-owned fields are read-only;
    - the secret is never in a GET payload;
    - the role map rows send `{group, role}`;
    - the test result card renders;
    - the page is read-only without `canEdit`.
  - *Accept:* `bun run typecheck && bun run lint && bun test src/features/admin`.
- **T5.** Docs. `docs/OPERATIONS.md` gets "Setting up single sign-on in Settings", covering
  `.env` precedence and an internal IdP via `.env`. Correct `README.md`'s OIDC section where
  it says `.env` only.

## 4. Out of scope

- SAML (ADR 0017).
- Several providers.
- SCIM.
- The unverified-email linking fix (backlog row from `ADM-11`).

## 5. Handoff (developer appends below)

Give the commits, the exact commands with their counts, the D4 reader list, the compose `up`
evidence, anything *not verified* and why, and the output of `git status --short`, `git log
--oneline origin/main..HEAD` and `git ls-remote --heads origin <branch>`.

## 6. Review (planner)

### Round 1 (planner, 2026-11-11) — branch at `ee51567b`

Read the server side of the diff against `origin/main` (`routes/auth.rs`,
`routes/settings.rs` SSO handlers, `runtime_settings.rs` resolver and authenticator, the store
module, the migration, `lakehouse-auth/oidc.rs`, compose). **Not read this round:** the
console (`sso-page.tsx` and its tests); D10 changes what the form shows, so it is reviewed
after the fix. I ran no check locally; the pull request (#455, draft) runs the suite.

There is no developer handoff. The first session died on 2026-11-10 after T1 and T2; the
second pushed T3 to T5 and I stopped it during its final local run, because the findings
below need a new round anyway. The D4 reader list and the compose `up` evidence are still
owed (S7).

**BLOCKER**

- **B1. A provider set in Settings cannot sign anyone in** (D10; the plan's gap).
  `resolve_sso` fills the authorize, token and redirect addresses from `.env` only
  (`runtime_settings.rs:1329-1359`), and nothing reads `authorization_endpoint` or
  `token_endpoint` from discovery. With no `OIDC_*` variable set and a connection saved in
  Settings, `GET /api/auth/providers` reports OIDC, the sign-in page shows the button, and
  `oidc_start` answers 503 "OIDC_AUTHORIZE_URL is not configured". The tests do not see it
  because `sso_settings_route.rs:139-141` sets `OIDC_TOKEN_URL` and `OIDC_JWKS_URL`.
  - *Accept:* one test with **no `OIDC_*` variable set at all**: save through `PUT`, then
    `oidc_start` redirects to the discovery document's authorization endpoint with the
    redirect address built from `CONSOLE_PUBLIC_URL`, and the callback exchanges the code at
    the discovery's token endpoint and creates the session. A second test: no
    `CONSOLE_PUBLIC_URL` gives `"ready": false`, and `providers` reports no OIDC.
- **B2. The test sign-in talks to the wrong provider.**
  - `post_sso_test` (`routes/settings.rs:2433-2443`) takes the authorize address from the
    **live** connection, or else guesses `<issuer>/authorize`. No common provider serves that
    path (principle 14), and the redirect falls back to a relative `/api/auth/oidc/callback`.
  - `handle_oidc_test_callback` (`routes/auth.rs`) exchanges the code at the **live**
    `token_url`, or at an empty string, and checks the token against the **live** `jwks_url`.
    With a live connection saved, the draft's client secret is sent to a different provider.
  - All three come from the draft's discovery (D10).
- **B3. The address check fails open** (principle 3; D6).
  - In `handle_oidc_test_callback`, `if let Some(..) && let Ok(approved) = resolve_checked(..)`
    leaves `http_client = None` when the check **fails**, and
    `new_with_http_client(config, None, ..)` then fetches with the default client. A refused
    address must store a plain result ("The provider's address is not one this deployment may
    reach") and fetch nothing.
  - In `runtime_settings::oidc` the same `&& let Ok(j_appr)` shape skips the key-set host
    silently when its check fails.
  - The addresses a discovery document names are not checked at all.

**SHOULD-FIX**

- **S1. Every failed code exchange is reported as "The provider refused the client ID or
  secret"**, including a provider that cannot be reached. Tell them apart: refused
  (`invalid_client`), unreachable, anything else. Fixed sentences only, never the provider's
  text.
- **S2. The result lacks `emailVerified` and `overage`** (D8). `emailVerified` is what
  `SEC-65` will need an administrator to see.
- **S3. `sqlx::query_scalar("SELECT name FROM role")` is written twice in
  `routes/settings.rs`.** The store already lists roles (rule 4).
- **S4. A run with no secret falls back to `OIDC_CLIENT_SECRET` in the callback.** A run uses
  only what was sealed into it when it was created.
- **S5. The authenticator is built twice by hand** (pinned client, role map, clock skew): in
  `runtime_settings::oidc` and in `handle_oidc_test_callback`. One function, two callers
  (rule 4). The same for the nonce check in `validate_for_test`: use the check the sign-in
  path uses.
- **S6. D9 gains a line:** the form shows `notReadyReason` when `ready` is false.
- **S7. The next handoff owes** the D4 reader list, the compose `up` evidence (rule 8) and
  every command with its counts.

