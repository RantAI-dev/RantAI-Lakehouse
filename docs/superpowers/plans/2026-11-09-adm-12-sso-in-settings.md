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
| `58eeb91f` | This handoff. |
| `c319f194` | Refinement of B3: pin only the issuer and the discovery document's three addresses (the ones that were checked), not an environment value D10 says is used as it is. |

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

### Handoff (developer, 2026-11-11, round 2 fixes)

Base `01b38fde` (the round 2 review), plus `origin/main` at `49781423` (which
brings `ADM-13`, #454, to `4189c0b1`). Branch `feat/adm-12-oidc-in-settings`,
pushed after every commit.

| Commit | What |
| --- | --- |
| `1c7ecbd8` | Merge `origin/main` into the branch (R2-S4). |
| `31af6c4d` | The authenticator and sign-in readiness are two facts (R2-B1). |
| `312d4796` | The Deployment page names the SSO connection as the console does (R2-B2). |
| `beea6246` | The runtime is built outside the lock; a ready one is kept 10 minutes (R2-S1). |
| `6022c9f8` | Console: `noopener,noreferrer`, `emailVerified`/`overage`, `useService` (R2-S2, R2-S3). |
| this commit | This handoff. |

**Each finding**

- **R2-B1 — closed.** `SwappableOidc` now documents and keeps two facts
  apart. `authenticator` is `Some` whenever the issuer and the client ID are
  set — for a server-owned connection (`.env`) without the three sign-in
  addresses too, which is `main`'s behaviour — and
  `runtime_settings::oidc` (the bearer path in
  `auth::AuthenticatedPrincipal`, and `get_deployment`) reads that one. New
  `SwappableOidc::sign_in_authenticator` is the second fact (ready *and* an
  authenticator) and is what `providers`, `oidc_start`, `oidc_callback` and
  `handle_oidc_test_callback` take; `get_sso` keeps reading
  `not_ready_reason`. A server-owned connection missing an address reports
  `SsoNotReady::ServerAddressesMissing` while keeping its authenticator.
  Tests: `routes::auth::tests::an_issuer_and_a_client_id_alone_report_no_sign_in_but_keep_the_authenticator`
  (`oidc()` is `Some`, `providers` answers `oidc: false`) and
  `routes::auth::tests::a_bearer_token_is_accepted_with_only_an_issuer_and_a_client_id`
  (`#[sqlx::test]`; only `OIDC_ISSUER` and `OIDC_CLIENT_ID` set, a mock
  provider's token, `GET /api/auth/me` → 200). The Deployment-page half is
  `deployment_settings_route::every_feature_is_on_when_its_settings_are_in_place`,
  which passes **unchanged**.
  One interpretation to review: a *console-set* connection with no return
  address still builds nothing, because the redirect gate stays ahead of the
  discovery read in `build_sso_runtime` (round 1 behaviour, and what
  `sso_settings_route::without_a_public_address_the_connection_is_not_ready_and_no_button_shows`
  asserts: it must name `CONSOLE_PUBLIC_URL` before a provider is reached).
  "The address checks passed" in the finding is read as "they were reached
  and passed", which this shape satisfies; the finding's own tests are the
  server-owned ones, which are the ones fixed.
- **R2-B2 — closed.** `every_feature_is_off_and_names_what_is_missing` now
  expects `missing: ["issuer", "clientId"], where: "console"` for
  `singleSignOn`, with the why in the test and the commit (the connection is
  saveable in Settings now, the same shape `consoleAddress` has). Only that
  value changed.
- **R2-S1 — closed.** `cached_or_build` reads the slot under the read lock
  (`cached_runtime`), builds outside any lock, and stores under a short write
  lock; when two requests build at once the newer runtime is kept.
  `is_fresh` keeps a ready runtime for `READY_SSO_RUNTIME_TTL` (10 minutes; a
  named constant whose comment records that the number is proposed in review
  round 2) and a not-ready one for `SETTINGS_CACHE_TTL` (30 s). `put_sso`'s
  clearing is unchanged. Tests, one per behaviour:
  `runtime_settings::tests::a_ready_runtime_is_kept_for_ten_minutes` (aged
  past 30 s: the same authenticator `Arc` and the same `built_at` come back),
  `a_not_ready_runtime_is_retried_after_the_settings_ttl` (rebuilt, still not
  ready, authenticator still present) and `a_save_clears_a_ready_runtime_at_once`
  (the save's two invalidation lines, a different authenticator afterwards).
- **R2-S2 — closed.** `window.open(url, "_blank", "noopener,noreferrer")` in
  `sso-page.tsx`; test
  `sso-page.test.tsx::opens the provider's page without an opener and without a referrer`.
- **R2-S3 — closed.** The card shows `Email verified` and `Group overage`
  (the second says "no group-based roles were applied" when true) and
  "Not reported" for a claim the provider did not send; `SsoTestResult` in
  `src/services/contracts/settings.ts` carries both. The result is read with
  `useService` on `[ssoTestId]`, not an effect. Tests:
  `the test result card shows emailVerified and overage` and
  `the test result card says a claim the provider did not send was not reported`.
- **R2-S4 — closed.** See the merge below.

**The merge (R2-S4).** `git merge origin/main` was hunk-clean: no conflict
markers, no `--ours`/`--theirs`. Both sides only add in the two files
`ADM-13` touched, so every hunk kept both: `policy.rs` has the eight SSO
rows and the ten `/api/ai/*` `RequiresAnyPermission` rows side by side, and
`tests/route_auth.rs` keeps the ADM-13 assistant/pages tests beside the
ADM-12 SSO test (87 tests). The merge commit's message records this. Migration
`0134_sso_settings.sql` is unchanged; `main` ends at `0133_pages_per_role.sql`.
The merged tree compiled (`check --all-targets`) and
`--lib route_policy_tests` passed before the fix commits.

**Commands run, with counts** (scoped; `cargo-locked`, from `rust/`,
`sg docker` for the ones that need containers).

- `cargo fmt --check`: clean (after every Rust commit).
- `cargo-locked check -p lakehouse-api --all-targets`: clean, after the merge
  and after the R2-B1 and R2-S1 edits.
- `cargo-locked clippy -p lakehouse-auth -p lakehouse-api --all-targets -- -D warnings`:
  clean (two runs; the first caught a `doc_markdown` backtick and an
  `unchecked_time_subtraction` in the new tests, both fixed in place).
- `--test route_auth`: 87 passed (after the merge; re-run after R2-B1/S1: 87).
- `--test sso_settings_route`: 5 passed (after the merge, after R2-B1, after
  R2-S1).
- `--lib route_policy_tests`: 2 passed (after the merge).
- `--test deployment_settings_route`: 6 passed + 1 failed (the R2-B2
  expectation, before it was changed), then 7 passed.
- `--lib routes::auth`: 88 passed (after R2-B1); `--lib routes::auth::tests`:
  29 passed (after R2-S1).
- `--lib routes::settings::`: 23 passed; `--lib auth::tests`: 56 passed;
  `--lib runtime_settings`: 27 passed, then 30 passed with the three R2-S1
  tests.
- `--test sec11_guard`: 4 passed.
- `bun install --frozen-lockfile`: 678 packages, lockfile unchanged.
- `bun run typecheck`: clean. `bun run lint`: 0 errors, 11 warnings (all
  pre-existing, elsewhere).
- `bun test src/features/admin`: 106 passed (was 103);
  `bun test src/features/admin/sso-page.test.tsx`: 10 passed.

**Compose.** Not changed this round (it was in round 1), so no `up` was run
for it. Nothing this round touches `docker-compose.yml`.

**Not verified locally: the rest, left to CI.** The full workspace
`cargo test --workspace`, `cargo test --doc`, the other `lakehouse-api`
integration binaries, the console's other suites, and `docker compose config`
were not run: no compose edit this round and AGENTS.md scopes checks to what
a commit touched.

`git status --short`: empty. `git ls-remote --heads origin
feat/adm-12-oidc-in-settings`:
`6022c9f8918653c218cef1abfd03e51f29acce8b	refs/heads/feat/adm-12-oidc-in-settings`.


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

### Round 2 (planner, 2026-11-11) — branch at `237aca35`

Read the round 1 fixes on the server side (`runtime_settings.rs` `build_sso_runtime`,
`cached_or_build`, `draft_sso_settings`, `pinned_client`; `routes/auth.rs`
`handle_oidc_test_callback`; `state.rs`), skimmed the console (`sso-page.tsx`, its tests, the
client), and read CI on #455. I ran no check locally.

**Round 1, as the code stands**

| Finding | State | Where |
| --- | --- | --- |
| B1 | Closed for a connection set in Settings; it broke a connection set in `.env` (R2-B1) | `build_sso_runtime` reads the three addresses from discovery, the redirect from `CONSOLE_PUBLIC_URL`; `ready`/`notReadyReason` |
| B2 | Closed | `sso_runtime_for_settings(draft_sso_settings(..))`, never the cache |
| B3 | Closed | Issuer and each discovery address pass `https://` and `resolve_checked` before an authenticator exists; a refusal builds none |
| S1, S2, S4, S5 | Closed | Fixed sentences per cause; the run's own sealed secret; one builder |
| S3, S6, S7 | Closed | Store role list; the form shows the reason; handoff with the reader list and the compose `up` |

The developer's reported mismatch is accepted: a console issuer must be `https://`, the mock
provider is plain, so the end-to-end tests are unit tests in `routes::auth` behind
`tls_proxy::HttpsFront`, with `AppState::sso_http_override` behind `#[cfg(test)]`. The address
checks still run; a service build cannot set the override.

**BLOCKER**

- **R2-B1. A connection set in `.env` lost its authenticator** (CI:
  `deployment_settings_route::every_feature_is_on_when_its_settings_are_in_place`). On `main`
  the authenticator exists whenever `OIDC_ISSUER` and `OIDC_CLIENT_ID` are set. Now
  `build_sso_runtime` returns none for a server-owned connection that lacks
  `OIDC_AUTHORIZE_URL`, `OIDC_TOKEN_URL` or `OIDC_REDIRECT_URI`. Two things read it:
  - `src/auth.rs:98`, the bearer-token path: a deployment that sets only the issuer and the
    client ID to accept bearer tokens now refuses every one of them;
  - `routes/settings.rs:1167`, the Deployment page: it reports single sign-on off and names
    `OIDC_ISSUER` and `OIDC_CLIENT_ID` as missing while both are set.

  Two facts, kept apart:
  - **the authenticator** exists whenever the issuer and the client ID are set (and, for a
    console-set connection, the address checks passed), as on `main`;
  - **ready for sign-in** additionally needs the authorize, token and redirect addresses.
    `providers`, `oidc_start` and `GET /api/settings/sso` use this one.

  The Deployment page keeps its meaning ("its settings are in place": issuer and client ID).
  Tests: with only `OIDC_ISSUER` and `OIDC_CLIENT_ID` set, a bearer token is still accepted,
  the Deployment page says on, and `providers` reports no OIDC.
- **R2-B2. CI is red on two tests** of `tests/deployment_settings_route.rs`.
  - `every_feature_is_on_when_its_settings_are_in_place`: R2-B1. The test does not change.
  - `every_feature_is_off_and_names_what_is_missing` expects `missing: ["OIDC_ISSUER",
    "OIDC_CLIENT_ID"], where: "server"` and gets `["issuer", "clientId"], "console"`. The new
    answer is the right one now that the connection can be set in Settings (the same shape
    `consoleAddress` has). Change the expectation and say why in the commit.

**SHOULD-FIX**

- **R2-S1. A working connection is rebuilt every 30 seconds, under the write lock.**
  `cached_or_build` drops a ready runtime after `SETTINGS_CACHE_TTL` and rebuilds it while
  holding `state.oidc`'s write lock: a discovery request (10 s timeout), four address
  look-ups and a new key-set cache. The bearer-token path waits on that lock. Do the network
  work outside the lock, and keep a ready runtime for 10 minutes *(proposed)*; a not-ready one
  is still retried after 30 seconds.
- **R2-S2. `window.open(res.authorizeUrl, "_blank")`** (`sso-page.tsx:199`) gives the page
  being tested a handle on the console's tab. Pass `"noopener,noreferrer"`.
- **R2-S3. The result card** shows `emailVerified` and `overage`, and the result is loaded
  with `useService` like the rest of the form, not a bare call in an effect
  (`sso-page.tsx:113`).
- **R2-S4. Merge `origin/main`.** `ADM-13` (#454) changed `policy.rs` and
  `tests/route_auth.rs`; resolve hunk by hunk. The migration stays `0134`.

### Round 3 (planner, 2026-11-11) — branch at `0744cc34`

Read the round 2 fixes (`1c7ecbd8..0744cc34`): `runtime_settings.rs` (`SwappableOidc`,
`sign_in_authenticator`, `cached_or_build`, `is_fresh`, `build_sso_runtime`), the three call
sites in `routes/auth.rs`, and the test change in `deployment_settings_route.rs`. I ran no
check locally; #455 runs the suite.

| Finding | State | Where |
| --- | --- | --- |
| R2-B1 | Closed | The authenticator exists for issuer and client ID; `sign_in_authenticator()` is the ready fact. Tests: bearer token accepted with only the two variables; `providers` reports no OIDC; the Deployment test passes unchanged |
| R2-B2 | Closed | Only the expected `singleSignOn` value changed, with the reason |
| R2-S1 | Closed for a connection that can sign in; see R3-S1 | Build outside the lock; ready kept ten minutes *(proposed)*; not ready retried after 30 seconds |
| R2-S2, R2-S3 | Closed | `noopener,noreferrer`; `emailVerified` and `overage` on the card; `useService` |
| R2-S4 | Closed | `main` merged; migration stays `0134` |

The developer's flagged reading is accepted: a connection set in Settings with no
`CONSOLE_PUBLIC_URL` builds nothing, and says so in `notReadyReason`. Its only use is
sign-in.

**No open BLOCKER.**

**SHOULD-FIX**

- **R3-S1. A bearer-only `.env` connection is rebuilt every 30 seconds.** `is_fresh` gives ten
  minutes only to a runtime that is ready for sign-in. A server-owned connection with an
  issuer and a client ID but no sign-in addresses has an authenticator and `is_ready()` false,
  so it gets `SETTINGS_CACHE_TTL` and a new authenticator, with an empty key-set cache, every
  30 seconds. On `main` that authenticator is built once at start-up. A server-owned
  connection cannot change while the process runs: keep its runtime for the life of the
  process (or until a save clears the slot), ready or not. A test: an aged server-owned
  runtime without sign-in addresses is reused, same authenticator.

`origin/main` (`01d507c8`: WEB-7 S1 #461 and two docs PRs) is merged in the commit that
carries this note; no conflict.

