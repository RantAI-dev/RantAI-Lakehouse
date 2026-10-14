# Credentials done properly

| | |
| --- | --- |
| Module | Data (Sources) |
| Backlog | `SRC-12` |
| Kind | Feature |
| Spec | `docs/core/specs/src-12.md`. This page follows it row by row; anything the spec does not say is under "Beyond the spec". One *(proposed)* number, decision 4 |
| Status | Planned. Decisions signed 2026-10-14 |
| Plan | Not written; it waits for the signatures below. Size L: the plan builds it in three parts (decision 13) |

## Problem

From the spec: credentials are plain files, there is no secret manager, OAuth
sign-in is unsupported, and rotation is manual. Checked in the code on
2026-10-14 (`main` at `112cd8b` with `SRC-10`), row by row. ADR 0002 and its
four addenda describe the design this builds on.

- **External secret managers.** None. A credential is one of three kinds: an
  environment variable the operator sets, a file the operator drops, or a
  value the user types, which the API writes as a file. The code that reads
  a credential is built to take another kind (ADR 0002 sketches a `vault:`
  one), and two lists of allowed kinds, one in the API and one in the
  orchestrator, must both admit it.
- **At rest without a secret manager.** The files are plain text on a
  volume shared by the API (which writes them) and the orchestrator (which
  reads them for loads). ADR 0002 addendum 4 names encryption as the
  follow-up. Nothing in compose refuses to start for a missing secret.
- **OAuth sign-in.** None. A REST connector can use OAuth 2.0 client
  credentials: loads fetch a token once per run and never refresh it, and
  the connection test says it cannot perform the exchange. There is no
  sign-in flow and no callback for connectors. Google Sheets, the one type
  in the picker that needs a user's sign-in, is listed as not supported.
- **Rotation.** A typed credential can already be replaced without
  recreating the connector, after a test against the source, and `SEC-14`
  makes re-pointing a connector require new credentials. What is manual: an
  operator's variable or file is changed by the operator and read again only
  after a restart. A run already started keeps the value it read.

## What the user can do when this is done

| Spec row | Spec target | What they can do |
| --- | --- | --- |
| External secret managers | HashiCorp Vault, AWS Secrets Manager, Google Secret Manager, Azure Key Vault (Airbyte's four) | 1. Point a connector's credential at a secret in the installation's secret manager instead of typing it; the product keeps no copy. |
| At rest without a secret manager | Encrypted (AES-256-GCM, proposed) with a key from the environment; compose refuses to start without it | 2. Keep typing credentials in the console, knowing that what is stored cannot be read without the installation's key. 3. (operator) Be stopped at start-up when the key is missing. |
| OAuth sign-in | OAuth 2.0 authorisation-code flow with token refresh for every connector whose source offers it | 4. Connect a REST source by signing in to it once in a browser window; loads keep working after the first token expires. |
| Rotation | Rotate a credential without recreating the connector; old value unusable at once | 5. Replace any credential, of any kind, from the connector's page; the next test and the next run use the new one, and the product keeps no copy of the old one. |

## Not included

From the spec: anything not in the Target table. In particular:

- Writing new secrets into a secret manager from the console. The console
  references secrets that exist there (decision 2).
- A Google Sheets loader. Sheets stays not supported; OAuth is one of the
  two things it lacks.
- Change-capture (CDC) credentials: Debezium reads them from the
  environment, and that stays until `SRC-4`.
- Changing the password at the source. The product cannot make an old
  password stop working there; the customer does that.

## Asking the assistant

From the spec: not part of the dashboards assistant (`AI-16`). The
assistant's tool that creates connectors keeps refusing credential values.

## Decisions

Signed by the product owner on 2026-10-14 ("setuju semua", on the planner's recommendation). Rows 1 to 4, 6 and 9 settle how a spec row is met; the others are
beyond the spec. The planner's recommendation is the Default.

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Spec "the four secret managers": who reads them. | The API (for tests and listing) and the orchestrator (for loads) each read the secret from the manager when they need it, as both read files today, and keep no copy: no cache, read at every test and at the start of every run. One secret manager per installation, set by the operator in the environment (its address and how the product signs in to it: a token or role for Vault, the instance's role for AWS, a service account for Google, a managed identity or app registration for Azure). It is not set in the console, because the sign-in to the manager is itself a secret. || 2026-10-14 |
| 2 | Spec "the four": what a user does in the console. | When a secret manager is set, the credential form offers "Reference a secret" (the secret's path or name, and the key inside it for Vault and AWS). The product checks the reference by reading it once and testing the connection, then stores only the reference. Typing a value stays possible unless the operator turns it off. || 2026-10-14 |
| 3 | Spec "the four": how each is proven. | Vault and AWS Secrets Manager by gates against local stand-ins (Vault's own development server; an AWS-compatible local service). Google and Azure have no local stand-in: they are proven by tests against their documented interfaces, and the result document says they were *not run against the real service*. || 2026-10-14 |
| 4 | Spec *(proposed)*: AES-256-GCM with a key from the environment; compose refuses to start without it. | Accept. One key per installation, 32 random bytes, given to the API and the orchestrator; `${…:?}` in compose so neither starts without it, and no default in `.env.example` (rule 5). Every stored value gets its own random nonce. Files written before this are encrypted at the first start with the key, then the plain copies are removed. || 2026-10-14 |
| 5 | Beyond the spec: changing the encryption key. | A command the operator runs with the old and the new key: every stored credential is re-encrypted, then the old key is no longer accepted. Until the command has run, the old key keeps working. || 2026-10-14 |
| 6 | Spec "OAuth 2.0 authorisation-code flow … for every connector whose source offers it". | Built for the REST connector, the one supported type whose sources offer it. The user enters the source's sign-in address, token address, scopes and the app's id and secret, presses "Sign in", approves in a browser window, and the product stores the refresh token as the credential (encrypted, decision 4). PKCE is used. The callback address is the product's own and is shown to copy into the source's app settings. || 2026-10-14 |
| 7 | Spec "with token refresh". | Loads refresh the access token before it expires and once more if the source refuses it; a refresh that fails fails the run with "sign in again", and the connector page offers the button. The client-credentials kind gets the same refresh. The connection test performs the exchange, so OAuth connectors can be tested. || 2026-10-14 |
| 8 | Beyond the spec: signing out. | Removing the credential, or deleting the connector, revokes the refresh token at the source when the source offers revocation, and says so when it does not. || 2026-10-14 |
| 9 | Spec "old value unusable at once": what the product can promise. | Once a credential is replaced: the next test and every run that starts afterwards use the new value; a step that is already running finishes with the value it read, because that cannot be taken back; the product keeps no copy of the old value, including after an interrupted change, and checks that at start-up. A reference to a secret manager is read fresh each time, so changing the secret there takes effect at the next run. || 2026-10-14 |
| 10 | Beyond the spec: an operator's environment variable. | Unchanged: still read at start-up, and changing one still needs a restart. The connector page says which kind each credential is and, for this kind, that a restart is needed. || 2026-10-14 |
| 11 | Beyond the spec: who does what. | `connector:manage` on a connector of one's own tenant references a secret, signs in, replaces or removes a credential, as today. The secret manager and the key are the operator's, in the environment. Every change, sign-in and sign-out is written to the audit trail without the value or the reference's secret parts. || 2026-10-14 |
| 12 | Beyond the spec: errors. | Fixed texts: the manager cannot be reached, the reference does not exist, the product may not read it, the key is wrong, the sign-in was refused or expired. Never a manager's or a provider's own text (principle 4). || 2026-10-14 |
| 13 | Beyond the spec: order of work. Size L. | Three parts on the phase branch, each with its own gate: A, encryption at rest, the key change and rotation (decisions 4, 5, 9, 10); B, the secret managers (1 to 3); C, OAuth (6 to 8). || 2026-10-14 |

## Limits to tell a customer

- One secret manager per installation.
- Google Secret Manager and Azure Key Vault are tested against their
  documented interfaces, not against the services themselves.
- A run already loading when a credential changes finishes with the old
  one.
- An operator's environment variable is read at start-up only.
- Changing a password at the source is the customer's step; the product
  only stops using the old one.

## Acceptance checklist

Run on a real deployment by the product owner. Mark each Pass, Fail, or Not
run with the reason. Rows 1 to 4 are the spec's checklist, in its order.

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | Spec `SRC-12-AC1`: with Vault set, reference a connector's password in Vault; test and run it; look in the credentials volume | The test and the run succeed; the volume holds no copy | |
| 2 | Spec `SRC-12-AC2`: with no secret manager, type a credential; read its stored file without the key | The file cannot be read as the credential | |
| 3 | Spec `SRC-12-AC3`: connect a REST source with "Sign in"; run it again after its access token has expired | One sign-in; both runs succeed | |
| 4 | Spec `SRC-12-AC4`: as a user without `connector:manage`, reference, sign in, replace; and use a reference that does not exist | Refused; the failure shows an honest message with no manager or provider text | |
| 5 | (operator) Start without the key | Compose refuses to start and names the variable | |
| 6 | Replace a typed credential, then run | The run uses the new one; no copy of the old one is left | |
| 7 | (operator) Change the key with the command; start again | Every connector still tests and runs | |
| 8 | Remove an OAuth credential | The source's token is revoked, or the page says the source cannot revoke it | |

**Accepted by:** __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` item moved to Released; follow-ups added
- [ ] `CHANGELOG.md` entry a customer can read
