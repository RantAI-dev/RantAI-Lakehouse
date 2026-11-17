# `SRC-12` result: credentials done properly

Proven on pull request #144, CI run 38719076389 (commit `daaec3c9`), on
2026-10-15, by three gates on GitHub-hosted `ubuntu-latest` runners. One run
each. Design: ADR 0002 addenda 5, 6 and 7.

## At rest without a secret manager (part A) — job "Rust · SRC-12 credentials at rest"

| Step | Result |
| --- | --- |
| A credential typed through the API | Stored sealed (AES-256-GCM, 60 bytes, mode 600); the plain value is not in the file |
| Connection test and load through the sealed file | Pass; the orchestrator loaded 3 rows |
| The key change command, then the old key removed | Every stored credential re-sealed (different bytes, still not the plain value); test passes; a second load succeeds under the new key alone |
| Compose without `CONNECTOR_SECRETS_KEY` | Refuses to start and names the variable |

## External secret managers (part B) — job "Rust · SRC-12 secret managers (Vault, AWS)"

The same steps ran against Vault's development server and against an
AWS-compatible stand-in (`moto`):

| Step | Vault | AWS |
| --- | --- | --- |
| A reference to nothing, or to another manager's scheme | Refused, fixed text | Refused, fixed text |
| A valid reference outside `SECRET_MANAGER_ALLOWED_PREFIXES`; one with `..` | Refused by the prefix rule; by the grammar | Same |
| Save a reference: read once, connection tested | Pass | Pass |
| The credentials volume afterwards | No file for the connector, no file holding the value | Same |
| Test and load through the reference | 3 rows | 3 rows |
| The secret changed in the manager | Used by the very next test and run; nothing kept | Same |

Google Secret Manager and Azure Key Vault are **not run against the real
services**: their clients are proven by tests against recorded responses of
their documented REST interfaces only.

## OAuth sign-in for REST connectors (part C) — job "Rust · SRC-12 OAuth sign-in"

Against a provider the gate runs itself (5-second access tokens, rotating
refresh tokens, a revocation endpoint):

| Step | Result |
| --- | --- |
| The app's client secret saved before any sign-in | Stored sealed, marked unverified |
| Sign in with PKCE S256 | The refresh token stored sealed; no access token stored |
| First load | 5 rows; 2 access tokens requested during the run |
| Second load after the access token expired | Succeeded on a refresh-token grant; the rotated refresh token was sealed in place of the old one |
| Connection test | Performs the refresh and keeps the rotated token (passed twice) |
| A wrong `state`, a replayed `state` | Refused with the fixed page; the provider was never called |
| A refresh token the provider no longer honours | The run fails once as `sign_in_refused` (not retried); signing in again restores it |
| Sign out; delete the connector | Each revoked the stored token at the provider and removed the files |

## Not measured

- Google Secret Manager and Azure Key Vault against the real services.
- A real OAuth provider; the gate's provider follows the RFC, not a vendor's
  quirks (only `client_secret_post` is supported).
- The time a read from a manager adds to a run on a real network.
- More than one run of each gate.
