# Staging signs in through Keycloak

Deployment task (no product behaviour change). One PR. Base: `origin/main` at
`b91105e8cea89051f9730f399c0b04dbaae83821`, verified 2026-11-17.
Requester: the product owner, 2026-11-17 ("apakah bisa kamu setupkan SSO
providenya di staging? untuk sso itu pakai Keycloak, jadi nanti di rantai
lakehouse ini disambungkan Keycloaknya").

## Problem

The deployment can already sign people in through an OIDC provider
(`SEC-3`, `ADM-12`), and the repository already contains a **real** Keycloak
for the automated gate (`ops/sso/docker-compose.keycloak.override.yml`,
`ops/g10/g10_sso_test.py`). Nothing in it was wired to a **deployment**: the
staging stack had every `OIDC_*` variable empty, so the console showed no
SSO button and the API answered `503` on `/api/auth/oidc/start`.

Verified anchors at the base above:

| Site | Observation |
| --- | --- |
| `rust/crates/lakehouse-api/src/runtime_settings.rs:1317` | `server_owns_connection` — the environment owns the whole connection when `OIDC_ISSUER` or `OIDC_CLIENT_ID` is set; otherwise the console's saved row is used. |
| `rust/crates/lakehouse-api/src/routes/settings.rs:2069` | A console-saved issuer must pass `check_issuer_address`: `https://` only, and it must resolve outside the deployment's network. A provider on the office LAN over plain HTTP cannot be saved in the console. |
| `rust/crates/lakehouse-api/src/runtime_settings.rs:1131` | The same check, shared with the assistant's address. |
| `rust/crates/lakehouse-api/src/routes/auth.rs:838` | The flow cookie is `Path=/api/auth/oidc`, `SameSite=Lax` — so the callback must sit under `/api/auth/oidc` and be a top-level navigation. |
| `rust/crates/lakehouse-api/src/routes/auth.rs:78` | The session cookie is set on the origin the callback answered on. |
| `rust/crates/lakehouse-auth/src/oidc.rs:496` | The discovery document is trusted only when its `issuer` equals the configured issuer byte for byte. |
| `src/features/auth/login-page.tsx:43` | The SSO button is behind the build-time `NEXT_PUBLIC_SSO_ENABLED` flag. |
| `next.config.ts` (`rewrites()`) | The console proxies `/api/*` to the API, so the browser stays on the console's origin. |
| `.github/workflows/deploy-staging.yml` | The staging deploy writes `.env` from `.env.example` + `.env.staging` + repository secrets, then `scripts/compose.sh … up -d --build --remove-orphans`. |
| `docker-compose.yml:442-469` | The API's `OIDC_*` are already passed through from `.env` (`${OIDC_ISSUER:-}` and friends) — no compose change is needed for the API's side. |

## Decisions

1. **The provider runs in the staging stack, in its own compose file.**
   `ops/sso/docker-compose.staging-keycloak.override.yml` (profile `sso`)
   adds `keycloak`, `keycloak-db-init` and `keycloak-realm-init`. A second
   file, not the base `docker-compose.yml`: the gate's override is a CI-only
   stack that publishes **no** host port and runs `start-dev` with an
   in-container H2 database, and staging wants the opposite of all three
   (published port, Postgres-backed realm, `start`). The gate's file is not
   touched, so the `g10-sso` CI job keeps its own context.
2. **The connection lives in `.env`, written by the deploy workflow.** The
   console's SSO form refuses an `http://` issuer on a private address
   (anchor 2), so an internal provider cannot be saved there — the
   documented path is the environment ("Internal IdP via `.env`" in
   `docs/OPERATIONS.md`).
3. **One address for the browser and the API.** The issuer is one string
   (anchor 6), and the browser has to reach it, so the provider publishes a
   host port and the API reaches the same published address. `KC_HOSTNAME`
   carries it.
4. **The callback returns to the console, not to the API.** The session
   cookie is set on the answering origin (anchor 5), and the console proxies
   `/api/*` (anchor 8), so the callback is the console's own address and the
   person stays signed in on the origin they signed in on.
5. **The address is not in the repository** (`AGENTS.md` rule 12): it is the
   `staging` environment's variable `SSO_PUBLIC_BASE_URL` (plus
   `SSO_CONSOLE_URL` and `SSO_REALM`). The credentials are repository
   secrets (`STAGING_KEYCLOAK_ADMIN_PASSWORD`, `STAGING_OIDC_CLIENT_SECRET`,
   `STAGING_SSO_USER_{A,B,C}_PASSWORD`), and the realm renderer injects the
   client secret and the users' passwords into the imported realm.
6. **The SSO button's build-time flag is read from `.env`**, so the
   standalone frontend image and the compose `console` service agree on one
   value (`NEXT_PUBLIC_SSO_ENABLED=true` in `.env.staging`).

## What changed

| File | Change |
| --- | --- |
| `ops/sso/docker-compose.staging-keycloak.override.yml` | New: the provider, its database initialiser and its realm renderer. |
| `ops/sso/staging-realm.template.json` | New: the realm (client, two groups, three users) with the secret, the passwords and the callback left empty. |
| `ops/sso/render_staging_realm.py` | New: fills those three values in, failing closed when one is missing. |
| `.github/workflows/deploy-staging.yml` | Writes the whole connection into `.env`, starts the `sso` profile, waits for the discovery document, and builds the frontend with the flag. |
| `.env.staging` | `NEXT_PUBLIC_SSO_ENABLED=true`. |
| `docker-compose.yml` | The `console` service's build takes `NEXT_PUBLIC_SSO_ENABLED` as a build arg, like `RUST_API_URL`. |
| `docs/OPERATIONS.md` | "Keycloak on the staging deployment": what runs, where the address comes from, the settings, how to verify, the limits. |
| `CHANGELOG.md` | The entry. |

## Verification

Run on the staging VM (the self-hosted runner's host), against the stack
itself, 2026-11-17:

1. `docker compose -f docker-compose.yml -f ops/sso/docker-compose.staging-keycloak.override.yml --profile sso --profile seaweedfs --profile dagster --profile trino config --quiet` → exit 0.
2. `docker compose … --profile sso up -d keycloak` → the realm renderer wrote `/import/lakehouse-realm.json`, the database initialiser created the `keycloak` database, and Keycloak logged `Realm 'lakehouse' imported` and `Keycloak 26.8.1 … started in 38.107s`.
3. The discovery document answers on the published address and names the same issuer, authorize, token and JWKS addresses the API is configured with.
4. `GET /api/auth/oidc/start` answers `302` to Keycloak's authorize endpoint with `code_challenge_method=S256` and the console's callback as `redirect_uri`.
5. The full browser path, driven as a browser drives it (console origin → Keycloak's login form → callback → `GET /api/auth/me`):
   - the user in the group `analysts` → the Analyst role's 7 permissions (`query:read`, `catalog:read`, `lineage:read`, `assistant:use`, `ops:read`, `alert:read`, `governance:read`);
   - the user in no group → **0** permissions;
   - the user in the group `lakehouse-admins` → the admin wildcard `*:*`.
   Each sign-in created its account on first use (`OIDC_JIT_PROVISIONING=true`).
6. The build-time flag, A/B on the two frontend images: the bundle of the image built **without** the flag contains no `Continue with SSO` at all (the branch is dead-code eliminated); the bundle built **with** it does.

**Not verified:** the automated `g10-sso` CI job was not re-run (the gate's
own compose file and realm are untouched by this change); the browser UI was
not clicked through by hand (no browser attached to the session), so step 5
was driven by a script that follows the same redirects a browser follows,
and step 6 is a bundle inspection rather than a screenshot.

## Limits, honestly

- The provider speaks plain HTTP on the office LAN (`KC_HTTP_ENABLED`):
  the staging deployment has no TLS terminator. The issuer is an `http://`
  address, which is why the connection is in `.env` and not in the console's
  form.
- The provider's admin console (`{base}/admin`) is reachable from the LAN.
- `--import-realm` fills only an **empty** realm: a realm an administrator
  has changed is not overwritten by a later deploy, and a realm that is
  deleted is re-imported.
- Changing `SSO_PUBLIC_BASE_URL` breaks every sign-in until the provider's
  hostname, the issuer and the registered callback move together.
- The console's SSO form shows this connection as **set by the server** and
  refuses to change it (409) — the environment owns it, by design.
