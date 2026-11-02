# OPS-15: a Keycloak on staging, so QA can test single sign-on

Backlog `OPS-15` (Task, Operations, P2). Base: `main` at `9c066426`. Planner: Claude Opus.
Developer: another agent (`AGENTS.md`).

## 0. What is wrong, with evidence

QA cannot sign in with SSO on staging. There is no identity provider there, and the console
there does not show the SSO button.

- **Staging has no OIDC settings.** `deploy-staging.yml` builds `.env` from `.env.example` plus
  `.env.staging` plus repository secrets (`:42-160`). `.env.example:284-313` ships every
  `OIDC_*` empty, and neither `.env.staging` nor the workflow sets one. The API's
  `OIDC_*` come from `${OIDC_*:-}` (`docker-compose.yml:413-435`). With `OIDC_ISSUER` and
  `OIDC_CLIENT_ID` unset, OIDC is off and `oidc/*` answers 503 (`docs/OPERATIONS.md:1261-1264`).
- **The console on staging hides the SSO button.** `login-page.tsx:42` reads the build-time flag
  `NEXT_PUBLIC_SSO_ENABLED === "true"`. `Dockerfile.frontend` takes it as a build arg. Staging
  builds the console with `docker build -f Dockerfile.frontend -t lakehouse-frontend:staging .`
  (`deploy-staging.yml:204-208`) and no `--build-arg`, so the flag is empty.
- **Staging starts three profiles, not every profile:** `seaweedfs`, `dagster` and `trino`,
  through `scripts/compose.sh` (`:162-179`).
- **A Keycloak already exists, but only for the gate.** `ops/sso/docker-compose.keycloak.override.yml`
  (profile `sso`) runs `quay.io/keycloak/keycloak:26.8.1`, pinned by digest (`:70`), in
  `start-dev`. Its realm (`lakehouse`) is rendered at run time from `ops/sso/realm.template.json`
  by `ops/sso/render_realm.py` (the one-shot `keycloak-realm-init`, `:45`). That keeps every
  secret out of the repository. The file is fixed to the gate in three ways:
  - **the issuer is in-network only**: `KC_HOSTNAME: http://keycloak:8080` (`:82`), no port is
    published, and the redirect URI is `http://lakehouse-api:8080/...`;
  - **the API's values are the gate's**: `OIDC_ROLE_MAP: analysts=Analyst` and
    `OIDC_CLOCK_SKEW_SECONDS: "0"`, in a `lakehouse-api` block (`:98-124`);
  - **the realm is the gate's**: one group, users `alice` and `bob`, and two negative-case
    clients. `render_realm.py` hardcodes that template, that output and those two users
    (`:38-50`).

  CI runs it as job `g10-sso` "Rust · G10 Keycloak SSO sign-in" (`ci.yml:1115-1183`), with
  `ops/g10/g10_sso_test.py`.
- **The five QA roles exist.** `0002_seed_identity.sql:40-46` seeds Analyst, Approver, Data
  Engineer, Platform Admin and Dashboard Viewer. A mapped role is added to the local roles for
  that login only, and is never persisted (`lakehouse-auth/src/oidc.rs:930-960`).
  `OIDC_ROLE_MAP` is `group=Role Name,…` (`config.rs:1359-1378`).

## 1. Decisions

- **D1. A staging-only override file, next to the gate's, with its own profile.** The file is
  `ops/sso/docker-compose.keycloak.staging.yml`, with profile `sso-staging`. It holds
  `keycloak-qa-realm-init` and `keycloak-qa`, and nothing for `lakehouse-api`: the base file's
  `${OIDC_*:-}` already pass the values from `.env` (D5). The gate's file and the gate are not
  changed. *(planner default)*
- **D2. One image pin (rule 4).** `keycloak-qa` uses
  `extends: { file: docker-compose.keycloak.override.yml, service: keycloak }`, and
  `keycloak-qa-realm-init` extends `keycloak-realm-init` the same way. Each overrides
  `profiles`, `environment`, `volumes`, `ports` and `depends_on`.
  - Compose's `extends` does not carry `depends_on` in every version. The developer proves the
    merged result with `docker compose … config` **and** a real `up` (rule 8).
  - If `extends` cannot express it, fall back to a copy of the pin plus a unit test that asserts
    both files name the same `image:` string. Say which one in the handoff.
- **D3. The QA realm (`qa`) lives in its own template, `ops/sso/realm.qa.template.json`.**
  - Groups, with `full.path=false` on the client's group mapper, as the gate's realm has
    (`OPERATIONS.md:1324-1328`): `platform-admins`, `data-engineers`, `analysts`,
    `dashboard-viewers`, `approvers`.
  - Users, each in exactly one group: `qa-admin`, `qa-engineer`, `qa-analyst`, `qa-viewer` and
    `qa-approver`. Add `qa-nogroup`, in no group, so QA can check that an unmapped user gets no
    role. Emails are `<user>@qa.invalid` (rule 12).
  - One confidential client, `lakehouse`, with PKCE S256 and the standard flow only. Its
    `redirectUris` are rendered from `OIDC_REDIRECT_URI` (D6).
  - None of the gate's negative-case clients.
  - Two approvers exist (`qa-admin` holds `*:*`, and `qa-approver`), so `SEC-57`'s "someone
    else approves" can be tested on staging.
- **D4. `render_realm.py` is generalized, not copied (rule 4).** It reads, from the environment
  with today's values as defaults:
  - `REALM_TEMPLATE` and `REALM_OUTPUT`;
  - `REALM_USER_PASSWORDS`, as `user=ENV_VAR,…`; the default is `alice=SSO_USER_A_PASSWORD,
    bob=SSO_USER_B_PASSWORD`;
  - `REALM_REDIRECT_URI`, optional: when set, it replaces the confidential client's
    `redirectUris`. When unset, the template's value stays.

  The gate's output must stay byte for byte the same. A test runs the renderer with the gate's
  environment and compares the output with today's, so the gate's rendering cannot drift. The
  same fail-closed rules hold for both realms: a missing value, an empty secret, or a missing
  client or user exits non-zero.
- **D5. No default credentials (rule 5), and every secret is a repository secret.** Required in
  the override with `${X:?}`, and checked by name in the deploy step, which fails closed as the
  other `STAGING_*` secrets already do (`deploy-staging.yml:105-122`):
  - `STAGING_KEYCLOAK_ADMIN_PASSWORD` → `KEYCLOAK_ADMIN_PASSWORD` (the bootstrap admin);
  - `STAGING_OIDC_CLIENT_SECRET` → `OIDC_CLIENT_SECRET` (the realm's client and the API);
  - `STAGING_SSO_QA_PASSWORD` → `SSO_QA_PASSWORD`, one password for the six QA users.

  *(planner default: one shared QA password. QA is one team and the realm is staging-only. The
  owner can ask for one per user, which would be six secrets.)* The owner hands the password to
  QA outside the repository.
- **D6. Addresses are repository variables, never in the repository (rule 12).**
  - `vars.STAGING_KEYCLOAK_URL`: the address a QA browser opens Keycloak at, for example
    `http://<staging-host>:<port>`.
  - `vars.STAGING_CONSOLE_URL`: the console as QA opens it.
  - `vars.STAGING_KEYCLOAK_HOST_PORT`: the host port `keycloak-qa` publishes, as
    `ports: ["${KEYCLOAK_QA_HOST_PORT:?}:8080"]`. The service publishes its own port, as the
    console and the API do today. This is not `network_mode: service:`.

  The deploy step writes into `.env`:
  - `OIDC_ISSUER=${STAGING_KEYCLOAK_URL}/realms/qa`. `KC_HOSTNAME` is the same browser URL, so
    the token's `iss` equals it byte for byte;
  - `OIDC_AUTHORIZE_URL` on the browser URL;
  - `OIDC_TOKEN_URL` and `OIDC_JWKS_URL` on the **in-network** address
    `http://keycloak-qa:8080/realms/qa/protocol/openid-connect/{token,certs}`. Keycloak is
    started with `KC_HOSTNAME_BACKCHANNEL_DYNAMIC=true`, so a back-channel call on the internal
    address is accepted while `iss` stays the browser URL. The developer verifies this against
    Keycloak 26.8 (rule 1): one real sign-in, and the decoded `iss` quoted in the handoff;
  - `OIDC_REDIRECT_URI=${STAGING_CONSOLE_URL}/api/auth/oidc/callback`. The callback must be on
    the console's origin, not the API's port. The browser starts the flow at the console's
    `/api/auth/oidc/start` (`login-page.tsx:68`), and the flow cookie is `Path=/api/auth/oidc`
    on that origin (`routes/auth.rs:682-684`), so the callback must arrive on the same origin.
    `OPERATIONS.md:1277` says `<api-origin>`; on staging the console proxies `/api`, so the
    browser's API origin is the console's. The page gets one sentence saying so;
  - `OIDC_CLIENT_ID=lakehouse`, `OIDC_PROVIDER_NAME=keycloak-qa`,
    `OIDC_JIT_PROVISIONING=true`, `OIDC_GROUPS_CLAIM=groups`;
  - `OIDC_ROLE_MAP=platform-admins=Platform Admin,data-engineers=Data Engineer,analysts=Analyst,dashboard-viewers=Dashboard Viewer,approvers=Approver`.

  The non-secret lines (client id, role map, claim, provider name) go in `.env.staging`, which
  is committed. The address lines are built from the variables in the workflow.
- **D7. `start-dev`, ephemeral, re-imported on recreate.** This is the gate's mode
  (`override.yml:7-13`): plain HTTP and an in-container H2. The realm is code, so a recreated
  container imports it again. The Lakehouse side (JIT users, sessions) is in Postgres and
  survives. QA loses nothing that matters, except Keycloak-side sessions on a redeploy.
  *(planner default; a production-mode Keycloak with a database and TLS is out of scope.)*
- **D8. The console on staging shows the SSO button.** `deploy-staging.yml:207` passes
  `--build-arg NEXT_PUBLIC_SSO_ENABLED=true`.
- **D9. Staging proves it after every deploy.** A new step after the API health check:
  - `curl -sf "$STAGING_KEYCLOAK_URL/realms/qa/.well-known/openid-configuration"`, with
    `issuer` equal to `OIDC_ISSUER`;
  - `curl -s -o /dev/null -w '%{http_code} %{redirect_url}' http://localhost:8080/api/auth/oidc/start`
    answers a redirect whose address starts with `OIDC_AUTHORIZE_URL`.

  A failure fails the deploy, by name.
- **D10. The admin console is reachable on the published port**, behind the generated
  bootstrap admin password (D5). On a staging VM that is acceptable. The page says so.
  *(planner default)*

## 2. Anchors (verified at `9c066426`)

- `ops/sso/docker-compose.keycloak.override.yml`:
  - header `:1-31`, volumes `:32-37`;
  - `keycloak-realm-init` `:45-60`;
  - `keycloak` `:68-96` (image `:70`, command `:72`, `KC_HOSTNAME` `:82`);
  - `lakehouse-api` `:98-124`, `sso-test-runner` `:126-156`.
- `ops/sso/render_realm.py:38-50` (the hardcoded template, output, client and users),
  `:53-92`.
- `ops/sso/realm.template.json` (groups `:14`, client `redirectUris` `:32`, users `:96`).
- `.github/workflows/deploy-staging.yml`:
  - `.env` setup `:42-160` (the required-secret loop `:105-122`, the `env:` block `:141-160`);
  - compose up `:162-179`, health `:181-198`;
  - frontend build `:204-208`, summary `:228-240`.
- `.github/workflows/ci.yml:1115-1183` (`g10-sso`), which is unchanged and must stay green.
- `.env.example:284-313`, `:339-341`; `.env.staging` (the committed staging overrides).
- `docker-compose.yml:413-435` (the API's `OIDC_*`).
- `src/features/auth/login-page.tsx:42`, `:68`; `Dockerfile.frontend` (`NEXT_PUBLIC_SSO_ENABLED`
  build arg).
- `rust/crates/lakehouse-api/src/routes/auth.rs:79-81`, `:682-684` (cookies);
  `config.rs:1359-1378` (`parse_role_map`).
- `rust/crates/lakehouse-auth/src/oidc.rs:930-960` (mapped roles are additive, per login).
- `rust/migrations/0002_seed_identity.sql:40-46` (the roles).
- `docs/OPERATIONS.md:1253-1330` (SSO and the Keycloak section).

## 3. Tasks (one commit each, in order)

No Rust changes, so no cargo command runs.

- **T1. The renderer and the QA realm.**
  - Generalize `render_realm.py` (D4) and add `realm.qa.template.json` (D3).
  - Add `ops/sso/test_render_realm.py`, with no network and no container, writing to a temp
    dir. Test names are full sentences:
    - the gate's environment renders exactly what it rendered before;
    - the QA realm renders its six users, its client secret and the given redirect URI;
    - a missing password variable, an empty secret, or an unknown user in
      `REALM_USER_PASSWORDS` exits non-zero;
    - no output file is written when it refuses.
  - *Accept:* `python3 -m pytest ops/sso -q`.
- **T2. The staging override.**
  - Add `docker-compose.keycloak.staging.yml` (D1, D2, D6, D7), with a why-header in the style
    of the gate's (`OPS-15`, the decisions, no secret in the file).
  - Add the non-secret `OIDC_*` lines to `.env.staging` (D6), with a comment.
  - Prove it from a clean project (rule 8), with throwaway values in a local `.env`:
    `docker compose -p ops15 -f docker-compose.yml -f ops/sso/docker-compose.keycloak.staging.yml --profile sso-staging up -d lakehouse-api keycloak-qa`.
    Then:
    - open the discovery document on the published port;
    - sign in once as `qa-analyst` through the real login form (a browser, or the G10 script's
      form driver run by hand);
    - check that `/api/auth/me` carries `query:read`;
    - repeat as `qa-nogroup` and check that it carries no mapped role.

    Quote the commands and the `iss`. Then `down -v`.
  - Also: `docker compose --profile '*' config --quiet` with both override files, and the G10
    job's own commands (`ci.yml:1165-1183`) run once locally, to show the gate is unchanged.
  - *Accept:* the above, plus `python3 -m pytest ops/sso -q`.
- **T3. The staging deploy.**
  - `deploy-staging.yml`:
    - the three secrets in the required loop and the `env:` block (D5);
    - the `.env` address lines from the three variables (D6), each added with the
      "delete, then one line" pattern the file already uses;
    - the override and the profile in the `compose.sh` call;
    - the build arg (D8), the proof step (D9), and a Keycloak line in the summary that uses the
      variable, not a literal.
  - *Accept:* `actionlint .github/workflows/deploy-staging.yml` if it is installed, otherwise
    write *not verified* with the reason. The real proof is the first staging deploy after
    merge. The planner watches it and records the run id in the review. Before merge, the
    owner must create the three secrets and three variables, or the deploy fails closed by
    name (D5).
- **T4. The docs.**
  - `docs/OPERATIONS.md`, Keycloak section: a "Staging (QA)" table beside "Local gate value"
    and "Real Keycloak", with placeholders only, the console-origin callback note (D6), D7 and
    D10.
  - A QA note: the six users, their groups, the role each maps to, and that the password comes
    from the owner. No host and no password.
  - *Accept:* `git diff --stat` touches only `docs/`.

Full block before handoff:
- `python3 -m pytest ops/sso -q`;
- `docker compose --profile '*' config --quiet`, plus the T2 real `up` from a clean project;
- `git status`, with nothing rendered (`lakehouse-realm.json`, `.env`) staged.

## 4. Out of scope

- A production-mode Keycloak (database, TLS, a real domain) and a Keycloak in the release
  bundle.
- Entra ID or Google on staging (`SEC-3` D2 runs those by hand).
- Changing the G10 gate, its realm or its override, beyond the renderer's backward-compatible
  options (D4).
- Making mapped roles persistent or authoritative (`oidc.rs:960-966`, a separate follow-up).
- A runtime replacement for `NEXT_PUBLIC_SSO_ENABLED` (`.env.example:339-340` names the
  proposal).

## 5. Handoff (developer appends below)

## 6. Review (planner appends below)

---

### Proposed BACKLOG.md row

| `OPS-15` | Task | QA cannot test single sign-on on staging: no identity provider runs there, staging sets no `OIDC_*`, and its console is built without `NEXT_PUBLIC_SSO_ENABLED`. Add a staging-only Keycloak (`ops/sso/docker-compose.keycloak.staging.yml`, profile `sso-staging`, the G10 gate's pinned image through `extends`) with a QA realm whose groups map to Platform Admin, Data Engineer, Analyst, Dashboard Viewer and Approver (`OIDC_ROLE_MAP`), one user per group plus one with none. Generalize `ops/sso/render_realm.py` rather than copy it. Secrets come from repository secrets with `${X:?}`, and addresses from repository variables. Each deploy proves discovery and the start redirect | Operations | P2 | Todo |  | owner request 2026-11-02 (QA-1); `ci.yml` `g10-sso` |
