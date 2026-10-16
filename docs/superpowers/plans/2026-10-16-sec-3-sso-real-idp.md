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

### Handoff (2026-10-16, round 1)

Base `origin/main` = `53d05f0024a6`; branch `feat/sec-3-sso-real-idp`, worktree
`/home/shiro/rantai/wt/sec-3`, pushed to `origin` as
`36ce01c548a6`. One commit per task:

```
36ce01c548a6 docs(sso): add the per-provider single sign-on runbook and SEC-3 results
b54f0edeaf91 ci(sso): run the G10 Keycloak sign-in gate on pull requests
4f62a3ab4e1a test(sso): add the G10 Keycloak sign-in gate
40fcf408ab6d feat(sso): add the SEC-3 Keycloak gate override and realm render
fa15bb53b446 docs(plan): SEC-3 spec and plan, single sign-on against real identity providers
```

**Commands run, verbatim, with their results.**

T1 (E1, E2) — config resolves: `docker compose -f docker-compose.yml -f
ops/sso/docker-compose.keycloak.override.yml --profile '*' config --quiet`
→ exit 0. Resolved config inspected: three services (`keycloak`,
`keycloak-realm-init`, `sso-test-runner`, all `profiles=['sso']`, no host
ports), `lakehouse-api`'s `OIDC_*` override correct, volume
`keycloak_realm_import` present. Keycloak image
`quay.io/keycloak/keycloak:26.8.1` pinned by tag **and** digest
`sha256:79c623a22abacf3fe6c00a5fb30f5b7843d717b234033b80995891c06116d244`,
the digest cross-checked two ways: `docker buildx imagetools inspect` and the
Quay API `https://quay.io/api/v1/repository/keycloak/keycloak/tag/?specificTag=26.8.1`
(`manifest_digest`), both matching; 26.8.1 is the newest stable 26.x at the
time. `render_realm.py` unit-checked by importing the module (happy path
renders the client secret and both user passwords; an empty/blank secret or
user-B password exits non-zero).

T2 (E3) — the gate, real run (rule 8), under the heavy lock, project
`sec3sso`, `down -v` via `trap`, ports remapped. `sso-test-runner` output
(`review-logs/sec3-t2b.log`, `GATE_EXIT=0`):

```
[g10] lakehouse-api /health: visible after 0.0s
[g10] keycloak discovery document: visible after 0.2s
[g10] discovery reachable; issuer and endpoints match the API's configuration
[g10] alice signed in through Keycloak; permissions=['catalog:read', 'lineage:read', 'query:read']
[g10] bob (no group) signed in with no mapped role; permissions=[]
[g10] a token minted for audience lakehouse-wrongaud was refused with 401
[g10] a token past its 5s expiry was refused with 401
[g10] PASS
```

A defect in the first run was fixed in this commit: `finished.headers.get_list("Set-Cookie")`
raised `AttributeError` (`requests`' `CaseInsensitiveDict` has no `get_list`);
replaced with `session.cookies.get("lh_session")`.

T3 (E4) — `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))"`
→ 27 jobs, `g10-sso` present, every job in `ci-required.needs`, no extra.
`bash scripts/ci/tests/test_required_gate.sh` → **46 passed, 0 failed**
(includes the structural self-test that a job absent from `ci-required` fails
it). The job's CI-only `.env` validated in isolation:
`docker compose -p sec3ci --env-file <ci .env> -f docker-compose.yml -f
ops/sso/docker-compose.keycloak.override.yml --profile sso config --quiet`
→ exit 0.

T4 (E5) — `python3 ops/lint/check_intra_package_imports.py &&
python3 ops/lint/check_bare_iceberg_count.py &&
python3 ops/lint/check_compose_init_readiness.py` → `LINTS_OK`. T4 check
("every setting named exists in `config.rs`"): every `OIDC_*` token in the
changed docs is one of the 12 in `rust/crates/lakehouse-api/src/config.rs`
(the only doc token not in `config.rs` is the pre-existing
`OIDC_MOCK_HOST_PORT`).

Before each push: `docker run --rm -v "$PWD":/repo -w /repo
zricethezav/gitleaks:v8.30.1 dir /repo --config /repo/.gitleaks.toml
--no-banner --redact` → `no leaks found`. Run with the gitignored `.env`
moved aside, because `dir` does not honour `.gitignore` (it otherwise flags
`.env`).

**Not verified, with the reason.** No Rust command (`cargo fmt/clippy/test`):
no Rust file changed (E6). No `bun` command: no console file changed. No
Dagster pytest: no Dagster file changed. The base `docker-compose.yml` alone
was not re-checked: it is unchanged. The CI job itself is not run here; T3's
check is that the planner's PR runs it. The full workspace block is the
reviewer's and CI's, not this handoff's.

**Premises and decisions.** (1) E3 writes "a new `ops/g*` directory" with no
number; used `ops/g10/` (no `ops/g5`, `ops/g10` free). (2) E4's paths fall
onto the coarse `rust`/`stack` change scopes, so the job also runs on
unrelated rust/ops changes — noted in the job's comment. (3)
`scripts/ci/tests/test_required_gate.sh` fails any `ci.yml` job that is
absent from `ci-required.needs` or lacks a result variable in
`required_gate.sh`, so "the same pattern as the other gate jobs" (E4)
requires the `ci-required` wiring that T3 adds; it is not improvised scope.
(4) `docs/plans/` matches a machine-local `plans/` entry in
`.git/info/exclude`, so `docs/plans/SEC-3-RESULT.md` was staged with
`git add -f`, exactly as the other tracked `docs/plans/*.md`.

**Finding (E5's verify step, reported, not fixed — E6).** The API builds the
JWKS URL as `{OIDC_ISSUER}/.well-known/jwks.json` when `OIDC_JWKS_URL` is
unset (`rust/crates/lakehouse-api/src/state.rs`), and never reads `jwks_uri`
from the provider's discovery document. Entra ID and Google publish keys
elsewhere, so both require `OIDC_JWKS_URL`; reading `jwks_uri` from discovery
would remove that step for every provider. Written up in
`docs/OPERATIONS.md` and `docs/plans/SEC-3-RESULT.md`.

Final state:

```
$ git status --short
(empty)

$ git log --oneline origin/main..HEAD | head
36ce01c5 docs(sso): add the per-provider single sign-on runbook and SEC-3 results
b54f0ede ci(sso): run the G10 Keycloak sign-in gate on pull requests
4f62a3ab test(sso): add the G10 Keycloak sign-in gate
40fcf408 feat(sso): add the SEC-3 Keycloak gate override and realm render
fa15bb53 docs(plan): SEC-3 spec and plan, single sign-on against real identity providers

$ git ls-remote --heads origin feat/sec-3-sso-real-idp
36ce01c548a6	refs/heads/feat/sec-3-sso-real-idp
```

## 7. Review

Round 1, reviewed 2026-10-16 at `03295020` (developer: DeepSeek v4.1 flash,
opencode free). No `BLOCKER`.

Checked and found right: the override runs only under the `sso` profile
and only with `-f`; every secret is `${X:?}` and generated at run time by
the gate script, none committed (gitleaks `git` over `origin/main..HEAD`:
no leaks); Keycloak pinned by tag and digest; no host ports; the gate
(`ops/g10/g10_sso_test.py`) follows CODE-STANDARD §5 and its run log
(`review-logs/sec3-t2b.log`) shows every step and `[g10] PASS`,
`GATE_EXIT=0`: alice signs in with the mapped Analyst role, bob gets no
role, a wrong-audience token and an expired token are refused with 401.
The CI job is wired into `ci-required` as the gate test requires. The
Entra ID and Google runbooks match the vendors' current behaviour.

Fixed by the planner (docs only): the two vendor citations were bare
paths; they are now full links. The handoff pasted full 40-character
hashes twice (cut to 12 here; rule in `dev-rules.md`).

Filed from the developer's finding and the Entra limit: `SEC-38` (read
`jwks_uri` from the provider's discovery document so `OIDC_JWKS_URL` is
not needed) and `SEC-39` (an Entra user whose groups overflow the claim
gets no mapped role, silently; at least log and show it).

`SEC-3` stays In Progress after merge: AC2 and AC3 are the product
owner's sign-in tests with Entra ID and Google, recorded in
`docs/plans/SEC-3-RESULT.md`.
