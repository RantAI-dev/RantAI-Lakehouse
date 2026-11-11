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

### Handoff (developer, 2026-11-11, round 1 fixes)

Base `e606e2c3`. Branch `feat/adm-12-oidc-in-settings`, pushed after every commit.

| Commit | What |
| --- | --- |
| `01ffda16` | `lakehouse-auth`: the cached discovery read also returns `authorization_endpoint` and `token_endpoint` (D10 half of B1/B2, S5). |
| `ddce0a6e` | One SSO runtime builder with a pinned client; addresses from discovery; `ready`/`notReadyReason`; `providers` hides a connection that cannot be used (B1, B3, S5). |
| `af392fb5` | The test sign-in uses the draft's own discovery; classified exchange failures; `emailVerified`/`overage`; a run's own sealed secret only (B2, S1, S2, S4, S5's nonce check). |
| `d3449193` | Role names from `lakehouse_store::identity::list_roles` (S3). |
| `7dc10317` | Console: `ready`/`notReadyReason` in the contract and the reason on the form (S6). |
| `8c74fbd7` | `docs/OPERATIONS.md`: `CONSOLE_PUBLIC_URL`, and the addresses from the discovery document (T5/D10). |
| this commit | This handoff. |

**Each finding**

- **B1 — closed.** `resolve_sso` still merges the connection (D3), and
  `runtime_settings::build_sso_runtime` now fills the authorize, token, key-set
  and redirect addresses: `.env` wins per value, else the provider's discovery
  document, and the redirect is `<CONSOLE_PUBLIC_URL>/api/auth/oidc/callback`.
  `sso_runtime`/`sso`/`oidc` are the one path the live routes and the test path
  use. `GET /api/settings/sso` answers `ready` and `notReadyReason`;
  `GET /api/auth/providers` answers `oidc: false` when not ready. `PUT` saves
  regardless. Tests: `routes::auth::tests::a_connection_saved_in_settings_signs_in_with_no_oidc_env`
  (no `OIDC_*` at all; `PUT`, `oidc_start` → the discovery document's
  authorization endpoint with the `CONSOLE_PUBLIC_URL` callback, `oidc_callback`
  → the discovery token endpoint, session created, JIT identity created) and
  `tests/sso_settings_route.rs::without_a_public_address_the_connection_is_not_ready_and_no_button_shows`
  (`PUT` → `ready:false` naming `CONSOLE_PUBLIC_URL`, GET the same,
  `providers.oidc == false`).
- **B2 — closed.** `post_sso_test` and `handle_oidc_test_callback` both build a
  runtime from the run's stored draft (`runtime_settings::draft_sso_settings` +
  `sso_runtime_for_settings`, the same builder as the live path): authorize,
  token and key set from the draft's own discovery document; no guessed
  `<issuer>/authorize`, no live connection, no relative redirect.
  `routes::auth::tests::a_draft_whose_provider_differs_sends_nothing_to_the_saved_one`
  asserts the saved provider receives **zero** requests (not even discovery) and
  the exchange happens at the draft's `/token`.
- **B3 — closed.** `build_sso_runtime` checks the issuer, then every address the
  discovery document names, with `check_external_https_address` (https +
  `resolve_checked`) **before** building any authenticator; a refusal answers a
  fixed `SsoNotReady` sentence and no client is exposed, so nothing is fetched.
  The `if let … && let Ok(…)` shapes that left `http_client = None` are gone
  from both paths. The discovery-named addresses are now checked (they were not
  before).
- **S1 — closed.** `exchange_code_for_id_token` returns
  `ExchangeFailure::{Refused, Unreachable, Other}` (401/403 or
  `error: invalid_client` = refused; a request that never completed =
  unreachable; anything else = other) and each has one fixed sentence. The
  provider's status/body is compared, never stored, logged as text, or returned.
  Test: `a_test_sign_in_that_cannot_reach_the_provider_says_so` stores
  "The identity provider could not be reached."
- **S2 — closed.** `OidcTestClaims`/the stored result carry `emailVerified` and
  `overage`. Tests: `a_test_sign_in_reports_the_draft_providers_claims_and_creates_nothing`
  (`email_verified: true` → `emailVerified: true`, `overage: false`) and
  `a_test_sign_in_with_a_group_overage_says_so` (`overage: true`, no mapped
  roles).
- **S3 — closed.** Both `SELECT name FROM role` queries are gone;
  `put_sso`/`post_sso_test` take the names from
  `lakehouse_store::identity::list_roles`.
- **S4 — closed.** The callback opens only `run.draft_secret`; there is no
  `OIDC_CLIENT_SECRET` fallback. `post_sso_test` still seals the saved (or
  environment) secret into the run when the draft omits one (D8's "use the saved
  one"), which is the run's own sealed secret from then on. Test:
  `a_test_run_with_no_secret_sends_no_client_secret` (`clientSecret: null` with
  `OIDC_CLIENT_SECRET=env-secret`: the token request has no `client_secret`).
- **S5 — closed.** One builder (`build_sso_runtime`) makes the authenticator and
  its client for the live and test paths, and one `check_nonce` is used by
  `authenticate_with_nonce` and `validate_for_test`.
- **S6 — closed.** `SsoSettings` carries `ready`/`notReadyReason`; the form
  shows the reason in a `role="status"` notice and still saves. Test:
  `sso-page.test.tsx::shows the reason when the saved connection is not ready`.
- **S7 — closed** by this section.

**The D4 reader list** (every place that reads the authenticator or the SSO
settings, with file and function):

- `rust/crates/lakehouse-api/src/runtime_settings.rs` — the definitions:
  `resolve_sso` (pure merge), `sso_row`, `sso_or_fail`, `cached_or_build`,
  `sso_runtime_or_fail`, `sso_runtime`, `sso`, `oidc`, `build_sso_runtime`,
  `sso_runtime_for_settings`, `draft_sso_settings`.
- `rust/crates/lakehouse-api/src/auth.rs` — `AuthenticatedPrincipal::from_request_parts`
  (the bearer-JWT branch) → `runtime_settings::oidc(state)`.
- `rust/crates/lakehouse-api/src/routes/auth.rs` — `oidc_start` →
  `sso_runtime`; `providers` → `sso_runtime`; `oidc_callback` → `oidc` then
  `sso_runtime`; `handle_oidc_test_callback` → `sso_runtime_for_settings`.
- `rust/crates/lakehouse-api/src/routes/settings.rs` — `get_sso` →
  `sso_runtime_or_fail`; `put_sso` → `sso_or_fail` (to know which fields the
  environment owns) then `sso_runtime_or_fail` after the save; `post_sso_test` →
  `sso_runtime_for_settings`; `get_deployment` → `sso_or_fail` + `oidc`.
- `rust/crates/lakehouse-api/src/state.rs` — `AppState::new` starts the cache as
  `None` (the runtime is built on the first request, because its checks need the
  resolved config and the pool); `AppState::sso_http_client` is the test seam.
- `rust/crates/lakehouse-api/src/routes/settings.rs::put_sso` also clears
  `state.oidc` on save.

**The test seam for the mock provider (and the mismatch with the plan's hint).**
`CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS=true` alone is not enough: a console-set
issuer must be `https://` (D6), `wiremock` serves plaintext only, and the API's
`reqwest` is built with `rustls-tls` (`webpki-roots`), so it trusts no locally
minted certificate. Weakening either check to make the test reach a plain mock
was not an option. The seam used:

1. `src/tls_proxy.rs::HttpsFront` (test-only): a TLS front in front of
   `wiremock`, presenting a certificate for `127.0.0.1` signed by a CA minted per
   test (`rcgen`, already a dev-dependency).
2. `AppState::sso_http_override` (`#[cfg(test)]`, like `webhook_sender_override`):
   a client that trusts that CA. `build_sso_runtime` uses it instead of the
   pinned client; the `https://` check and `resolve_checked` still run on every
   address, and the redirect is still built from `CONSOLE_PUBLIC_URL`. Only the
   DNS pins are replaced, and only in this crate's test build (a service build
   has neither the field nor the accessor's `Some` branch).

Consequence to review: integration tests link the library **without**
`cfg(test)`, so the B1 end-to-end test and the draft tests (B2/S1/S2/S4) live in
`routes::auth`'s unit tests, not in `tests/sso_settings_route.rs`. That file keeps
the 403 matrix, the readiness test, the 404/expiry test and the audit test; its
two old draft tests were removed because they reached the provider through
`OIDC_TOKEN_URL`/`OIDC_JWKS_URL` in `.env` — exactly what hid B1. The old
`saving_a_config_makes_the_next_providers_call_report_oidc_with_no_restart`
asserted `oidc: true` for an unreachable configuration, which is the defect B1
describes; it now uses the same `https` mock and asserts the discovery document
was read.

**Compose evidence.** `docker-compose.yml` is changed by this branch (T1/D3), so:

- `docker compose --profile '*' config --quiet` → exit 0 (with throwaway
  `POSTGRES_PASSWORD`, `RUSTFS_ACCESS_KEY`, `RUSTFS_SECRET_KEY`,
  `LAKEKEEPER_ENCRYPTION_KEY`, `CONNECTOR_SECRETS_KEY`; no `.env` written).
- `docker compose -p adm12r1 build lakehouse-api` → built
  `rantai-lakehouse-api:local` from this branch (cache hits for the dependency
  layer), and `build oidc-mock`.
- `docker compose -p adm12r1 up -d --no-build lakehouse-api` (host ports moved
  to 18080/18123/19000/15432 because another developer's stack holds the
  defaults) → `adm12r1-lakehouse-api-1 … Up (healthy)`; log line
  `{"message":"lakehouse-api listening","addr":"0.0.0.0:8080"}`; migrations
  129–134 applied.
- `curl http://127.0.0.1:18080/api/auth/providers` →
  `{"oidc":false,"providerName":null}`, `GET /health` → 200.
- `docker exec … env | grep '^OIDC_'` shows the twelve names compose passes,
  **all empty** (no `OIDC_*` value set).
- `docker compose -p adm12r1 down -v --remove-orphans` → volumes and network
  removed; `docker ps -a --filter name=adm12r1` → nothing.

**Commands run, with counts.**

- `cargo fmt --check` (after every Rust commit): clean.
- `cargo clippy -p lakehouse-auth -p lakehouse-api --all-targets -- -D warnings`:
  clean.
- `cargo test -p lakehouse-auth oidc`: 8 unit tests passed; `--test oidc discovery`
  5 passed; `--test oidc no_sign_in_endpoints` 1 passed.
- `cargo test -p lakehouse-api --lib runtime_settings`: 27 passed.
- `cargo test -p lakehouse-api --lib routes::auth`: 86 passed.
- `cargo test -p lakehouse-api --lib routes::settings`: 23 passed.
- `cargo test -p lakehouse-api --test sso_settings_route`: 5 passed.
- `cargo test -p lakehouse-api --test route_auth`: 84 passed.
- `cargo test -p lakehouse-api --test sec11_guard`: 4 passed.
- `bun install --frozen-lockfile`: 678 packages, lockfile unchanged.
- `bun run typecheck`: clean.
- `bun run lint`: 0 errors, 11 warnings (all pre-existing, elsewhere).
- `bun test src/features/admin`: 103 passed, 0 failed. (One whole-suite run had
  `sign-in-settings.test.tsx` time out in `waitFor`; it passes alone
  (4 passed) and the suite re-run is 103/0. Not a change in this branch.)

**Not verified locally: the rest, left to CI.** The full workspace
`cargo test --workspace`, `cargo test --doc`, the other `lakehouse-api`
integration binaries, and the console's other suites were not run (AGENTS.md:
scoped checks per commit; CI runs the full suite on the PR).


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

