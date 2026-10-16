# `SEC-3` Single sign-on against real identity providers: plan

Planner: Claude Opus (Security & Delivery lane). Developer: a non-Opus agent.
Base: `origin/main` at the commit this branch was cut from. Branch:
`feat/sec-3-sso-real-idp`, worktree `/home/shiro/rantai/wt/sec-3`. Spec:
`docs/core/specs/sec-3.md` (D1–D3 signed 2026-10-16). A Task.

## 1. Anchors (verify each; line numbers drift)

- `rust/crates/lakehouse-api/src/routes/auth.rs`: `oidc_start` (~683),
  `oidc_callback` (~865), the code exchange sending `client_secret` when
  `OIDC_CLIENT_SECRET` is set (~1077); unit tests from ~1212.
- `rust/crates/lakehouse-api/src/config.rs`: `OIDC_ISSUER`,
  `OIDC_CLIENT_ID`, `OIDC_CLIENT_SECRET`, `OIDC_AUTHORIZE_URL`,
  `OIDC_TOKEN_URL`, `OIDC_REDIRECT_URI`, `OIDC_JWKS_URL` (default
  `{OIDC_ISSUER}/.well-known/jwks.json`), `OIDC_GROUPS_CLAIM` (default
  `groups`), `OIDC_ROLE_MAP`, `OIDC_JIT_PROVISIONING`,
  `OIDC_CLOCK_SKEW_SECONDS`, `OIDC_PROVIDER_NAME`.
- `rust/crates/lakehouse-auth/` (`OidcAuthenticator`, README).
- `ops/g4/g4_test.py` `step_oidc_authorization_code_round_trip` (~601):
  the existing round trip against `oidc-mock`; reuse its shape.
- `ops/g6/docker-compose.g6.override.yml`,
  `ops/backups/docker-compose.backups.override.yml`: the precedent for a
  gate-only compose override passed with `-f`.

## 2. Decisions

- E1. Keycloak runs only in a gate override,
  `ops/sso/docker-compose.keycloak.override.yml`, never in the default
  compose file. Image pinned by version and digest (the newest stable
  Keycloak 26.x; quote where the digest came from). Started in `dev`
  mode is acceptable for a gate; say so in the header.
- E2. The realm is imported from a JSON file under `ops/sso/`: one client
  (confidential, standard flow, PKCE S256 required, redirect URI the API's
  callback), a `groups` mapper emitting group names into `groups`, two
  users, one in a group mapped by `OIDC_ROLE_MAP`, one in none. No literal
  password or client secret in the repository: the gate generates them at
  run time and imports them (rule 5; gitleaks must stay clean).
- E3. The gate is a new `ops/g*` directory following the gate rules in
  `docs/CODE-STANDARD.md` §5 (readiness polling, one line per step,
  `PASS` / `FAILED: <reason>`, non-zero exit). Steps: discovery reachable;
  sign in user A through the real Keycloak login form (an HTTP client
  posting the form, as G4 does with the mock), session cookie set,
  `/api/auth/me` shows the mapped role; user B signs in and gets no role
  beyond the default; a token for another audience is refused; an expired
  token is refused (Keycloak access-token lifespan set short for the gate).
- E4. A CI job runs the gate on pull requests that touch `rust/crates/lakehouse-api/src/routes/auth.rs`,
  `rust/crates/lakehouse-auth/**`, `ops/sso/**` or the gate itself, in the
  same pattern as the other gate jobs in `.github/workflows/ci.yml`
  (change scopes); its `.env` sets CI-only throwaway values only.
- E5. The vendors (D2) are not automated. The plan prepares, for each of
  Entra ID and Google: the exact settings (issuer, JWKS URL, authorize and
  token URLs, groups claim, redirect URI), the steps the product owner
  follows in the vendor's console, and an empty results table the owner
  fills in. Known differences to state, verified against the vendor's
  current documentation (quote the URL): Entra ID's issuer is per tenant
  (`https://login.microsoftonline.com/<tenant>/v2.0`), its keys are at the
  `jwks_uri` of its discovery document, and its `groups` claim carries
  object ids (so `OIDC_ROLE_MAP` names ids), with an overage case when a
  user is in many groups; Google's issuer is `https://accounts.google.com`,
  its keys are at its discovery `jwks_uri`, and it sends no groups claim
  (so roles come from JIT provisioning defaults or an admin, not from the
  token). Neither vendor serves keys at `{issuer}/.well-known/jwks.json`,
  so `OIDC_JWKS_URL` must be set for both: verify, and if the console
  should read `jwks_uri` from discovery instead, report it as a finding;
  do not change the Rust code in this plan.
- E6. No Rust change unless the Keycloak gate exposes a defect. If it
  does, stop and report it with the failing step; the planner decides.

## 3. Tasks

**T1. Keycloak override and realm (E1, E2).** *Check:*
`docker compose -f docker-compose.yml -f ops/sso/docker-compose.keycloak.override.yml --profile '*' config --quiet`
with an `ops/init-env.sh` `.env`; gitleaks `dir` clean apart from `.env`.

**T2. The gate (E3), proven with a real run** (rule 8) under the heavy
lock, project `sec3sso`, ports remapped, `down -v` at the end via `trap`.
*Check:* the gate's full output quoted, `PASS`.

**T3. CI job (E4).** *Check:* YAML parses; the job's change scope names
the paths in E4. The planner opens a draft PR so CI runs it.

**T4. Vendor runbooks (E5) and docs.** `docs/OPERATIONS.md` single sign-on
section: Keycloak, Entra ID, Google, each with settings, steps, limits;
`docs/plans/SEC-3-RESULT.md` with the Keycloak gate's result and two
empty vendor tables (date, settings without secrets, outcome, notes) for
the owner. `CHANGELOG.md` one bullet. *Check:* every setting named exists
in `config.rs`.

## 4. Pull request

One slice. The vendor results (AC2, AC3) are filled in by the product
owner after merge; the item is Done when they are.

## 5. Out of scope

SAML, SCIM, OIDC sign-in throttling, any Rust change (E6), changing
`oidc-mock` or G4.

## 6. Handoff

(developer)

## 7. Review

(planner)
