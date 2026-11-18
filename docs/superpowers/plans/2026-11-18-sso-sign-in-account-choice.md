# SSO: the sign-in asks which account, not the browser's last one

Bug fix; one PR. Base: `origin/main` at the merge of #567
(`cb855d5b923d978f14371c3e61d88f1d8be1ef8b`), verified 2026-11-18.
Reported by the product owner, 2026-11-18, against the staging deployment
that #567 set up: "when I log in with SSO as anyone, the login succeeds, but
after logging out and logging in with SSO again it logs me in automatically
as the account I logged in with the first time."

## The defect

The second single sign-in on the same browser never showed the provider's
login form: the provider answered the authorization request from the session
it already held for that browser, so the person was signed in as whoever had
used that machine last.

Measured on the staging VM, 2026-11-18, with one cookie jar (what one
browser is):

```
1st sign-in (fresh jar):       signed in as alice@staging.rantai.invalid
2nd sign-in (same jar, bob):   NO LOGIN FORM (silent re-auth)

cookies the browser now holds:
   192.168.18.143  KEYCLOAK_IDENTITY   path=/realms/lakehouse/
   192.168.18.143  KEYCLOAK_SESSION    path=/realms/lakehouse/
   192.168.18.143  AUTH_SESSION_ID     path=/realms/lakehouse/
   192.168.18.143  KC_AUTH_SESSION_HASH path=/realms/lakehouse/
   192.168.18.143  KC_RESTART          path=/realms/lakehouse/
   192.168.18.143  lh_oidc_flow        path=/api/auth/oidc
   192.168.18.143  lh_session          path=/
```

The provider's cookies are the cause, and the deployment never ends them:
"Sign out" revokes this deployment's own session and clears `lh_session`
(`routes/auth.rs:414`) and nothing else. The provider's session is the
provider's, and the authorize request carried no `prompt`, so the provider
answered it from that session (`OpenID Connect Core` §3.1.2.1: without
`prompt`, the provider may re-authenticate silently).

This is standard OIDC behaviour, not a provider bug — but on a shared
machine it signs the next person in as the previous one, and it is why
"sign out, then sign in again" could not change the account.

## Anchors at the base above

| Site | Observation |
| --- | --- |
| `rust/crates/lakehouse-api/src/routes/auth.rs:939` | `oidc_start` builds the authorize request: `response_type`, `client_id`, `redirect_uri`, `scope`, `state`, `nonce`, PKCE. No `prompt`. |
| `rust/crates/lakehouse-api/src/routes/auth.rs:414` | `logout` revokes the session and clears the cookie; the provider's session is untouched. |
| `rust/crates/lakehouse-api/src/config.rs:630` | The browser flow's addresses are read from the environment only, never from a request. The `prompt` belongs to the same set. |
| `rust/crates/lakehouse-api/src/runtime_settings.rs:1319` | `resolve_sso` — one merge rule for every SSO setting; the `prompt` is a server-side setting (the console's SSO form does not carry it). |
| `rust/crates/lakehouse-api/src/routes/settings.rs:2116` | `GET /api/settings/sso` — the console's view of the connection. The `prompt` is not one of its fields and stays that way. |

## Decisions

1. **Send `prompt=login` by default.** The provider shows its login form, so
   the account is the person's choice on every sign-in. It is the standard
   OIDC parameter, it needs no provider-side configuration, and it costs
   nothing but a click on a shared machine.
2. **`OIDC_PROMPT` names it, and `off` opts out.** `OIDC_PROMPT=off` sends
   nothing and the provider reuses its own session — the comfortable
   behaviour on a private machine. Anything else is forwarded verbatim
   (`login`, `consent`, `select_account`, and the OIDC standard's own
   `none`, which means something else and is not this deployment's to
   reinterpret).

   **An empty value is the default, not an opt-out.** The first attempt at
   this change made "present but empty" mean "send nothing", with the
   default applied when the variable was absent — and that is wrong in a
   compose deployment: `docker-compose.yml` passes every `OIDC_*` variable
   as `${OIDC_PROMPT:-}`, so the variable is **always** present in the
   process, empty when the operator said nothing. The default would never
   have applied, and the whole setting would have been a no-op. `Config`
   therefore resolves an empty value to the default (`login`), and `off` is
   the one spelling for "send nothing". This is the same shape as every
   other `OIDC_*` setting in this file: `truthy()` treats empty and absent
   alike, and the deployment says what it wants by naming a value.
3. **The `prompt` is the deployment's own setting**, read only from the
   environment: a request cannot change how the provider is asked to
   authenticate, exactly like `OIDC_AUTHORIZE_URL` and `OIDC_REDIRECT_URI`.
4. **Not an RP-initiated logout.** Ending the provider's session at "Sign
   out" is the other way to fix this (redirect to the provider's
   `end_session_endpoint`), but it needs the discovery document to carry
   that endpoint, a post-logout redirect URI registered with the provider,
   and a frontend that follows a redirect instead of clearing state. That is
   a bigger change with more ways to fail halfway; `prompt=login` fixes the
   reported behaviour in one parameter. Recorded as the follow-up if the
   product wants "sign out ends the provider's session too".

## What changed

| File | Change |
| --- | --- |
| `rust/crates/lakehouse-api/src/config.rs` | `oidc_prompt` + `oidc_prompt_from_env`, `DEFAULT_OIDC_PROMPT`, the `Debug` field, and the tests. |
| `rust/crates/lakehouse-api/src/runtime_settings.rs` | `EffectiveSso::prompt` + its source; `resolve_sso` and `draft_sso_settings` fill them. |
| `rust/crates/lakehouse-api/src/routes/auth.rs` | `oidc_start` sends the `prompt`; the tests assert the default, another value, and "nothing sent". |
| `rust/crates/lakehouse-api/src/routes/settings.rs` | The test's `EffectiveSso` literal carries the new fields. |
| `docker-compose.yml` | The `lakehouse-api` service passes `OIDC_PROMPT` through, like every other `OIDC_*` setting — without it the value in `.env` never reaches the API process. |
| `.env.example`, `docs/CONFIGURATION.md`, `docs/OPERATIONS.md` | The setting is documented, including what "sign out, then sign in again" does. |
| `.github/workflows/deploy-staging.yml` | The staging connection writes `OIDC_PROMPT=login` explicitly. |
| `CHANGELOG.md` | The entry. |

## Verification

- The defect, measured on the staging VM before the change: the second
  sign-in in one cookie jar returned **no login form** (the output above).
- The change, measured on the staging VM after it: the second sign-in in one
  cookie jar shows the provider's login form, and the sign-in lands on the
  account that was typed, not the previous one.
- The unit tests: `config.rs` (absent / set / present-and-empty /
  whitespace), `runtime_settings.rs` (the resolver's three cases),
  `routes/auth.rs` (the authorize URL carries `prompt=login` by default,
  `prompt=consent` when asked, and no `prompt` at all when the operator
  asked for none).

**Not verified by hand:** the login page was not clicked through in a
browser (no browser attached to the session); the flow was driven by a
script that follows the same redirects a browser follows, with one cookie
jar, which is what the defect is about.

## Limits, honestly

- With `prompt=login`, every sign-in asks the provider to authenticate
  again. On a provider behind MFA that is an MFA prompt each time. The
  operator can turn it off (`OIDC_PROMPT=off`), at the cost of the reported
  behaviour coming back.
- "Sign out" still does not end the provider's session: it ends this
  deployment's session only. The next sign-in asks the provider to
  authenticate again, but the provider's session cookie is still in the
  browser.
